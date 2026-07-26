mod cli;
mod error;
mod resize;
mod util;

use cli::Cli;
use error::AppError;
use resize::{Mode, resize_fill, resize_fit, resize_image};
use util::{get_input_image, get_out_mmap, write_to_stdout};

fn run() -> Result<(), AppError> {
    let args = Cli::parse()?;

    let (in_image, img_dims) = get_input_image(&args.in_path)?;

    let (resize_buf, resize_dims) = resize_image(in_image, img_dims, args.out_dims, &args.mode)?;

    let final_dims = match args.mode {
        Mode::Normal => resize_dims,
        Mode::Fit | Mode::Fill => args.out_dims,
    };

    let fill_out_buffer = |out_buf: &mut [u8]| match args.mode {
        Mode::Normal | Mode::Fill => resize_fill(&resize_buf, out_buf, resize_dims, final_dims),
        Mode::Fit => resize_fit(&resize_buf, out_buf, resize_dims, final_dims),
    };

    // stdout is a pipe, not a file, so it can't be mmap'd. fill a vec instead and write that to
    // stdout
    if args.out_to_stdout() {
        let mut out_buf = vec![0u8; final_dims.0 * final_dims.1 * 4];
        fill_out_buffer(&mut out_buf);
        write_to_stdout(&out_buf)?;
        return Ok(());
    }

    // match on given mode
    let mut out_mmap = get_out_mmap(&args.out_path, final_dims)?;
    fill_out_buffer(&mut out_mmap);

    // flush buffer to disk
    out_mmap.flush_async()?;

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
