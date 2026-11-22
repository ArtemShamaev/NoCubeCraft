//! Blocks: a material and a shape per 25 cm cell. Shapes (slabs, slopes, corners) let the
//! terrain and buildings follow gentle curves instead of stair-stepping.

use std::sync::LazyLock;

use bevy::math::{IVec3, Vec3};
use serde::{Deserialize, Serialize};

/// Edge of one cell, in metres.
pub const CELL: f32 = 0.25;
/// Cells per chunk edge.
pub const CHUNK_CELLS: i32 = 64;
/// Chunk edge at the finest level of detail, in metres.
pub const CHUNK_SIZE: f32 = CELL * CHUNK_CELLS as f32;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum Material {
    #[default]
    Air,
    Grass,
    Dirt,
    Stone,
    Sand,
    CoalOre,
    IronOre,
    Planks,
    Wood,
    Leaves,
    Snow,
    Bricks,
    DiamondOre,
    Sandstone,
    Gravel,
    Clay,
    Ice,
    Cactus,
    /// A frame that holds every block it touches to its structure.
    Brace,
    /// Sets the structure it belongs to moving as one physical body.
    Activator,
    /// Violet rock of the Shards.
    Shardstone,
    /// The glowing heart of a shard.
    Core,
    /// A way between the world and the Shards.
    Rift,
    /// A hoard left in ruins: digging it out gives loot.
    Cache,
    /// The Shards' altar, where the Warden is called.
    Altar,
}

impl Material {
    pub const ALL: [Material; 25] = [
        Material::Air,
        Material::Grass,
        Material::Dirt,
        Material::Stone,
        Material::Sand,
        Material::CoalOre,
        Material::IronOre,
        Material::Planks,
        Material::Wood,
        Material::Leaves,
        Material::Snow,
        Material::Bricks,
        Material::DiamondOre,
        Material::Sandstone,
        Material::Gravel,
        Material::Clay,
        Material::Ice,
        Material::Cactus,
        Material::Brace,
        Material::Activator,
        Material::Shardstone,
        Material::Core,
        Material::Rift,
        Material::Cache,
        Material::Altar,
    ];

    pub fn from_u8(value: u8) -> Option<Material> {
        Self::ALL.get(value as usize).copied()
    }
}

/// Six cube sides, in the order +X, -X, +Y, -Y, +Z, -Z.
pub const SIDES: [IVec3; 6] = [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z];

pub fn opposite(side: usize) -> usize {
    side ^ 1
}

/// What fills a cell. Shape ids: 0 cube, 1 bottom slab, 2 top slab, 3..7 slopes,
/// 7..11 outer corners, 11..15 inner corners; the last three are rotated a quarter turn each.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Shape(u8);

impl Shape {
    pub const CUBE: Shape = Shape(0);
    pub const SLAB: Shape = Shape(1);
    pub const SLAB_TOP: Shape = Shape(2);
    const COUNT: u8 = 15;

    /// A ramp rising towards `dir`: 0 = +X, 1 = +Z, 2 = -X, 3 = -Z.
    pub fn slope(dir: u8) -> Shape {
        Shape(3 + dir % 4)
    }

    /// A pyramid corner whose high point is towards the diagonal `dir`:
    /// 0 = (+X, +Z), 1 = (-X, +Z), 2 = (-X, -Z), 3 = (+X, -Z).
    pub fn outer_corner(dir: u8) -> Shape {
        Shape(7 + dir % 4)
    }

    /// A cell full except for a low corner opposite the diagonal `dir`.
    pub fn inner_corner(dir: u8) -> Shape {
        Shape(11 + dir % 4)
    }

    pub fn from_id(id: u8) -> Option<Shape> {
        (id < Self::COUNT).then_some(Shape(id))
    }

    pub fn geometry(self) -> &'static ShapeGeometry {
        &GEOMETRY[self.0 as usize]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Block {
    pub material: Material,
    pub shape: Shape,
    /// Smeared with glue: sticks to the structure a physics activator sets moving.
    pub glued: bool,
}

impl Block {
    pub const AIR: Block = Block { material: Material::Air, shape: Shape::CUBE, glued: false };

    pub fn new(material: Material, shape: Shape) -> Self {
        Self { material, shape, glued: false }
    }

    pub fn solid(material: Material) -> Self {
        Self::new(material, Shape::CUBE)
    }

    pub fn is_air(self) -> bool {
        self.material == Material::Air
    }

    /// Whether this block completely covers its `side`.
    pub fn covers(self, side: usize) -> bool {
        !self.is_air() && self.shape.geometry().full & (1 << side) != 0
    }

    pub fn to_bits(self) -> u16 {
        self.material as u16 | (self.shape.0 as u16) << 8 | (self.glued as u16) << 15
    }

    pub fn from_bits(bits: u16) -> Option<Block> {
        Some(Block {
            material: Material::from_u8(bits as u8)?,
            shape: Shape::from_id((bits >> 8) as u8 & 0x7f)?,
            glued: bits & 0x8000 != 0,
        })
    }
}

/// A face of a shape, in unit-cell coordinates, wound counter-clockwise seen from outside.
pub struct Polygon {
    pub vertices: Vec<Vec3>,
    pub normal: Vec3,
    /// The cube side this face lies on, if any; such faces hide behind a neighbour covering that side.
    pub side: Option<usize>,
}

pub struct ShapeGeometry {
    pub polygons: Vec<Polygon>,
    /// Bit per side that the shape covers completely.
    pub full: u8,
    /// Sides whose face is hidden by an identical neighbouring shape (e.g. slabs side by side).
    pub self_hiding: u8,
}

static GEOMETRY: LazyLock<Vec<ShapeGeometry>> = LazyLock::new(|| {
    let mut shapes = vec![cube(), slab(false), slab(true)];
    for builder in [slope as fn() -> ShapeGeometry, outer_corner, inner_corner] {
        for turns in 0..4 {
            shapes.push(rotate(builder(), turns));
        }
    }
    shapes
});

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

/// Builds polygons with outward winding, using a point inside the shape for orientation.
fn shape(inside: Vec3, faces: Vec<(Vec<Vec3>, Option<usize>)>, full: u8, self_hiding: u8) -> ShapeGeometry {
    let polygons = faces
        .into_iter()
        .map(|(mut vertices, side)| {
            let centroid = vertices.iter().copied().sum::<Vec3>() / vertices.len() as f32;
            let mut normal = (vertices[1] - vertices[0]).cross(vertices[2] - vertices[0]).normalize();
            if normal.dot(centroid - inside) < 0.0 {
                vertices.reverse();
                normal = -normal;
            }
            Polygon { vertices, normal, side }
        })
        .collect();
    ShapeGeometry { polygons, full, self_hiding }
}

/// The four corners of cube side `side`, at the given height range for vertical sides.
fn side_quad(side: usize, y0: f32, y1: f32) -> Vec<Vec3> {
    match side {
        0 => vec![v(1., y0, 0.), v(1., y1, 0.), v(1., y1, 1.), v(1., y0, 1.)],
        1 => vec![v(0., y0, 0.), v(0., y0, 1.), v(0., y1, 1.), v(0., y1, 0.)],
        2 => vec![v(0., y1, 0.), v(0., y1, 1.), v(1., y1, 1.), v(1., y1, 0.)],
        3 => vec![v(0., y0, 0.), v(1., y0, 0.), v(1., y0, 1.), v(0., y0, 1.)],
        4 => vec![v(0., y0, 1.), v(1., y0, 1.), v(1., y1, 1.), v(0., y1, 1.)],
        _ => vec![v(0., y0, 0.), v(0., y1, 0.), v(1., y1, 0.), v(1., y0, 0.)],
    }
}

fn cube() -> ShapeGeometry {
    let faces = (0..6).map(|s| (side_quad(s, 0.0, 1.0), Some(s))).collect();
    shape(Vec3::splat(0.5), faces, 0b11_1111, 0)
}

fn slab(top: bool) -> ShapeGeometry {
    let (y0, y1) = if top { (0.5, 1.0) } else { (0.0, 0.5) };
    let mut faces: Vec<_> = [0, 1, 4, 5].into_iter().map(|s| (side_quad(s, y0, y1), Some(s))).collect();
    let (full_side, half_side) = if top { (2, 3) } else { (3, 2) };
    faces.push((side_quad(full_side, 0.0, 1.0), Some(full_side)));
    let mut inner = side_quad(half_side, 0.0, 1.0);
    for p in &mut inner {
        p.y = 0.5;
    }
    faces.push((inner, None));
    shape(v(0.5, (y0 + y1) / 2.0, 0.5), faces, 1 << full_side, 0b11_0011)
}

/// Rising towards +X.
fn slope() -> ShapeGeometry {
    let faces = vec![
        (side_quad(3, 0.0, 1.0), Some(3)),
        (side_quad(0, 0.0, 1.0), Some(0)),
        (vec![v(0., 0., 0.), v(0., 0., 1.), v(1., 1., 1.), v(1., 1., 0.)], None),
        (vec![v(0., 0., 0.), v(1., 0., 0.), v(1., 1., 0.)], Some(5)),
        (vec![v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.)], Some(4)),
    ];
    shape(v(0.75, 0.25, 0.5), faces, 1 << 3 | 1 << 0, 1 << 4 | 1 << 5)
}

/// High point above the (+X, +Z) corner.
fn outer_corner() -> ShapeGeometry {
    let faces = vec![
        (side_quad(3, 0.0, 1.0), Some(3)),
        (vec![v(1., 0., 0.), v(1., 0., 1.), v(1., 1., 1.)], Some(0)),
        (vec![v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.)], Some(4)),
        (vec![v(0., 0., 0.), v(0., 0., 1.), v(1., 1., 1.)], None),
        (vec![v(0., 0., 0.), v(1., 0., 0.), v(1., 1., 1.)], None),
    ];
    shape(v(0.8, 0.2, 0.8), faces, 1 << 3, 0)
}

/// Full except for a low corner at (-X, -Z).
fn inner_corner() -> ShapeGeometry {
    let faces = vec![
        (side_quad(3, 0.0, 1.0), Some(3)),
        (side_quad(0, 0.0, 1.0), Some(0)),
        (side_quad(4, 0.0, 1.0), Some(4)),
        (vec![v(0., 0., 0.), v(0., 0., 1.), v(0., 1., 1.)], Some(1)),
        (vec![v(0., 0., 0.), v(1., 0., 0.), v(1., 1., 0.)], Some(5)),
        (vec![v(1., 1., 0.), v(0., 1., 1.), v(1., 1., 1.)], Some(2)),
        (vec![v(0., 0., 0.), v(1., 1., 0.), v(0., 1., 1.)], None),
    ];
    shape(v(0.7, 0.6, 0.7), faces, 1 << 3 | 1 << 0 | 1 << 4, 0)
}

/// Turns a shape a quarter turn at a time about the vertical axis: +X goes to +Z.
fn rotate(geometry: ShapeGeometry, turns: u8) -> ShapeGeometry {
    // Side index after one turn: +X -> +Z -> -X -> -Z -> +X; up and down stay.
    const TURN: [usize; 6] = [4, 5, 2, 3, 1, 0];
    let turn_point = |p: Vec3| v(1.0 - p.z, p.y, p.x);
    let turn_vector = |n: Vec3| v(-n.z, n.y, n.x);
    let turn_mask = |mask: u8| (0..6).filter(|s| mask & (1 << s) != 0).fold(0u8, |m, s| m | 1 << TURN[s]);

    let mut g = geometry;
    for _ in 0..turns {
        for polygon in &mut g.polygons {
            for p in &mut polygon.vertices {
                *p = turn_point(*p);
            }
            polygon.normal = turn_vector(polygon.normal);
            polygon.side = polygon.side.map(|s| TURN[s]);
        }
        g.full = turn_mask(g.full);
        g.self_hiding = turn_mask(g.self_hiding);
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_faces_point_outwards_and_lie_on_their_sides() {
        for id in 0..Shape::COUNT {
            let g = Shape(id).geometry();
            for polygon in &g.polygons {
                let n = (polygon.vertices[1] - polygon.vertices[0]).cross(polygon.vertices[2] - polygon.vertices[0]);
                assert!(n.normalize().dot(polygon.normal) > 0.99, "shape {id}: winding disagrees with normal");
                if let Some(side) = polygon.side {
                    let axis = SIDES[side].as_vec3();
                    assert!(polygon.normal.dot(axis) > 0.99, "shape {id}: face on side {side} points elsewhere");
                    let plane = if axis.max_element() > 0.0 { 1.0 } else { 0.0 };
                    for p in &polygon.vertices {
                        assert!((p.dot(axis.abs()) - plane).abs() < 1e-5, "shape {id}: vertex off its side");
                    }
                }
            }
        }
    }

    #[test]
    fn rotated_slopes_rise_the_right_way() {
        // The high side of slope(dir) is fully covered.
        for (dir, side) in [(0, 0), (1, 4), (2, 1), (3, 5)] {
            let block = Block::new(Material::Stone, Shape::slope(dir));
            assert!(block.covers(side), "slope {dir} should cover side {side}");
            assert!(!block.covers(opposite(side)));
            assert!(block.covers(3));
            assert!(!block.covers(2));
        }
    }

    #[test]
    fn bits_round_trip() {
        let block = Block::new(Material::Bricks, Shape::inner_corner(2));
        assert_eq!(Block::from_bits(block.to_bits()), Some(block));
        let glued = Block { glued: true, ..block };
        assert_eq!(Block::from_bits(glued.to_bits()), Some(glued));
        assert_eq!(Block::from_bits(0xff00), None);
    }
}
