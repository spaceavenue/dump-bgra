mod resize;
mod util;

use std::env;
use std::io::{Write, stdout};

use anyhow::{Context, bail};
use resize::{Mode, resize_fill, resize_fit, resize_image};
use util::{get_input_image, get_out_mmap};

fn main() -> anyhow::Result<()> {
    let args = env::args().collect::<Vec<String>>();
    let dims: Vec<&str> = args[1].split('x').collect();
    let width = dims[0].parse::<usize>().context("failed to parse width")?;
    let height = dims[1].parse::<usize>().context("failed to parse height")?;
    let out_dims = (width, height);
    if width == 0 || height == 0 {
        bail!("width and height must be non-zero!");
    }
    let mode = Mode::parse(&args[2]).context("invalid mode, must be fill or fit.")?;
    let in_path = args[3].clone();
    let out_path = args[4].clone();
    let to_stdout = out_path == "-";

    let (in_image, img_dims) = get_input_image(&in_path)?;

    let (resize_buf, resize_dims) = resize_image(in_image, img_dims, out_dims, &mode)?;

    let fill_out_buffer = |out_buf: &mut [u8]| match mode {
        Mode::Fill => resize_fill(&resize_buf, out_buf, resize_dims, out_dims),
        Mode::Fit => resize_fit(&resize_buf, out_buf, resize_dims, out_dims),
    };

    // stdout is a pipe, not a file, so it can't be mmap'd. fill a vec instead and write that to
    // stdout
    if to_stdout {
        let len = out_dims.0 * out_dims.1 * 4;
        let mut out_buf = Vec::with_capacity(len);
        unsafe { out_buf.set_len(len) };
        fill_out_buffer(&mut out_buf);
        let mut lock = stdout().lock();
        lock.write_all(&out_buf).and_then(|()| lock.flush())?;
        return Ok(());
    }

    // get output mmap and fill it
    let mut out_mmap = get_out_mmap(&out_path, out_dims)?;
    fill_out_buffer(&mut out_mmap);

    // flush buffer to disk
    out_mmap.flush_async()?;

    Ok(())
}
