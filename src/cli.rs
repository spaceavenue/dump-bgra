use std::env;

use crate::error::AppError;
use crate::resize::Mode;

pub struct Cli {
    pub out_dims: (usize, usize),
    pub mode: Mode,
    pub in_path: String,
    pub out_path: String,
}
impl Cli {
    pub fn parse() -> Result<Self, AppError> {
        let args = env::args().collect::<Vec<String>>();
        let (mode, out_dims, in_path, out_path) = match &args[1..] {
            [mode_str, _, _] if Mode::parse(mode_str).is_some() => {
                return Err(AppError::InvalidArgs(
                    "Missing arguments",
                    "'fit'/'fill' need <width> and <height>",
                ));
            }
            [w_str, h_str, mode_str, in_path, out_path] => {
                let width = w_str.parse::<usize>()?;
                let height = h_str.parse::<usize>()?;
                if width == 0 || height == 0 {
                    return Err(AppError::InvalidArgs(
                        "Invalid dimensions",
                        "both dimensions must be non-zero",
                    ));
                }
                let mode = Mode::parse(mode_str)
                    .ok_or(AppError::InvalidArgs("Invalid mode", "must be fill or fit"))?;

                (mode, (width, height), in_path.clone(), out_path.clone())
            }
            _ => {
                eprintln!(
                    "Usage:\n  {} <width> <height> <fill|fit> <input> <output | ->",
                    args[0]
                );
                std::process::exit(1);
            }
        };
        Ok(Self {
            out_dims,
            mode,
            in_path,
            out_path,
        })
    }
    pub fn out_to_stdout(&self) -> bool {
        self.out_path == "-"
    }
}
