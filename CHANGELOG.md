# Unreleased

# v0.3.1
 - Implement `BitOr` for `ShouldRender`

# v0.3.0
## Breaking

 - Replaced input handling with a new declarative system for specifying key
   bindings and how to react in response to input events. There's a new
   `Component` lifecycle method `bindings()` which replaces the old `input_binding`
   and `has_focus` methods. The newly introduced `Bindings` type allows
   registering handlers which will run in response to key patterns.
 - Enable support for animated components in zi-term (crossterm backend)
 - A new experimental notification api for binding queries
 - Fix trying to draw while the app is exiting
 - Upgrade all dependencies of zi and zi-term to latest available


# v0.2.0
## Breaking

 - Simplifies the public layout API functions and dealing with array of
   components (latter thanks to const generics). In particular:
   - The free functions `row`, `row_reverse`, `column`, `column_reverse`,
     `container` and their iterator versions have been replaced with more
     flexible methods on the `Layout`, `Container` and `Item` structs.
   - `layout::component` and `layout::component_with_*` have also been removed
     in favour of utility methods on the extension trait `CompoentExt`. This is
     automatically implemented by all components.
 - Moves the responsibility of running the event loop from the `App` struct and
   into the backend. This inversion of control help reduce sys dependencies in
   `zi::App` (moves tokio dependency to the `crossterm` backend which was moved to
   a separate crate, yay). This change allows for different implementations of
   the event loop (e.g. using winit which will come in handy for a new
   experimental wgpu backend).
