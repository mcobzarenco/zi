//! The `App` application runtime, which runs the event loop and draws your
//! components.

use futures::{self, stream::StreamExt};
use smallvec::SmallVec;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::{
    self,
    runtime::{Builder as RuntimeBuilder, Runtime},
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
};

use crate::{
    component::{
        layout::{LaidCanvas, LaidComponent, Layout},
        template::{ComponentId, DynamicMessage, DynamicProperties, Renderable, Template},
        BindingMatch, BindingTransition, LinkMessage, ShouldRender,
    },
    error::Result,
    frontend::{Event, Frontend},
    terminal::{Canvas, Key, Position, Rect, Size},
};

/// The `App` application runtime, which runs the event loop and draws your
/// components.
pub struct App {
    root: Layout,
    components: HashMap<ComponentId, MountedComponent>,
    layouts: HashMap<ComponentId, Layout>,
    subscriptions: ComponentSubscriptions,
    controller: InputController,
    link: LinkChannel,
}

impl App {
    /// Creates a new application runtime. You should provide an initial layout
    /// containing the root components.
    ///
    /// ```
    /// # use zi::prelude::*;
    /// use zi::components::text::{Text, TextProperties};
    ///
    /// let mut app = App::new(layout::component::<Text>(
    ///    TextProperties::new().content("Hello, world!"),
    /// ));
    /// ```
    pub fn new(root: Layout) -> Self {
        Self {
            components: HashMap::new(),
            layouts: HashMap::new(),
            subscriptions: ComponentSubscriptions::new(),
            controller: InputController::new(),
            link: LinkChannel::new(),
            root,
        }
    }

    /// Starts the event loop. This is the main entry point of a Zi application.
    /// It draws and presents the components to the backend, handles user input
    /// and delivers messages to components. This method returns either when
    /// prompted using the [`exit`](struct.ComponentLink.html#method.exit)
    /// method on [ComponentLink](struct.ComponentLink.html) or on error.
    ///
    /// ```no_run
    /// # use zi::prelude::*;
    /// # use zi::components::text::{Text, TextProperties};
    /// # fn main() -> zi::Result<()> {
    /// # let mut app = App::new(layout::component::<Text>(
    /// #     TextProperties::new().content("Hello, world!"),
    /// # ));
    /// app.run_event_loop(zi::frontend::default()?)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn run_event_loop(&mut self, mut frontend: impl Frontend) -> Result<()> {
        let mut screen = Canvas::new(frontend.size()?);
        let mut poll_state = PollState::Dirty(None);
        let mut last_drawn = Instant::now() - REDRAW_LATENCY;
        let mut runtime = RuntimeBuilder::new()
            .basic_scheduler()
            .enable_all()
            .build()?;
        let mut num_frame = 0;

        loop {
            match poll_state {
                PollState::Dirty(new_size) => {
                    let now = Instant::now();

                    // Draw
                    if let Some(screen_size) = new_size {
                        log::debug!("Screen resized {} -> {}", screen.size(), screen_size);
                        screen.resize(screen_size);
                    }

                    let frame = Rect::new(Position::new(0, 0), screen.size());
                    let statistics = self.draw(&mut screen, frame, num_frame);
                    let drawn_time = now.elapsed();

                    // Present
                    let now = Instant::now();
                    let num_bytes_presented = frontend.present(&screen)?;

                    log::debug!(
                        "Frame {}: {} comps [{}] draw {:.1}ms pres {:.1}ms diff {}b",
                        num_frame,
                        self.components.len(),
                        statistics,
                        drawn_time.as_secs_f64() * 1000.0,
                        now.elapsed().as_secs_f64() * 1000.0,
                        num_bytes_presented,
                    );
                    last_drawn = Instant::now();
                    num_frame += 1;
                }
                PollState::Exit => {
                    break;
                }
                _ => {}
            }

            poll_state = self.poll_events_batch(&mut runtime, &mut frontend, last_drawn)?;
        }

        Ok(())
    }

    #[inline]
    fn draw(&mut self, screen: &mut Canvas, frame: Rect, generation: Generation) -> DrawStatistics {
        let Self {
            ref mut components,
            ref mut layouts,
            ref mut subscriptions,
            ref link,
            ..
        } = *self;

        subscriptions.clear();

        let mut first = true;
        let mut pending = Vec::new();
        let mut statistics = DrawStatistics::default();
        loop {
            let (layout, frame2, position_hash, parent_changed) = if first {
                first = false;
                (&mut self.root, frame, 0, false)
            } else if let Some((component_id, frame, position_hash)) = pending.pop() {
                let component = components
                    .get_mut(&component_id)
                    .expect("Layout is cached only for mounted components");
                let layout = layouts
                    .entry(component_id)
                    .or_insert_with(|| component.view());
                let changed = component.should_render;
                if changed {
                    *layout = component.view()
                }
                component.set_generation(generation);
                (layout, frame, position_hash, changed)
            } else {
                break;
            };

            layout.0.crawl(
                frame2,
                position_hash,
                &mut |LaidComponent {
                          frame,
                          position_hash,
                          template,
                      }| {
                    let component_id = template.generate_id(position_hash);
                    let mut new_component = false;
                    let component = components.entry(component_id).or_insert_with(|| {
                        new_component = true;
                        let renderable = template.create(component_id, frame, link.sender.clone());
                        MountedComponent {
                            renderable,
                            frame,
                            should_render: ShouldRender::Yes.into(),
                            generation,
                        }
                    });

                    if !new_component {
                        let mut changed =
                            parent_changed && component.change(template.dynamic_properties());
                        if frame != component.frame {
                            changed = component.resize(frame) || changed;
                        }
                        if changed {
                            statistics.changed += 1;
                        } else {
                            statistics.nop += 1;
                        }
                    } else {
                        statistics.new += 1;
                    }

                    if component.has_focus() {
                        subscriptions.add_focused(component_id);
                    }

                    if let Some(message) = component.tick() {
                        subscriptions.add_tickable(component_id, message);
                    }

                    // log::debug!(
                    //     "should_render={} new={} parent_changed={} [{} at {}]",
                    //     component.should_render,
                    //     new_component,
                    //     parent_changed,
                    //     component_id,
                    //     frame,
                    // );

                    pending.push((component_id, frame, position_hash));
                },
                &mut |LaidCanvas { frame, canvas, .. }| {
                    screen.copy_region(&canvas, frame);
                },
            );
        }

        // Drop components that are not part of the current layout tree, i.e. do
        // not appear on the screen.
        components.retain(
            |component_id,
             &mut MountedComponent {
                 generation: component_generation,
                 ..
             }| {
                if component_generation < generation {
                    statistics.deleted += 1;
                    layouts.remove(component_id);
                    false
                } else {
                    true
                }
            },
        );

        statistics
    }

    /// Poll as many events as we can respecting REDRAW_LATENCY and REDRAW_LATENCY_SUSTAINED_IO
    #[inline]
    fn poll_events_batch(
        &mut self,
        runtime: &mut Runtime,
        frontend: &mut impl Frontend,
        last_drawn: Instant,
    ) -> Result<PollState> {
        let mut force_redraw = false;
        let mut first_event_time: Option<Instant> = None;
        let mut poll_state = PollState::Clean;

        while !force_redraw && !poll_state.exit() {
            let timeout_duration = {
                let since_last_drawn = last_drawn.elapsed();
                if poll_state.dirty() && since_last_drawn >= REDRAW_LATENCY {
                    Duration::from_millis(0)
                } else if poll_state.dirty() {
                    REDRAW_LATENCY - since_last_drawn
                } else {
                    Duration::from_millis(if self.subscriptions.tickable.is_empty() {
                        240
                    } else {
                        60
                    })
                }
            };
            (runtime.block_on(async {
                tokio::select! {
                    link_message = self.link.receiver.recv() => {
                        poll_state = self.handle_link_message(
                            frontend,
                            link_message.expect("At least one sender exists."),
                        )?;
                        Ok(())
                    }
                    input_event = frontend.event_stream().next() => {
                        poll_state = self.handle_input_event(input_event.expect(
                            "At least one sender exists.",
                        )?)?;
                        force_redraw = poll_state.dirty()
                            && (first_event_time.get_or_insert_with(Instant::now).elapsed()
                                >= SUSTAINED_IO_REDRAW_LATENCY
                                || poll_state.resized());
                        Ok(())
                    }
                    _ = tokio::time::delay_for(timeout_duration) => {
                        for TickSubscription {
                            component_id,
                            message,
                        } in self.subscriptions.tickable.drain(..)
                        {
                            poll_state = PollState::Dirty(None);
                            match self.components.get_mut(&component_id) {
                                Some(component) => {
                                    component.update(message);
                                }
                                None => {
                                    log::debug!(
                                        "Received message for nonexistent component (id: {}).",
                                        component_id,
                                    );
                                }
                            }
                        }
                        force_redraw = true;
                        Ok(())
                    }
                }
            }) as Result<()>)?;
        }

        Ok(poll_state)
    }

    #[inline]
    fn handle_link_message(
        &mut self,
        frontend: &mut impl Frontend,
        message: LinkMessage,
    ) -> Result<PollState> {
        Ok(match message {
            LinkMessage::Component(component_id, dyn_message) => {
                let should_render = self
                    .components
                    .get_mut(&component_id)
                    .map(|component| component.update(dyn_message))
                    .unwrap_or_else(|| {
                        log::debug!(
                            "Received message for nonexistent component (id: {}).",
                            component_id,
                        );
                        false
                    });
                if should_render {
                    PollState::Dirty(None)
                } else {
                    PollState::Clean
                }
            }
            LinkMessage::Exit => PollState::Exit,
            LinkMessage::RunExclusive(process) => {
                frontend.suspend()?;
                let maybe_message = process();
                frontend.resume()?;
                // force_redraw = true;
                if let Some((component_id, dyn_message)) = maybe_message {
                    self.components
                        .get_mut(&component_id)
                        .map(|component| component.update(dyn_message))
                        .unwrap_or_else(|| {
                            log::debug!(
                                "Received message for nonexistent component (id: {}).",
                                component_id,
                            );
                            false
                        });
                }
                PollState::Dirty(None)
            }
        })
    }

    #[inline]
    fn handle_input_event(&mut self, event: Event) -> Result<PollState> {
        Ok(match event {
            Event::Key(key) => {
                self.handle_key(key)?;
                PollState::Dirty(None) // handle_event should return whether we need to rerender
            }
            Event::Resize(size) => PollState::Dirty(Some(size)),
        })
    }

    #[inline]
    fn handle_key(&mut self, key: Key) -> Result<()> {
        let Self {
            ref mut components,
            ref subscriptions,
            ref mut controller,
            ..
        } = *self;
        let mut clear_controller = false;
        let mut changed_focus = false;

        controller.push(key);
        for component_id in subscriptions.focused.iter() {
            let focused_component = components
                .get_mut(component_id)
                .expect("A focused component should be mounted.");
            let binding = focused_component.input_binding(&controller.keys);
            match binding.transition {
                BindingTransition::Continue => {}
                BindingTransition::Clear => {
                    clear_controller = true;
                }
                BindingTransition::ChangedFocus => {
                    changed_focus = true;
                }
            }
            if let Some(message) = binding.message {
                focused_component.update(message);
            }

            // If the focus has changed we don't notify other focused components
            // deeper in the tree.
            if changed_focus {
                controller.keys.clear();
                return Ok(());
            }
        }

        // If any component returned `BindingTransition::Clear`, we clear the controller.
        if clear_controller {
            controller.keys.clear();
        }

        Ok(())
    }
}

struct LinkChannel {
    sender: UnboundedSender<LinkMessage>,
    receiver: UnboundedReceiver<LinkMessage>,
}

impl LinkChannel {
    fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self { sender, receiver }
    }
}

struct ComponentSubscriptions {
    focused: SmallVec<[ComponentId; 2]>,
    tickable: SmallVec<[TickSubscription; 2]>,
}

impl ComponentSubscriptions {
    fn new() -> Self {
        Self {
            focused: SmallVec::new(),
            tickable: SmallVec::new(),
        }
    }

    #[inline]
    fn clear(&mut self) {
        self.focused.clear();
        self.tickable.clear();
    }

    fn add_focused(&mut self, component_id: ComponentId) {
        self.focused.push(component_id);
    }

    fn add_tickable(&mut self, component_id: ComponentId, message: DynamicMessage) {
        self.tickable.push(TickSubscription {
            component_id,
            message,
        });
    }
}

struct TickSubscription {
    component_id: ComponentId,
    message: DynamicMessage,
}

#[derive(Debug, PartialEq)]
enum PollState {
    Clean,
    Dirty(Option<Size>),
    Exit,
}

impl PollState {
    fn dirty(&self) -> bool {
        match *self {
            Self::Dirty(_) => true,
            _ => false,
        }
    }

    fn resized(&self) -> bool {
        match *self {
            Self::Dirty(Some(_)) => true,
            _ => false,
        }
    }

    fn exit(&self) -> bool {
        Self::Exit == *self
    }
}

type Generation = usize;

struct MountedComponent {
    renderable: Box<dyn Renderable>,
    frame: Rect,
    generation: Generation,
    should_render: bool,
}

impl MountedComponent {
    #[inline]
    fn change(&mut self, properties: DynamicProperties) -> bool {
        self.should_render = self.renderable.change(properties).into() || self.should_render;
        self.should_render
    }

    #[inline]
    fn resize(&mut self, frame: Rect) -> bool {
        self.should_render = self.renderable.resize(frame).into() || self.should_render;
        self.frame = frame;
        self.should_render
    }

    #[inline]
    fn update(&mut self, message: DynamicMessage) -> bool {
        self.should_render = self.renderable.update(message).into() || self.should_render;
        self.should_render
    }

    #[inline]
    fn view(&mut self) -> Layout {
        self.should_render = false;
        self.renderable.view()
    }

    #[inline]
    fn has_focus(&self) -> bool {
        self.renderable.has_focus()
    }

    #[inline]
    fn input_binding(&self, pressed: &[Key]) -> BindingMatch<DynamicMessage> {
        self.renderable.input_binding(pressed)
    }

    #[inline]
    fn tick(&self) -> Option<DynamicMessage> {
        self.renderable.tick()
    }

    #[inline]
    fn set_generation(&mut self, generation: Generation) {
        self.generation = generation;
    }
}

struct InputController {
    keys: SmallVec<[Key; 8]>,
}

impl InputController {
    fn new() -> Self {
        Self {
            keys: SmallVec::new(),
        }
    }

    fn push(&mut self, key: Key) {
        self.keys.push(key);
    }
}

impl std::fmt::Display for InputController {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::result::Result<(), std::fmt::Error> {
        for key in self.keys.iter() {
            match key {
                Key::Char(' ') => write!(formatter, "SPC ")?,
                Key::Char('\n') => write!(formatter, "RET ")?,
                Key::Char('\t') => write!(formatter, "TAB ")?,
                Key::Char(char) => write!(formatter, "{} ", char)?,
                Key::Ctrl(char) => write!(formatter, "C-{} ", char)?,
                Key::Alt(char) => write!(formatter, "A-{} ", char)?,
                Key::F(number) => write!(formatter, "F{} ", number)?,
                Key::Esc => write!(formatter, "ESC ")?,
                key => write!(formatter, "{:?} ", key)?,
            }
        }
        Ok(())
    }
}

const REDRAW_LATENCY: Duration = Duration::from_millis(10);
const SUSTAINED_IO_REDRAW_LATENCY: Duration = Duration::from_millis(100);

#[derive(Default)]
struct DrawStatistics {
    new: usize,
    changed: usize,
    deleted: usize,
    nop: usize,
}

impl std::fmt::Display for DrawStatistics {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            formatter,
            "{} new {} upd {} del {} nop",
            self.new, self.changed, self.deleted, self.nop
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{ComponentId, DynamicMessage, LinkMessage};

    #[test]
    fn sizes() {
        eprintln!(
            "std::mem::size_of::<(ComponentId, DynamicMessage)>() == {}",
            std::mem::size_of::<(ComponentId, DynamicMessage)>()
        );
        eprintln!(
            "std::mem::size_of::<LinkMessage>() == {}",
            std::mem::size_of::<LinkMessage>()
        );
    }
}
