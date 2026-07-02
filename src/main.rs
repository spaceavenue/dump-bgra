use fast_image_resize::{
    self as fir, FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer,
    images::{self, Image},
};
use image::{DynamicImage, ImageError, ImageReader};
use memmap2::{Mmap, MmapMut};
use rayon::{
    iter::{IndexedParallelIterator, ParallelIterator},
    slice::{ParallelSlice, ParallelSliceMut},
};
use std::{
    env,
    fs::{File, OpenOptions},
    io::Cursor,
};

fn get_dimensions(in_w: u32, in_h: u32, out_w: u32, out_h: u32, mode: &str) -> (usize, usize) {
    let w_ratio = out_w as f64 / in_w as f64;
    let h_ratio = out_h as f64 / in_h as f64;

    let scale = match mode {
        "fill" => w_ratio.max(h_ratio),
        "fit" => w_ratio.min(h_ratio),
        _ => 1.0,
    };
    (
        (in_w as f64 * scale).round() as usize,
        (in_h as f64 * scale).round() as usize,
    )
}

fn swap_channels(rgba_buf: &[u8], bgra_bf: &mut [u8]) {
    bgra_bf
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

fn resizer_init(
    in_image: DynamicImage,
    scaled_dims: (usize, usize),
) -> (Resizer, ResizeOptions, Image<'static>, Image<'static>) {
    let resizer = fir::Resizer::new();

    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear));

    let Ok(in_view) = images::Image::from_vec_u8(
        in_image.width(),
        in_image.height(),
        in_image.into_rgba8().into_raw(),
        PixelType::U8x4,
    ) else {
        eprintln!("Failed to create input image view.");
        std::process::exit(1)
    };
    let out_view = images::Image::new(scaled_dims.0 as u32, scaled_dims.1 as u32, PixelType::U8x4);
    (resizer, options, in_view, out_view)
}

fn resize_fit(
    out_mmap: &mut MmapMut,
    resized: &[u8],
    scaled_dims: (usize, usize),
    width: usize,
    height: usize,
) {
    // create a black background in parallel (because why the hell not)
    out_mmap.par_chunks_exact_mut(4).for_each(|pixel| {
        pixel[0] = 0;
        pixel[1] = 0;
        pixel[2] = 0;
        pixel[3] = 255;
    });

    let x_offset = (width - scaled_dims.0) / 2;
    let y_offset = (height - scaled_dims.1) / 2;

    // this is kinda hard to follow and theres probably a better way of doing this
    out_mmap
        // operate on the entire row
        .par_chunks_exact_mut(width * 4)
        .enumerate()
        // skip the offset height and take the rest
        .skip(y_offset)
        .take(scaled_dims.1)
        // operate on the entire *resized* row
        .zip(resized.par_chunks_exact(scaled_dims.0 * 4))
        .for_each(|((_, map_row), resized_row)| {
            // offset start and end, end is just start offset + size of resized row
            let start_byte = x_offset * 4;
            let end_byte = start_byte + resized_row.len();
            // swap RGBA -> BGRA
            swap_channels(resized_row, &mut map_row[start_byte..end_byte]);
        });
}

fn resize_fill(
    out_mmap: &mut MmapMut,
    resized: &[u8],
    scaled_dims: (usize, usize),
    width: usize,
    height: usize,
) {
    // Calculate cropping offsets to center out the extra pixels
    let x_offset = (scaled_dims.0 - width) / 2;
    let y_offset = (scaled_dims.1 - height) / 2;

    out_mmap
        .par_chunks_exact_mut(width * 4)
        .enumerate()
        .for_each(|(y_idx, mmap_row)| {
            // (offset + row index) * stride + column offset
            let src_y = (y_offset + y_idx) * (scaled_dims.0 * 4) + (x_offset * 4);
            let src_row = &resized[src_y..src_y + (width * 4)];

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

    let Ok(width) = args[1].parse::<u32>() else {
        eprintln!("Failed to parse width.");
        std::process::exit(1)
    };
    let Ok(height) = args[2].parse::<u32>() else {
        eprintln!("Failed to parse height.");
        std::process::exit(1)
    };
    let mode = args[3].as_str();
    let in_path = &args[4];
    let out_path = &args[5];

    let in_mmap = unsafe {
        Mmap::map(&File::open(in_path).unwrap_or_else(|e| {
            eprintln!("Failed to open input file: {e}");
            std::process::exit(1)
        }))
        .unwrap_or_else(|e| {
            eprintln!("Failed to mmap input file: {e}");
            std::process::exit(1)
        })
    };

    let in_image = ImageReader::new(Cursor::new(in_mmap))
        .with_guessed_format()
        .map_err(ImageError::from)
        .and_then(|i| i.decode())
        .unwrap_or_else(|e| {
            eprintln!("Failed to decode image: {e}");
            std::process::exit(1)
        });

    let out_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(out_path)
        .unwrap_or_else(|e| {
            eprintln!("Failed to open output file: {e}");
            std::process::exit(1)
        });
    out_file
        .set_len((width * height * 4) as u64)
        .unwrap_or_else(|e| {
            eprintln!("Failed to set output file len: {e}");
            std::process::exit(1)
        });
    let mut out_mmap = unsafe {
        Mmap::map(&out_file)
            .and_then(|m| m.make_mut())
            .unwrap_or_else(|e| {
                eprintln!("Failed to mmap output file: {e}");
                std::process::exit(1)
            })
    };

    let scaled_dims = get_dimensions(in_image.width(), in_image.height(), width, height, mode);
    let (mut resizer, options, in_view, mut out_view) = resizer_init(in_image, scaled_dims);
    let _ = resizer.resize(&in_view, &mut out_view, Some(&options));

    match mode {
        // zoom into center and crop
        "fill" => {
            resize_fill(
                &mut out_mmap,
                out_view.buffer(),
                scaled_dims,
                width as usize,
                height as usize,
            );
        }
        // fit image inside output dimensions, using letterboxing. bit more complicated to do faster
        "fit" => {
            resize_fit(
                &mut out_mmap,
                out_view.buffer(),
                scaled_dims,
                width as usize,
                height as usize,
            );
        }
        _ => {
            eprintln!("Invalid mode");
            std::process::exit(1)
        }
    };

    out_mmap.flush_async().unwrap_or_else(|e| {
        eprintln!("Failed to flush output mmap to disk: {e}");
        std::process::exit(1)
    });
}
