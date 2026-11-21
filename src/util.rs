use std::fs::{File, OpenOptions};
use std::io::Cursor;
use std::path::Path;

use image::ImageReader;
use memmap2::{Mmap, MmapMut};

pub fn get_input_image(path: impl AsRef<Path>) -> anyhow::Result<(Vec<u8>, (usize, usize))> {
    let in_mmap = unsafe { Mmap::map(&File::open(path)?)? };
    let image = ImageReader::new(Cursor::new(in_mmap))
        .with_guessed_format()?
        .decode()?;
    let image_dims = (image.width() as usize, image.height() as usize);
    Ok((image.into_rgba8().into_raw(), image_dims))
}

pub fn get_out_mmap(out_path: &String, (width, height): (usize, usize)) -> anyhow::Result<MmapMut> {
    let out_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(out_path)?;
    out_file.set_len((width * height * 4) as u64)?;
    let out_mmap = unsafe { MmapMut::map_mut(&out_file)? };
    Ok(out_mmap)
}
