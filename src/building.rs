//! What a click builds or digs: block size, shape and orientation, and the cells involved.

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};

use crate::terrain::{CELL, Shape};

/// Edge of what is placed or dug at once, in 25 cm cells.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockSize {
    Quarter,
    Half,
    Full,
}

impl BlockSize {
    pub fn cells(self) -> i32 {
        match self {
            BlockSize::Quarter => 1,
            BlockSize::Half => 2,
            BlockSize::Full => 4,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            BlockSize::Quarter => "25 см",
            BlockSize::Half => "50 см",
            BlockSize::Full => "1 м",
        }
    }

    pub fn next(self) -> Self {
        match self {
            BlockSize::Full => BlockSize::Half,
            BlockSize::Half => BlockSize::Quarter,
            BlockSize::Quarter => BlockSize::Full,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuildShape {
    Cube,
    Slab,
    SlabTop,
    Slope,
    Stairs,
    Pillar,
}

impl BuildShape {
    pub const ALL: [BuildShape; 6] = [
        BuildShape::Cube,
        BuildShape::Slab,
        BuildShape::SlabTop,
        BuildShape::Slope,
        BuildShape::Stairs,
        BuildShape::Pillar,
    ];

    pub fn name(self) -> &'static str {
        match self {
            BuildShape::Cube => "куб",
            BuildShape::Slab => "плита",
            BuildShape::SlabTop => "плита сверху",
            BuildShape::Slope => "скос",
            BuildShape::Stairs => "ступени",
            BuildShape::Pillar => "столб",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

#[derive(Resource)]
pub struct BuildTool {
    pub size: BlockSize,
    pub shape: BuildShape,
}

impl Default for BuildTool {
    fn default() -> Self {
        Self { size: BlockSize::Full, shape: BuildShape::Cube }
    }
}

/// The horizontal direction a player looks along, as a slope direction (0 +X, 1 +Z, 2 -X, 3 -Z).
pub fn facing(forward: Vec3) -> u8 {
    if forward.x.abs() > forward.z.abs() {
        if forward.x > 0.0 { 0 } else { 2 }
    } else if forward.z > 0.0 {
        1
    } else {
        3
    }
}

/// Cells (relative to the block's low corner) and their shapes for a block of `n` cells,
/// rising towards `dir` where the shape has a direction.
pub fn compose(shape: BuildShape, n: i32, dir: u8) -> Vec<(IVec3, Shape)> {
    let mut cells = Vec::new();
    // Position along `dir`, 0 at the low end.
    let along = |p: IVec3| match dir {
        0 => p.x,
        1 => p.z,
        2 => n - 1 - p.x,
        _ => n - 1 - p.z,
    };
    let half = (n / 2).max(1);
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let p = IVec3::new(x, y, z);
                let t = along(p);
                let cell = match shape {
                    BuildShape::Cube => Some(Shape::CUBE),
                    BuildShape::Slab if n == 1 => Some(Shape::SLAB),
                    BuildShape::Slab => (y < half).then_some(Shape::CUBE),
                    BuildShape::SlabTop if n == 1 => Some(Shape::SLAB_TOP),
                    BuildShape::SlabTop => (y >= n - half).then_some(Shape::CUBE),
                    BuildShape::Slope if y < t => Some(Shape::CUBE),
                    BuildShape::Slope => (y == t).then_some(Shape::slope(dir)),
                    BuildShape::Stairs if n == 1 => Some(Shape::SLAB),
                    BuildShape::Stairs => {
                        let height = if t < half { half } else { n };
                        (y < height).then_some(Shape::CUBE)
                    }
                    BuildShape::Pillar => {
                        let lo = n / 4;
                        let hi = n - n / 4;
                        ((lo..hi).contains(&x) && (lo..hi).contains(&z)).then_some(Shape::CUBE)
                    }
                };
                if let Some(s) = cell {
                    cells.push((p, s));
                }
            }
        }
    }
    cells
}

/// Low corner of an `n`-cell block that sits on the face of `cell` facing `normal`:
/// flush against the face, snapped to the block grid sideways.
pub fn place_origin(cell: IVec3, normal: IVec3, n: i32) -> IVec3 {
    let adjacent = cell + normal;
    let snapped = adjacent.div_euclid(IVec3::splat(n)) * n;
    let mut origin = snapped;
    for axis in 0..3 {
        if normal[axis] > 0 {
            origin[axis] = adjacent[axis];
        } else if normal[axis] < 0 {
            origin[axis] = adjacent[axis] - (n - 1);
        }
    }
    origin
}

/// Low corner of the `n`-cell grid block containing `cell`, for digging.
pub fn dig_origin(cell: IVec3, n: i32) -> IVec3 {
    cell.div_euclid(IVec3::splat(n)) * n
}

/// A see-through preview of the cells a click would fill, in metres from the block's corner.
pub fn ghost_mesh(cells: &[(IVec3, Shape)]) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for (cell, shape) in cells {
        for polygon in &shape.geometry().polygons {
            let base = positions.len() as u32;
            for v in &polygon.vertices {
                positions.push(((cell.as_vec3() + *v) * CELL).to_array());
                normals.push(polygon.normal.to_array());
            }
            for k in 1..polygon.vertices.len() as u32 - 1 {
                indices.extend([base, base + k, base + k + 1]);
            }
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_slope_is_a_ramp() {
        let cells = compose(BuildShape::Slope, 4, 0);
        // 4 columns of heights 1..4: 0+1+2+3 cubes and 4 slopes per row, 4 rows.
        assert_eq!(cells.len(), (6 + 4) * 4);
        let top = |x: i32| cells.iter().filter(|(p, _)| p.x == x && p.z == 0).map(|(p, _)| p.y).max().unwrap();
        assert_eq!([top(0), top(1), top(2), top(3)], [0, 1, 2, 3]);
        assert!(cells.iter().any(|(p, s)| *p == IVec3::new(3, 3, 0) && *s == Shape::slope(0)));
    }

    #[test]
    fn slope_turns_with_the_player() {
        let towards_minus_x = compose(BuildShape::Slope, 4, 2);
        let top =
            |x: i32| towards_minus_x.iter().filter(|(p, _)| p.x == x && p.z == 0).map(|(p, _)| p.y).max().unwrap();
        assert_eq!([top(0), top(1), top(2), top(3)], [3, 2, 1, 0]);
    }

    #[test]
    fn shapes_have_expected_volume() {
        assert_eq!(compose(BuildShape::Cube, 4, 0).len(), 64);
        assert_eq!(compose(BuildShape::Slab, 4, 0).len(), 32);
        assert_eq!(compose(BuildShape::SlabTop, 2, 0).len(), 4);
        assert_eq!(compose(BuildShape::Stairs, 4, 1).len(), 2 * 2 * 4 + 4 * 2 * 4);
        assert_eq!(compose(BuildShape::Pillar, 4, 0).len(), 16);
        assert_eq!(compose(BuildShape::Slab, 1, 0), vec![(IVec3::ZERO, Shape::SLAB)]);
    }

    #[test]
    fn placed_blocks_sit_on_the_clicked_face() {
        // Clicking the top of cell (5, 3, 6) with a 1 m block: starts right above, snapped sideways.
        assert_eq!(place_origin(IVec3::new(5, 3, 6), IVec3::Y, 4), IVec3::new(4, 4, 4));
        // Clicking the bottom: the block hangs below.
        assert_eq!(place_origin(IVec3::new(5, 3, 6), IVec3::NEG_Y, 4), IVec3::new(4, -1, 4));
        // Negative coordinates snap downwards.
        assert_eq!(place_origin(IVec3::new(-1, 0, 0), IVec3::X, 4), IVec3::new(0, 0, 0));
        assert_eq!(dig_origin(IVec3::new(-1, 5, 7), 4), IVec3::new(-4, 4, 4));
    }
}
