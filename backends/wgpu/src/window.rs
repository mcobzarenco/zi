use crossfont::Size as FontSize;
use parking_lot::RwLock;
use std::{ops::DerefMut, sync::Arc};
use tokio::sync::mpsc::UnboundedSender;
use winit::{
    dpi::PhysicalSize,
    event::{ModifiersState, VirtualKeyCode},
    event_loop::{ControlFlow, EventLoop, EventLoopProxy},
    platform::unix::EventLoopExtUnix,
    window::Window,
    window::WindowBuilder,
};
use zi::{
    app::Event,
    terminal::{Canvas, Key, Size},
};

use crate::{
    error::{Error, Result},
    state::GpuState,
};

pub(super) struct AppWindow {
    canvas: Arc<RwLock<Option<Canvas>>>,
    size: Arc<RwLock<Size>>,
    window: Window,
    event_loop: EventLoop<()>,
    sender: UnboundedSender<Result<Event>>,
    gpu_state: GpuState,
}

impl AppWindow {
    pub fn new(
        title: &str,
        canvas: Arc<RwLock<Option<Canvas>>>,
        size: Arc<RwLock<Size>>,
        event_sender: UnboundedSender<Result<Event>>,
    ) -> Result<Self> {
        let event_loop = EventLoop::new_any_thread();
        let window = WindowBuilder::new()
            .with_decorations(true)
            .with_inner_size(PhysicalSize {
                width: 1280,
                height: 1024,
            })
            .with_resizable(true)
            .with_title(title)
            .build(&event_loop)
            .map_err(Error::Window)?;

        let gpu_state = futures::executor::block_on(GpuState::new(&window))?;
        Ok(Self {
            canvas,
            size,
            window,
            event_loop,
            sender: event_sender,
            gpu_state,
        })
    }

    pub fn event_loop_proxy(&self) -> EventLoopProxy<()> {
        self.event_loop.create_proxy()
    }

    pub fn run(self) {
        use winit::event::*;

        let Self {
            canvas,
            size,
            window,
            event_loop,
            sender,
            mut gpu_state,
        } = self;

        let mut modifiers = ModifiersState::empty();

        let mut font_size = 16.0f32;

        event_loop.run(move |event, _, control_flow| match event {
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
                            // let new_cell_size = PhysicalSize::new(
                            //     (gpu_state.cell_size.width + 1) as u32,
                            //     (gpu_state.cell_size.height + 2) as u32,
                            // );

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
                            *size.write() = new_grid_size;
                            sender
                                .send(Ok(super::Event::Resize(new_grid_size)))
                                .expect("send resize");
                        } else if *virtual_keycode == VirtualKeyCode::Minus && modifiers.ctrl() {
                            // let new_cell_size = PhysicalSize::new(
                            //     std::cmp::max(1, gpu_state.cell_size.width.saturating_sub(1)),
                            //     std::cmp::max(2, gpu_state.cell_size.height.saturating_sub(2)),
                            // );

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
                            *size.write() = new_grid_size;
                            sender
                                .send(Ok(super::Event::Resize(new_grid_size)))
                                .expect("send resize");
                        } else {
                            let key = map_key(*virtual_keycode, &modifiers);
                            if let Some(key) = key {
                                sender.send(Ok(super::Event::Key(key))).expect("send key");
                            }
                        }
                    }
                    _ => {}
                },
                WindowEvent::Resized(physical_size) => {
                    gpu_state.resize(*physical_size);

                    let new_size = Size::new(
                        (physical_size.width / gpu_state.glyph_cache.cell_size().width) as usize,
                        (physical_size.height / gpu_state.glyph_cache.cell_size().height) as usize,
                    );
                    *size.write() = new_size;
                    sender
                        .send(Ok(super::Event::Resize(new_size)))
                        .expect("send key");
                }
                WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                    // new_inner_size is &&mut so we have to dereference it twice
                    gpu_state.resize(**new_inner_size);
                }
                _ => {}
            },
            Event::RedrawRequested(_) => {
                let mut maybe_canvas = None;
                std::mem::swap(canvas.write().deref_mut(), &mut maybe_canvas);
                if let Some(canvas) = maybe_canvas {
                    gpu_state.update(&canvas);
                }

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
            Event::UserEvent(payload) => {
                log::warn!("user event!");
                window.request_redraw();
            }
            _ => {}
        });

        // while !self.shutdown.read().deref() {
        //     {
        //         let mut maybe_canvas = None;
        //         std::mem::swap(self.canvas.write().deref_mut(), &mut maybe_canvas);
        //         if let Some(canvas) = maybe_canvas {
        //             self.gpu_state.update(&canvas);
        //         }
        //     }

        //     match self.gpu_state.render() {
        //         Ok(_) => {}
        //         // Recreate the swap_chain if lost
        //         Err(wgpu::SwapChainError::Lost) => self.gpu_state.resize(self.gpu_state.size),
        //         // The system is out of memory, we should probably quit
        //         // Err(wgpu::SwapChainError::OutOfMemory) => *control_flow = ControlFlow::Exit,
        //         // All other errors (Outdated, Timeout) should be resolved by the next frame
        //         Err(e) => eprintln!("{:?}", e),
        //     }

        //     self.window.update();
        //     let ctrl = self.window.is_key_down(minifb::Key::LeftCtrl)
        //         || self.window.is_key_down(minifb::Key::RightCtrl);
        //     let alt = self.window.is_key_down(minifb::Key::LeftAlt)
        //         || self.window.is_key_down(minifb::Key::RightAlt);
        //     self.window.get_keys_pressed(KeyRepeat::Yes).map(|keys| {
        //         for key in keys.into_iter().filter_map(|key| map_key(key, ctrl, alt)) {
        //             eprintln!("sending key {:?}", key);
        //             self.sender.send(Ok(Event::Key(key))).expect("send key");
        //         }
        //     });
        // }

        // self.window.cl

        // while window.is_open() && !window.is_key_down(Key::Escape) {
        //     // for i in buffer.iter_mut() {
        //     //     *i = 0; // write something more funny here!
        //     // }

        //     // We unwrap here as we want this code to exit if it fails. Real applications may want to handle this in a different way
        //     window.update_with_buffer(&buffer, WIDTH, HEIGHT).unwrap();
        // }
        // });
    }
}

// /// Calculate the cell dimensions based on font metrics.
// ///
// /// This will return a tuple of the cell width and height.
// #[inline]
// fn compute_cell_size(config: &Config, metrics: &crossfont::Metrics) -> (f32, f32) {
//     let offset_x = f64::from(config.ui_config.font.offset.x);
//     let offset_y = f64::from(config.ui_config.font.offset.y);
//     (
//         (metrics.average_advance + offset_x).floor().max(1.) as f32,
//         (metrics.line_height + offset_y).floor().max(1.) as f32,
//     )
// }

// /// Calculate the size of the window given padding, terminal dimensions and cell size.
// fn window_size(
//     config: &Config,
//     dimensions: Dimensions,
//     cell_width: f32,
//     cell_height: f32,
//     dpr: f64,
// ) -> PhysicalSize<u32> {
//     let padding = config.ui_config.window.padding(dpr);

//     let grid_width = cell_width * dimensions.columns.0.max(MIN_COLUMNS) as f32;
//     let grid_height = cell_height * dimensions.lines.max(MIN_SCREEN_LINES) as f32;

//     let width = (padding.0).mul_add(2., grid_width).floor();
//     let height = (padding.1).mul_add(2., grid_height).floor();

//     PhysicalSize::new(width as u32, height as u32)
// }

#[inline]
fn map_key(key: VirtualKeyCode, modifiers: &ModifiersState) -> Option<Key> {
    match key {
        VirtualKeyCode::Back => Some(Key::Backspace),
        VirtualKeyCode::Left => Some(Key::Left),
        VirtualKeyCode::Right => Some(Key::Right),
        VirtualKeyCode::Up => Some(Key::Up),
        VirtualKeyCode::Down => Some(Key::Down),
        VirtualKeyCode::Home => Some(Key::Home),
        VirtualKeyCode::End => Some(Key::End),
        VirtualKeyCode::PageUp => Some(Key::PageUp),
        VirtualKeyCode::PageDown => Some(Key::PageDown),
        VirtualKeyCode::Delete => Some(Key::Delete),
        VirtualKeyCode::Insert => Some(Key::Insert),
        VirtualKeyCode::Escape => Some(Key::Esc),
        maybe_char => map_char(maybe_char).map(|character| {
            if modifiers.ctrl() {
                Key::Ctrl(character)
            } else if modifiers.alt() {
                Key::Alt(character)
            } else {
                Key::Char(character)
            }
        }),
    }
}

#[inline]
fn map_char(key: VirtualKeyCode) -> Option<char> {
    Some(match key {
        VirtualKeyCode::A => 'a',
        VirtualKeyCode::B => 'b',
        VirtualKeyCode::C => 'c',
        VirtualKeyCode::D => 'd',
        VirtualKeyCode::E => 'e',
        VirtualKeyCode::F => 'f',
        VirtualKeyCode::G => 'g',
        VirtualKeyCode::H => 'h',
        VirtualKeyCode::I => 'i',
        VirtualKeyCode::J => 'j',
        VirtualKeyCode::K => 'k',
        VirtualKeyCode::L => 'l',
        VirtualKeyCode::M => 'm',
        VirtualKeyCode::N => 'n',
        VirtualKeyCode::O => 'o',
        VirtualKeyCode::P => 'p',
        VirtualKeyCode::Q => 'q',
        VirtualKeyCode::R => 'r',
        VirtualKeyCode::S => 's',
        VirtualKeyCode::T => 't',
        VirtualKeyCode::U => 'u',
        VirtualKeyCode::V => 'v',
        VirtualKeyCode::W => 'w',
        VirtualKeyCode::X => 'x',
        VirtualKeyCode::Y => 'y',
        VirtualKeyCode::Z => 'z',
        VirtualKeyCode::Apostrophe => '\'',
        // VirtualKeyCode::Backquote => '`',
        VirtualKeyCode::Backslash => '\\',
        VirtualKeyCode::Comma => ',',
        VirtualKeyCode::Equals => '=',
        VirtualKeyCode::LBracket => '(',
        VirtualKeyCode::Minus => '-',
        VirtualKeyCode::Period => '.',
        VirtualKeyCode::RBracket => ')',
        VirtualKeyCode::Semicolon => ';',
        VirtualKeyCode::Slash => '/',
        VirtualKeyCode::Return => '\n',
        VirtualKeyCode::Space => ' ',
        VirtualKeyCode::Tab => '\t',
        _ => {
            return None;
        }
    })
}
