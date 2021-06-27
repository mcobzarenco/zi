//! An experimental graphical backend using wgpu

mod error;
mod font_rasterizer;
mod input;
mod state;
mod texture;

use crossfont::Size as FontSize;
use winit::{
    dpi::PhysicalSize,
    event_loop::{ControlFlow, EventLoop, EventLoopProxy},
    window::Window,
    window::WindowBuilder,
};
use zi::{
    app::{self, App, AppMessage, MessageSender},
    Error as ZiError, Layout, Size,
};

use crate::state::GpuState;

pub use error::{Error, Result};

#[derive(Debug, Clone)]
struct EventLoopMessageSender(EventLoopProxy<AppMessage>);

impl MessageSender for EventLoopMessageSender {
    fn send(&self, message: AppMessage) {
        self.0
            .send_event(message)
            .map_err(|_| ()) // tokio's SendError doesn't implement Debug
            .expect("App receiver needs to outlive senders for inter-component messages");
    }

    fn clone_box(&self) -> Box<dyn MessageSender> {
        Box::new(self.clone())
    }
}

/// Experimental graphical backend using wgpu
pub struct GpuBackend {
    window: Window,
    event_loop: EventLoop<AppMessage>,
    gpu_state: GpuState,
}

impl GpuBackend {
    /// Create a new backend instance.
    ///
    /// This method initialises the underlying tty device, enables raw mode,
    /// hides the cursor and enters alternative screen mode. Additionally, an
    /// async event stream with input events from stdin is started.
    pub fn new(title: &str) -> Result<Self> {
        let event_loop = EventLoop::with_user_event();
        let window = WindowBuilder::new()
            .with_decorations(true)
            .with_inner_size(PhysicalSize {
                width: 1280,
                height: 1024,
            })
            .with_resizable(true)
            .with_title(title)
            .build(&event_loop)?;
        let gpu_state = futures::executor::block_on(GpuState::new(&window))?;

        Ok(Self {
            window,
            event_loop,
            gpu_state,
        })
    }

    pub fn run_event_loop(self, layout: Layout) -> Result<()> {
        use winit::event::*;

        let Self {
            window,
            event_loop,
            mut gpu_state,
        } = self;

        let pixel_size = window.inner_size();
        let cell_size = gpu_state.glyph_cache.cell_size();
        let grid_size = Size::new(
            (pixel_size.width / cell_size.width) as usize,
            (pixel_size.height / cell_size.height) as usize,
        );
        let mut app = App::new(
            EventLoopMessageSender(event_loop.create_proxy()),
            grid_size,
            layout,
        );
        let mut modifiers = ModifiersState::empty();
        let mut font_size = 16.0f32;

        event_loop.run(move |event, _, control_flow| {
            match event {
                Event::WindowEvent {
                    ref event,
                    window_id,
                } if window_id == window.id() => match event {
                    WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                    WindowEvent::ModifiersChanged(new_modifiers) => modifiers = *new_modifiers,
                    WindowEvent::KeyboardInput { input, .. } => match input {
                        KeyboardInput {
                            state: ElementState::Pressed,
                            virtual_keycode: Some(virtual_keycode),
                            ..
                        } => {
                            if *virtual_keycode == VirtualKeyCode::Equals && modifiers.ctrl() {
                                let initial_cell_size = gpu_state.glyph_cache.cell_size();
                                font_size = (font_size + 1.0).min(192.0);
                                gpu_state
                                    .update_font_size(
                                        window.scale_factor() as f32,
                                        FontSize::new(font_size),
                                    )
                                    .unwrap();
                                let new_cell_size = gpu_state.glyph_cache.cell_size();

                                log::info!(
                                    "Resized cell size {:?} -> {:?} (new font size {})",
                                    initial_cell_size,
                                    new_cell_size,
                                    font_size,
                                );

                                let new_grid_size = Size::new(
                                    (window.inner_size().width / new_cell_size.width) as usize,
                                    (window.inner_size().height / new_cell_size.height) as usize,
                                );
                                app.resize(new_grid_size);
                                window.request_redraw();
                            } else if *virtual_keycode == VirtualKeyCode::Minus && modifiers.ctrl()
                            {
                                let initial_cell_size = gpu_state.glyph_cache.cell_size();
                                font_size = (font_size - 1.0).max(1.0);
                                gpu_state
                                    .update_font_size(
                                        window.scale_factor() as f32,
                                        FontSize::new(font_size),
                                    )
                                    .unwrap();
                                let new_cell_size = gpu_state.glyph_cache.cell_size();

                                log::info!(
                                    "Resized cell size {:?} -> {:?} (new font size {})",
                                    initial_cell_size,
                                    new_cell_size,
                                    font_size,
                                );

                                let new_grid_size = Size::new(
                                    (window.inner_size().width / new_cell_size.width) as usize,
                                    (window.inner_size().height / new_cell_size.height) as usize,
                                );
                                app.resize(new_grid_size);
                                window.request_redraw();
                            } else {
                                let key = input::map_key(*virtual_keycode, &modifiers);
                                if let Some(key) = key {
                                    app.handle_input(app::Event::Key(key)).unwrap();
                                }

                                if app.poll_state().exit() {
                                    *control_flow = ControlFlow::Exit;
                                } else if app.poll_state().dirty() {
                                    window.request_redraw();
                                }
                            }
                        }
                        _ => {}
                    },
                    WindowEvent::Resized(physical_size) => {
                        gpu_state.resize(*physical_size);

                        let new_size = Size::new(
                            (physical_size.width / gpu_state.glyph_cache.cell_size().width)
                                as usize,
                            (physical_size.height / gpu_state.glyph_cache.cell_size().height)
                                as usize,
                        );
                        app.resize(new_size);
                    }
                    WindowEvent::ScaleFactorChanged {
                        new_inner_size: physical_size,
                        ..
                    } => {
                        // new_inner_size is &&mut so we have to dereference it twice
                        gpu_state.resize(**physical_size);
                        let new_size = Size::new(
                            (physical_size.width / gpu_state.glyph_cache.cell_size().width)
                                as usize,
                            (physical_size.height / gpu_state.glyph_cache.cell_size().height)
                                as usize,
                        );
                        app.resize(new_size);
                    }
                    _ => {}
                },
                Event::RedrawRequested(_) => {
                    match app.draw() {
                        Ok(canvas) => {
                            gpu_state.update(canvas);
                        }
                        Err(ZiError::Exiting) => {
                            *control_flow = ControlFlow::Exit;
                            log::error!("App is exiting during draw call");
                            return;
                        } // Err(error) => {
                          //     *control_flow = ControlFlow::Exit;
                          //     log::error!("App returned error while drawng: {}", error);
                          //     return;
                          // }
                    };

                    match gpu_state.render() {
                        Ok(_) => {}
                        // Recreate the swap_chain if lost
                        Err(wgpu::SwapChainError::Lost) => gpu_state.resize(gpu_state.size),
                        // The system is out of memory, we should probably quit
                        Err(wgpu::SwapChainError::OutOfMemory) => *control_flow = ControlFlow::Exit,
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
                    app.handle_message(message).unwrap();

                    if app.poll_state().exit() {
                        *control_flow = ControlFlow::Exit;
                    } else if app.poll_state().dirty() {
                        window.request_redraw();
                    }
                }
                _ => {}
            };
        });
    }
}

// impl Drop for GpuBackend {
//     fn drop(&mut self) {
//         // *self.shutdown.write() = true;
//         // self.window_thread
//         //     .take()
//         //     .map(|window_thread| window_thread.join());
//         // log::info!("joined");
//     }
// }
