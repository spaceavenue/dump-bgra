use std::env;
use std::fs::{File, OpenOptions};
use std::io::{Cursor, Write};

use fast_image_resize::images::Image;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::{ImageError, ImageReader};
use memmap2::{Mmap, MmapMut};
use rayon::iter::{IndexedParallelIterator, ParallelIterator};
use rayon::slice::ParallelSliceMut;

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

fn get_dimensions(
    in_dims: (usize, usize),
    out_dims: (usize, usize),
    mode: &Mode,
) -> (usize, usize) {
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

fn get_resized_view(
    in_image: Vec<u8>,
    in_dims: (usize, usize),
    scaled_dims: (usize, usize),
) -> Image<'static> {
    // returns a resized view for an image buffer based on the given scaled dimensions.

    let mut resizer = Resizer::new();
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear));

    let in_view = Image::from_vec_u8(
        in_dims.0 as u32,
        in_dims.1 as u32,
        in_image,
        PixelType::U8x4,
    )
    .unwrap_or_else(|e| dies("Failed to create input image view", e));

    let mut out_view = Image::new(scaled_dims.0 as u32, scaled_dims.1 as u32, PixelType::U8x4);

    resizer
        .resize(&in_view, &mut out_view, Some(&options))
        .unwrap_or_else(|e| dies("Failed to create output view", e));
    out_view
}

fn copy_and_swap_channels(rgba_buf: &[u8], bgra_buf: &mut [u8]) {
    // copies and swaps red and blue channels
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

fn resize_fit(
    in_buf: &[u8],
    out_buf: &mut [u8],
    in_dims: (usize, usize),
    out_dims: (usize, usize),
) {
    // in this function,
    // in_buf -> scaled image buffer from FIR
    // out_buf -> mmap buffer

    // a black pixel and a helper to assign black pixels
    const BLACK: [u8; 4] = [0, 0, 0, 255];
    fn set_to_black(row: &mut [u8]) {
        row.as_chunks_mut::<4>()
            .0
            .iter_mut()
            .for_each(|p| *p = BLACK);
    }
    let x_offset = (out_dims.0 - in_dims.0) / 2;
    let y_offset = (out_dims.1 - in_dims.1) / 2;
    let in_stride = in_dims.0 * 4;
    let out_stride = out_dims.0 * 4;

    // this is kinda hard to follow and theres probably a better way of doing this
    out_buf
        // operate on the entire output buffer row
        .par_chunks_exact_mut(out_stride)
        .enumerate()
        // here, y_idx is tracking the entire output buffer, and map_row is a row in said buffer
        .for_each(|(y_idx, map_row)| {
            // first, set rows which are outside the scaled image to black, in parallel. y_offset is
            // where the rows of the scaled image start
            if y_idx < y_offset || y_idx >= y_offset + in_dims.1 {
                set_to_black(map_row);
                return;
            }

            // the start of the scaled image, we subtract y_idx - y_offset to index the scaled image
            // buffer. this will not underflow because we already checked for all y_idx < y_offset
            // above
            let src_row_start = (y_idx - y_offset) * in_stride;
            // this indexes scaled image rows, so src_row is our actual image row
            let src_row = &in_buf[src_row_start..src_row_start + in_stride];

            // split at (x_offset * 4), which is the stride of the columnar letterbox, and then at
            // scaled_stride, which is the stride of our scaled image. so:
            // left -> black
            // pixels, mid -> image
            // right -> black pixels
            let (left, rest) = map_row.split_at_mut(x_offset * 4);
            let (mid, right) = rest.split_at_mut(in_stride);

            // do the actual setting and copying
            set_to_black(left);
            copy_and_swap_channels(src_row, mid);
            set_to_black(right);
        });
}

fn resize_fill(
    in_buf: &[u8],
    out_buf: &mut [u8],
    in_dims: (usize, usize),
    out_dims: (usize, usize),
) {
    // get center cropping offsets, will be used later to copy only the pixels that "align" with the
    // output buffer in the center
    let x_offset = (in_dims.0 - out_dims.0) / 2;
    let y_offset = (in_dims.1 - out_dims.1) / 2;
    let out_stride = out_dims.0 * 4;
    let in_stride = in_dims.0 * 4;
    out_buf
        // iterate over entire output rows
        .par_chunks_exact_mut(out_stride)
        .enumerate()
        .for_each(|(y_idx, map_row)| {
            // (offset + output row index) * stride + column offset
            let src_y = (y_offset + y_idx) * in_stride + (x_offset * 4);
            let src_row = &in_buf[src_y..src_y + (out_stride)];

            copy_and_swap_channels(src_row, map_row);
        });
}

fn get_out_mmap(out_path: &String, out_dims: (usize, usize)) -> MmapMut {
    // create output file with the given read+write+create+truncate options. allows overwriting
    // files that already exist or creating ones that dont.
    let out_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(out_path)
        .unwrap_or_else(|e| dies("Failed to open output file", e));
    // set length to given dimensions
    out_file
        .set_len((out_dims.0 * out_dims.1 * 4) as u64)
        .unwrap_or_else(|e| dies("Failed to set output file len", e));
    // make a mutable mmap. will be passed as the output buffer, the flushed to disk later
    let out_mmap = unsafe {
        Mmap::map(&out_file)
            .and_then(|m| m.make_mut())
            .unwrap_or_else(|e| dies("Failed to mmap output file", e))
    };
    out_mmap
}

fn main() {
    let args = env::args().collect::<Vec<String>>();
    if args.len() < 6 {
        eprintln!(
            "Usage: {} <width> <height> <fill|fit|norm> <input_image> < output_raw_bgra | - >",
            args[0]
        );
        std::process::exit(1)
    }

    let width = args[1]
        .parse::<usize>()
        .unwrap_or_else(|e| dies("Failed to parse width", e));
    let height = args[2]
        .parse::<usize>()
        .unwrap_or_else(|e| dies("Failed to parse height", e));
    if width == 0 || height == 0 {
        dies("Invalid dimensions", "both dimensions must be non-zero")
    }
    let out_dims = (width, height);

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

    let img_dims = (in_image.width() as usize, in_image.height() as usize);

    let (in_buf, in_dims) = match mode {
        Mode::Normal => (in_image.into_rgba8().into_raw(), img_dims),
        Mode::Fit | Mode::Fill => {
            let scaled_dims = get_dimensions(img_dims, out_dims, &mode);
            let out_view =
                get_resized_view(in_image.into_rgba8().into_raw(), img_dims, scaled_dims);
            (out_view.into_vec(), scaled_dims)
        }
    };

    let fill_out_buffer = |out_buf: &mut [u8]| match mode {
        Mode::Normal => resize_fill(&in_buf, out_buf, in_dims, in_dims),
        Mode::Fill => resize_fill(&in_buf, out_buf, in_dims, out_dims),
        Mode::Fit => resize_fit(&in_buf, out_buf, in_dims, out_dims),
    };

    // stdout is a pipe, not a file, so it can't be mmap'd. fill a vec instead and write that to
    // stdout
    if out_to_stdout {
        let mut out_buf = match mode {
            Mode::Normal => vec![0u8; img_dims.0 * img_dims.1 * 4],
            Mode::Fit | Mode::Fill => vec![0u8; out_dims.0 * out_dims.1 * 4],
        };

        fill_out_buffer(&mut out_buf);

        let mut lock = std::io::stdout().lock();
        lock.write_all(&out_buf)
            .unwrap_or_else(|e| dies("Failed to write to stdout", e));
        lock.flush()
            .unwrap_or_else(|e| dies("Failed to flush stdout", e));
        return;
    }

    // match on given mode
    let mut out_mmap = match mode {
        Mode::Normal => get_out_mmap(out_path, img_dims),
        Mode::Fit | Mode::Fill => get_out_mmap(out_path, out_dims),
    };
    fill_out_buffer(&mut out_mmap);

    // flush buffer to disk
    out_mmap
        .flush_async()
        .unwrap_or_else(|e| dies("Failed to flush output mmap to disk", e));
}
