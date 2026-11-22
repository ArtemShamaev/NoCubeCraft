//! Ruins scattered over the world: broken watchtowers and desert pyramids, each hiding a cache.

use std::f32::consts::TAU;

use bevy::math::{IVec2, Vec2, Vec3};

use super::{
    blocks::{Block, Material},
    generator::{Biome, Generator, hash2, value_noise},
};

/// At most one structure per square this wide.
const REGION: f32 = 160.0;
const CHANCE: f32 = 0.55;
/// Furthest any structure reaches from its centre, horizontally.
const REACH: f32 = 10.0;
const TOWER_RADIUS: f32 = 3.0;
const TOWER_WALL: f32 = 0.75;
const PYRAMID_HALF: f32 = 9.0;
/// Foundations go this deep, so a structure never floats over a dip.
const FOUNDATION: f32 = 3.0;
/// Cleared space above a structure's floor.
const HEADROOM: f32 = 10.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Tower,
    Pyramid,
}

#[derive(Clone, Copy, Debug)]
struct Structure {
    kind: Kind,
    center: Vec2,
    /// Top of the floor.
    floor: f32,
    seed: u32,
}

impl Generator {
    /// The structure in a region, if it has one and the ground there suits it.
    fn structure_at(&self, region: IVec2) -> Option<Structure> {
        let h = hash2(region + IVec2::new(31_337, -9_001));
        if (h % 1000) as f32 / 1000.0 > CHANCE {
            return None;
        }
        let jitter = |bits: u32| REACH + (bits % 1000) as f32 / 1000.0 * (REGION - 2.0 * REACH);
        let center = region.as_vec2() * REGION + Vec2::new(jitter(h >> 10), jitter(h >> 20));
        let ground = self.height(center.x, center.y);
        let kind = match self.biome(center.x, center.y, ground) {
            Biome::Desert => Kind::Pyramid,
            Biome::Swamp => return None,
            _ => Kind::Tower,
        };
        let r = if kind == Kind::Tower { TOWER_RADIUS } else { PYRAMID_HALF };
        let corners = [(-r, -r), (r, -r), (-r, r), (r, r)].map(|(x, z)| self.height(center.x + x, center.y + z));
        let (lo, hi) = corners.iter().fold((ground, ground), |(lo, hi), h| (lo.min(*h), hi.max(*h)));
        if hi - lo > FOUNDATION || lo < super::SEA_LEVEL + 1.0 {
            return None;
        }
        Some(Structure { kind, center, floor: (ground * 4.0).round() / 4.0, seed: h })
    }

    /// The nearest structure within a few regions of `p`, by name and floor centre.
    pub fn nearest_structure(&self, p: Vec3) -> Option<(&'static str, Vec3)> {
        let home = (Vec2::new(p.x, p.z) / REGION).floor().as_ivec2();
        (-4..=4)
            .flat_map(|z| (-4..=4).map(move |x| home + IVec2::new(x, z)))
            .filter_map(|r| self.structure_at(r))
            .map(|s| {
                let name = if s.kind == Kind::Tower { "Руины башни" } else { "Пирамида" };
                (name, Vec3::new(s.center.x, s.floor, s.center.y))
            })
            .min_by(|a, b| a.1.distance(p).total_cmp(&b.1.distance(p)))
    }

    /// Builds whatever structures reach into an n³ region of cells, like `plant_trees`.
    pub(super) fn build_structures(&self, out: &mut [Block], min: Vec3, cell: f32, n: usize) {
        let max = min + Vec3::splat(cell * n as f32);
        let lo = ((Vec2::new(min.x, min.z) - REACH) / REGION).floor().as_ivec2();
        let hi = ((Vec2::new(max.x, max.z) + REACH) / REGION).floor().as_ivec2();
        for rz in lo.y..=hi.y {
            for rx in lo.x..=hi.x {
                let Some(s) = self.structure_at(IVec2::new(rx, rz)) else { continue };
                if s.floor - FOUNDATION > max.y || s.floor + HEADROOM < min.y {
                    continue;
                }
                let first = ((Vec3::new(s.center.x - REACH, s.floor - FOUNDATION, s.center.y - REACH) - min) / cell)
                    .floor()
                    .as_ivec3()
                    .max(bevy::math::IVec3::ZERO);
                let last = ((Vec3::new(s.center.x + REACH, s.floor + HEADROOM, s.center.y + REACH) - min) / cell)
                    .ceil()
                    .as_ivec3()
                    .min(bevy::math::IVec3::splat(n as i32 - 1));
                for z in first.z..=last.z {
                    for y in first.y..=last.y {
                        for x in first.x..=last.x {
                            let p = min + (Vec3::new(x as f32, y as f32, z as f32) + 0.5) * cell;
                            if let Some(block) = s.block(p) {
                                out[x as usize + n * (y as usize + n * z as usize)] = block;
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Structure {
    /// What the structure puts at a point: `None` leaves the world as it is.
    fn block(&self, p: Vec3) -> Option<Block> {
        let d = Vec2::new(p.x, p.z) - self.center;
        let up = p.y - self.floor;
        if up < -FOUNDATION || up >= HEADROOM {
            return None;
        }
        // The cache: half a metre of loot in the middle of the floor.
        if d.abs().max_element() < 0.25 && (0.0..0.5).contains(&up) {
            return Some(Block::solid(Material::Cache));
        }
        match self.kind {
            Kind::Tower => self.tower(d, up),
            Kind::Pyramid => self.pyramid(d, up),
        }
    }

    fn tower(&self, d: Vec2, up: f32) -> Option<Block> {
        let r = d.length();
        if r >= TOWER_RADIUS {
            return None;
        }
        if up < 0.0 {
            return Some(Block::solid(Material::Bricks));
        }
        if r < TOWER_RADIUS - TOWER_WALL {
            return Some(Block::AIR);
        }
        // The wall crumbles to a ragged top, and a doorway faces a random way.
        let angle = d.y.atan2(d.x);
        let ring = Vec3::new(angle.cos() * 1.5, angle.sin() * 1.5, 0.0);
        let height = 4.0 + 5.0 * value_noise(ring, self.seed);
        let door = (self.seed % 628) as f32 / 100.0;
        let off = (angle - door).rem_euclid(TAU);
        if up >= height || (up < 2.25 && off.min(TAU - off) < 0.3) {
            return Some(Block::AIR);
        }
        let mossy = value_noise(Vec3::new(d.x, up, d.y) * 2.0, self.seed ^ 0x55) > 0.62;
        Some(Block::solid(if mossy { Material::Stone } else { Material::Bricks }))
    }

    fn pyramid(&self, d: Vec2, up: f32) -> Option<Block> {
        let a = d.abs().max_element();
        if a >= PYRAMID_HALF {
            return None;
        }
        if up < 0.0 {
            return Some(Block::solid(Material::Sandstone));
        }
        // Half-metre steps up to a point.
        let step = (up / 0.5).floor() * 0.5;
        if a >= PYRAMID_HALF - step {
            return Some(Block::AIR);
        }
        let chamber = d.abs().max_element() < 2.5 && up < 3.0;
        let corridor = d.y.abs() < 0.75 && d.x > 0.0 && up < 2.25;
        Some(if chamber || corridor { Block::AIR } else { Block::solid(Material::Sandstone) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn some_structures_stand_somewhere() {
        let g = Generator::new(1337);
        let found: Vec<Kind> = (-6..6)
            .flat_map(|x| (-6..6).map(move |z| IVec2::new(x, z)))
            .filter_map(|r| g.structure_at(r))
            .map(|s| s.kind)
            .collect();
        assert!(found.contains(&Kind::Tower), "{found:?}");
        let s = (-30..30)
            .flat_map(|x| (-30..30).map(move |z| IVec2::new(x, z)))
            .find_map(|r| g.structure_at(r))
            .expect("a structure");
        let min = Vec3::new(s.center.x - 4.0, s.floor - 1.0, s.center.y - 4.0);
        let blocks = g.sample(min, 0.25, 32, true);
        assert!(blocks.iter().any(|b| b.material == Material::Cache));
    }
}
