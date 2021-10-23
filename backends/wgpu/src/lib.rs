//! A graphical backend for [Zi](https://docs.rs/zi) using
//! [wgpu](https://docs.rs/wgpu) and [winit](https://docs.rs/winit).
//!
//! # Getting Started
//!
//!
//!

#![allow(clippy::float_cmp)]

mod error;
mod font_rasterizer;
mod input;
mod state;
mod texture;

pub use winit::{self, dpi::PhysicalSize, window::WindowBuilder};

pub use crate::error::{Error, Result};

use crossfont::Size as FontSize;
use winit::{
    event::{Event, ModifiersState, VirtualKeyCode},
    event_loop::{ControlFlow, EventLoop, EventLoopProxy},
    window::Window,
};
use zi::{
    app::{App, ComponentMessage, MessageSender},
    Layout, Size,
};

use crate::state::GpuState;

/// A GPU accelerated Zi backend using winit and wgpu
pub struct GpuBackend {
    window: Window,
    event_loop: EventLoop<ComponentMessage>,
    gpu_state: GpuState,
}

impl GpuBackend {
    /// Create a new backend instance.
    ///
    /// This method initialises the underlying window and GPU state.
    pub fn new(builder: WindowBuilder) -> Result<Self> {
        let event_loop = EventLoop::with_user_event();
        let window = builder.build(&event_loop)?;
        let gpu_state = futures::executor::block_on(GpuState::new(&window))?;

        Ok(Self {
            window,
            event_loop,
            gpu_state,
        })
    }

    // pub fn new(title: &str) -> Result<Self> {
    //     let event_loop = EventLoop::with_user_event();
    //     let window = WindowBuilder::new()
    //         .with_decorations(true)
    //         .with_inner_size(PhysicalSize {
    //             width: 1280,
    //             height: 1024,
    //         })
    //         .with_resizable(true)
    //         .with_title(title)
    //         .build(&event_loop)?;
    //     let gpu_state = futures::executor::block_on(GpuState::new(&window))?;

    //     Ok(Self {
    //         window,
    //         event_loop,
    //         gpu_state,
    //     })
    // }

    /// Renders a [`Layout`] and runs the event loop.
    ///
    /// This method initialises the underlying window and GPU state.
    pub fn run(self, layout: Layout) -> ! {
        let mut runtime = GpuBackendRuntime::new(
            self.window,
            self.gpu_state,
            EventLoopMessageSender(self.event_loop.create_proxy()),
            layout,
        );
        self.event_loop
            .run(move |event, _, control_flow| runtime.handle_event(event, control_flow));
    }
}

#[derive(Debug, Clone)]
struct EventLoopMessageSender(EventLoopProxy<ComponentMessage>);

impl MessageSender for EventLoopMessageSender {
    fn send(&self, message: ComponentMessage) {
        self.0
            .send_event(message)
            .map_err(|_| ()) // tokio's SendError doesn't implement Debug
            .expect("App receiver needs to outlive senders for inter-component messages");
    }

    fn clone_box(&self) -> Box<dyn MessageSender> {
        Box::new(self.clone())
    }
}

struct GpuBackendRuntime {
    app: App,
    gpu_state: GpuState,
    modifiers: ModifiersState,
    window: Window,
    font_size: f32,
}

impl GpuBackendRuntime {
    fn new(
        window: Window,
        gpu_state: GpuState,
        sender: EventLoopMessageSender,
        layout: Layout,
    ) -> Self {
        let grid_size = compute_grid_size(window.inner_size(), gpu_state.glyph_cache.cell_size());
        let app = App::new(sender, grid_size, layout);
        Self {
            app,
            gpu_state,
            font_size: 16f32,
            modifiers: ModifiersState::empty(),
            window,
        }
    }

    fn handle_event<'a>(
        &mut self,
        event: Event<'a, ComponentMessage>,
        control_flow: &mut ControlFlow,
    ) {
        use winit::event::*;

        match event {
            Event::WindowEvent {
                ref event,
                window_id,
            } if window_id == self.window.id() => {
                match event {
                    WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                    WindowEvent::ModifiersChanged(new_modifiers) => self.modifiers = *new_modifiers,
                    WindowEvent::KeyboardInput {
                        input:
                            KeyboardInput {
                                state: ElementState::Pressed,
                                virtual_keycode: Some(virtual_keycode),
                                ..
                            },
                        ..
                    } => {
                        self.handle_key_press(*virtual_keycode, control_flow);
                    }
                    WindowEvent::Resized(surface_size) => self.resize_window(*surface_size),
                    WindowEvent::ScaleFactorChanged {
                        new_inner_size: surface_size,
                        ..
                    } => {
                        // surface_size is &&mut so we have to dereference it twice
                        self.resize_window(**surface_size);
                    }
                    _ => {}
                }
            }
            Event::RedrawRequested(_) if !self.app.poll_state().exit() => {
                let canvas = self.app.draw();
                self.gpu_state.update(canvas);
                match self.gpu_state.render() {
                    Ok(_) => {}
                    // Recreate the swap_chain if lost
                    Err(wgpu::SurfaceError::Lost) => self.gpu_state.resize(self.gpu_state.size),
                    // The system is out of memory, we should probably quit
                    Err(wgpu::SurfaceError::OutOfMemory) => *control_flow = ControlFlow::Exit,
                    // All other errors (Outdated, Timeout) should be resolved by the next frame
                    Err(e) => eprintln!("{:?}", e),
                }
            }
            Event::MainEventsCleared => {
                // RedrawRequested will only trigger once, unless we manually
                // request it.
                *control_flow = ControlFlow::Wait;
            }
            Event::UserEvent(message) => {
                log::warn!("user event!");
                self.app.handle_message(message);

                if self.app.poll_state().exit() {
                    *control_flow = ControlFlow::Exit;
                } else if self.app.poll_state().dirty() {
                    self.window.request_redraw();
                }
            }
            _ => {}
        };
    }

    fn handle_key_press(
        &mut self,
        virtual_keycode: VirtualKeyCode,
        control_flow: &mut ControlFlow,
    ) {
        if virtual_keycode == VirtualKeyCode::Equals && self.modifiers.ctrl() {
            self.change_font_size((self.font_size + 1.0).min(192.0));
        } else if virtual_keycode == VirtualKeyCode::Minus && self.modifiers.ctrl() {
            self.change_font_size((self.font_size - 1.0).max(1.0));
        } else {
            let key = input::map_key(virtual_keycode, &self.modifiers);
            if let Some(key) = key {
                self.app.handle_input(zi::terminal::Event::KeyPress(key));
            }

            if self.app.poll_state().exit() {
                *control_flow = ControlFlow::Exit;
            } else if self.app.poll_state().dirty() {
                self.window.request_redraw();
            }
        }
    }

    fn resize_window(&mut self, surface_size: PhysicalSize<u32>) {
        self.gpu_state.resize(surface_size);
        let grid_size = compute_grid_size(surface_size, self.gpu_state.glyph_cache.cell_size());
        self.app.handle_resize(grid_size);
    }

    fn change_font_size(&mut self, font_size: f32) {
        self.font_size = font_size;

        let initial_cell_size = self.gpu_state.glyph_cache.cell_size();
        self.gpu_state
            .update_font_size(
                self.window.scale_factor() as f32,
                FontSize::new(self.font_size),
            )
            .unwrap();
        let new_cell_size = self.gpu_state.glyph_cache.cell_size();

        log::info!(
            "Resized cell size {:?} -> {:?} (new font size {})",
            initial_cell_size,
            new_cell_size,
            self.font_size,
        );

        let grid_size = compute_grid_size(
            self.window.inner_size(),
            self.gpu_state.glyph_cache.cell_size(),
        );
        self.app.handle_resize(grid_size);
        self.window.request_redraw();
    }
}

fn compute_grid_size(surface_size: PhysicalSize<u32>, cell_size: PhysicalSize<u32>) -> Size {
    Size::new(
        (surface_size.width / cell_size.width) as usize,
        (surface_size.height / cell_size.height) as usize,
    )
}
