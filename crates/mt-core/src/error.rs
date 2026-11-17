use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("image: {0}")]
    Image(#[from] image::ImageError),

    #[error("png optimisation: {0}")]
    Oxipng(#[from] oxipng::PngError),

    #[error("unsupported: {0}")]
    Unsupported(String),

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn other(msg: impl fmt::Display) -> Self {
        Error::Other(msg.to_string())
    }
    pub fn invalid(msg: impl fmt::Display) -> Self {
        Error::Invalid(msg.to_string())
    }
    pub fn unsupported(msg: impl fmt::Display) -> Self {
        Error::Unsupported(msg.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
