use rayon::iter::{IndexedParallelIterator, ParallelIterator};
use rayon::slice::{ParallelSlice, ParallelSliceMut};

pub fn is_qoz(data: &[u8]) -> bool {
    data.starts_with(b"QOZ1")
}

pub fn decode(input: &[u8]) -> Result<(qoz::Header, Vec<u8>), qoz::QozError> {
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
