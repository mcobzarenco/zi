use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// A representation of a runtime error encountered by Zi.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Exiting")]
    Exiting,
}
