use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer, images::Image};
use image::{GenericImageView, ImageError, ImageReader};
use memmap2::Mmap;
use rayon::{
    iter::{IndexedParallelIterator, ParallelIterator},
    slice::ParallelSliceMut,
};
use std::{
    env,
    fs::{File, OpenOptions},
    io::{Cursor, Write},
};

enum Mode {
    Fit,
    Fill,
    Normal,
}
impl Mode {
    fn parse(mode: &str) -> Option<Self> {
        match mode {
            "fit" => Some(Self::Fit),
            "fill" => Some(Self::Fill),
            "norm" => Some(Self::Normal),
            _ => None,
        }
    }
}

fn dies(context: &str, err: impl std::fmt::Display) -> ! {
    // dies of cringe
    eprintln!("{context}: {err}");
    std::process::exit(1)
}

fn get_dimensions(in_dims: (u32, u32), out_dims: (u32, u32), mode: &Mode) -> (usize, usize) {
    let w_ratio = out_dims.0 as f64 / in_dims.0 as f64;
    let h_ratio = out_dims.1 as f64 / in_dims.1 as f64;

    let scale = match mode {
        Mode::Fill => w_ratio.max(h_ratio),
        Mode::Fit => w_ratio.min(h_ratio),
        _ => unreachable!(),
    };

    let scaled_w = (in_dims.0 as f64 * scale).round() as usize;
    let scaled_h = (in_dims.1 as f64 * scale).round() as usize;

    // if .round() is off by a pixel, usize can underflow elsewhere. so we're clamping here
    match mode {
        Mode::Fill => (
            scaled_w.max(out_dims.0 as usize),
            scaled_h.max(out_dims.1 as usize),
        ),
        Mode::Fit => (
            scaled_w.min(out_dims.0 as usize),
            scaled_h.min(out_dims.1 as usize),
        ),
        _ => unreachable!(),
    }
}

fn swap_channels(rgba_buf: &[u8], bgra_buf: &mut [u8]) {
    // swaps red and blue channels, in parallel
    bgra_buf
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(rgba_buf.as_chunks::<4>().0.iter())
        .for_each(|(bgra, rgba)| {
            bgra[0] = rgba[2];
            bgra[1] = rgba[1];
            bgra[2] = rgba[0];
            bgra[3] = rgba[3];
        });
}

fn get_resized_view(
    in_image: Vec<u8>,
    in_dims: (u32, u32),
    scaled_dims: (usize, usize),
) -> Image<'static> {
    let mut resizer = Resizer::new();

    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear));

    let in_view = Image::from_vec_u8(in_dims.0, in_dims.1, in_image, PixelType::U8x4)
        .unwrap_or_else(|e| dies("Failed to create input image view", e));

    let mut out_view = Image::new(scaled_dims.0 as u32, scaled_dims.1 as u32, PixelType::U8x4);

    resizer
        .resize(&in_view, &mut out_view, Some(&options))
        .unwrap_or_else(|e| dies("Failed to create output view", e));
    out_view
}

fn resize_fit(
    out_buf: &mut [u8],
    resized: &[u8],
    in_dims: (usize, usize),
    out_dims: (usize, usize),
) {
    // a black pixel
    const BLACK: [u8; 4] = [0, 0, 0, 255];
    // helper to assign black pixels
    fn set_to_black(row: &mut [u8]) {
        row.as_chunks_mut::<4>()
            .0
            .iter_mut()
            .for_each(|p| *p = BLACK);
    }
    let x_offset = (out_dims.0 - in_dims.0) / 2;
    let y_offset = (out_dims.1 - in_dims.1) / 2;
    let stride = out_dims.0 * 4;
    let scaled_stride = in_dims.0 * 4;

    // this is kinda hard to follow and theres probably a better way of doing this
    out_buf
        // operate on the entire output row
        .par_chunks_exact_mut(stride)
        .enumerate()
        .for_each(|(y_idx, map_row)| {
            // first, set rows which are outside the scaled image letterbox to black, in parallel
            if y_idx < y_offset || y_idx >= y_offset + in_dims.1 {
                set_to_black(map_row);
                return;
            }

            // all setup to get left letterbox, right letterbox and the actual image over to the mmap
            let src_row_start = (y_idx - y_offset) * scaled_stride;
            let src_row = &resized[src_row_start..src_row_start + scaled_stride];

            let (left, rest) = map_row.split_at_mut(x_offset * 4);
            let (mid, right) = rest.split_at_mut(scaled_stride);

            set_to_black(left);
            swap_channels(src_row, mid);
            set_to_black(right);
        });
}

fn resize_fill(
    out_buf: &mut [u8],
    resized: &[u8],
    in_dims: (usize, usize),
    out_dims: (usize, usize),
) {
    // Calculate cropping offsets to center out the extra pixels
    let x_offset = (in_dims.0 - out_dims.0) / 2;
    let y_offset = (in_dims.1 - out_dims.1) / 2;
    let stride = out_dims.0 * 4;
    let scaled_stride = in_dims.0 * 4;
    out_buf
        .par_chunks_exact_mut(stride)
        .enumerate()
        .for_each(|(y_idx, mmap_row)| {
            // (offset + row index) * stride + column offset
            let src_y = (y_offset + y_idx) * scaled_stride + (x_offset * 4);
            let src_row = &resized[src_y..src_y + (stride)];

            swap_channels(src_row, mmap_row);
        });
}

fn main() {
    let args = env::args().collect::<Vec<String>>();
    if args.len() < 6 {
        eprintln!(
            "Usage: {} <width> <height> <mode> <input_image> <output_raw_bgra>",
            args[0]
        );
        std::process::exit(1)
    }

    let width = args[1]
        .parse::<u32>()
        .unwrap_or_else(|e| dies("Failed to parse width", e));
    let height = args[2]
        .parse::<u32>()
        .unwrap_or_else(|e| dies("Failed to parse height", e));
    if width == 0 || height == 0 {
        dies("Invalid dimensions", "both dimensions must be non-zero")
    }

    let mode = Mode::parse(args[3].as_str())
        .unwrap_or_else(|| dies("Invalid mode", "must be fill, fit or norm"));
    let in_path = &args[4];
    let out_path = &args[5];
    let out_to_stdout = out_path == "-";

    let in_mmap = unsafe {
        Mmap::map(&File::open(in_path).unwrap_or_else(|e| dies("Failed to open input file", e)))
            .unwrap_or_else(|e| dies("Failed to mmap input file", e))
    };

    let in_image = ImageReader::new(Cursor::new(in_mmap))
        .with_guessed_format()
        .map_err(ImageError::from)
        .and_then(|i| i.decode())
        .unwrap_or_else(|e| dies("Failed to decode image", e));

    let in_dims = in_image.dimensions();
    let scaled_dims = get_dimensions(in_dims, (width, height), &mode);

    let out_view = get_resized_view(in_image.into_rgba8().into_raw(), in_dims, scaled_dims);

    // stdout is a pipe, not a file, so it can't be mmap'd. build the frame
    // in a plain heap buffer and write it out in one shot instead.
    if out_to_stdout {
        let mut out_buf = vec![0u8; (width * height * 4) as usize];
        match mode {
            Mode::Fill => resize_fill(
                &mut out_buf,
                out_view.buffer(),
                scaled_dims,
                (width as usize, height as usize),
            ),
            Mode::Fit => resize_fit(
                &mut out_buf,
                out_view.buffer(),
                scaled_dims,
                (width as usize, height as usize),
            ),
            Mode::Normal => (),
        }

        let mut lock = std::io::stdout().lock();
        lock.write_all(&out_buf)
            .unwrap_or_else(|e| dies("Failed to write to stdout", e));
        lock.flush()
            .unwrap_or_else(|e| dies("Failed to flush stdout", e));
        return;
    }

    let out_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(out_path)
        .unwrap_or_else(|e| dies("Failed to open output file", e));

    out_file
        .set_len((width * height * 4) as u64)
        .unwrap_or_else(|e| dies("Failed to set output file len", e));

    let mut out_mmap = unsafe {
        Mmap::map(&out_file)
            .and_then(|m| m.make_mut())
            .unwrap_or_else(|e| dies("Failed to mmap output file", e))
    };

    match mode {
        // zoom into center and crop
        Mode::Fill => {
            resize_fill(
                &mut out_mmap,
                out_view.buffer(),
                scaled_dims,
                (width as usize, height as usize),
            );
        }
        // fit image inside output dimensions, using letterboxing. bit more complicated to do faster
        Mode::Fit => {
            resize_fit(
                &mut out_mmap,
                out_view.buffer(),
                scaled_dims,
                (width as usize, height as usize),
            );
        }
        Mode::Normal => (),
    };

    out_mmap
        .flush_async()
        .unwrap_or_else(|e| dies("Failed to flush output mmap to disk", e));
}
