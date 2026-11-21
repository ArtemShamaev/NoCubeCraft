//! Terrain textures (ambientCG, CC0) packed into texture arrays with full mip chains.

use bevy::{
    asset::RenderAssetUsages,
    image::{
        CompressedImageFormats, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor, ImageType,
    },
    prelude::*,
    render::render_resource::{
        Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
        TextureViewDimension,
    },
};

use super::blocks::Material;

/// Edge of each layer, in pixels.
const SIZE: u32 = 512;

macro_rules! layer {
    ($name:literal) => {
        [
            include_bytes!(concat!("../../assets/textures/terrain/", $name, "_color.jpg")).as_slice(),
            include_bytes!(concat!("../../assets/textures/terrain/", $name, "_normal.jpg")).as_slice(),
            include_bytes!(concat!("../../assets/textures/terrain/", $name, "_rough_ao.jpg")).as_slice(),
        ]
    };
}

/// Colour, normal and roughness/occlusion files per layer, in layer order.
const FILES: [[&[u8]; 3]; 9] = [
    layer!("grass"),
    layer!("dirt"),
    layer!("stone"),
    layer!("sand"),
    layer!("planks"),
    layer!("bark"),
    layer!("leaves"),
    layer!("snow"),
    layer!("bricks"),
];

/// Grass shows this layer on its sides, like a cut through the turf.
pub const DIRT_LAYER: u32 = 1;

/// The texture array layer a material is drawn with.
pub fn layer(material: Material) -> u32 {
    match material {
        Material::Air | Material::Grass => 0,
        Material::Dirt => DIRT_LAYER,
        Material::Stone | Material::CoalOre | Material::IronOre | Material::DiamondOre => 2,
        Material::Sand => 3,
        Material::Planks => 4,
        Material::Wood => 5,
        Material::Leaves => 6,
        Material::Snow => 7,
        Material::Bricks => 8,
    }
}

pub struct TerrainTextures {
    pub color: Handle<Image>,
    pub normal: Handle<Image>,
    pub rough_ao: Handle<Image>,
}

pub fn load(images: &mut Assets<Image>) -> TerrainTextures {
    TerrainTextures {
        color: images.add(array(0, true)),
        normal: images.add(array(1, false)),
        rough_ao: images.add(array(2, false)),
    }
}

/// One texture array from the `map`-th file of every layer.
fn array(map: usize, srgb: bool) -> Image {
    let levels = SIZE.ilog2() + 1;
    let mut data = Vec::new();
    for files in FILES {
        let image = Image::from_buffer(
            files[map],
            ImageType::Extension("jpg"),
            CompressedImageFormats::NONE,
            srgb,
            ImageSampler::Default,
            RenderAssetUsages::RENDER_WORLD,
        )
        .expect("bundled terrain textures decode");
        let mut level = image.data.expect("decoded pixels");
        assert_eq!(level.len(), (SIZE * SIZE * 4) as usize, "terrain textures must be {SIZE}x{SIZE}");
        let mut size = SIZE;
        // Layer-major: every mip of a layer before the next layer.
        data.extend_from_slice(&level);
        for _ in 1..levels {
            level = halve(&level, size);
            size /= 2;
            data.extend_from_slice(&level);
        }
    }

    let format = if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm };
    Image {
        data: Some(data),
        texture_descriptor: TextureDescriptor {
            label: None,
            size: Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: FILES.len() as u32 },
            mip_level_count: levels,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        },
        sampler: ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            anisotropy_clamp: 4,
            ..default()
        }),
        texture_view_descriptor: Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..default()
        }),
        asset_usage: RenderAssetUsages::RENDER_WORLD,
        ..default()
    }
}

/// Box-filters an RGBA8 square image to half its size.
fn halve(pixels: &[u8], size: u32) -> Vec<u8> {
    let half = size / 2;
    let mut out = vec![0u8; (half * half * 4) as usize];
    for y in 0..half {
        for x in 0..half {
            for c in 0..4 {
                let at = |dx: u32, dy: u32| pixels[(((y * 2 + dy) * size + x * 2 + dx) * 4 + c) as usize] as u32;
                out[((y * half + x) * 4 + c) as usize] = ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halving_averages() {
        let pixels = [[0u8, 0, 0, 255], [100, 0, 0, 255], [0, 200, 0, 255], [100, 200, 40, 255]].concat();
        assert_eq!(halve(&pixels, 2), vec![50, 100, 10, 255]);
    }

    #[test]
    fn every_solid_material_has_a_layer() {
        for m in Material::ALL {
            assert!((layer(m) as usize) < FILES.len());
        }
    }
}
