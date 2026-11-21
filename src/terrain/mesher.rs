//! Turns a chunk of blocks into a mesh: cube faces are merged into large rectangles
//! (greedy meshing), shaped cells add their own faces, and corners get ambient occlusion.

use std::collections::HashMap;

use bevy::math::{IVec3, Vec3};
use meshopt::{SimplifyOptions, VertexDataAdapter, simplify_with_attributes_and_locks, typed_to_bytes};

use super::{
    blocks::{Block, CHUNK_CELLS, Material, SIDES, Shape, opposite},
    textures::layer,
};

/// Cells per axis given to the mesher: the chunk plus a one-cell apron on each side.
pub const SAMPLES: i32 = CHUNK_CELLS + 2;
/// How many cells below the surface chunk borders still get walls, hiding cracks
/// against neighbours drawn at a different level of detail.
const SKIRT_DEPTH: i32 = 3;
/// Brightness for 0..=3 open neighbours around a vertex.
const AO_CURVE: [f32; 4] = [0.5, 0.68, 0.84, 1.0];

#[derive(Default)]
pub struct ChunkMesh {
    /// Relative to the chunk origin, in metres.
    pub positions: Vec<[f32; 3]>,
    /// Material colour, with ambient occlusion in alpha so the shader can fade it with distance.
    pub colors: Vec<[f32; 4]>,
    /// Normal of the surface smoothed over about a metre, ignoring individual blocks. Far away
    /// the shader lights with this, so terraces of small blocks read as gentle slopes.
    pub smooth: Vec<[f32; 3]>,
    /// Texture layer, and a surface code: 0 plain, 1 coal, 2 iron, -1 skirt.
    pub ore: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl ChunkMesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn triangles(&self) -> Vec<[u32; 3]> {
        self.indices.as_chunks::<3>().0.to_vec()
    }
}

struct Grid<'a> {
    blocks: &'a [Block],
}

impl Grid<'_> {
    fn at(&self, p: IVec3) -> Block {
        self.blocks[(p.x + SAMPLES * (p.y + SAMPLES * p.z)) as usize]
    }

    fn solid(&self, p: IVec3) -> bool {
        !self.at(p).is_air()
    }

    /// Ambient occlusion at the corner (`du`, `dv`) of the face of `cell` on `side`.
    fn ao(&self, cell: IVec3, side: usize, du: i32, dv: i32) -> usize {
        let (u, v) = tangents(side);
        let front = cell + SIDES[side];
        let a = self.solid(front + u * du);
        let b = self.solid(front + v * dv);
        let c = self.solid(front + u * du + v * dv);
        if a && b { 0 } else { 3 - (a as usize + b as usize + c as usize) }
    }

    /// Points away from nearby solid cells; `p` is in grid units (apron included).
    fn smooth_normal(&self, p: Vec3, fallback: Vec3) -> Vec3 {
        let base = p.round().as_ivec3();
        let mut sum = Vec3::ZERO;
        for z in -2..2 {
            for y in -2..2 {
                for x in -2..2 {
                    let c = (base + IVec3::new(x, y, z)).clamp(IVec3::ZERO, IVec3::splat(SAMPLES - 1));
                    if self.solid(c) {
                        sum -= (base + IVec3::new(x, y, z)).as_vec3() + 0.5 - p;
                    }
                }
            }
        }
        let n = sum.normalize_or(fallback);
        // Never let it face away from the actual face, or lighting flips.
        if n.dot(fallback) < 0.1 { (n + fallback).normalize_or(fallback) } else { n }
    }

    /// Material at the top of the solid column above `cell`, so skirts peeking through
    /// cracks match the ground around them.
    fn surface_material(&self, cell: IVec3) -> Material {
        let mut c = cell;
        while c.y + 1 < SAMPLES && self.solid(c + IVec3::Y) {
            c.y += 1;
        }
        self.at(c).material
    }

    fn in_apron(p: IVec3) -> bool {
        p.cmplt(IVec3::ONE).any() || p.cmpgt(IVec3::splat(CHUNK_CELLS)).any()
    }

    /// Whether the face of `block` at `cell` on `side` can be seen.
    fn exposed(&self, cell: IVec3, block: Block, side: usize) -> bool {
        let neighbour_cell = cell + SIDES[side];
        let neighbour = self.at(neighbour_cell);
        let hidden = neighbour.covers(opposite(side))
            || (neighbour.shape == block.shape
                && !neighbour.is_air()
                && block.shape.geometry().self_hiding & (1 << side) != 0);
        if !hidden {
            return true;
        }
        // Skirt: walls along the chunk border just under the surface.
        side != 2
            && side != 3
            && Self::in_apron(neighbour_cell)
            && (1..=SKIRT_DEPTH).any(|k| {
                let above = cell + IVec3::Y * k;
                above.y < SAMPLES && !self.at(above).covers(3)
            })
    }
}

/// The two in-plane axes of a side, ordered so that u × v points along the side's normal.
fn tangents(side: usize) -> (IVec3, IVec3) {
    let (u, v) = match side / 2 {
        0 => (IVec3::Y, IVec3::Z),
        1 => (IVec3::Z, IVec3::X),
        _ => (IVec3::X, IVec3::Y),
    };
    if side.is_multiple_of(2) { (u, v) } else { (v, u) }
}

/// `blocks` holds `SAMPLES`³ cells starting one cell below the chunk origin on every axis;
/// `cell` is the edge length in metres.
pub fn mesh(blocks: &[Block], cell: f32) -> ChunkMesh {
    assert_eq!(blocks.len(), (SAMPLES * SAMPLES * SAMPLES) as usize);
    let grid = Grid { blocks };
    let mut out = ChunkMesh::default();
    for side in 0..6 {
        greedy_cubes(&grid, side, cell, &mut out);
    }
    shaped_cells(&grid, cell, &mut out);
    out
}

#[derive(Clone, Copy, PartialEq)]
struct FaceKey {
    material: Material,
    ao: [u8; 4],
    skirt: bool,
}

/// Exposed cube faces on one side, merged slice by slice into rectangles. Faces only merge
/// when their corner occlusion is uniform, so shading stays correct.
fn greedy_cubes(grid: &Grid, side: usize, cell: f32, out: &mut ChunkMesh) {
    let (u, v) = tangents(side);
    let normal = SIDES[side];
    let n = CHUNK_CELLS as usize;
    let mut mask: Vec<Option<FaceKey>> = vec![None; n * n];

    for slice in 1..=CHUNK_CELLS {
        for j in 0..n {
            for i in 0..n {
                let c = normal.abs() * slice + u * (i as i32 + 1) + v * (j as i32 + 1);
                let block = grid.at(c);
                mask[i + j * n] =
                    (!block.is_air() && block.shape == Shape::CUBE && grid.exposed(c, block, side)).then(|| {
                        // Skirts are only ever seen through cracks; plain light lets them merge.
                        let front = c + normal;
                        let skirt = Grid::in_apron(front) && grid.at(front).covers(opposite(side));
                        let (material, ao) = if skirt {
                            (grid.surface_material(c), [3; 4])
                        } else {
                            (
                                block.material,
                                [(-1, -1), (1, -1), (1, 1), (-1, 1)].map(|(du, dv)| grid.ao(c, side, du, dv) as u8),
                            )
                        };
                        FaceKey { material, ao, skirt }
                    });
            }
        }

        for j in 0..n {
            let mut i = 0;
            while i < n {
                let Some(key) = mask[i + j * n] else {
                    i += 1;
                    continue;
                };
                // Cells on the chunk border stay single, so their vertices line up with the
                // neighbour's and no T-junction cracks open along the seam.
                let border = i == 0 || j == 0 || i == n - 1 || j == n - 1;
                let uniform = !border && key.ao.iter().all(|a| *a == key.ao[0]);
                let mut width = 1;
                let mut height = 1;
                if uniform {
                    while i + width < n - 1 && mask[i + width + j * n] == Some(key) {
                        width += 1;
                    }
                    'grow: while j + height < n - 1 {
                        for k in 0..width {
                            if mask[i + k + (j + height) * n] != Some(key) {
                                break 'grow;
                            }
                        }
                        height += 1;
                    }
                }
                for jj in 0..height {
                    for ii in 0..width {
                        mask[i + ii + (j + jj) * n] = None;
                    }
                }

                // Cell coordinates (apron included) of the rectangle's corners on the face plane.
                let plane = normal.abs() * (slice + i32::from(side.is_multiple_of(2)));
                let at = |du: usize, dv: usize| plane + u * (i + du) as i32 + v * (j + dv) as i32 + u + v;
                let corners = [at(0, 0), at(width, 0), at(width, height), at(0, height)];
                emit_quad(grid, out, corners, cell, normal.as_vec3(), &key);
                i += width;
            }
        }
    }
}

/// `corners` are in grid units (apron included).
fn emit_quad(grid: &Grid, out: &mut ChunkMesh, corners: [IVec3; 4], cell: f32, normal: Vec3, key: &FaceKey) {
    let base = out.positions.len() as u32;
    let (material, ao) = (key.material, key.ao);
    let color = [1.0; 3];
    // Skirts are flagged so the shader lights them like the ground.
    let ore = if key.skirt { [layer(material) as f32, -1.0] } else { ore_weights(material) };
    for (p, a) in corners.iter().zip(ao) {
        out.positions.push(((*p - IVec3::ONE).as_vec3() * cell).to_array());
        out.smooth.push(grid.smooth_normal(p.as_vec3(), normal).to_array());
        out.colors.push([color[0], color[1], color[2], AO_CURVE[a as usize]]);
        out.ore.push(ore);
    }
    // Split along the diagonal with more light so occlusion doesn't streak.
    if ao[0] as u32 + ao[2] as u32 >= ao[1] as u32 + ao[3] as u32 {
        out.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    } else {
        out.indices.extend([base + 1, base + 2, base + 3, base + 1, base + 3, base]);
    }
}

/// Texture layer and surface code (see [`ChunkMesh::ore`]).
fn ore_weights(material: Material) -> [f32; 2] {
    let code = match material {
        Material::CoalOre => 1.0,
        Material::IronOre => 2.0,
        Material::DiamondOre => 3.0,
        _ => 0.0,
    };
    [layer(material) as f32, code]
}

/// Slabs, slopes and corners: each face emitted on its own.
fn shaped_cells(grid: &Grid, cell: f32, out: &mut ChunkMesh) {
    for z in 1..=CHUNK_CELLS {
        for y in 1..=CHUNK_CELLS {
            for x in 1..=CHUNK_CELLS {
                let c = IVec3::new(x, y, z);
                let block = grid.at(c);
                if block.is_air() || block.shape == Shape::CUBE {
                    continue;
                }
                let color = [1.0; 3];
                let ore = ore_weights(block.material);
                let origin = (c - IVec3::ONE).as_vec3();
                for polygon in &block.shape.geometry().polygons {
                    if polygon.side.is_some_and(|s| !grid.exposed(c, block, s)) {
                        continue;
                    }
                    // Occlusion from the side the face mostly looks towards.
                    let n = polygon.normal;
                    let side = dominant_side(n);
                    let (u, v) = tangents(side);
                    let base = out.positions.len() as u32;
                    for p in &polygon.vertices {
                        let corner = |axis: IVec3| if p.dot(axis.as_vec3()) > 0.5 { 1 } else { -1 };
                        let k = AO_CURVE[grid.ao(c, side, corner(u), corner(v))];
                        out.positions.push(((origin + *p) * cell).to_array());
                        out.smooth.push(grid.smooth_normal(c.as_vec3() + *p, n).to_array());
                        out.colors.push([color[0], color[1], color[2], k]);
                        out.ore.push(ore);
                    }
                    for k in 1..polygon.vertices.len() as u32 - 1 {
                        out.indices.extend([base, base + k, base + k + 1]);
                    }
                }
            }
        }
    }
}

/// The cube side closest to a direction; ties go to up or down.
fn dominant_side(n: Vec3) -> usize {
    let a = n.abs();
    if a.y >= a.x - 1e-4 && a.y >= a.z - 1e-4 {
        if n.y > 0.0 { 2 } else { 3 }
    } else if a.x >= a.z {
        if n.x > 0.0 { 0 } else { 1 }
    } else if n.z > 0.0 {
        4
    } else {
        5
    }
}

impl ChunkMesh {
    /// For distant chunks: joins faces that share corners and lets meshopt collapse flat runs
    /// of small blocks into large triangles. Shading switches to the smoothed normals, which
    /// is what the shader uses at that distance anyway. Chunk borders stay put.
    pub fn simplified(self, size: f32, max_error: f32) -> ChunkMesh {
        let mut welded = ChunkMesh::default();
        // Faces only join within one texture layer, so no triangle blends two materials.
        let mut index_of: HashMap<([i32; 3], i32), u32> = HashMap::new();
        let mut layers_at: HashMap<[i32; 3], i32> = HashMap::new();
        let mut boundary: Vec<[i32; 3]> = Vec::new();
        let mut weights: Vec<f32> = Vec::new();
        let remap: Vec<u32> = (0..self.positions.len())
            .map(|i| {
                let p = self.positions[i];
                let pos = p.map(|v| (v * 256.0).round() as i32);
                let layer = self.ore[i][0] as i32;
                if *layers_at.entry(pos).or_insert(layer) != layer {
                    boundary.push(pos);
                }
                *index_of.entry((pos, layer)).or_insert_with(|| {
                    welded.positions.push(p);
                    welded.smooth.push([0.0; 3]);
                    welded.colors.push([0.0; 4]);
                    welded.ore.push([0.0; 2]);
                    weights.push(0.0);
                    (welded.positions.len() - 1) as u32
                })
            })
            .collect();
        for (i, &w) in remap.iter().enumerate() {
            let w = w as usize;
            weights[w] += 1.0;
            for k in 0..3 {
                welded.smooth[w][k] += self.smooth[i][k];
            }
            for k in 0..4 {
                welded.colors[w][k] += self.colors[i][k];
            }
            welded.ore[w][0] = self.ore[i][0];
            welded.ore[w][1] = welded.ore[w][1].max(self.ore[i][1]);
        }
        for (w, n) in weights.iter().enumerate() {
            welded.smooth[w] = Vec3::from(welded.smooth[w]).normalize_or(Vec3::Y).to_array();
            welded.colors[w] = welded.colors[w].map(|c| c / n);
        }
        let indices: Vec<u32> = self.indices.iter().map(|&i| remap[i as usize]).collect();

        let vertices =
            VertexDataAdapter::new(typed_to_bytes(&welded.positions), 12, 0).expect("tightly packed positions");
        let attributes: Vec<f32> = (0..welded.positions.len())
            .flat_map(|i| {
                let (c, n) = (welded.colors[i], welded.smooth[i]);
                [c[0], c[1], c[2], n[0], n[1], n[2]]
            })
            .collect();
        const WEIGHTS: [f32; 6] = [3.0, 3.0, 3.0, 0.3, 0.3, 0.3];
        welded.indices = simplify_with_attributes_and_locks(
            &indices,
            &vertices,
            &attributes,
            &WEIGHTS,
            WEIGHTS.len() * size_of::<f32>(),
            // Vertices on the chunk's faces stay exactly where they are, so neighbours meet.
            // (Skirts join the surface to its border, so meshopt wouldn't see them as edges.)
            // Material boundaries are locked too: both sides keep the same vertices there.
            &{
                let boundary: std::collections::HashSet<[i32; 3]> = boundary.into_iter().collect();
                welded
                    .positions
                    .iter()
                    .map(|p| {
                        p.iter().any(|v| v.abs() < 1e-3 || (v - size).abs() < 1e-3)
                            || boundary.contains(&p.map(|v| (v * 256.0).round() as i32))
                    })
                    .collect::<Vec<_>>()
            },
            0,
            max_error,
            SimplifyOptions::ErrorAbsolute,
            None,
        );
        welded
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn grid_with(fill: impl Fn(IVec3) -> Block) -> Vec<Block> {
        let mut blocks = Vec::with_capacity((SAMPLES * SAMPLES * SAMPLES) as usize);
        for z in 0..SAMPLES {
            for y in 0..SAMPLES {
                for x in 0..SAMPLES {
                    blocks.push(fill(IVec3::new(x, y, z)));
                }
            }
        }
        blocks
    }

    /// Every directed edge of a closed surface has exactly one partner going the other way.
    fn assert_closed(mesh: &ChunkMesh) {
        let key = |i: u32| {
            let p = mesh.positions[i as usize];
            (p[0] * 64.0).round() as i32 * 1_000_000
                + (p[1] * 64.0).round() as i32 * 1000
                + (p[2] * 64.0).round() as i32
        };
        let mut edges: HashMap<(i32, i32), i32> = HashMap::new();
        for t in mesh.indices.as_chunks::<3>().0 {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *edges.entry((key(a), key(b))).or_default() += 1;
            }
        }
        for (&(a, b), &count) in &edges {
            assert_eq!(edges.get(&(b, a)).copied().unwrap_or(0), count, "open edge {a} -> {b}");
        }
    }

    #[test]
    fn single_cube_is_closed_with_outward_normals() {
        let blocks = grid_with(|p| if p == IVec3::splat(10) { Block::solid(Material::Stone) } else { Block::AIR });
        let mesh = mesh(&blocks, 1.0);
        assert_eq!(mesh.indices.len(), 36);
        assert_eq!(mesh.smooth.len(), mesh.positions.len());
        assert_closed(&mesh);
        let center = Vec3::splat(9.5);
        for t in mesh.indices.as_chunks::<3>().0 {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(mesh.positions[i as usize]));
            let n = (b - a).cross(c - a);
            assert!(n.dot((a + b + c) / 3.0 - center) > 0.0, "inward-facing triangle");
        }
    }

    #[test]
    fn every_shape_alone_is_closed() {
        for id in 0..15 {
            let shape = Shape::from_id(id).unwrap();
            let blocks =
                grid_with(|p| if p == IVec3::splat(5) { Block::new(Material::Dirt, shape) } else { Block::AIR });
            let mesh = mesh(&blocks, 0.25);
            assert_closed(&mesh);
        }
    }

    #[test]
    fn flat_ground_merges_into_few_quads() {
        let blocks = grid_with(|p| if p.y <= 20 { Block::solid(Material::Grass) } else { Block::AIR });
        let mesh = mesh(&blocks, 0.25);
        // One rectangle inside, single cells only along the border ring, merged skirts.
        let quads = mesh.indices.len() / 6;
        assert!(quads < 4 * 64 + 40, "{quads} quads");
    }

    #[test]
    fn slopes_next_to_each_other_hide_shared_sides() {
        let row = |p: IVec3| {
            if p.y == 10 && p.x == 10 && (10..14).contains(&p.z) {
                Block::new(Material::Dirt, Shape::slope(0))
            } else {
                Block::AIR
            }
        };
        let joined = mesh(&grid_with(row), 1.0).indices.len();
        let single = mesh(
            &grid_with(
                |p| if p == IVec3::splat(10) { Block::new(Material::Dirt, Shape::slope(0)) } else { Block::AIR },
            ),
            1.0,
        )
        .indices
        .len();
        // Four slopes in a row: the two triangles at each of the three joints are dropped.
        assert_eq!(joined, single * 4 - 3 * 2 * 3);
    }
}
