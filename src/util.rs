use std::fs::{File, OpenOptions};
use std::io::Cursor;
use std::path::Path;

use image::ImageReader;
use memmap2::{Mmap, MmapMut};
use rayon::iter::{IndexedParallelIterator, ParallelIterator};
use rayon::slice::{ParallelSlice, ParallelSliceMut};

pub fn decode(input: &[u8]) -> anyhow::Result<(qoz::Header, Vec<u8>)> {
    // check if theres 3 channels. if there are, we have to convert to an RGBA buffer first
    // since that's what the rest of the program expects.
    let header = qoz::read_header(input)?;
    let pixels = if header.channels != qoz::Channels::Rgba {
        let (_, rgb) = qoz::decode(input)?;
        let mut rgba = vec![0u8; (header.width * header.height * 4) as usize];
        rgb.par_chunks(3 * header.width as usize)
            .zip(rgba.par_chunks_mut(4 * header.width as usize))
            .for_each(|(src_row, dst_row)| {
                dst_row
                    .chunks_exact_mut(4)
                    .zip(src_row.chunks_exact(3))
                    .for_each(|(d, s)| {
                        d[0] = s[0];
                        d[1] = s[1];
                        d[2] = s[2];
                        d[3] = 255u8;
                    });
            });
        rgba
    } else {
        let (_, rgba) = qoz::decode(input)?;
        rgba
    };
    Ok((header, pixels))
}

pub fn get_input_image(path: impl AsRef<Path>) -> anyhow::Result<(Vec<u8>, (usize, usize))> {
    let in_mmap = unsafe { Mmap::map(&File::open(path)?)? };
    if (&in_mmap).starts_with(b"QOZ1") {
        let (header, data) = decode(&in_mmap)?;
        return Ok((data, (header.width as usize, header.height as usize)));
    }
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
