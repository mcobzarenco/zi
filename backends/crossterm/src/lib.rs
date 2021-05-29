//! A terminal backend implementation for [Zi](https://docs.rs/zi) using
//! [crossterm](https://docs.rs/crossterm)
mod error;
mod painter;
mod utils;

pub use self::error::{Error, Result};

use crossterm::{self, queue, QueueableCommand};
use futures::stream::{Stream, StreamExt};
use std::{
    io::{self, BufWriter, Stdout, Write},
    pin::Pin,
    time::{Duration, Instant},
};
use tokio::{
    self,
    runtime::{Builder as RuntimeBuilder, Runtime},
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
};

use self::{
    painter::{FullPainter, IncrementalPainter, PaintOperation, Painter},
    utils::MeteredWriter,
};
use zi::{
    app::{App, ComponentMessage, MessageSender},
    terminal::{Canvas, Colour, Key, Size, Style},
    Layout,
};

/// Creates a new backend with an incremental painter. It only draws those
/// parts of the terminal that have changed since last drawn.
///
/// ```no_run
/// # use zi::prelude::*;
/// # use zi::components::text::{Text, TextProperties};
/// fn main() -> zi_crossterm::Result<()> {
///     zi_crossterm::incremental()?
///         .run_event_loop(Text::with(TextProperties::new().content("Hello, world!")))
/// }
/// ```
pub fn incremental() -> Result<Crossterm<IncrementalPainter>> {
    Crossterm::<IncrementalPainter>::new()
}

/// Creates a new backend with a full painter. It redraws the whole canvas on
/// every canvas.
///
/// ```no_run
/// # use zi::prelude::*;
/// # use zi::components::text::{Text, TextProperties};
/// fn main() -> zi_crossterm::Result<()> {
///     zi_crossterm::full()?
///         .run_event_loop(Text::with(TextProperties::new().content("Hello, world!")))
/// }
/// ```
pub fn full() -> Result<Crossterm<FullPainter>> {
    Crossterm::<FullPainter>::new()
}

/// A terminal backend implementation for [Zi](https://docs.rs/zi) using
/// [crossterm](https://docs.rs/crossterm)
///
/// ```no_run
/// # use zi::prelude::*;
/// # use zi::components::text::{Text, TextProperties};
/// fn main() -> zi_crossterm::Result<()> {
///     zi_crossterm::incremental()?
///         .run_event_loop(Text::with(TextProperties::new().content("Hello, world!")))
/// }
/// ```
pub struct Crossterm<PainterT: Painter = IncrementalPainter> {
    target: MeteredWriter<BufWriter<Stdout>>,
    painter: PainterT,
    events: Option<EventStream>,
    link: LinkChannel,
}

impl<PainterT: Painter> Crossterm<PainterT> {
    /// Create a new backend instance.
    ///
    /// This method initialises the underlying tty device, enables raw mode,
    /// hides the cursor and enters alternative screen mode. Additionally, an
    /// async event stream with input events from stdin is started.
    pub fn new() -> Result<Self> {
        let mut backend = Self {
            target: MeteredWriter::new(BufWriter::with_capacity(1 << 20, io::stdout())),
            painter: PainterT::create(
                crossterm::terminal::size()
                    .map(|(width, height)| Size::new(width as usize, height as usize))?,
            ),
            events: Some(new_event_stream()),
            link: LinkChannel::new(),
        };
        initialise_tty::<PainterT, _>(&mut backend.target)?;
        Ok(backend)
    }

    /// Starts the event loop. This is the main entry point of a Zi application.
    /// It draws and presents the components to the backend, handles user input
    /// and delivers messages to components. This method returns either when
    /// prompted using the [`exit`](struct.ComponentLink.html#method.exit)
    /// method on [`ComponentLink`](struct.ComponentLink.html) or on error.
    ///
    /// ```no_run
    /// # use zi::prelude::*;
    /// # use zi::components::text::{Text, TextProperties};
    /// fn main() -> zi_crossterm::Result<()> {
    ///     zi_crossterm::incremental()?
    ///         .run_event_loop(Text::with(TextProperties::new().content("Hello, world!")))
    /// }
    /// ```
    pub fn run_event_loop(&mut self, layout: Layout) -> Result<()> {
        let mut tokio_runtime = RuntimeBuilder::new_current_thread().enable_all().build()?;
        let mut app = App::new(
            UnboundedMessageSender(self.link.sender.clone()),
            self.size()?,
            layout,
        );

        loop {
            let canvas = app.draw();

            let last_drawn = Instant::now();
            let num_bytes_presented = self.present(canvas)?;
            let presented_time = last_drawn.elapsed();

            // log::debug!(
            //     "Frame {}: {} comps [{}] draw {:.1}ms pres {:.1}ms diff {}b",
            //     self.runtime.num_frame,
            //     self.components.len(),
            //     statistics,
            //     drawn_time.as_secs_f64() * 1000.0,
            //     presented_time.as_secs_f64() * 1000.0,
            //     num_bytes_presented,
            // );

            log::debug!(
                "Frame: pres {:.1}ms diff {}b",
                presented_time.as_secs_f64() * 1000.0,
                num_bytes_presented,
            );

            self.poll_events_batch(&mut tokio_runtime, &mut app, last_drawn)?;
        }
    }

    /// Suspends the event stream.
    ///
    /// This is used when running something that needs exclusive access to the underlying
    /// terminal (i.e. to stdin and stdout). For example spawning an external editor to collect
    /// or display text. The `resume` function is called upon returning to the application.
    #[inline]
    pub fn suspend(&mut self) -> Result<()> {
        self.events = None;
        Ok(())
    }

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
    #[inline]
    pub fn resume(&mut self) -> Result<()> {
        self.painter = PainterT::create(self.size()?);
        self.events = Some(new_event_stream());
        initialise_tty::<PainterT, _>(&mut self.target)
    }

    /// Poll as many events as we can respecting REDRAW_LATENCY and REDRAW_LATENCY_SUSTAINED_IO
    #[inline]
    fn poll_events_batch(
        &mut self,
        runtime: &mut Runtime,
        app: &mut App,
        last_drawn: Instant,
    ) -> Result<()> {
        let Self {
            ref mut link,
            ref mut events,
            ..
        } = *self;
        let mut force_redraw = false;
        let mut first_event_time: Option<Instant> = None;

        while !force_redraw && !app.poll_state().exit() {
            let timeout_duration = {
                let since_last_drawn = last_drawn.elapsed();
                if app.poll_state().dirty() && since_last_drawn >= REDRAW_LATENCY {
                    Duration::from_millis(0)
                } else if app.poll_state().dirty() {
                    REDRAW_LATENCY - since_last_drawn
                } else {
                    Duration::from_millis(if app.is_tickable() { 60 } else { 240 })
                }
            };
            (runtime.block_on(async {
                tokio::select! {
                    link_message = link.receiver.recv() => {
                        app.handle_message(
                            link_message.expect("at least one sender exists"),
                        );
                        Ok(())
                    }
                    input_event = events.as_mut().expect("backend events are suspended").next() => {
                        match input_event.expect(
                            "at least one sender exists",
                        )? {
                            FilteredEvent::Input(input_event) => app.handle_input(input_event),
                            FilteredEvent::Resize(size) => app.handle_resize(size),
                        };
                        force_redraw = app.poll_state().dirty()
                            && (first_event_time.get_or_insert_with(Instant::now).elapsed()
                                >= SUSTAINED_IO_REDRAW_LATENCY
                                || app.poll_state().resized());
                        Ok(())
                    }
                    _ = tokio::time::sleep(timeout_duration) => {
                        // app.tick();
                        force_redraw = true;
                        Ok(())
                    }
                }
            }) as Result<()>)?;
        }

        Ok(())
    }

    /// Returns the size of the underlying terminal.
    #[inline]
    fn size(&self) -> Result<Size> {
        Ok(crossterm::terminal::size()
            .map(|(width, height)| Size::new(width as usize, height as usize))?)
    }

    /// Draws the [`Canvas`](../terminal/struct.Canvas.html) to the terminal.
    #[inline]
    fn present(&mut self, canvas: &Canvas) -> Result<usize> {
        let Self {
            ref mut target,
            ref mut painter,
            ..
        } = *self;
        let initial_num_bytes_written = target.num_bytes_written();
        painter.paint(canvas, |operation| {
            match operation {
                PaintOperation::WriteContent(grapheme) => {
                    queue!(target, crossterm::style::Print(grapheme))?
                }
                PaintOperation::SetStyle(style) => queue_set_style(target, style)?,
                PaintOperation::MoveTo(position) => queue!(
                    target,
                    crossterm::cursor::MoveTo(position.x as u16, position.y as u16)
                )?, // Go to the begining of line (`MoveTo` uses 0-based indexing)
            }
            Ok(())
        })?;
        target.flush()?;
        Ok(target.num_bytes_written() - initial_num_bytes_written)
    }
}

impl<PainterT: Painter> Drop for Crossterm<PainterT> {
    fn drop(&mut self) {
        queue!(
            self.target,
            crossterm::style::ResetColor,
            crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
            crossterm::cursor::Show,
            crossterm::terminal::LeaveAlternateScreen
        )
        .expect("Failed to clear screen when closing `crossterm` backend.");
        crossterm::terminal::disable_raw_mode()
            .expect("Failed to disable raw mode when closing `crossterm` backend.");
    }
}

const REDRAW_LATENCY: Duration = Duration::from_millis(10);
const SUSTAINED_IO_REDRAW_LATENCY: Duration = Duration::from_millis(100);

struct LinkChannel {
    sender: UnboundedSender<ComponentMessage>,
    receiver: UnboundedReceiver<ComponentMessage>,
}

impl LinkChannel {
    fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self { sender, receiver }
    }
}

#[derive(Debug, Clone)]
struct UnboundedMessageSender(UnboundedSender<ComponentMessage>);

impl MessageSender for UnboundedMessageSender {
    fn send(&self, message: ComponentMessage) {
        self.0
            .send(message)
            .map_err(|_| ()) // tokio's SendError doesn't implement Debug
            .expect("App receiver needs to outlive senders for inter-component messages");
    }

    fn clone_box(&self) -> Box<dyn MessageSender> {
        Box::new(self.clone())
    }
}

#[inline]
fn initialise_tty<PainterT: Painter, TargetT: Write>(target: &mut TargetT) -> Result<()> {
    target
        .queue(crossterm::terminal::EnterAlternateScreen)?
        .queue(crossterm::cursor::Hide)?;
    crossterm::terminal::enable_raw_mode()?;
    queue_set_style(target, &PainterT::INITIAL_STYLE)?;
    target.flush()?;
    Ok(())
}

#[inline]
fn queue_set_style(target: &mut impl Write, style: &Style) -> Result<()> {
    use crossterm::style::{
        Attribute, Color, SetAttribute, SetBackgroundColor, SetForegroundColor,
    };

    // Bold
    if style.bold {
        queue!(target, SetAttribute(Attribute::Bold))?;
    } else {
        // Using Reset is not ideal as it resets all style attributes. The correct thing to do
        // would be to use `NoBold`, but it seems this is not reliably supported (at least it
        // didn't work for me in tmux, although it does in alacritty).
        // Also see https://github.com/crossterm-rs/crossterm/issues/294
        queue!(target, SetAttribute(Attribute::Reset))?;
    }

    // Underline
    if style.underline {
        queue!(target, SetAttribute(Attribute::Underlined))?;
    } else {
        queue!(target, SetAttribute(Attribute::NoUnderline))?;
    }

    // Background
    {
        let Colour { red, green, blue } = style.background;
        queue!(
            target,
            SetBackgroundColor(Color::Rgb {
                r: red,
                g: green,
                b: blue
            })
        )?;
    }

    // Foreground
    {
        let Colour { red, green, blue } = style.foreground;
        queue!(
            target,
            SetForegroundColor(Color::Rgb {
                r: red,
                g: green,
                b: blue
            })
        )?;
    }

    Ok(())
}

enum FilteredEvent {
    Input(zi::terminal::Event),
    Resize(Size),
}

type EventStream = Pin<Box<dyn Stream<Item = Result<FilteredEvent>> + Send + 'static>>;

#[inline]
fn new_event_stream() -> EventStream {
    Box::pin(
        crossterm::event::EventStream::new()
            .filter_map(|event| async move {
                match event {
                    Ok(crossterm::event::Event::Key(key_event)) => Some(Ok(FilteredEvent::Input(
                        zi::terminal::Event::KeyPress(map_key(key_event)),
                    ))),
                    Ok(crossterm::event::Event::Resize(width, height)) => Some(Ok(
                        FilteredEvent::Resize(Size::new(width as usize, height as usize)),
                    )),
                    Ok(_) => None,
                    Err(error) => Some(Err(error.into())),
                }
            })
            .fuse(),
    )
}

#[inline]
fn map_key(key: crossterm::event::KeyEvent) -> Key {
    use crossterm::event::{KeyCode, KeyModifiers};
    match key.code {
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Delete => Key::Delete,
        KeyCode::Insert => Key::Insert,
        KeyCode::F(u8) => Key::F(u8),
        KeyCode::Null => Key::Null,
        KeyCode::Esc => Key::Esc,
        KeyCode::Char(char) if key.modifiers.contains(KeyModifiers::CONTROL) => Key::Ctrl(char),
        KeyCode::Char(char) if key.modifiers.contains(KeyModifiers::ALT) => Key::Alt(char),
        KeyCode::Char(char) => Key::Char(char),
        KeyCode::Enter => Key::Char('\n'),
        KeyCode::Tab => Key::Char('\t'),
    }
}
