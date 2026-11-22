use bevy::{
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

use super::{blocks::Material, textures::TerrainTextures};

pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TerrainExtension>;

/// Block textures from arrays (one layer per material), bevelled cell edges and ore flecks
/// on top of the standard PBR material.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct TerrainExtension {
    #[texture(100, dimension = "2d_array")]
    #[sampler(101)]
    color: Handle<Image>,
    #[texture(102, dimension = "2d_array")]
    normal: Handle<Image>,
    #[texture(103, dimension = "2d_array")]
    rough_ao: Handle<Image>,
}

impl MaterialExtension for TerrainExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://nocubecraft/terrain/terrain.wesl".into()
    }
}

pub fn new_material(textures: TerrainTextures) -> TerrainMaterial {
    ExtendedMaterial {
        base: StandardMaterial { perceptual_roughness: 0.92, reflectance: 0.2, ..default() },
        extension: TerrainExtension { color: textures.color, normal: textures.normal, rough_ao: textures.rough_ao },
    }
}

/// Average colour of each material's texture, in linear space: for debris, the held block and
/// anything else drawn without the terrain textures.
pub fn albedo(material: Material) -> [f32; 3] {
    match material {
        Material::Air => [0.0; 3],
        Material::Grass => [0.118, 0.153, 0.026],
        Material::Dirt => [0.201, 0.141, 0.081],
        Material::Stone | Material::CoalOre | Material::IronOre | Material::DiamondOre => [0.077, 0.072, 0.058],
        Material::Sand => [0.469, 0.349, 0.166],
        Material::Planks => [0.078, 0.044, 0.023],
        Material::Wood => [0.195, 0.135, 0.080],
        Material::Leaves => [0.066, 0.133, 0.006],
        Material::Snow => [0.742, 0.863, 0.973],
        Material::Bricks => [0.219, 0.181, 0.127],
        Material::Sandstone => [0.32, 0.24, 0.12],
        Material::Gravel => [0.238, 0.209, 0.131],
        Material::Clay => [0.349, 0.190, 0.107],
        Material::Ice => [0.25, 0.4, 0.5],
        Material::Cactus => [0.05, 0.14, 0.02],
        Material::Brace => [0.17, 0.08, 0.035],
        Material::Activator => [0.15, 0.27, 0.25],
        Material::Shardstone => [0.12, 0.07, 0.15],
        Material::Core => [0.2, 0.6, 0.65],
        Material::Rift => [0.35, 0.15, 0.6],
        Material::Cache => [0.45, 0.3, 0.08],
        Material::Altar => [0.4, 0.3, 0.5],
    }
}
