use fast_image_resize::images::Image;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use rayon::iter::{IndexedParallelIterator, ParallelIterator};
use rayon::slice::ParallelSliceMut;

pub enum Mode {
    Fit,
    Fill,
}
impl Mode {
    pub fn parse(mode: &str) -> Option<Self> {
        match mode {
            "fit" => Some(Self::Fit),
            "fill" => Some(Self::Fill),
            _ => None,
        }
    }
}

fn get_scaled_dimensions(
    in_dims: (usize, usize),
    out_dims: (usize, usize),
    mode: &Mode,
) -> (usize, usize) {
    let w_ratio = out_dims.0 as f64 / in_dims.0 as f64;
    let h_ratio = out_dims.1 as f64 / in_dims.1 as f64;

    let scale = match mode {
        Mode::Fill => w_ratio.max(h_ratio),
        Mode::Fit => w_ratio.min(h_ratio),
    };

    let scaled_w = (in_dims.0 as f64 * scale).round() as usize;
    let scaled_h = (in_dims.1 as f64 * scale).round() as usize;

    // if .round() is off by a pixel, usize can underflow elsewhere. so we're clamping here
    match mode {
        Mode::Fill => (scaled_w.max(out_dims.0), scaled_h.max(out_dims.1)),
        Mode::Fit => (scaled_w.min(out_dims.0), scaled_h.min(out_dims.1)),
    }
}

// returns a resized view for an image buffer based on the given scaled dimensions.
fn get_resized_view(
    in_image: Vec<u8>,
    in_dims: (usize, usize),
    scaled_dims: (usize, usize),
) -> anyhow::Result<Vec<u8>> {
    let mut resizer = Resizer::new();
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear));

    let in_view = Image::from_vec_u8(
        in_dims.0 as u32,
        in_dims.1 as u32,
        in_image,
        PixelType::U8x4,
    )?;

    let mut out_view = Image::new(scaled_dims.0 as u32, scaled_dims.1 as u32, PixelType::U8x4);

    resizer.resize(&in_view, &mut out_view, Some(&options))?;
    Ok(out_view.into_vec())
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

pub fn resize_image(
    in_image: Vec<u8>,
    img_dims: (usize, usize),
    out_dims: (usize, usize),
    mode: &Mode,
) -> anyhow::Result<(Vec<u8>, (usize, usize))> {
    let scaled_dims = get_scaled_dimensions(img_dims, out_dims, mode);
    // already at the target size
    if scaled_dims == img_dims {
        return Ok((in_image, scaled_dims));
    }
    match mode {
        Mode::Fill | Mode::Fit => Ok((
            get_resized_view(in_image, img_dims, scaled_dims)?,
            scaled_dims,
        )),
    }
}

pub fn resize_fit(
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

pub fn resize_fill(
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
