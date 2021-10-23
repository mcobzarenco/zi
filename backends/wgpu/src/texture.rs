use std::num::NonZeroU32;

use super::Error;

pub struct Texture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl Texture {
    pub fn from_slice(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: Option<&str>,
        buffer: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Self, Error> {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        });

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            buffer,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: NonZeroU32::new(4 * width).unwrap().into(),
                rows_per_image: NonZeroU32::new(height).unwrap().into(),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Ok(Self { texture, view })
    }
}
