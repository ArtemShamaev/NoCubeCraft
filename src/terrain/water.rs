//! Lakes and seas: still water filling every column whose ground lies below sea level.

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

use super::generator::Generator;

pub const SEA_LEVEL: f32 = 15.5;
/// Water is meshed in square tiles this wide, independent of terrain chunks.
pub const TILE: f32 = 32.0;
/// Water quads are this many metres wide.
const CELL: f32 = 2.0;
/// Quads reaching a little under the shore keep the waterline free of gaps; the ground hides them.
const SHORE_OVERLAP: f32 = 1.5;

/// Water surface of one tile, relative to the tile's corner at height zero.
pub fn mesh(generator: &Generator, tile: IVec2) -> Option<Mesh> {
    let origin = tile.as_vec2() * TILE;
    let n = (TILE / CELL) as i32 + 1;
    let wet: Vec<bool> = (0..n * n)
        .map(|i| {
            let p = origin + Vec2::new((i % n) as f32, (i / n) as f32) * CELL;
            generator.height(p.x, p.y) < SEA_LEVEL + SHORE_OVERLAP
        })
        .collect();

    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for z in 0..n - 1 {
        for x in 0..n - 1 {
            let corners = [(x, z), (x + 1, z), (x + 1, z + 1), (x, z + 1)];
            if !corners.iter().any(|&(x, z)| wet[(x + z * n) as usize]) {
                continue;
            }
            let base = positions.len() as u32;
            positions.extend(corners.map(|(x, z)| Vec3::new(x as f32 * CELL, SEA_LEVEL, z as f32 * CELL)));
            indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
        }
    }
    if positions.is_empty() {
        return None;
    }

    let normals = vec![Vec3::Y; positions.len()];
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32(indices));
    Some(mesh)
}

pub type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterExtension>;

/// Moving waves, sun glints and a view-dependent tint over the standard material.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct WaterExtension {}

impl MaterialExtension for WaterExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://barnocraft/terrain/water.wesl".into()
    }
}

pub fn new_material() -> WaterMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::srgba(0.1, 0.3, 0.4, 0.75),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.05,
            reflectance: 0.6,
            // Visible from below while swimming.
            cull_mode: None,
            double_sided: true,
            ..default()
        },
        extension: WaterExtension {},
    }
}
