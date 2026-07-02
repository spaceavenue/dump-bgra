use rayon::{
    iter::{IndexedParallelIterator, ParallelIterator},
    slice::{ParallelSlice, ParallelSliceMut},
};
use std::{
    env,
    fs::{File, OpenOptions},
    io::Cursor,
};

use image::{
    ImageBuffer, ImageError, ImageReader, Rgba,
    imageops::{FilterType, overlay},
};
use memmap2::Mmap;

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
    let mode = &args[3];
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

    let image = ImageReader::new(Cursor::new(in_mmap))
        .with_guessed_format()
        .map_err(ImageError::from)
        .and_then(|i| i.decode())
        .unwrap_or_else(|e| {
            eprintln!("Failed to decode image: {e}");
            std::process::exit(1)
        });
    let transformed_image = if mode == "fill" {
        image
            .resize_to_fill(width, height, FilterType::Triangle)
            .into_rgba8()
    } else if mode == "fit" {
        let resized = image.resize(width, height, FilterType::Triangle);
        let mut centered = ImageBuffer::from_pixel(width, height, Rgba([0, 0, 0, 255]));
        overlay(
            &mut centered,
            &resized,
            ((width - resized.width()) / 2) as i64,
            ((height - resized.height()) / 2) as i64,
        );
        centered
    } else {
        eprintln!("Invalid mode");
        std::process::exit(1)
    };

    let out_image_size = (width * height * 4) as usize;

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

    out_file.set_len(out_image_size as u64).unwrap_or_else(|e| {
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

    out_mmap
        .par_chunks_exact_mut(4)
        .zip(transformed_image.as_raw().par_chunks_exact(4))
        .for_each(|(bgra_pixel, rgba_pixel)| {
            // Source RGBA layout: [0]=R, [1]=G, [2]=B, [3]=A
            // Target BGRA layout: [0]=B, [1]=G, [2]=R, [3]=A
            bgra_pixel[0] = rgba_pixel[2]; // Blue
            bgra_pixel[1] = rgba_pixel[1]; // Green
            bgra_pixel[2] = rgba_pixel[0]; // Red
            bgra_pixel[3] = rgba_pixel[3]; // Alpha
        });

    out_mmap.flush_async().unwrap_or_else(|e| {
        eprintln!("Failed to flush output mmap to disk: {e}");
        std::process::exit(1)
    });
}
