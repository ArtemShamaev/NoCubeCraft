//! Deterministic world: height map, shaped surface cells, caves, ores, trees and ruins.

use bevy::math::{IVec2, IVec3, Vec3};
use noise::{Fbm, MultiFractal, NoiseFn, Perlin, RidgedMulti};

use super::{
    blocks::{Block, Material, Shape},
    shards::{self, Shards},
    water::SEA_LEVEL,
};

const BASE_HEIGHT: f64 = 24.0;
/// Columns whose surface is below this get sand instead of grass.
const BEACH_HEIGHT: f32 = SEA_LEVEL + 2.5;
/// Peaks above this are snowy.
const SNOW_HEIGHT: f32 = 82.0;
/// Pines replace broadleaf trees above this.
const PINE_HEIGHT: f32 = 50.0;
/// Slopes steeper than this (rise per metre) are bare rock.
const CLIFF_SLOPE: f32 = 1.3;
const TOPSOIL_DEPTH: f32 = 3.0;
/// Caves fade out towards the surface so the ground isn't riddled with holes,
/// except at entrances, where tunnels break through to daylight.
const CAVE_MIN_DEPTH: f32 = 4.0;
const CAVE_FULL_DEPTH: f32 = 16.0;
const CAVE_RADIUS: f32 = 0.085;
/// Big caverns ("cheese caves") open up this deep.
const CAVERN_MIN_DEPTH: f32 = 10.0;
const CAVERN_THRESHOLD: f32 = 0.56;
/// Iron only appears below this height, diamonds far deeper.
const IRON_MAX_Y: f32 = 10.0;
const DIAMOND_MAX_Y: f32 = -8.0;
/// Trees stand on a jittered grid with cells this wide; at most one per cell.
const TREE_CELL: i32 = 7;
/// Furthest a tree reaches from its trunk, horizontally and upwards.
pub const TREE_REACH: f32 = 3.5;
pub const TREE_HEIGHT: f32 = 12.0;

pub struct Generator {
    continents: Fbm<Perlin>,
    hills: Fbm<Perlin>,
    mountains: RidgedMulti<Perlin>,
    cave_a: Perlin,
    cave_b: Perlin,
    cavern: Perlin,
    entrances: Perlin,
    coal: Perlin,
    iron: Perlin,
    forest: Fbm<Perlin>,
    snow: Perlin,
    temperature: Fbm<Perlin>,
    humidity: Fbm<Perlin>,
    pub shards: Shards,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Biome {
    Plains,
    Forest,
    Taiga,
    Snowy,
    Desert,
    Savanna,
    Swamp,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TreeKind {
    Broadleaf,
    Pine,
    Acacia,
    Cactus,
}

impl Generator {
    pub fn new(seed: u32) -> Self {
        Self {
            continents: Fbm::new(seed).set_octaves(4).set_frequency(1.0 / 700.0),
            hills: Fbm::new(seed.wrapping_add(1)).set_octaves(5).set_frequency(1.0 / 140.0),
            mountains: RidgedMulti::new(seed.wrapping_add(2)).set_octaves(5).set_frequency(1.0 / 400.0),
            cave_a: Perlin::new(seed.wrapping_add(3)),
            cave_b: Perlin::new(seed.wrapping_add(4)),
            cavern: Perlin::new(seed.wrapping_add(9)),
            entrances: Perlin::new(seed.wrapping_add(10)),
            coal: Perlin::new(seed.wrapping_add(5)),
            iron: Perlin::new(seed.wrapping_add(6)),
            forest: Fbm::new(seed.wrapping_add(7)).set_octaves(3).set_frequency(1.0 / 220.0),
            snow: Perlin::new(seed.wrapping_add(8)),
            temperature: Fbm::new(seed.wrapping_add(11)).set_octaves(3).set_frequency(1.0 / 900.0),
            humidity: Fbm::new(seed.wrapping_add(12)).set_octaves(3).set_frequency(1.0 / 700.0),
            shards: Shards::new(seed),
        }
    }

    pub fn height(&self, x: f32, z: f32) -> f32 {
        let p = [x as f64, z as f64];
        let continent = self.continents.get(p);
        let hills = self.hills.get(p) * 12.0;
        // Mountains rise only where the continental noise is high.
        let mountain_mask = ((continent - 0.1) / 0.4).clamp(0.0, 1.0);
        let mountains = (self.mountains.get(p) * 0.5 + 0.5).powi(2) * 90.0 * mountain_mask;
        (BASE_HEIGHT + continent * 28.0 + hills + mountains) as f32
    }

    /// Lowest and highest ground in a square area, with some slack for peaks between samples.
    pub fn height_bounds(&self, x0: f32, z0: f32, size: f32) -> (f32, f32) {
        const N: usize = 9;
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for i in 0..N * N {
            let x = x0 + (i % N) as f32 / (N - 1) as f32 * size;
            let z = z0 + (i / N) as f32 / (N - 1) as f32 * size;
            let h = self.height(x, z);
            lo = lo.min(h);
            hi = hi.max(h);
        }
        let slack = 2.0 + size / 8.0;
        (lo - slack, hi + slack)
    }

    /// Temperature and humidity, about -1..1; high ground is colder.
    fn climate(&self, x: f32, z: f32, height: f32) -> (f32, f32) {
        let p = [x as f64, z as f64];
        let t = self.temperature.get(p) as f32 * 1.6 - ((height - 40.0) / 90.0).max(0.0);
        (t, self.humidity.get(p) as f32 * 1.6)
    }

    pub fn biome(&self, x: f32, z: f32, height: f32) -> Biome {
        let (t, h) = self.climate(x, z, height);
        if t < -0.4 {
            Biome::Snowy
        } else if t < -0.12 {
            Biome::Taiga
        } else if t > 0.35 {
            if h < -0.05 { Biome::Desert } else { Biome::Savanna }
        } else if h > 0.3 && height < 26.0 {
            Biome::Swamp
        } else if h > -0.05 {
            Biome::Forest
        } else {
            Biome::Plains
        }
    }

    /// Colour for grass and leaves: lush, dry, cold or swampy, blended smoothly.
    pub fn foliage_tint(&self, x: f32, z: f32) -> [f32; 3] {
        let (t, h) = self.climate(x, z, self.height(x, z));
        let ramp = |a: f32, b: f32, v: f32| {
            let k = ((v - a) / (b - a)).clamp(0.0, 1.0);
            k * k * (3.0 - 2.0 * k)
        };
        let lush = Vec3::new(1.0, 1.0, 1.0);
        let dry = Vec3::new(1.45, 1.1, 0.55);
        let cold = Vec3::new(0.75, 0.92, 0.9);
        let swamp = Vec3::new(0.55, 0.62, 0.32);
        let mut c = lush.lerp(dry, ramp(0.15, 0.45, t));
        c = c.lerp(cold, ramp(-0.05, -0.4, t));
        c = c.lerp(swamp, ramp(0.2, 0.35, h) * ramp(0.35, 0.15, t.abs()));
        c.to_array()
    }

    /// Chance of a tree in each tree cell: dense woods with meadows in between.
    fn forest(&self, x: f32, z: f32) -> f32 {
        let n = self.forest.get([x as f64, z as f64]) as f32;
        ((n + 0.1) * 2.5).clamp(0.0, 0.8)
    }

    /// `n`³ cells of edge `cell`, the first one's low corner at `min`; x fastest, then y, then z.
    /// Caves and ores only appear when `detail` is set; far away nobody sees them.
    pub fn sample(&self, min: Vec3, cell: f32, n: usize, detail: bool) -> Vec<Block> {
        if shards::contains(min.y, min.y + cell * n as f32) {
            return self.shards.sample(min, cell, n);
        }
        let columns = Columns::new(self, min, cell, n);
        let mut lattice = Lattice::new(min.floor().as_ivec3(), cell * n as f32);
        let mut out = vec![Block::AIR; n * n * n];

        for z in 0..n {
            for x in 0..n {
                let column = columns.at(x as i32, z as i32);
                let top = columns.top(x as i32, z as i32);
                for y in 0..n {
                    let y0 = min.y + y as f32 * cell;
                    if y0 >= top {
                        break;
                    }
                    let mut block = match columns.surface_shape(x as i32, z as i32, y0, cell) {
                        Some(shape) => Block::new(column.surface, shape),
                        None => continue,
                    };
                    let center =
                        Vec3::new(min.x + (x as f32 + 0.5) * cell, y0 + cell * 0.5, min.z + (z as f32 + 0.5) * cell);
                    let depth = column.height - center.y;
                    if y0 + cell < column.height - 0.3 {
                        block.material = if depth < TOPSOIL_DEPTH { column.subsoil } else { Material::Stone };
                    }
                    if detail && depth > column.cave_top && lattice.cave(self, center, depth, column.cave_top) {
                        continue;
                    }
                    if detail && block.material == Material::Stone {
                        block.material = lattice.ore(self, center);
                    }
                    out[x + n * (y + n * z)] = block;
                }
            }
        }

        self.plant_trees(&mut out, min, cell, n);
        self.build_structures(&mut out, min, cell, n);
        out
    }

    fn tree_at(&self, id: IVec2) -> Option<Tree> {
        let h = hash2(id);
        let x = (id.x * TREE_CELL) as f32 + (h % 1000) as f32 / 1000.0 * TREE_CELL as f32;
        let z = (id.y * TREE_CELL) as f32 + ((h >> 10) % 1000) as f32 / 1000.0 * TREE_CELL as f32;
        let ground = self.height(x, z);
        let (density, kind) = match self.biome(x, z, ground) {
            Biome::Forest => (1.0, TreeKind::Broadleaf),
            Biome::Plains => (0.1, TreeKind::Broadleaf),
            Biome::Swamp => (0.55, TreeKind::Broadleaf),
            Biome::Taiga => (0.9, TreeKind::Pine),
            Biome::Snowy => (0.15, TreeKind::Pine),
            Biome::Savanna => (0.12, TreeKind::Acacia),
            Biome::Desert => (0.06, TreeKind::Cactus),
        };
        let chance = if kind == TreeKind::Cactus { density } else { self.forest(x, z) * density };
        if (h >> 20) as f32 / 4096.0 > chance {
            return None;
        }
        if !(BEACH_HEIGHT + 0.5..SNOW_HEIGHT - 8.0).contains(&ground) {
            return None;
        }
        let slope = (self.height(x + 1.0, z) - self.height(x - 1.0, z))
            .abs()
            .max((self.height(x, z + 1.0) - self.height(x, z - 1.0)).abs());
        if slope > 1.0 {
            return None;
        }
        let h2 = hash2(id + IVec2::new(7919, 104_729));
        let scale = 0.8 + (h2 % 1000) as f32 / 1000.0 * 0.5;
        let kind = if kind == TreeKind::Broadleaf && ground > PINE_HEIGHT { TreeKind::Pine } else { kind };
        Some(Tree { base: Vec3::new(x, ground, z), scale, kind, seed: h2 })
    }

    fn plant_trees(&self, out: &mut [Block], min: Vec3, cell: f32, n: usize) {
        let max = min + Vec3::splat(cell * n as f32);
        let lo = ((min.x - TREE_REACH) / TREE_CELL as f32).floor() as i32;
        let hi = ((max.x + TREE_REACH) / TREE_CELL as f32).floor() as i32;
        let lo_z = ((min.z - TREE_REACH) / TREE_CELL as f32).floor() as i32;
        let hi_z = ((max.z + TREE_REACH) / TREE_CELL as f32).floor() as i32;
        for cz in lo_z..=hi_z {
            for cx in lo..=hi {
                let Some(tree) = self.tree_at(IVec2::new(cx, cz)) else { continue };
                let bottom = tree.base.y - 1.0;
                if bottom > max.y || tree.base.y + TREE_HEIGHT < min.y {
                    continue;
                }
                tree.rasterize(out, min, cell, n);
            }
        }
    }
}

/// Height map over a region plus a one-column border, with per-column surface materials.
struct Columns {
    n: i32,
    cells: Vec<Column>,
    /// Ground height at cell corners: (n + 1)² for the region, corner (0, 0) at its low corner.
    corners: Vec<f32>,
}

#[derive(Clone, Copy)]
struct Column {
    height: f32,
    surface: Material,
    subsoil: Material,
    /// Depth below which tunnels may run: shallow at cave entrances.
    cave_top: f32,
}

impl Columns {
    fn new(generator: &Generator, min: Vec3, cell: f32, n: usize) -> Self {
        let n = n as i32 + 2;
        let heights: Vec<f32> = (0..n * n)
            .map(|i| {
                let (x, z) = (i % n - 1, i / n - 1);
                generator.height(min.x + (x as f32 + 0.5) * cell, min.z + (z as f32 + 0.5) * cell)
            })
            .collect();
        let at = |x: i32, z: i32| heights[(x.clamp(0, n - 1) + z.clamp(0, n - 1) * n) as usize];
        let cells = (0..n * n)
            .map(|i| {
                let (x, z) = (i % n, i / n);
                let height = at(x, z);
                let slope =
                    ((at(x + 1, z) - at(x - 1, z)).abs().max((at(x, z + 1) - at(x, z - 1)).abs())) / (2.0 * cell);
                let wx = min.x + (x as f32 - 0.5) * cell;
                let wz = min.z + (z as f32 - 0.5) * cell;
                let snow_line = SNOW_HEIGHT + generator.snow.get([wx as f64 / 30.0, wz as f64 / 30.0]) as f32 * 8.0;
                let biome = generator.biome(wx, wz, height);
                let snow_line = match biome {
                    Biome::Snowy => f32::MIN,
                    Biome::Taiga => snow_line - 30.0,
                    _ => snow_line,
                };
                let bed = generator.snow.get([wx as f64 / 9.0, wz as f64 / 9.0]) as f32;
                let (surface, subsoil) = if height < SEA_LEVEL - 1.0 {
                    // Lake and sea floors: sand, with patches of clay and gravel.
                    let floor = if bed > 0.35 {
                        Material::Clay
                    } else if bed < -0.4 {
                        Material::Gravel
                    } else {
                        Material::Sand
                    };
                    (floor, floor)
                } else if biome == Biome::Desert {
                    (Material::Sand, Material::Sandstone)
                } else if height < BEACH_HEIGHT {
                    (Material::Sand, Material::Sand)
                } else if slope > CLIFF_SLOPE {
                    (Material::Stone, Material::Stone)
                } else if height > snow_line {
                    (Material::Snow, if height > SNOW_HEIGHT + 10.0 { Material::Ice } else { Material::Stone })
                } else if biome == Biome::Swamp {
                    (Material::Grass, Material::Clay)
                } else {
                    (Material::Grass, Material::Dirt)
                };
                let entrance = generator.entrances.get([wx as f64 / 70.0, wz as f64 / 70.0]) as f32;
                let open = ((entrance - 0.38) / 0.12).clamp(0.0, 1.0);
                let cave_top = CAVE_MIN_DEPTH + (0.2 - CAVE_MIN_DEPTH) * open;
                Column { height, surface, subsoil, cave_top }
            })
            .collect();
        // Each corner is shared by four columns; averaging them keeps neighbours in agreement.
        let corners = (0..(n - 1) * (n - 1))
            .map(|i| {
                let (x, z) = (i % (n - 1), i / (n - 1));
                (at(x, z) + at(x + 1, z) + at(x, z + 1) + at(x + 1, z + 1)) / 4.0
            })
            .collect();
        Self { n, cells, corners }
    }

    /// Corner heights of region cell (x, z): +X+Z, -X+Z, -X-Z, +X-Z (the corner direction order).
    fn corner_heights(&self, x: i32, z: i32) -> [f32; 4] {
        let w = self.n - 1;
        let at = |x: i32, z: i32| self.corners[(x + z * w) as usize];
        [at(x + 1, z + 1), at(x, z + 1), at(x, z), at(x + 1, z)]
    }

    /// Highest point of a cell column's surface.
    fn top(&self, x: i32, z: i32) -> f32 {
        self.corner_heights(x, z).into_iter().fold(f32::MIN, f32::max)
    }

    /// `x`, `z` are region cell coordinates; the border is at -1 and n.
    fn at(&self, x: i32, z: i32) -> Column {
        self.cells[((x + 1) + (z + 1) * self.n) as usize]
    }

    /// The shape of a cell whose bottom is at `y0`, from how the ground crosses its four
    /// corners; `None` leaves it empty. Neighbouring cells share corners, so the shapes join
    /// into a continuous surface: ramps and corners instead of little walls.
    fn surface_shape(&self, x: i32, z: i32, y0: f32, cell: f32) -> Option<Shape> {
        let fill = self.corner_heights(x, z).map(|h| (h - y0) / cell);
        if fill.iter().all(|f| *f >= 1.0) {
            return Some(Shape::CUBE);
        }
        if fill.iter().all(|f| *f <= 0.0) {
            return None;
        }
        // Each corner rounds to the cell's top or bottom; neighbours round shared corners the
        // same way, so their faces always meet.
        let high = fill.map(|f| f >= 0.5);
        match high.iter().filter(|h| **h).count() {
            0 => None,
            1 => Some(Shape::outer_corner(high.iter().position(|h| *h).unwrap() as u8)),
            3 => {
                let low = high.iter().position(|h| !*h).unwrap();
                Some(Shape::inner_corner(((low + 2) % 4) as u8))
            }
            4 => Some(Shape::CUBE),
            _ => match (0..4).find(|&i| high[i] && high[(i + 1) % 4]) {
                // Two neighbouring corners up: a ramp rising towards that side.
                Some(i) => Some(Shape::slope(((i + 1) % 4) as u8)),
                // Opposite corners up: a saddle, closest to a full cell.
                None => Some(Shape::CUBE),
            },
        }
    }
}

/// Noise values on a 1 m lattice, computed on demand and interpolated, so caves and ores cost
/// one noise evaluation per cubic metre instead of one per cell.
struct Lattice {
    origin: IVec3,
    size: i32,
    values: Vec<[f32; 5]>,
}

const UNSET: f32 = f32::NAN;

impl Lattice {
    fn new(origin: IVec3, extent: f32) -> Self {
        let size = extent.ceil() as i32 + 3;
        Self { origin, size, values: vec![[UNSET; 5]; (size * size * size) as usize] }
    }

    /// Cave A, cave B, coal, iron and cavern noise at a lattice point.
    fn get(&mut self, generator: &Generator, p: IVec3) -> [f32; 5] {
        let l = p - self.origin;
        let inside = l.cmpge(IVec3::ZERO).all() && l.cmplt(IVec3::splat(self.size)).all();
        let compute = || {
            let q = [p.x as f64 / 48.0, p.y as f64 / 32.0, p.z as f64 / 48.0];
            let o = [p.x as f64 / 5.0, p.y as f64 / 5.0, p.z as f64 / 5.0];
            [
                generator.cave_a.get(q) as f32,
                generator.cave_b.get(q) as f32,
                generator.coal.get(o) as f32,
                generator.iron.get(o) as f32,
                generator.cavern.get([p.x as f64 / 34.0, p.y as f64 / 18.0, p.z as f64 / 34.0]) as f32,
            ]
        };
        if !inside {
            return compute();
        }
        let i = (l.x + self.size * (l.y + self.size * l.z)) as usize;
        if self.values[i][0].is_nan() {
            self.values[i] = compute();
        }
        self.values[i]
    }

    /// Tunnels follow the intersection of two noise iso-surfaces ("spaghetti caves"); deeper
    /// down, a third noise hollows out big caverns. `top` is how deep tunnels start here.
    fn cave(&mut self, generator: &Generator, p: Vec3, depth: f32, top: f32) -> bool {
        let base = p.floor().as_ivec3();
        let t = p - base.as_vec3();
        let (mut a, mut b, mut c) = (0.0, 0.0, 0.0);
        for corner in 0..8 {
            let o = IVec3::new(corner & 1, (corner >> 1) & 1, corner >> 2);
            let w = (if o.x == 1 { t.x } else { 1.0 - t.x })
                * (if o.y == 1 { t.y } else { 1.0 - t.y })
                * (if o.z == 1 { t.z } else { 1.0 - t.z });
            let v = self.get(generator, base + o);
            a += v[0] * w;
            b += v[1] * w;
            c += v[4] * w;
        }
        let deep = ((depth - CAVERN_MIN_DEPTH) / 8.0).clamp(0.0, 1.0);
        if deep > 0.0 && c > CAVERN_THRESHOLD + 0.2 * (1.0 - deep) {
            return true;
        }
        let fade = ((depth - top) / (CAVE_FULL_DEPTH - top)).clamp(0.0, 1.0);
        a.abs().max(b.abs()) < CAVE_RADIUS * fade.sqrt()
    }

    /// Ore comes in whole cubic metres.
    fn ore(&mut self, generator: &Generator, p: Vec3) -> Material {
        let v = self.get(generator, p.floor().as_ivec3());
        if p.y < DIAMOND_MAX_Y && v[3] < -0.62 {
            Material::DiamondOre
        } else if p.y < IRON_MAX_Y && v[3] > 0.55 {
            Material::IronOre
        } else if v[2] > 0.5 {
            Material::CoalOre
        } else if v[2] < -0.62 {
            Material::Gravel
        } else {
            Material::Stone
        }
    }
}

struct Tree {
    base: Vec3,
    scale: f32,
    kind: TreeKind,
    seed: u32,
}

impl Tree {
    fn rasterize(&self, out: &mut [Block], min: Vec3, cell: f32, n: usize) {
        let s = self.scale;
        let (trunk_height, trunk_radius) = match self.kind {
            TreeKind::Pine => (7.0 * s, 0.26 * s),
            TreeKind::Broadleaf => (4.2 * s, 0.32 * s),
            TreeKind::Acacia => (3.6 * s, 0.24 * s),
            TreeKind::Cactus => (1.4 + s * 1.2, 0.26),
        };
        let trunk = if self.kind == TreeKind::Cactus { Material::Cactus } else { Material::Wood };
        let reach = Vec3::new(TREE_REACH, TREE_HEIGHT, TREE_REACH);
        let lo = ((self.base - Vec3::new(reach.x, 1.0, reach.z) - min) / cell).floor().as_ivec3().max(IVec3::ZERO);
        let hi = ((self.base + reach - min) / cell).ceil().as_ivec3().min(IVec3::splat(n as i32 - 1));
        let crown = self.base + Vec3::Y * (trunk_height + 0.4 * s);
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let p = min + (IVec3::new(x, y, z).as_vec3() + 0.5) * cell;
                    let i = x as usize + n * (y as usize + n * z as usize);
                    let up = p.y - self.base.y;
                    let radial = Vec3::new(p.x - self.base.x, 0.0, p.z - self.base.z).length();
                    if radial < trunk_radius.max(cell * 0.5) && (-0.8..trunk_height).contains(&up) {
                        out[i] = Block::solid(trunk);
                    } else if out[i].is_air() && self.leaf(p, crown) {
                        out[i] = Block::solid(Material::Leaves);
                    }
                }
            }
        }
    }

    fn leaf(&self, p: Vec3, crown: Vec3) -> bool {
        let s = self.scale;
        let lump = value_noise(p * 1.3, self.seed) - 0.5;
        match self.kind {
            TreeKind::Cactus => return false,
            // A wide flat canopy.
            TreeKind::Acacia => {
                let d = (p - crown) / (Vec3::new(3.0, 0.75, 3.0) * s);
                return d.length_squared() < 1.0 + lump * 0.7;
            }
            _ => {}
        }
        if self.kind == TreeKind::Pine {
            // A cone of drooping tiers.
            let t = (p.y - (self.base.y + 1.6 * s)) / (7.6 * s);
            if !(0.0..1.0).contains(&t) {
                return false;
            }
            let tier = 0.8 + 0.2 * ((t * 22.0).sin());
            let radius = 2.3 * s * (1.0 - t) * tier;
            Vec3::new(p.x - self.base.x, 0.0, p.z - self.base.z).length() < radius + lump * 0.6
        } else {
            let d = (p - crown) / (Vec3::new(2.4, 1.9, 2.4) * s);
            d.length_squared() < 1.0 + lump * 0.9
        }
    }
}

pub(super) fn hash2(p: IVec2) -> u32 {
    let mut h = (p.x as u32).wrapping_mul(0x8da6_b343) ^ (p.y as u32).wrapping_mul(0xd816_3841);
    h = h.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    h = ((h >> ((h >> 28) + 4)) ^ h).wrapping_mul(277_803_737);
    (h >> 22) ^ h
}

fn hash3(p: IVec3, seed: u32) -> f32 {
    let h = hash2(IVec2::new(p.x ^ seed as i32, p.y.wrapping_mul(31) ^ p.z.wrapping_mul(977)));
    h as f32 / u32::MAX as f32
}

/// Smooth value noise in [0, 1].
pub(super) fn value_noise(p: Vec3, seed: u32) -> f32 {
    let base = p.floor().as_ivec3();
    let t = p - base.as_vec3();
    let t = t * t * (Vec3::splat(3.0) - 2.0 * t);
    let mut sum = 0.0;
    for corner in 0..8 {
        let o = IVec3::new(corner & 1, (corner >> 1) & 1, corner >> 2);
        let w = (if o.x == 1 { t.x } else { 1.0 - t.x })
            * (if o.y == 1 { t.y } else { 1.0 - t.y })
            * (if o.z == 1 { t.z } else { 1.0 - t.z });
        sum += hash3(base + o, seed) * w;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_world() {
        let min = Vec3::new(-40.0, 10.0, 300.0);
        let a = Generator::new(7).sample(min, 0.25, 24, true);
        let b = Generator::new(7).sample(min, 0.25, 24, true);
        assert_eq!(a, b);
        let c = Generator::new(8).sample(min, 0.25, 24, true);
        assert_ne!(a, c);
    }

    #[test]
    fn single_cell_matches_region() {
        let g = Generator::new(3);
        let min = Vec3::new(12.0, 18.0, -7.0);
        let region = g.sample(min, 0.25, 16, true);
        for (i, expected) in region.iter().enumerate().step_by(37) {
            let cell = IVec3::new((i % 16) as i32, (i / 16 % 16) as i32, (i / 256) as i32);
            let one = g.sample(min + cell.as_vec3() * 0.25, 0.25, 1, true);
            assert_eq!(one[0], *expected, "cell {cell}");
        }
    }

    #[test]
    fn surface_is_shaped() {
        // Somewhere on a hillside there must be slopes or corners, not only cubes.
        let g = Generator::new(1337);
        let h = g.height(40.0, 40.0);
        let blocks = g.sample(Vec3::new(32.0, h - 8.0, 32.0), 0.25, 64, false);
        let shaped = blocks.iter().filter(|b| !b.is_air() && b.shape != Shape::CUBE).count();
        assert!(shaped > 100, "only {shaped} shaped cells");
    }
}
