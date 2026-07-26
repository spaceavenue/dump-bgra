use std::fs::{File, OpenOptions};
use std::io::{Cursor, Write, stdout};
use std::path::Path;

use image::ImageReader;
use memmap2::{Mmap, MmapMut};

use crate::error::AppError;

pub fn get_input_image(path: impl AsRef<Path>) -> Result<(Vec<u8>, (usize, usize)), AppError> {
    let in_mmap = unsafe { Mmap::map(&File::open(path)?)? };

    let image = ImageReader::new(Cursor::new(in_mmap))
        .with_guessed_format()?
        .decode()?;
    let image_dims = (image.width() as usize, image.height() as usize);
    Ok((image.into_rgba8().into_raw(), image_dims))
}

pub fn get_out_mmap(out_path: &String, out_dims: (usize, usize)) -> Result<MmapMut, AppError> {
    // create output file with the given read+write+create+truncate options. allows overwriting
    // files that already exist or creating ones that dont.
    let out_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(out_path)?;
    // set length to given dimensions
    out_file.set_len((out_dims.0 * out_dims.1 * 4) as u64)?;
    // make a mutable mmap. will be passed as the output buffer, the flushed to disk later
    let out_mmap = unsafe { MmapMut::map_mut(&out_file)? };
    Ok(out_mmap)
}

pub fn write_to_stdout(out_buf: &[u8]) -> Result<(), AppError> {
    let mut lock = stdout().lock();
    lock.write_all(out_buf).and_then(|()| lock.flush())?;
    Ok(())
}
