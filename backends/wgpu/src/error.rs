use thiserror::Error;

/// Alias for `Result` with a backend error
pub type Result<T> = std::result::Result<T, Error>;

/// Wgpu backend error type
#[derive(Debug, Error)]
pub enum Error {
    #[error("Backend error {0}")]
    Window(#[from] winit::error::OsError),

    #[error("Crossfont error {0}")]
    Crossfont(#[from] crossfont::Error),
    // #[error("Wgpu error {0}")]
    // Wgpu(#[from] wgpu::Error),
}
