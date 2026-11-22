//! The Shards: islands of violet rock adrift high above the world, over nothing at all.
//! Glowing cores grow deep inside them; a rift on the first island leads back home, and an
//! altar on a far island calls the Warden, the end of the journey.

use bevy::math::{Vec2, Vec3};
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};

use super::blocks::{Block, Material};

/// Height of the islands' tops, in metres.
pub const SHARDS_Y: f32 = 2100.0;
/// The islands lie within this far below and above `SHARDS_Y`.
pub const SHARDS_BELOW: f32 = 40.0;
pub const SHARDS_ABOVE: f32 = 12.0;
/// Where the way back home stands, on an island that is always there.
pub const RIFT_HOME: Vec2 = Vec2::new(0.5, 0.5);
/// Where the altar stands, on its own island.
pub const ALTAR: Vec2 = Vec2::new(0.5, -100.5);
/// The altar's island is flat this far around it.
const ALTAR_YARD: f32 = 7.0;
/// The rift home is a pillar this tall.
const RIFT_HEIGHT: f32 = 1.0;
/// Cores lie at least this deep under an island's top, in rare small veins.
const CORE_DEPTH: f32 = 5.0;
const CORE_THRESHOLD: f64 = 0.72;

pub fn contains(lo: f32, hi: f32) -> bool {
    hi > SHARDS_Y - SHARDS_BELOW && lo < SHARDS_Y + SHARDS_ABOVE
}

pub struct Shards {
    islands: Fbm<Perlin>,
    surface: Perlin,
    cores: Perlin,
}

impl Shards {
    pub fn new(seed: u32) -> Self {
        Self {
            islands: Fbm::new(seed.wrapping_add(20)).set_octaves(3).set_frequency(1.0 / 70.0),
            surface: Perlin::new(seed.wrapping_add(21)),
            cores: Perlin::new(seed.wrapping_add(22)),
        }
    }

    /// The top and bottom of the island at a column, if there is one.
    pub fn island(&self, x: f32, z: f32) -> Option<(f32, f32)> {
        let mut mask = self.islands.get([x as f64, z as f64]) as f32;
        // The first island, around the rift home, and the altar's, with stepping stones between.
        let at = Vec2::new(x, z);
        mask += (1.0 - at.distance(RIFT_HOME) / 18.0).max(0.0);
        mask += (1.0 - at.distance(ALTAR) / 16.0).max(0.0);
        let along = (at - RIFT_HOME).dot(ALTAR - RIFT_HOME) / RIFT_HOME.distance_squared(ALTAR);
        let path = at.distance(RIFT_HOME.lerp(ALTAR, along.clamp(0.0, 1.0)));
        mask += (0.85 * (1.0 - path / 6.0)).max(0.0);
        let strength = ((mask - 0.2) / 0.4).clamp(0.0, 1.0);
        if strength <= 0.0 {
            return None;
        }
        let bumps = self.surface.get([x as f64 / 14.0, z as f64 / 14.0]) as f32 * 2.0;
        let mut top = SHARDS_Y + bumps + strength * 3.0;
        if at.distance(ALTAR) < ALTAR_YARD {
            top = SHARDS_Y + 3.0;
        }
        // Hanging cones: thin at the rim, deep in the middle.
        let bottom = top - 1.0 - strength.powf(0.7) * (SHARDS_BELOW - 8.0);
        Some((top, bottom))
    }

    /// Blocks for an n³ cube of cells starting at `min`, like `Generator::sample`.
    pub fn sample(&self, min: Vec3, cell: f32, n: usize) -> Vec<Block> {
        let mut out = vec![Block::AIR; n * n * n];
        let home = (RIFT_HOME / cell).floor().as_ivec2();
        for z in 0..n {
            for x in 0..n {
                let (cx, cz) = (min.x + (x as f32 + 0.5) * cell, min.z + (z as f32 + 0.5) * cell);
                let Some((top, bottom)) = self.island(cx, cz) else { continue };
                let column = (Vec2::new(cx, cz) / cell).floor().as_ivec2();
                for y in 0..n {
                    let cy = min.y + (y as f32 + 0.5) * cell;
                    let i = x + n * (y + n * z);
                    if column == home && cy > top && cy < top + RIFT_HEIGHT {
                        out[i] = Block::solid(Material::Rift);
                    }
                    if let Some(material) = altar(Vec2::new(cx, cz) - ALTAR, cy - top) {
                        out[i] = Block::solid(material);
                    }
                    if cy <= bottom || cy > top {
                        continue;
                    }
                    let p = [cx as f64 / 3.0, cy as f64 / 3.0, cz as f64 / 3.0];
                    let core = top - cy > CORE_DEPTH && self.cores.get(p) > CORE_THRESHOLD;
                    out[i] = Block::solid(if core { Material::Core } else { Material::Shardstone });
                }
            }
        }
        out
    }
}

/// The altar, relative to its centre on the island's top: a block of altar stone ringed by
/// four pillars with glowing caps.
fn altar(d: Vec2, up: f32) -> Option<Material> {
    if up <= 0.0 || d.length() > ALTAR_YARD {
        return None;
    }
    if d.abs().max_element() < 0.5 && up < 1.0 {
        return Some(Material::Altar);
    }
    let pillar = (d.abs() - Vec2::splat(3.25)).abs().max_element() < 0.25;
    match up {
        _ if !pillar => None,
        u if u < 2.5 => Some(Material::Shardstone),
        u if u < 3.0 => Some(Material::Altar),
        _ => None,
    }
}

/// Where someone arriving through a rift stands: next to the rift home, on its island.
pub fn arrival(shards: &Shards) -> Vec3 {
    let at = RIFT_HOME + Vec2::new(1.5, 0.0);
    let (top, _) = shards.island(at.x, at.y).expect("the first island is always there");
    Vec3::new(at.x, top + 0.3, at.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_island_holds_the_rift() {
        let shards = Shards::new(7);
        let (top, bottom) = shards.island(RIFT_HOME.x, RIFT_HOME.y).expect("an island at home");
        assert!(top - bottom > 10.0);
        let cell = super::super::CELL;
        let min = Vec3::new(0.0, (top / cell).floor() * cell - 2.0, 0.0);
        let blocks = shards.sample(min, cell, 16);
        assert!(blocks.iter().any(|b| b.material == Material::Rift));
    }

    #[test]
    fn islands_lead_from_the_rift_to_the_altar() {
        for seed in [7, 42, 1337, 2024] {
            let shards = Shards::new(seed);
            let open = (0..=100)
                .filter(|i| {
                    let p = RIFT_HOME.lerp(ALTAR, *i as f32 / 100.0);
                    shards.island(p.x, p.y).is_none()
                })
                .count();
            assert!(open <= 3, "seed {seed}: {open} m of void on the way");
        }
    }

    #[test]
    fn the_altar_stands_on_its_island() {
        let shards = Shards::new(7);
        let (top, _) = shards.island(ALTAR.x, ALTAR.y).expect("an island under the altar");
        let cell = super::super::CELL;
        let min = Vec3::new(ALTAR.x - 2.0, top - 1.0, ALTAR.y - 2.0);
        let blocks = shards.sample(min, cell, 16);
        assert!(blocks.iter().any(|b| b.material == Material::Altar));
    }
}
