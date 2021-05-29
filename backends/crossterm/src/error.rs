use std::io;
use thiserror::Error;

/// Alias for `Result` with a backend error.
pub type Result<T> = std::result::Result<T, Error>;

/// Error type for
#[derive(Debug, Error)]
pub enum Error {
    /// Error originating from [crossterm](https://docs.rs/crossterm)
    #[error(transparent)]
    Crossterm(#[from] crossterm::ErrorKind),

    /// IO error
    #[error(transparent)]
    Io(io::Error),
}
