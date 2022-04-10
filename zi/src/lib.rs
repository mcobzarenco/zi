//! Zi is a library for building modern terminal user interfaces.
//!
//! A user interface in Zi is built as a tree of stateful components. Components
//! let you split the UI into independent, reusable pieces, and think about each
//! piece in isolation.
//!
//! The [`App`](app/struct.App.html) runtime keeps track of components as they are
//! mounted, updated and eventually removed and only calls `view()` on those UI
//! components that have changed and have to be re-rendered. Lower level and
//! independent of the components, the terminal backend will incrementally
//! redraw only those parts of the screen that have changed.
//!
//!
//! # A Basic Example
//!
//! The following is a complete example of a Zi application which implements a
//! counter. It should provide a good sample of the different
//! [`Component`](trait.Component.html) methods and how they fit together.
//!
//! A slightly more complex version which includes styling can be found at
//! `examples/counter.rs`.
//!
//! ![zi-counter-example](https://user-images.githubusercontent.com/797170/137802270-0a4a50af-1fd5-473f-a52c-9d3a107809d0.gif)
//!
//! Anyone familiar with Yew, Elm or React + Redux should be familiar with all
//! the high-level concepts. Moreover, the names of some types and functions are
//! the same as in `Yew`.
//!
//! ```ignore
//! use zi::{
//!     components::{
//!         border::{Border, BorderProperties},
//!         text::{Text, TextAlign, TextProperties},
//!     },
//!     prelude::*,
//! };
//! use zi_term::Result;
//!
//!
//! // Message type handled by the `Counter` component.
//! enum Message {
//!     Increment,
//!     Decrement,
//! }
//!
//! // The `Counter` component.
//! struct Counter {
//!     // The state of the component -- the current value of the counter.
//!     count: usize,
//!
//!     // A `ComponentLink` allows us to send messages to the component in reaction
//!     // to user input as well as to gracefully exit.
//!     link: ComponentLink<Self>,
//! }
//!
//! // Components implement the `Component` trait and are the building blocks of the
//! // UI in Zi. The trait describes stateful components and their lifecycle.
//! impl Component for Counter {
//!     // Messages are used to make components dynamic and interactive. For simple
//!     // or pure components, this will be `()`. Complex, stateful components will
//!     // typically use an enum to declare multiple Message types. In this case, we
//!     // will emit two kinds of message (`Increment` or `Decrement`) in reaction
//!     // to user input.
//!     type Message = Message;
//!
//!     // Properties are the inputs to a Component passed in by their parent.
//!     type Properties = ();
//!
//!     // Creates ("mounts") a new `Counter` component.
//!     fn create(
//!         _properties: Self::Properties,
//!         _frame: Rect,
//!         link: ComponentLink<Self>,
//!     ) -> Self {
//!         Self { count: 0, link }
//!     }
//!
//!     // Returns the current visual layout of the component.
//!     //  - The `Border` component wraps a component and draws a border around it.
//!     //  - The `Text` component displays some text.
//!     fn view(&self) -> Layout {
//!         Border::with(BorderProperties::new(Text::with(
//!             TextProperties::new()
//!                 .align(TextAlign::Centre)
//!                 .content(format!("Counter: {}", self.count)),
//!         )))
//!     }
//!
//!     // Components handle messages in their `update` method and commonly use this
//!     // method to update their state and (optionally) re-render themselves.
//!     fn update(&mut self, message: Self::Message) -> ShouldRender {
//!         self.count = match message {
//!             Message::Increment => self.count.saturating_add(1),
//!             Message::Decrement => self.count.saturating_sub(1),
//!         };
//!         ShouldRender::Yes
//!     }
//!
//!    // Updates the key bindings of the component.
//!    //
//!    // This method will be called after the component lifecycle methods. It is
//!    // used to specify how to react in response to keyboard events, typically
//!    // by sending a message.
//!    fn bindings(&self, bindings: &mut Bindings<Self>) {
//!        // If we already initialised the bindings, nothing to do -- they never
//!        // change in this example
//!        if !bindings.is_empty() {
//!            return;
//!        }
//!        // Set focus to `true` in order to react to key presses
//!        bindings.set_focus(true);
//!
//!        // Increment
//!        bindings.add("increment", [Key::Char('+')], || Message::Increment);
//!        bindings.add("increment", [Key::Char('=')], || Message::Increment);
//!
//!        // Decrement
//!        bindings.add("decrement", [Key::Char('-')], || Message::Decrement);
//!
//!        // Exit
//!        bindings.add("exit", [Key::Ctrl('c')], |this: &Self| this.link.exit());
//!        bindings.add("exit", [Key::Esc], |this: &Self| this.link.exit());
//!    }
//! }
//!
//! fn main() -> zi_term::Result<()> {
//!   zi_term::incremental()?.run_event_loop(Counter::with(()))
//! }
//! ```
//!
//! More examples can be found in the `examples` directory of the git
//! repository.

pub mod app;
pub mod components;
pub mod terminal;

pub use component::{
    bindings::{AnyCharacter, BindingQuery, Bindings, EndsWith, Keymap, NamedBindingQuery},
    layout::{self, ComponentExt, ComponentKey, Container, FlexBasis, FlexDirection, Item},
    Callback, Component, ComponentLink, Layout, ShouldRender,
};
pub use terminal::{Background, Canvas, Colour, Foreground, Key, Position, Rect, Size, Style};

pub mod prelude {
    //! The Zi prelude.
    pub use super::{
        AnyCharacter, Bindings, Component, ComponentExt, ComponentLink, Container, FlexBasis,
        FlexDirection, Item, Layout, ShouldRender,
    };
    pub use super::{Background, Canvas, Colour, Foreground, Key, Position, Rect, Size, Style};
}

// Re-export 3rd party libraries to do with unicode segmentation.
//
// It is helpful to ensure downstream use the same version as zi to ensure any code relying on
// unicode segmentation matches what zi does internally.
pub use unicode_segmentation;
pub use unicode_width;

// Crate only modules
pub(crate) mod component;
pub(crate) mod text;
