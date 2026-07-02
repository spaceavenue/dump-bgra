use rayon::{
    iter::{IndexedParallelIterator, ParallelIterator},
    slice::{ParallelSlice, ParallelSliceMut},
};
use std::{
    env,
    fs::{File, OpenOptions},
    io::Cursor,
};

use image::{DynamicImage, ImageError, ImageReader, imageops::FilterType};
use memmap2::{Mmap, MmapMut};

fn resize_fit(out_mmap: &mut MmapMut, in_image: DynamicImage, width: usize, height: usize) {
    // black background, parallelized
    out_mmap.par_chunks_exact_mut(4).for_each(|pixel| {
        pixel[0] = 0;
        pixel[1] = 0;
        pixel[2] = 0;
        pixel[3] = 255;
    });
    let resized = in_image
        .resize(width as u32, height as u32, FilterType::Triangle)
        .to_rgba8();
    let h_resized = resized.height() as usize;
    let w_resized = resized.width() as usize;
    let x_offset = (width - h_resized) / 2;
    let y_offset = (height - w_resized) / 2;

    out_mmap
        .par_chunks_exact_mut(width * 4)
        .enumerate()
        .skip(y_offset)
        .take(h_resized)
        .zip(resized.as_raw().par_chunks_exact(w_resized * 4))
        .for_each(|((_, map_row), resized_row)| {
            let start_byte = x_offset * 4;
            let end_byte = start_byte + resized_row.len();
            let target_sub_row = &mut map_row[start_byte..end_byte];
            target_sub_row
                .chunks_exact_mut(4)
                .zip(resized_row.chunks_exact(4))
                .for_each(|(bgra, rgba)| {
                    bgra[0] = rgba[2]; // B
                    bgra[1] = rgba[1]; // G
                    bgra[2] = rgba[0]; // R
                    bgra[3] = rgba[3]; // A
                });
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

    match mode {
        // zoom into center and crop
        "fill" => {
            let resized = in_image
                .resize_to_fill(width, height, FilterType::Triangle)
                .into_rgba8();
            out_mmap
                .par_chunks_exact_mut(4)
                .zip(resized.as_raw().par_chunks_exact(4))
                .for_each(|(bgra_pixel, rgba_pixel)| {
                    bgra_pixel[0] = rgba_pixel[2];
                    bgra_pixel[1] = rgba_pixel[1];
                    bgra_pixel[2] = rgba_pixel[0];
                    bgra_pixel[3] = rgba_pixel[3];
                });
        }
        // fit image inside output dimensions, using letterboxing. bit more complicated to do faster
        "fit" => {
            resize_fit(&mut out_mmap, in_image, width as usize, height as usize);
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
