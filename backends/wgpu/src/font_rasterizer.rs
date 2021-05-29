pub(super) use crossfont::{Size as FontSize, Slant as FontSlant, Weight as FontWeight};

use crossfont::{
    BitmapBuffer, FontDesc, FontKey, GlyphKey, Metrics as FontMetrics, Rasterize, RasterizedGlyph,
    Rasterizer, Style as FontStyle,
};
use std::{collections::hash_map::HashMap, convert::TryFrom, i32, iter, num::NonZeroU32};
use unicode_width::UnicodeWidthStr;
use wgpu::{self, BindGroup, BindGroupLayout, Device, Queue, Sampler};
use winit::dpi::PhysicalSize;

use crate::{error::Error, texture::Texture};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct GlyphDescriptor {
    pub character: char,
    pub slant: FontSlant,
    pub weight: FontWeight,
}

#[derive(Clone, Debug)]
pub(super) struct CachedGlyph {
    pub id: u32,
    pub wide: bool,
    pub multicolour: bool,
}

pub(super) struct GlyphCache {
    sampler: Sampler,
    bind_group: BindGroup,
    bind_group_layout: BindGroupLayout,
    pub bind_group_is_outdated: bool,
    font_rasterizer: FontRasterizer,
    glyphs: Vec<Texture>,
    atlas: HashMap<GlyphDescriptor, CachedGlyph>,
    capacity: u32,
    cell_size: PhysicalSize<u32>,
}

/// Calculate the cell dimensions based on font metrics.
///
/// This will return a tuple of the cell width and height.
#[inline]
fn compute_cell_size(metrics: &FontMetrics) -> PhysicalSize<u32> {
    let offset_x = 0.0;
    let offset_y = 0.0;

    let mut pixel_width = (metrics.average_advance + offset_x).floor().max(1.);
    let mut pixel_height = (metrics.line_height + offset_y).floor().max(1.);

    if pixel_width < pixel_height / 2.0 {
        pixel_width = (pixel_height / 2.0).ceil();
    } else {
        pixel_width = pixel_width.ceil();
    }
    pixel_height = 2.0 * pixel_width;

    PhysicalSize::new(pixel_width as u32, pixel_height as u32)
}

impl GlyphCache {
    pub fn new(
        device: &Device,
        queue: &Queue,
        font_size: FontSize,
        dpr: f32,
        capacity: u32,
    ) -> Result<Self, Error> {
        let mut font_rasterizer = FontRasterizer::new(dpr, font_size)?;
        let cell_size = compute_cell_size(&font_rasterizer.metrics);

        let mut glyphs = Vec::new();
        let mut atlas = HashMap::new();
        let mut diffuse_rgba = vec![0u32; (cell_size.width * cell_size.height) as usize];
        Self::cache_ascii_glyphs(
            device,
            queue,
            &mut font_rasterizer,
            cell_size,
            &mut diffuse_rgba,
            &mut glyphs,
            &mut atlas,
        )?;

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let (bind_group_layout, bind_group) =
            Self::create_bind_group(device, &sampler, &glyphs, capacity);

        Ok(Self {
            sampler,
            bind_group,
            bind_group_layout,
            bind_group_is_outdated: false,
            font_rasterizer,
            glyphs,
            atlas,
            capacity,
            cell_size,
        })
    }

    pub fn update_font_size(
        &mut self,
        device: &Device,
        queue: &Queue,
        dpr: f32,
        font_size: FontSize,
    ) -> Result<(), Error> {
        if font_size == self.font_rasterizer.size && dpr == self.font_rasterizer.dpr {
            return Ok(());
        }

        let mut font_rasterizer = FontRasterizer::new(dpr, font_size)?;
        let cell_size = compute_cell_size(&font_rasterizer.metrics);

        self.glyphs.clear();
        self.atlas.clear();
        let mut diffuse_rgba = vec![0u32; (cell_size.width * cell_size.height) as usize];
        Self::cache_ascii_glyphs(
            device,
            queue,
            &mut font_rasterizer,
            cell_size,
            &mut diffuse_rgba,
            &mut self.glyphs,
            &mut self.atlas,
        )?;

        self.bind_group_is_outdated = true;
        self.font_rasterizer = font_rasterizer;
        self.cell_size = cell_size;

        Ok(())
    }

    pub fn get_or_insert(
        &mut self,
        device: &Device,
        queue: &Queue,
        glyph_descriptor: &GlyphDescriptor,
    ) -> Result<Option<CachedGlyph>, Error> {
        Ok(match self.atlas.get(glyph_descriptor) {
            Some(cached) => Some(cached.clone()),
            None if self.glyphs.len() + 1 >= self.capacity as usize => None,
            None => {
                let (pixel_width, pixel_height) = (
                    self.cell_size.width as usize,
                    self.cell_size.height as usize,
                );
                let glyph_cell_width = std::cmp::max(
                    UnicodeWidthStr::width(String::from(glyph_descriptor.character).as_str()),
                    1,
                );
                let pixel_width = pixel_width * glyph_cell_width;
                let mut diffuse_rgba = vec![0u32; pixel_width * pixel_height];
                let glyph = self.font_rasterizer.rasterize_glyph(glyph_descriptor)?;
                let cached_glyph = CachedGlyph {
                    id: u32::try_from(self.glyphs.len()).unwrap(),
                    wide: glyph_cell_width > 1,
                    multicolour: is_multicolour(&glyph),
                };
                self.atlas
                    .insert(glyph_descriptor.clone(), cached_glyph.clone());

                lay_glyph(
                    &self.font_rasterizer.metrics,
                    &glyph,
                    diffuse_rgba.as_mut_slice(),
                    pixel_width,
                );
                self.glyphs.push(super::texture::Texture::from_slice(
                    device,
                    queue,
                    Some("Some texture"),
                    bytemuck::cast_slice(&diffuse_rgba),
                    u32::try_from(pixel_width).expect("u32"),
                    u32::try_from(pixel_height).expect("u32"),
                )?);
                self.bind_group_is_outdated = true;

                Some(cached_glyph)
            }
        })
    }

    pub fn cell_size(&self) -> PhysicalSize<u32> {
        self.cell_size
    }

    pub fn bind_group(&self) -> &BindGroup {
        &self.bind_group
    }

    pub fn bind_group_layout(&self) -> &BindGroupLayout {
        &self.bind_group_layout
    }

    pub fn refresh_bind_group(&mut self, device: &Device, _queue: &Queue) {
        if self.bind_group_is_outdated {
            log::info!("Outdated bind group --> {}", self.glyphs.len());
            let (bind_group_layout, bind_group) =
                Self::create_bind_group(device, &self.sampler, &self.glyphs, self.capacity);
            self.bind_group_layout = bind_group_layout;
            self.bind_group = bind_group;
            self.bind_group_is_outdated = false
        }
    }

    fn cache_ascii_glyphs(
        device: &Device,
        queue: &Queue,
        font_rasterizer: &mut FontRasterizer,
        cell_size: PhysicalSize<u32>,
        diffuse_rgba: &mut Vec<u32>,
        glyphs: &mut Vec<Texture>,
        atlas: &mut HashMap<GlyphDescriptor, CachedGlyph>,
    ) -> Result<(), Error> {
        let pixel_height = cell_size.height as usize;
        for character in (32..126).filter_map(std::char::from_u32) {
            let glyph_cell_width =
                std::cmp::max(UnicodeWidthStr::width(String::from(character).as_str()), 1);
            let pixel_width = cell_size.width as usize * glyph_cell_width;
            diffuse_rgba.resize(pixel_width * pixel_height, 0);

            let glyph_descriptor = GlyphDescriptor {
                character,
                weight: FontWeight::Normal,
                slant: FontSlant::Normal,
            };

            let glyph = font_rasterizer.rasterize_glyph(&glyph_descriptor)?;
            atlas.insert(
                glyph_descriptor,
                CachedGlyph {
                    id: u32::try_from(glyphs.len()).unwrap(),
                    wide: glyph_cell_width > 1,
                    multicolour: is_multicolour(&glyph),
                },
            );

            lay_glyph(
                &font_rasterizer.metrics,
                &glyph,
                diffuse_rgba.as_mut_slice(),
                pixel_width,
            );
            glyphs.push(super::texture::Texture::from_slice(
                device,
                queue,
                Some("Some texture"),
                bytemuck::cast_slice(diffuse_rgba),
                u32::try_from(pixel_width).expect("u32"),
                u32::try_from(pixel_height).expect("u32"),
            )?);
        }
        Ok(())
    }

    fn create_bind_group(
        device: &Device,
        sampler: &Sampler,
        glyphs: &[Texture],
        capacity: u32,
    ) -> (BindGroupLayout, BindGroup) {
        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: Some(NonZeroU32::new(capacity).expect("at least 1 texture")),
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler {
                            comparison: false,
                            filtering: true,
                        },
                        count: None,
                    },
                ],
                label: Some("texture_bind_group_layout"),
            });

        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureViewArray(
                        glyphs
                            .iter()
                            .map(|texture| &texture.view)
                            .chain(
                                iter::repeat(&glyphs[0].view)
                                    .take((capacity as usize).saturating_sub(glyphs.len())),
                            )
                            .collect::<Vec<_>>()
                            .as_slice(),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
            label: Some("diffuse_bind_group"),
        });
        (texture_bind_group_layout, texture_bind_group)
    }
}

struct FontKeys {
    regular: FontKey,
    bold: FontKey,
    italic: FontKey,
    bold_italic: FontKey,
    emoji: FontKey,
}

struct FontRasterizer {
    rasterizer: Rasterizer,
    size: FontSize,
    metrics: FontMetrics,
    keys: FontKeys,
    dpr: f32,
}

impl FontRasterizer {
    pub fn new(dpr: f32, size: FontSize) -> Result<Self, Error> {
        let mut rasterizer = Rasterizer::new(dpr, false)?;
        let keys = Self::compute_font_keys(&mut rasterizer, size)?;

        // Need to load at least one glyph for the face before calling metrics.
        // The glyph requested here ('m' at the time of writing) has no special
        // meaning.
        rasterizer.get_glyph(GlyphKey {
            font_key: keys.regular,
            character: 'm',
            size,
        })?;
        let metrics = rasterizer.metrics(keys.regular, size)?;

        Ok(Self {
            rasterizer,
            size,
            metrics,
            keys,
            dpr,
        })
    }

    fn rasterize_glyph(&mut self, desc: &GlyphDescriptor) -> Result<RasterizedGlyph, Error> {
        match self.rasterizer.get_glyph(GlyphKey {
            character: desc.character,
            font_key: if desc.weight == FontWeight::Bold {
                self.keys.bold
            } else {
                self.keys.regular
            },
            size: self.size,
        }) {
            Err(crossfont::Error::MissingGlyph(_)) => match self.rasterizer.get_glyph(GlyphKey {
                character: desc.character,
                font_key: self.keys.emoji,
                size: self.size,
            }) {
                Err(crossfont::Error::MissingGlyph(glyph)) => Ok(glyph),
                result => Ok(result?),
            },
            result => Ok(result?),
        }
    }

    /// Computes font keys for (Regular, Bold, Italic, Bold Italic).
    fn compute_font_keys(rasterizer: &mut Rasterizer, size: FontSize) -> Result<FontKeys, Error> {
        let family = "monospace";

        // Load regular font
        let regular = load_font(
            rasterizer,
            family,
            size,
            FontSlant::Normal,
            FontWeight::Normal,
        )?;

        // Load bold font
        let bold = load_font(
            rasterizer,
            family,
            size,
            FontSlant::Normal,
            FontWeight::Bold,
        )?;

        // Load italic font
        let italic = load_font(
            rasterizer,
            family,
            size,
            FontSlant::Italic,
            FontWeight::Normal,
        )?;

        // Load bold italic font
        let bold_italic = load_font(
            rasterizer,
            family,
            size,
            FontSlant::Italic,
            FontWeight::Bold,
        )?;

        // Load emoji font
        let emoji = rasterizer.load_font(
            &FontDesc::new(
                "Noto Color Emoji",
                FontStyle::Description {
                    slant: FontSlant::Normal,
                    weight: FontWeight::Normal,
                },
            ),
            size,
        )?;

        Ok(FontKeys {
            regular,
            bold,
            italic,
            bold_italic,
            emoji,
        })
    }
}

#[inline]
pub fn lay_glyph(
    metrics: &FontMetrics,
    glyph: &RasterizedGlyph,
    buffer: &mut [u32],
    pixel_width: usize,
) {
    assert_eq!(buffer.len() % pixel_width, 0);
    let pixel_height = buffer.len() / pixel_width;

    let (glyph_buffer, stride) = {
        match glyph.buffer {
            BitmapBuffer::Rgb(ref buffer) => (buffer, 3),
            BitmapBuffer::Rgba(ref buffer) => (buffer, 4),
        }
    };

    let top = std::cmp::max(pixel_height as i32 + metrics.descent as i32 - glyph.top, 0) as usize;
    // let left = std::cmp::max((pixel_width as i32).saturating_sub(glyph.width) / 2, 0) as usize;
    let left = std::cmp::max(glyph.left, 0) as usize;

    for pixel_x in 0..pixel_width {
        for pixel_y in 0..pixel_height {
            if pixel_x >= (glyph.width as usize + left) as usize
                || pixel_x < left as usize
                || pixel_y >= (glyph.height as usize + top) as usize
                || pixel_y < top as usize
            {
                buffer[pixel_y * pixel_width + pixel_x] = 0;
                continue;
            }

            let font_index = stride
                * ((pixel_y - top as usize) * (glyph.width as usize) + (pixel_x - left as usize));
            let (red, green, blue, alpha) = (
                glyph_buffer[font_index] as u32,
                glyph_buffer[font_index + 1] as u32,
                glyph_buffer[font_index + 2] as u32,
                if stride == 4 {
                    glyph_buffer[font_index + 3] as u32
                } else {
                    glyph_buffer[font_index] as u32
                },
            );
            // buffer[pixel_y * pixel_width + pixel_x] = red << 24 | green << 16 | blue << 8;
            buffer[pixel_y * pixel_width + pixel_x] = red | green << 8 | blue << 16 | alpha << 24;
        }
    }
}

fn add_alpha_channel(buffer: &[u8]) -> Vec<u8> {
    assert_eq!(buffer.len() % 3, 0);
    let mut output = vec![0u8; buffer.len() + buffer.len() / 3];
    for (output, input) in output.chunks_exact_mut(4).zip(buffer.chunks_exact(3)) {
        output[0..3].copy_from_slice(input);
        output[3] = input[0];
    }
    output
}

fn is_multicolour(glyph: &RasterizedGlyph) -> bool {
    match glyph.buffer {
        BitmapBuffer::Rgb(_) => false,
        BitmapBuffer::Rgba(_) => true,
    }
}

fn load_font(
    rasterizer: &mut Rasterizer,
    font_family: &str,
    size: FontSize,
    slant: FontSlant,
    weight: FontWeight,
) -> Result<FontKey, Error> {
    Ok(rasterizer.load_font(
        &FontDesc::new(font_family, FontStyle::Description { slant, weight }),
        size,
    )?)
}

// debug_metrics(&mut rasterizer, "monospace", 16.0);
// debug_metrics(&mut rasterizer, "monospace", 20.0);
// debug_metrics(&mut rasterizer, "monospace", 24.0);
// debug_metrics(&mut rasterizer, "Ubuntu Mono", 16.0);
// debug_metrics(&mut rasterizer, "Ubuntu Mono", 20.0);
// debug_metrics(&mut rasterizer, "Ubuntu Mono", 24.0);
// debug_metrics(&mut rasterizer, "Ubuntu", 16.0);
// debug_metrics(&mut rasterizer, "Ubuntu", 20.0);
// debug_metrics(&mut rasterizer, "Ubuntu", 24.0);
// debug_metrics(&mut rasterizer, "Noto Mono", 16.0);
// debug_metrics(&mut rasterizer, "Noto Mono", 20.0);
// debug_metrics(&mut rasterizer, "Noto Mono", 24.0);
// debug_metrics(&mut rasterizer, "FreeMono", 16.0);
// debug_metrics(&mut rasterizer, "FreeMono", 20.0);
// debug_metrics(&mut rasterizer, "FreeMono", 24.0);
// debug_metrics(&mut rasterizer, "", 16.0);
// debug_metrics(&mut rasterizer, "Noto Color Emoji", 16.0);

// log::info!(
//     "AA1: {:?}",
//     rasterizer
//         .metrics(font_key_mono_normal, FontSize::new(16.0))?
//         .average_advance
// );
// log::info!(
//     "AA2: {:?}",
//     rasterizer
//         .metrics(font_key_mono_weight, FontSize::new(16.0))?
//         .average_advance
// );
// log::info!(
//     "AA3: {:?}",
//     rasterizer
//         .metrics(font_key_emoji, FontSize::new(16.0))?
//         .average_advance
// );
// fn debug_metrics(rasterizer: &mut Rasterizer, key: &str, size: f32) {
//     let font = rasterizer
//         .load_font(
//             &FontDesc::new(
//                 key,
//                 FontStyle::Description {
//                     slant: FontSlant::Normal,
//                     weight: FontWeight::Normal,
//                 },
//             ),
//             FontSize::new(size),
//         )
//         .unwrap();
//     rasterizer
//         .get_glyph(GlyphKey {
//             font_key: font,
//             character: 'm',
//             size: FontSize::new(16.0),
//         })
//         .unwrap();
//     let m1 = rasterizer.metrics(font, FontSize::new(size)).unwrap();
//     log::info!("==== {} {}pt ====", key, size);
//     log::info!("metrics: {:?}", m1);
// }
