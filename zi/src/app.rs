//! The application runtime. This is low-level module useful when you are
//! implementing a backend, but otherwise not meant to be used directly by an
//! end application.

use smallvec::SmallVec;
use std::{collections::HashMap, fmt::Debug, time::Instant};

use crate::{
    component::{
        bindings::{BindingQuery, DynamicBindings, KeySequenceSlice, NamedBindingQuery},
        layout::{LaidCanvas, LaidComponent, Layout},
        template::{ComponentId, DynamicMessage, DynamicProperties, Renderable},
        LinkMessage, ShouldRender,
    },
    terminal::{Canvas, Event, Key, Position, Rect, Size},
};

pub trait MessageSender: Debug + Send + Sync + 'static {
    fn send(&self, message: ComponentMessage);

    fn clone_box(&self) -> Box<dyn MessageSender>;
}

#[derive(Debug)]
pub struct ComponentMessage(pub(crate) LinkMessage);

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PollState {
    Clean,
    Dirty(Option<Size>),
    Exit,
}

#[derive(Debug)]
struct AppRuntime {
    screen: Canvas,
    poll_state: PollState,
    num_frame: usize,
}

impl AppRuntime {
    fn new(size: Size) -> Self {
        Self {
            screen: Canvas::new(size),
            poll_state: PollState::Dirty(None),
            num_frame: 0,
        }
    }
}

/// The application runtime.
///
/// The runtime encapsulates the whole state of the application. It performs
/// layout calculations and draws the component tree to a canvas. The runtime
/// owns all the components and manages their lifetime. It creates components
/// when first mounted and it is responsible for delivering messages, input and
/// layout events.
///
/// The runtime itself does not run an event loop. This is delegated to a
/// particular backend implementation to allow for maximum flexibility. For
/// instance, the `crossterm` terminal backend implements an event loop using
/// tokio channels for component message + mio for stdin input and resize
/// events.
///
/// Note: This is a low-level struct useful when you are implementing a backend
/// or for testing your application. For an end user application, you would
/// normally use a backend that wraps an App in an event loop, see the examples.
pub struct App {
    root: Layout,
    components: HashMap<ComponentId, MountedComponent>,
    layouts: HashMap<ComponentId, Layout>,
    subscriptions: ComponentSubscriptions,
    controller: InputController,
    runtime: AppRuntime,
    sender: Box<dyn MessageSender>,
}

impl App {
    /// Creates a new application runtime
    ///
    /// To instantiate a new runtime you need three things
    ///
    /// - a [`MessageSender`](trait.MessageSender.html) responsible for delivering
    ///   messages sent by components using [`ComponentLink`](../struct.ComponentLink.html)
    /// - the `size` of the initial canvas
    /// - the root [`Layout`](../struct.Layout.html) that will be rendered
    ///
    /// ```no_run
    /// use std::sync::mpsc;
    /// use zi::{
    ///     app::{App, ComponentMessage, MessageSender},
    ///     components::text::{Text, TextProperties},
    ///     prelude::*,
    /// };
    ///
    /// #[derive(Clone, Debug)]
    /// struct MessageQueue(mpsc::Sender<ComponentMessage>);
    ///
    /// impl MessageSender for MessageQueue {
    ///     fn send(&self, message: ComponentMessage) {
    ///         self.0.send(message).unwrap();
    ///     }
    ///
    ///     fn clone_box(&self) -> Box<dyn MessageSender> {
    ///         Box::new(self.clone())
    ///     }
    /// }
    ///
    /// # fn main() {
    /// let (sender, receiver) = mpsc::channel();
    /// let message_queue = MessageQueue(sender);
    /// let mut app = App::new(
    ///     message_queue,
    ///     Size::new(10, 10),
    ///     Text::with(TextProperties::new().content("Hello")),
    /// );
    ///
    /// loop {
    ///     // Deliver component messages. This would block forever as no component
    ///     // sends any messages.
    ///     let message = receiver.recv().unwrap();
    ///
    ///     app.handle_message(message);
    ///     app.handle_resize(Size::new(20, 20));
    ///
    ///     // Draw
    ///     let canvas = app.draw();
    ///     eprintln!("{}", canvas);
    /// }
    /// # }
    /// ```

    pub fn new(sender: impl MessageSender, size: Size, root: Layout) -> Self {
        Self {
            root,
            components: HashMap::new(),
            layouts: HashMap::new(),
            subscriptions: ComponentSubscriptions::new(),
            controller: InputController::new(),
            runtime: AppRuntime::new(size),
            sender: Box::new(sender),
        }
    }

    /// Return the application's poll state
    #[inline]
    pub fn poll_state(&self) -> PollState {
        self.runtime.poll_state
    }

    /// Return `true` if any components currently mounted are tickable
    #[inline]
    pub fn is_tickable(&mut self) -> bool {
        !self.subscriptions.tickable.is_empty()
    }

    /// Resizes the application's canvas lazily
    #[inline]
    pub fn tick(&mut self) {
        for TickSubscription {
            component_id,
            message,
        } in self.subscriptions.tickable.drain(..)
        {
            match self.components.get_mut(&component_id) {
                Some(component) => {
                    if component.update(message) {
                        self.runtime.poll_state.merge(PollState::Dirty(None));
                    }
                }
                None => {
                    log::debug!(
                        "Received message for nonexistent component (id: {}).",
                        component_id,
                    );
                }
            }
        }
    }

    /// Compute component layout and draw the application to a canvas
    ///
    /// This function flushes all pending changes to the component tree,
    /// computes the layout and redraws components where needed. After calling this
    /// function `poll_state()` will be `PollState::Clean`
    #[inline]
    pub fn draw(&mut self) -> &Canvas {
        match self.runtime.poll_state {
            PollState::Dirty(maybe_new_size) => {
                // Draw
                let now = Instant::now();
                if let Some(new_size) = maybe_new_size {
                    log::debug!(
                        "Screen resized {}x{} -> {}x{}",
                        self.runtime.screen.size().width,
                        self.runtime.screen.size().height,
                        new_size.width,
                        new_size.height
                    );
                    self.runtime.screen.resize(new_size);
                }

                let frame = Rect::new(Position::new(0, 0), self.runtime.screen.size());
                let statistics = self.draw_tree(frame, self.runtime.num_frame);
                let drawn_time = now.elapsed();

                // Present
                // let now = Instant::now();
                // let num_bytes_presented = backend.present(&self.runtime.screen)?;
                // let presented_time = now.elapsed();

                log::debug!(
                    "Frame {}: {} comps [{}] draw {:.1}ms",
                    self.runtime.num_frame,
                    self.components.len(),
                    statistics,
                    drawn_time.as_secs_f64() * 1000.0,
                    // presented_time.as_secs_f64() * 1000.0,
                    // num_bytes_presented,
                );
                self.runtime.num_frame += 1;
            }
            PollState::Exit => {
                panic!("tried drawing while the app is exiting");
            }
            _ => {}
        }
        self.runtime.poll_state = PollState::Clean;
        &self.runtime.screen
    }

    /// Resizes the application canvas. This operation is lazy and the mounted
    /// components won't be notified until [`draw`](method.draw.html) is called.
    pub fn handle_resize(&mut self, size: Size) {
        self.runtime.poll_state.merge(PollState::Dirty(Some(size)));
    }

    #[inline]
    pub fn handle_message(&mut self, message: ComponentMessage) {
        match message.0 {
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
                self.runtime.poll_state.merge(if should_render {
                    PollState::Dirty(None)
                } else {
                    PollState::Clean
                });
            }
            LinkMessage::Exit => {
                self.runtime.poll_state.merge(PollState::Exit);
            }
        }
    }

    #[inline]
    pub fn handle_input(&mut self, event: Event) {
        match event {
            Event::KeyPress(key) => {
                self.handle_key(key);
                // todo: handle_event should return whether we need to rerender
                self.runtime.poll_state.merge(PollState::Dirty(None));
            }
        }
    }

    #[inline]
    fn handle_key(&mut self, key: Key) {
        let Self {
            ref mut components,
            ref subscriptions,
            controller: ref mut input_controller,
            ..
        } = *self;
        let mut clear_controller = true;
        let mut binding_queries = SmallVec::<[_; 4]>::with_capacity(subscriptions.focused.len());

        input_controller.push(key);
        for component_id in subscriptions.focused.iter() {
            let focused_component = components
                .get_mut(component_id)
                .expect("focused component to be mounted");

            let binding_query = focused_component
                .bindings
                .keymap()
                .check_sequence(&input_controller.keys);
            binding_queries.push(binding_query.map(|binding_query| {
                NamedBindingQuery::new(focused_component.bindings.keymap(), binding_query)
            }));
            match focused_component
                .bindings
                .keymap()
                .check_sequence(&input_controller.keys)
            {
                Some(BindingQuery::Match(command_id)) => {
                    if let Some(message) = focused_component.renderable.run_command(
                        &focused_component.bindings,
                        *command_id,
                        &input_controller.keys,
                    ) {
                        focused_component.update(message);
                    }
                }
                Some(BindingQuery::PrefixOf(prefix_of)) => {
                    log::info!(
                        "{} ({} commands)",
                        KeySequenceSlice::from(input_controller.keys.as_slice()),
                        prefix_of.len()
                    );
                    clear_controller = false;
                }
                None => {}
            }
        }

        for component_id in subscriptions.notify.iter() {
            let notify_component = components
                .get_mut(component_id)
                .expect("component to be mounted");
            notify_component
                .renderable
                .notify_binding_queries(&binding_queries, &input_controller.keys);
        }

        // If any component returned `BindingTransition::Clear`, we clear the controller.
        if clear_controller {
            input_controller.keys.clear();
        }
    }

    #[inline]
    fn draw_tree(&mut self, frame: Rect, generation: Generation) -> DrawStatistics {
        let Self {
            ref mut components,
            ref mut layouts,
            ref mut runtime,
            ref mut subscriptions,
            ref sender,
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
                        let (renderable, bindings) =
                            template.create(component_id, frame, sender.clone_box());
                        MountedComponent {
                            renderable,
                            frame,
                            bindings,
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

                    component.update_bindings();
                    if component.bindings.focused() {
                        subscriptions.add_focused(component_id);
                    }

                    if component.bindings.notify() {
                        subscriptions.add_notify(component_id);
                    }

                    if let Some(message) = component.tick() {
                        subscriptions.add_tickable(component_id, message);
                    }

                    pending.push((component_id, frame, position_hash));
                },
                &mut |LaidCanvas { frame, canvas, .. }| {
                    runtime.screen.copy_region(canvas, frame);
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
}

struct ComponentSubscriptions {
    focused: SmallVec<[ComponentId; 2]>,
    notify: SmallVec<[ComponentId; 2]>,
    tickable: SmallVec<[TickSubscription; 2]>,
}

impl ComponentSubscriptions {
    fn new() -> Self {
        Self {
            focused: SmallVec::new(),
            notify: SmallVec::new(),
            tickable: SmallVec::new(),
        }
    }

    #[inline]
    fn clear(&mut self) {
        self.focused.clear();
        self.notify.clear();
        self.tickable.clear();
    }

    #[inline]
    fn add_focused(&mut self, component_id: ComponentId) {
        self.focused.push(component_id);
    }

    #[inline]
    fn add_notify(&mut self, component_id: ComponentId) {
        self.notify.push(component_id);
    }

    #[inline]
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

impl PollState {
    pub fn dirty(&self) -> bool {
        matches!(*self, Self::Dirty(_))
    }

    pub fn resized(&self) -> bool {
        matches!(*self, Self::Dirty(Some(_)))
    }

    pub fn exit(&self) -> bool {
        matches!(*self, Self::Exit)
    }

    pub fn merge(&mut self, poll_state: PollState) {
        *self = match (*self, poll_state) {
            (Self::Exit, _) | (_, Self::Exit) => Self::Exit,
            (Self::Clean, other) | (other, Self::Clean) => other,
            (Self::Dirty(_), resized @ Self::Dirty(Some(_))) => resized,
            (resized @ Self::Dirty(Some(_)), Self::Dirty(None)) => resized,
            (Self::Dirty(None), Self::Dirty(None)) => Self::Dirty(None),
        }
    }
}

type Generation = usize;

struct MountedComponent {
    renderable: Box<dyn Renderable>,
    frame: Rect,
    bindings: DynamicBindings,
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
    fn update_bindings(&mut self) {
        self.renderable.bindings(&mut self.bindings)
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
    use std::sync::mpsc;

    use super::*;
    use crate::{
        component::ComponentExt,
        components::text::{Text, TextProperties},
    };

    #[derive(Clone, Debug)]
    struct MessageQueue(mpsc::Sender<ComponentMessage>);

    impl MessageSender for MessageQueue {
        fn send(&self, message: ComponentMessage) {
            self.0.send(message).unwrap();
        }

        fn clone_box(&self) -> Box<dyn MessageSender> {
            Box::new(self.clone())
        }
    }

    impl MessageQueue {
        fn new(sender: mpsc::Sender<ComponentMessage>) -> Self {
            Self(sender)
        }
    }

    #[test]
    fn trivial_message_queue() {
        let (sender, _receiver) = mpsc::channel();
        let message_queue = MessageQueue::new(sender);

        let mut app = App::new(
            message_queue,
            Size::new(10, 10),
            Text::with(TextProperties::new().content("Hello")),
        );

        #[allow(clippy::never_loop)]
        loop {
            // Deliver component messages. This would block forever as no component
            // sends any messages.
            // let message = receiver
            //     .recv_timeout(Duration::new(1, 0))
            //     .expect_err("received an unexpected component message");

            // app.handle_message(message);
            app.handle_resize(Size::new(20, 20));

            // Draw
            let canvas = app.draw();
            eprintln!("{}", canvas);

            break;
        }
    }

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
