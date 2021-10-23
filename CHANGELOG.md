# Unreleased
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
