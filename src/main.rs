mod resize;
mod util;

use std::env;

use anyhow::{Context, bail};
use resize::{Mode, resize_fill, resize_fit, resize_image};
use util::{get_input_image, get_out_mmap, uninit_vec, write_to_stdout};

pub struct Cli {
    pub out_dims: (usize, usize),
    pub mode: Mode,
    pub in_path: String,
    pub out_path: String,
}

impl Cli {
    pub fn parse() -> anyhow::Result<Self> {
        let args = env::args().collect::<Vec<String>>();
        let dims: Vec<&str> = args[1].split('x').collect();
        let width = dims[0].parse::<usize>().context("failed to parse width")?;
        let height = dims[1].parse::<usize>().context("failed to parse height")?;
        if width == 0 || height == 0 {
            bail!("width and height must be non-zero!");
        }
        let mode = Mode::parse(&args[2]).context("invalid mode, must be fill or fit.")?;
        let in_path = args[3].clone();
        let out_path = args[4].clone();

        Ok(Self {
            out_dims: (width, height),
            mode,
            in_path,
            out_path,
        })
    }
    pub fn out_to_stdout(&self) -> bool {
        self.out_path == "-"
    }
}

fn main() -> anyhow::Result<()> {
    let args = Cli::parse()?;

    let (in_image, img_dims) = get_input_image(&args.in_path)?;

    let (resize_buf, resize_dims) = resize_image(in_image, img_dims, args.out_dims, &args.mode)?;

    let fill_out_buffer = |out_buf: &mut [u8]| match args.mode {
        Mode::Fill => resize_fill(&resize_buf, out_buf, resize_dims, args.out_dims),
        Mode::Fit => resize_fit(&resize_buf, out_buf, resize_dims, args.out_dims),
    };

    // stdout is a pipe, not a file, so it can't be mmap'd. fill a vec instead and write that to
    // stdout
    if args.out_to_stdout() {
        let len = args.out_dims.0 * args.out_dims.1 * 4;
        let mut out_buf = unsafe { uninit_vec(len) };
        fill_out_buffer(&mut out_buf);
        write_to_stdout(&out_buf)?;
        return Ok(());
    }

    // get output mmap and fill it
    let mut out_mmap = get_out_mmap(&args.out_path, args.out_dims)?;
    fill_out_buffer(&mut out_mmap);

    // flush buffer to disk
    out_mmap.flush_async()?;

    Ok(())
}
