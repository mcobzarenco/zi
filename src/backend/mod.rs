//! This module contains the `Backend` trait and provided implementations.

#[cfg(feature = "backend-crossterm")]
pub mod crossterm;
#[cfg(feature = "backend-crossterm")]
pub use self::crossterm::Crossterm;

pub(crate) mod painter;

mod utils;

use futures::Stream;
use std::io;
use thiserror::Error;

use crate::terminal::{Canvas, Key, Size};

/// A trait implemented by backends that draw a [`Canvas`](../terminal/struct.Canvas.html) to
/// an underlying device (e.g an ANSI terminal).
pub trait Backend {
    /// Stream with backend events.
    type EventStream: Stream<Item = Result<Event>> + Unpin;

    /// Returns the size of the underlying terminal.
    fn size(&self) -> Result<Size>;

    /// Draws the [`Canvas`](../terminal/struct.Canvas.html) to the terminal.
    fn present(&mut self, canvas: &Canvas) -> Result<usize>;

    /// Returns a stream with user input events.
    ///
    /// Guaranteed to never be called after a call to `suspend` and before the corresponding
    /// call to `resume.`
    fn event_stream(&mut self) -> &mut Self::EventStream;

    /// Suspends the event stream.
    ///
    /// This is used when running something that needs exclusive access to the underlying
    /// terminal (i.e. to stdin and stdout). For example spawning an external editor to collect
    /// or display text. The `resume` function is called upon returning to the application.
    fn suspend(&mut self) -> Result<()>;

    /// Recreates the event stream and reinitialises the underlying terminal.
    ///
    /// This function is used to return execution to the application after running something
    /// that needs exclusive access to the underlying backend. It will only be called after a
    /// call to `suspend`.
    ///
    /// In addition to restarting the event stream, this function should perform any other
    /// required initialisation of the backend. For ANSI terminals, this typically hides the
    /// cursor and saves the current screen content (i.e. "alternative screen mode") in order
    /// to restore the previous terminal content on exit.
    fn resume(&mut self) -> Result<()>;
}

/// Alias for `Result` with a backend error.
pub type Result<T> = std::result::Result<T, Error>;

/// Backend event
#[derive(Debug)]
pub enum Event {
    Key(Key),
    Resize(Size),
}

/// Backend error
#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    UnknownBackend(String),

    #[cfg(feature = "backend-crossterm")]
    #[error(transparent)]
    Crossterm(#[from] crossterm::Error),

    #[error(transparent)]
    Io(#[from] io::Error),
}

#[cfg(feature = "backend-crossterm")]
pub fn default() -> Result<crossterm::Crossterm> {
    //! Builds the default backend.
    crossterm::Crossterm::new()
}
