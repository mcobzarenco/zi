use glam::{Vec2, Vec3};
use std::convert::TryFrom;
use wgpu::{
    util::DeviceExt, Device, Instance, Queue, RenderPipeline, Surface, SurfaceConfiguration,
};
use winit::{dpi::PhysicalSize, window::Window};
use zi::terminal::{Canvas, Colour};

use crate::{
    error::Error,
    font_rasterizer::{CachedGlyph, FontSize, FontSlant, FontWeight, GlyphCache, GlyphDescriptor},
};

#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct Vertex {
    position: Vec3,
    background_color: Vec3,
    foreground_color: Vec3,
    tex_coords: Vec2,
    tex_index: u32,
}

impl Vertex {
    fn descriptor<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // position: Vec3
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                // background_color: Vec3,
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<Vec3>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
                // foreground_color: Vec3,
                wgpu::VertexAttribute {
                    offset: (2 * std::mem::size_of::<Vec3>()) as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x3,
                },
                // tex_coords: Vec2,
                wgpu::VertexAttribute {
                    offset: (3 * std::mem::size_of::<Vec3>()) as wgpu::BufferAddress,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // tex_index: u32,
                wgpu::VertexAttribute {
                    offset: (3 * std::mem::size_of::<Vec3>() + std::mem::size_of::<Vec2>())
                        as wgpu::BufferAddress,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Uint32,
                },
            ],
        }
    }
}

unsafe impl bytemuck::Pod for Vertex {}
unsafe impl bytemuck::Zeroable for Vertex {}

fn colour_to_vec3(colour: Colour) -> Vec3 {
    Vec3::new(
        colour.red as f32 / 255.0,
        colour.green as f32 / 255.0,
        colour.blue as f32 / 255.0,
    )
}

fn make_vertices(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    glyph_cache: &mut GlyphCache,
    canvas: &Canvas,
    surface_size: &PhysicalSize<u32>,
    cell_size: &PhysicalSize<u32>,
) -> Vec<Vertex> {
    let num_cells_x = surface_size.width / cell_size.width;
    let num_cells_y = surface_size.height / cell_size.height;

    let hf = (num_cells_x * cell_size.width) as f32 / surface_size.width as f32;
    let vf = (num_cells_y * cell_size.height) as f32 / surface_size.height as f32;

    let cell_width: f32 = 2.0 * hf / num_cells_x as f32;
    let cell_height: f32 = 2.0 * vf / num_cells_y as f32;

    let mut vertices = Vec::new();

    let max_x = std::cmp::min(num_cells_x, canvas.size().width as u32);
    let max_y = std::cmp::min(num_cells_y, canvas.size().height as u32);

    for x in 0..max_x {
        for y in 0..max_y {
            let textel = canvas.textel(x as usize, y as usize);
            if textel.is_none() {
                continue;
            }

            let mut quad_width = cell_width;
            for nextx in (x + 1)..max_x {
                if canvas.textel(nextx as usize, y as usize).is_none() {
                    quad_width += cell_width
                } else {
                    break;
                }
            }

            let xf = -1.0 + (x as f32) * cell_width;
            let yf = 1.0 - ((y + 1) as f32) * cell_height;

            // let color = Vec3::new(
            //     x as f32 / num_cells_x as f32,
            //     y as f32 / num_cells_y as f32,
            //     0.5,
            // );
            let (background_color, foreground_color, tex_index) = textel
                .as_ref()
                .map(|textel| {
                    let tex_index = textel
                        .grapheme
                        .as_str()
                        .chars()
                        .next()
                        .and_then(|character| {
                            glyph_cache
                                .get_or_insert(
                                    device,
                                    queue,
                                    &GlyphDescriptor {
                                        character,
                                        weight: if textel.style.bold {
                                            FontWeight::Bold
                                        } else {
                                            FontWeight::Normal
                                        },
                                        slant: FontSlant::Italic,
                                    },
                                )
                                .unwrap()
                                .map(
                                    |CachedGlyph {
                                         id, multicolour, ..
                                     }| {
                                        id | {
                                            if multicolour {
                                                1 << 14
                                            } else {
                                                0
                                            }
                                        }
                                    },
                                )
                        })
                        .unwrap_or(8192);
                    (
                        colour_to_vec3(textel.style.background),
                        colour_to_vec3(textel.style.foreground),
                        tex_index,
                    )
                })
                .unwrap_or_else(|| {
                    (
                        colour_to_vec3(Colour::black()),
                        colour_to_vec3(Colour::white()),
                        8192,
                    )
                });

            vertices.push(Vertex {
                position: [xf + quad_width, yf, 0.0].into(),
                background_color,
                foreground_color,
                tex_coords: [1.0, 1.0].into(),
                tex_index,
            });
            vertices.push(Vertex {
                position: [xf, yf + cell_height, 0.0].into(),
                background_color,
                foreground_color,
                tex_coords: [0.0, 0.0].into(),
                tex_index,
            });
            vertices.push(Vertex {
                position: [xf, yf, 0.0].into(),
                background_color,
                foreground_color,
                tex_coords: [0.0, 1.0].into(),
                tex_index,
            });

            vertices.push(Vertex {
                position: [xf + quad_width, yf + cell_height, 0.0].into(),
                background_color,
                foreground_color,
                tex_coords: [1.0, 0.0].into(),
                tex_index,
            });
            vertices.push(Vertex {
                position: [xf, yf + cell_height, 0.0].into(),
                background_color,
                foreground_color,
                tex_coords: [0.0, 0.0].into(),
                tex_index,
            });
            vertices.push(Vertex {
                position: [xf + quad_width, yf, 0.0].into(),
                background_color,
                foreground_color,
                tex_coords: [1.0, 1.0].into(),
                tex_index,
            });
        }
    }

    vertices
}

const MAX_SAMPLED_TEXTURES_PER_SHADER_STAGE: u32 = 2048;

pub(super) struct GpuState {
    surface: Surface,
    device: Device,
    queue: Queue,
    swap_chain_descriptor: SurfaceConfiguration,
    pub size: PhysicalSize<u32>,
    render_pipeline: RenderPipeline,

    pub glyph_cache: GlyphCache,
    vertex_buffer: Option<(wgpu::Buffer, usize)>,
}

impl GpuState {
    pub(super) async fn new(window: &Window) -> Result<Self, Error> {
        let size = window.inner_size();
        let instance = Instance::new(wgpu::Backends::PRIMARY);
        let surface = unsafe { instance.create_surface(window) };
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .unwrap();
        let features = wgpu::Features::default()
            | wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
            | wgpu::Features::SPIRV_SHADER_PASSTHROUGH;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    features,
                    limits: wgpu::Limits {
                        max_push_constant_size: 4,
                        max_sampled_textures_per_shader_stage:
                            MAX_SAMPLED_TEXTURES_PER_SHADER_STAGE,
                        ..wgpu::Limits::default()
                    },
                    label: None,
                },
                None, // Trace path
            )
            .await
            .expect("device");

        // Create swap chain
        let swap_chain_descriptor = SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface
                .get_preferred_format(&adapter)
                .expect("compatible surface"),
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
        };
        surface.configure(&device, &swap_chain_descriptor);

        let glyph_cache = GlyphCache::new(
            &device,
            &queue,
            FontSize::new(16.0),
            window.scale_factor() as f32,
            MAX_SAMPLED_TEXTURES_PER_SHADER_STAGE,
        )?;

        // Load shaders
        let vs_module = device.create_shader_module(&wgpu::include_spirv!("shader.vert.spv"));
        let fs_module = unsafe {
            // workaround for lack of texture array support in naga
            device.create_shader_module_spirv(&wgpu::include_spirv_raw!("shader.frag.spv"))
        };

        // Set up a render pipeline
        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[glyph_cache.bind_group_layout()],
                push_constant_ranges: &[],
            });
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vs_module,
                entry_point: "main",
                buffers: &[Vertex::descriptor()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &fs_module,
                entry_point: "main",
                targets: &[wgpu::ColorTargetState {
                    format: swap_chain_descriptor.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                }],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList, // 1.
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw, // 2.
                cull_mode: Some(wgpu::Face::Back),
                clamp_depth: false,
                // Setting this to anything other than Fill requires Features::NON_FILL_POLYGON_MODE
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None, // 1.
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
        });

        Ok(Self {
            surface,
            device,
            queue,
            swap_chain_descriptor,
            size,
            render_pipeline,

            glyph_cache,
            vertex_buffer: None,
        })
    }

    pub fn resize(&mut self, new_size: PhysicalSize<u32>) {
        self.size = new_size;
        self.swap_chain_descriptor.width = new_size.width;
        self.swap_chain_descriptor.height = new_size.height;
        self.surface
            .configure(&self.device, &self.swap_chain_descriptor);
        // self.vertex_buffer = self
        //     .device
        //     .create_buffer_init(&wgpu::util::BufferInitDescriptor {
        //         label: Some("Vertex Buffer"),
        //         contents: bytemuck::cast_slice(&make_vertices(
        //             self.size.width as usize,
        //             self.size.height as usize,
        //         )),
        //         usage: wgpu::BufferUsage::VERTEX,
        //     });
    }

    // fn input(&mut self, event: &WindowEvent) -> bool {
    //     false
    // }

    pub fn update(&mut self, canvas: &Canvas) {
        let cell_size = self.glyph_cache.cell_size();
        let vertices = make_vertices(
            &self.device,
            &self.queue,
            &mut self.glyph_cache,
            canvas,
            &self.size,
            &cell_size,
        );
        log::info!("cs: {:?}", canvas.size());
        log::info!("vertices: {}", vertices.len());
        self.vertex_buffer = Some((
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Vertex Buffer"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
            vertices.len(),
        ));
    }

    pub fn update_font_size(&mut self, dpr: f32, font_size: FontSize) -> Result<(), Error> {
        self.glyph_cache
            .update_font_size(&self.device, &self.queue, dpr, font_size)
    }

    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        self.glyph_cache
            .refresh_bind_group(&self.device, &self.queue);

        let frame = self.surface.get_current_texture()?;
        let view = &frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: true,
                    },
                }],
                depth_stencil_attachment: None,
            });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, self.glyph_cache.bind_group(), &[]);

            if let Some((vertex_buffer, vertex_buffer_len)) = self.vertex_buffer.as_ref() {
                render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                render_pass.draw(0..u32::try_from(*vertex_buffer_len).unwrap(), 0..1);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();

        Ok(())
    }
}
