//! Defines the `Component` trait and related types.

pub mod layout;
pub(crate) mod template;

pub use self::layout::{ComponentExt, Layout};

use smallvec::SmallVec;
use std::{
    any::{self, TypeId},
    cmp::Ordering,
    collections::hash_map::HashMap,
    fmt,
    marker::PhantomData,
    rc::Rc,
};
use tokio::sync::mpsc::UnboundedSender;

use self::template::{ComponentId, DynamicMessage};
use crate::terminal::{Key, Rect};

/// Components are the building blocks of the UI in Zi.
///
/// The trait describes stateful components and their lifecycle. This is the
/// main trait that users of the library will implement to describe their UI.
/// All components are owned directly by an [`App`](../struct.App.html) which
/// manages their lifecycle. An `App` instance will create new components,
/// update them in reaction to user input or to messages from other components
/// and eventually drop them when a component gone off screen.
///
/// Anyone familiar with Yew, Elm or React + Redux should be familiar with all
/// the high-level concepts. Moreover, the names of some types and functions are
/// the same as in `Yew`.
///
/// A component has to describe how:
///   - how to create a fresh instance from `Component::Properties` received from their parent (`create` fn)
///   - how to render it (`view` fn)
///   - how to update its inter
///
pub trait Component: Sized + 'static {
    /// Messages are used to make components dynamic and interactive. For simple
    /// components, this will be `()`. Complex ones will typically use
    /// an enum to declare multiple Message types.
    type Message: Send + 'static;

    /// Properties are the inputs to a Component.
    type Properties: Clone;

    /// Components are created with three pieces of data:
    ///   - their Properties
    ///   - the current position and size on the screen
    ///   - a `ComponentLink` which can be used to send messages and create callbacks for triggering updates
    ///
    /// Conceptually, there's an "update" method for each one of these:
    ///   - `change` when the Properties change
    ///   - `resize` when their current position and size on the screen changes
    ///   - `update` when the a message was sent to the component
    fn create(properties: Self::Properties, frame: Rect, link: ComponentLink<Self>) -> Self;

    /// Returns the current visual layout of the component.
    fn view(&self) -> Layout;

    /// When the parent of a Component is re-rendered, it will either be re-created or
    /// receive new properties in the `change` lifecycle method. Component's can choose
    /// to re-render if the new properties are different than the previously
    /// received properties.
    ///
    /// Root components don't have a parent and subsequently, their `change`
    /// method will never be called. Components which don't have properties
    /// should always return false.
    fn change(&mut self, _properties: Self::Properties) -> ShouldRender {
        ShouldRender::No
    }

    /// This method is called when a component's position and size on the screen changes.
    fn resize(&mut self, _frame: Rect) -> ShouldRender {
        ShouldRender::No
    }

    /// Components handle messages in their `update` method and commonly use this method
    /// to update their state and (optionally) re-render themselves.
    fn update(&mut self, _message: Self::Message) -> ShouldRender {
        ShouldRender::No
    }

    /// Whether the component is currently focused.
    fn has_focus(&self) -> bool {
        false
    }

    /// If the component is currently focused (see `has_focus`), `input_binding`
    /// will be called on every keyboard events.
    fn input_binding(&self, _pressed: &[Key]) -> BindingMatch<Self::Message> {
        BindingMatch {
            transition: BindingTransition::Clear,
            message: None,
        }
    }

    fn tick(&self) -> Option<Self::Message> {
        None
    }
}

/// Callback wrapper. Useful for passing callbacks in child components
/// `Properties`. An `Rc` wrapper is used to make it cloneable.
pub struct Callback<InputT, OutputT = ()>(pub Rc<dyn Fn(InputT) -> OutputT>);

impl<InputT, OutputT> Clone for Callback<InputT, OutputT> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<InputT, OutputT> fmt::Debug for Callback<InputT, OutputT> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Callback({} -> {} @ {:?})",
            any::type_name::<InputT>(),
            any::type_name::<OutputT>(),
            Rc::as_ptr(&self.0)
        )
    }
}

impl<InputT, OutputT> PartialEq for Callback<InputT, OutputT> {
    fn eq(&self, other: &Self) -> bool {
        // `Callback` is a fat pointer: vtable address + data address. We're
        // only comparing the pointers to the data portion for equality.
        //
        // This could fail if some of your objects can have the same address but
        // different concrete types, for example if one is stored in a field of
        // another, or if they are different zero-sized types.
        //
        // Comparing vtable addresses doesn't work either as "vtable addresses
        // are not guaranteed to be unique and could vary between different code
        // generation units. Furthermore vtables for different types could have
        // the same address after being merged together".
        //
        // References
        //  - https://rust-lang.github.io/rust-clippy/master/index.html#vtable_address_comparisons
        //  - https://users.rust-lang.org/t/rc-dyn-trait-ptr-equality
        std::ptr::eq(
            self.0.as_ref() as *const _ as *const (),
            other.0.as_ref() as *const _ as *const (),
        )
    }
}

impl<InputT, OutputT> Callback<InputT, OutputT> {
    pub fn emit(&self, value: InputT) -> OutputT {
        (self.0)(value)
    }
}

impl<InputT, OutputT, FnT> From<FnT> for Callback<InputT, OutputT>
where
    FnT: Fn(InputT) -> OutputT + 'static,
{
    fn from(function: FnT) -> Self {
        Self(Rc::new(function))
    }
}

/// A context for sending messages to a component or the runtime.
///
/// It can be used in a multi-threaded environment (implements `Sync` and
/// `Send`). Additionally, it can send messages to the runtime, in particular
/// it's used to gracefully stop a running [`App`](struct.App.html).
#[derive(Debug)]
pub struct ComponentLink<ComponentT> {
    sender: UnboundedSender<LinkMessage>,
    component_id: ComponentId,
    _component: PhantomData<fn() -> ComponentT>,
}

impl<ComponentT: Component> ComponentLink<ComponentT> {
    /// Sends a message to the component.
    pub fn send(&self, message: ComponentT::Message) {
        self.sender
            .send(LinkMessage::Component(
                self.component_id,
                DynamicMessage(Box::new(message)),
            ))
            .map_err(|_| ()) // tokio's SendError doesn't implement Debug
            .expect("App receiver needs to outlive senders for inter-component messages");
    }

    /// Creates a `Callback` which will send a message to the linked component's
    /// update method when invoked.
    pub fn callback<InputT>(
        &self,
        callback: impl Fn(InputT) -> ComponentT::Message + 'static,
    ) -> Callback<InputT> {
        let link = self.clone();
        Callback(Rc::new(move |input| link.send(callback(input))))
    }

    /// Sends a message to the `App` runtime requesting it to stop executing.
    ///
    /// This method only sends a message and returns immediately, the app will
    /// stop asynchronously and may deliver other pending messages before
    /// exiting.
    pub fn exit(&self) {
        self.sender
            .send(LinkMessage::Exit)
            .map_err(|_| ()) // tokio's SendError doesn't implement Debug
            .expect("App needs to outlive components");
    }

    /// Runs a closure that requires exclusive access to the backend (i.e.
    /// typically to stdin / stdout). For example spawning an external editor to
    /// collect some text.
    pub fn run_exclusive(
        &self,
        process: impl FnOnce() -> Option<ComponentT::Message> + Send + 'static,
    ) {
        let component_id = self.component_id;
        self.sender
            .send(LinkMessage::RunExclusive(Box::new(move || {
                process().map(|message| (component_id, DynamicMessage(Box::new(message))))
            })))
            .map_err(|_| ()) // tokio's SendError doesn't implement Debug
            .expect("App needs to outlive components");
    }

    pub(crate) fn new(sender: UnboundedSender<LinkMessage>, component_id: ComponentId) -> Self {
        assert_eq!(TypeId::of::<ComponentT>(), component_id.type_id());
        Self {
            sender,
            component_id,
            _component: PhantomData,
        }
    }
}

impl<ComponentT> Clone for ComponentLink<ComponentT> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            component_id: self.component_id,
            _component: PhantomData,
        }
    }
}

/// Type to indicate whether a component should be rendered again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShouldRender {
    Yes,
    No,
}

impl From<ShouldRender> for bool {
    fn from(should_render: ShouldRender) -> Self {
        matches!(should_render, ShouldRender::Yes)
    }
}

impl From<bool> for ShouldRender {
    fn from(should_render: bool) -> Self {
        if should_render {
            ShouldRender::Yes
        } else {
            ShouldRender::No
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingTransition {
    Continue,
    Clear,
    ChangedFocus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingMatch<Message> {
    pub transition: BindingTransition,
    pub message: Option<Message>,
}

impl<Message> BindingMatch<Message> {
    pub fn clear(message: impl Into<Option<Message>>) -> Self {
        Self {
            transition: BindingTransition::Clear,
            message: message.into(),
        }
    }
}

pub(crate) enum LinkMessage {
    Component(ComponentId, DynamicMessage),
    Exit,
    RunExclusive(Box<dyn FnOnce() -> Option<(ComponentId, DynamicMessage)> + Send + 'static>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct _HashBindings<Action>(HashMap<SmallVec<[Key; 2]>, Action>);

impl<Message> _HashBindings<Message> {
    pub fn _new(map: HashMap<SmallVec<[Key; 2]>, Message>) -> Self {
        Self(map)
    }
}

impl<Message: Clone> _HashBindings<Message> {
    pub fn _input_binding(&self, pressed: &[Key]) -> BindingMatch<Message> {
        for (binding, message) in self.0.iter() {
            let is_match = binding
                .iter()
                .zip(pressed.iter())
                .all(|(lhs, rhs)| *lhs == *rhs);
            if is_match {
                match pressed.len().cmp(&binding.len()) {
                    Ordering::Less => {
                        return BindingMatch {
                            transition: BindingTransition::Continue,
                            message: None,
                        };
                    }
                    Ordering::Equal => {
                        return BindingMatch {
                            transition: BindingTransition::Clear,
                            message: Some(message.clone()),
                        }
                    }
                    _ => {}
                }
            }
        }
        BindingMatch {
            transition: BindingTransition::Clear,
            message: None,
        }
    }
}
