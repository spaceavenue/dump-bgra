use std::num::ParseIntError;
use std::{fmt, io};

use fast_image_resize::{ImageBufferError, ResizeError};
use image::ImageError;

#[derive(Debug)]
pub enum AppError {
    Io(io::Error),
    Parse(ParseIntError),
    InvalidArgs(&'static str, &'static str),
    ImageDecode(&'static str, String),
    ImageProcessing(&'static str, String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Parse(err) => write!(f, "Parse error: {err}"),
            Self::InvalidArgs(title, detail) => write!(f, "{title}: {detail}"),
            Self::ImageProcessing(title, detail) => write!(f, "{title}: {detail}"),
            Self::ImageDecode(title, detail) => write!(f, "{title}: {detail}"),
        }
    }
}

impl From<io::Error> for AppError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}
impl From<ParseIntError> for AppError {
    fn from(err: ParseIntError) -> Self {
        Self::Parse(err)
    }
}
impl From<ImageError> for AppError {
    fn from(err: ImageError) -> Self {
        Self::ImageDecode("Image decode failed", err.to_string())
    }
}
impl From<qoz::QozError> for AppError {
    fn from(err: qoz::QozError) -> Self {
        Self::ImageDecode("Image decode failed", err.to_string())
    }
}
impl From<ImageBufferError> for AppError {
    fn from(err: ImageBufferError) -> Self {
        Self::ImageProcessing("Image resize failed", err.to_string())
    }
}
impl From<ResizeError> for AppError {
    fn from(err: ResizeError) -> Self {
        Self::ImageProcessing("Image resize failed", err.to_string())
    }
}
