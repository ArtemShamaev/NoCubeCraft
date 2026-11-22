//! Villages: where they stand, what they keep and build, and what things cost there.
//!
//! A village owns a store of goods and a treasury. Its people are paid from the treasury for
//! what they bring in and pay into it for their food, so coins go round; prices rise for what
//! is scarce and fall for what piles up, and a player can buy the surplus or sell what's short.

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    items::Item,
    mobs::rand01,
    net::AvatarOf,
    player::Player,
    terrain::{Biome, Block, CELL, Material, SEA_LEVEL, Terrain},
    villagers::{self, Job},
};

/// At most one village per square this wide.
const REGION: f32 = 256.0;
const CHANCE: f32 = 0.75;
/// Players this close bring a village to life.
const FOUND_RADIUS: f32 = 160.0;
/// Seconds between the store's own work: sawing, baking, smelting, and newcomers.
const TICK_SECS: f32 = 3.0;
/// Each house sleeps this many.
const BEDS_PER_HOUSE: usize = 2;
/// The most a single wage takes from the treasury.
const TREASURY_SHARE: f32 = 0.02;
/// Bread eaten to welcome a newcomer.
const WELCOME_BREAD: u32 = 4;

pub struct VillagesPlugin;

impl Plugin for VillagesPlugin {
    fn build(&self, app: &mut App) {
        app.replicate::<Village>()
            .init_resource::<Founded>()
            .add_systems(Update, (found, work_store).run_if(in_state(AppState::Game)).run_if(crate::net::authority))
            .add_systems(OnExit(AppState::Game), |mut founded: ResMut<Founded>| founded.0.clear());
    }
}

/// What a village trades in: how many go together, a fair price for that many, and how many
/// it likes to keep.
pub struct Good {
    pub item: Item,
    pub bundle: u32,
    price: f32,
    target: u32,
}

pub const GOODS: [Good; 10] = [
    Good { item: Item::Wood, bundle: 8, price: 2.0, target: 40 },
    Good { item: Item::Planks, bundle: 16, price: 2.0, target: 120 },
    Good { item: Item::Stone, bundle: 16, price: 2.0, target: 80 },
    Good { item: Item::Bricks, bundle: 16, price: 3.0, target: 60 },
    Good { item: Item::Wheat, bundle: 8, price: 2.0, target: 30 },
    Good { item: Item::Bread, bundle: 4, price: 3.0, target: 24 },
    Good { item: Item::Seeds, bundle: 8, price: 1.0, target: 12 },
    Good { item: Item::Coal, bundle: 8, price: 3.0, target: 16 },
    Good { item: Item::IronOre, bundle: 4, price: 4.0, target: 8 },
    Good { item: Item::IronIngot, bundle: 2, price: 5.0, target: 4 },
];

/// Seeds of founded villages; saved with the world.
#[derive(Resource, Default, Clone, Serialize, Deserialize)]
pub struct Founded(pub Vec<u32>);

#[derive(Component, Clone, Serialize, Deserialize)]
pub struct Village {
    pub id: u32,
    pub name: String,
    pub center: Vec3,
    /// How much of each of [`GOODS`] the store holds.
    pub stock: [u32; GOODS.len()],
    pub coins: f32,
    /// Buildings of the plan finished, and blocks placed of the next one.
    pub built: usize,
    pub placed: usize,
    /// Cubic metres dug out of the quarry.
    pub dug: u32,
}

fn good(item: Item) -> Option<usize> {
    GOODS.iter().position(|g| g.item == item)
}

impl Village {
    pub fn stock(&self, item: Item) -> u32 {
        good(item).map_or(0, |i| self.stock[i])
    }

    pub fn add(&mut self, item: Item, n: u32) {
        if let Some(i) = good(item) {
            self.stock[i] += n;
        }
    }

    /// Takes `n` if there are that many.
    pub fn take(&mut self, item: Item, n: u32) -> bool {
        let Some(i) = good(item).filter(|&i| self.stock[i] >= n) else { return false };
        self.stock[i] -= n;
        true
    }

    /// Coins one item is worth here now: dear when scarce, cheap when plentiful.
    pub fn unit_price(&self, item: Item) -> f32 {
        let Some(i) = good(item) else { return 0.0 };
        let g = &GOODS[i];
        let scarcity = (g.target as f32 / self.stock[i].max(1) as f32).clamp(0.5, 3.0);
        g.price / g.bundle as f32 * scarcity
    }

    fn bundle_price(&self, i: usize) -> u32 {
        (self.unit_price(GOODS[i].item) * GOODS[i].bundle as f32).round().max(1.0) as u32
    }

    /// What the village pays a player for a bundle of good `i`, if it wants one and can pay.
    pub fn buys(&self, i: usize) -> Option<u32> {
        let price = self.bundle_price(i);
        (self.stock[i] < GOODS[i].target * 2 && self.coins >= price as f32).then_some(price)
    }

    /// What the village asks for a bundle of good `i`, if it has to spare.
    pub fn sells(&self, i: usize) -> Option<u32> {
        (self.stock[i] >= GOODS[i].target + GOODS[i].bundle).then(|| self.bundle_price(i) + 1)
    }

    /// Pays up to `wage` from the treasury, but never more than a small share of it, so a
    /// poor village pays less and its coins settle where wages and bread purchases balance.
    /// Returns what was paid.
    pub fn pay(&mut self, wage: f32) -> f32 {
        let paid = wage.min(self.coins * TREASURY_SHARE).max(0.0);
        self.coins -= paid;
        paid
    }
}

// --- Where villages stand -------------------------------------------------------------------

fn hash(x: i32, z: i32, seed: u32) -> u32 {
    (x as u32).wrapping_mul(0x9e37_79b1) ^ (z as u32).wrapping_mul(0x85eb_ca77) ^ seed.wrapping_mul(0xc2b2_ae3d)
}

/// The village site in a region, if there is one: its id and the middle of its well.
fn site(terrain: &Terrain, region: IVec2) -> Option<(u32, Vec3)> {
    let id = hash(region.x, region.y, terrain.seed());
    if rand01(id) > CHANCE {
        return None;
    }
    let center = (region.as_vec2() + Vec2::new(0.2 + 0.6 * rand01(id ^ 1), 0.2 + 0.6 * rand01(id ^ 2))) * REGION;
    // Within the region, settle on the flattest of a few spots.
    let flat = |c: Vec2| {
        let heights: Vec<f32> = (-2..=2)
            .flat_map(|x| (-2..=2).map(move |z| c + Vec2::new(x as f32, z as f32) * 8.0))
            .map(|p| terrain.surface_height(p.x, p.y))
            .collect();
        let (lo, hi) = heights.iter().fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
        (hi - lo, lo)
    };
    let (best, (bump, low)) = (0..6)
        .map(|i| center + Vec2::from_angle(i as f32) * 24.0 * (i > 0) as u32 as f32)
        .map(|c| (c, flat(c)))
        .min_by(|a, b| a.1.0.total_cmp(&b.1.0))?;
    let livable =
        matches!(terrain.biome(best.x, best.y), Biome::Plains | Biome::Forest | Biome::Savanna | Biome::Taiga);
    if bump > 4.0 || low < SEA_LEVEL + 1.0 || !livable {
        return None;
    }
    Some((id, Vec3::new(best.x, terrain.surface_height(best.x, best.y), best.y)))
}

/// Village sites within `radius` of `p`.
pub fn sites_near(terrain: &Terrain, p: Vec3, radius: f32) -> Vec<(u32, Vec3)> {
    let lo = ((p.xz() - radius) / REGION).floor().as_ivec2();
    let hi = ((p.xz() + radius) / REGION).floor().as_ivec2();
    (lo.y..=hi.y)
        .flat_map(|z| (lo.x..=hi.x).map(move |x| IVec2::new(x, z)))
        .filter_map(|r| site(terrain, r))
        .filter(|(_, c)| c.xz().distance(p.xz()) < radius)
        .collect()
}

const NAMES: [&str; 12] = [
    "Берёзовка",
    "Ручьи",
    "Сосновка",
    "Камышино",
    "Овражки",
    "Лесное",
    "Каменка",
    "Ольховка",
    "Тихое",
    "Луговое",
    "Ясная Поляна",
    "Заречье",
];

/// Brings villages near players to life, with their first buildings and people.
fn found(
    mut commands: Commands,
    time: Res<Time>,
    mut terrain: ResMut<Terrain>,
    mut founded: ResMut<Founded>,
    players: Query<&Transform, Or<(With<Player>, With<AvatarOf>)>>,
    mut since: Local<f32>,
) {
    *since += time.delta_secs();
    if *since < 2.0 {
        return;
    }
    *since = 0.0;
    let near: Vec<(u32, Vec3)> =
        players.iter().flat_map(|p| sites_near(&terrain, p.translation, FOUND_RADIUS)).collect();
    for (id, center) in near {
        if founded.0.contains(&id) || !terrain.is_ready_around(center) {
            continue;
        }
        founded.0.push(id);
        let mut village = Village {
            id,
            name: NAMES[id as usize % NAMES.len()].into(),
            center,
            stock: [0; GOODS.len()],
            coins: 60.0,
            built: 0,
            placed: 0,
            dug: 0,
        };
        for (item, n) in [(Item::Planks, 60), (Item::Stone, 30), (Item::Bread, 12), (Item::Seeds, 12), (Item::Wood, 8)]
        {
            village.add(item, n);
        }
        // The well and the store stand from the start; the rest is up to the villagers.
        for _ in 0..2 {
            let (kind, origin) = plan(&terrain, center)[village.built];
            terrain.set_blocks(blocks(kind, origin, center).into_iter().flat_map(|(m, mat)| meter(m, mat)));
            village.built += 1;
        }
        info!("village {} founded at {center}", village.name);
        for (i, job) in [Job::Builder, Job::Woodcutter, Job::Farmer].into_iter().enumerate() {
            villagers::spawn(&mut commands, &village, job, gate(&village, i), id ^ i as u32);
        }
        commands.spawn((DespawnOnExit(AppState::Game), Replicated, village));
    }
}

/// The store's own work every few seconds: raw goods into useful ones, and newcomers.
fn work_store(
    mut commands: Commands,
    time: Res<Time>,
    mut villages: Query<&mut Village>,
    people: Query<&villagers::Villager>,
    mut since: Local<f32>,
) {
    *since += time.delta_secs();
    if *since < TICK_SECS {
        return;
    }
    *since = 0.0;
    for mut v in &mut villages {
        let short = |v: &Village, item| v.stock(item) < GOODS[good(item).unwrap_or(0)].target;
        if short(&v, Item::Planks) && v.take(Item::Wood, 1) {
            v.add(Item::Planks, 4);
        }
        if short(&v, Item::Bread) && v.take(Item::Wheat, 3) {
            v.add(Item::Bread, 1);
        }
        if short(&v, Item::Bricks) && v.stock(Item::Stone) > 20 && v.take(Item::Stone, 2) {
            v.add(Item::Bricks, 2);
        }
        if v.stock(Item::Coal) > 0 && v.take(Item::IronOre, 1) && v.take(Item::Coal, 1) {
            v.add(Item::IronIngot, 1);
        }
        // Newcomers settle where there are free beds and food to spare.
        let living: Vec<Job> = people.iter().filter(|p| p.village == v.id).map(|p| p.job).collect();
        let beds = 2 + houses_built(&v) * BEDS_PER_HOUSE;
        if living.len() < beds && v.stock(Item::Bread) >= WELCOME_BREAD * 2 && v.take(Item::Bread, WELCOME_BREAD) {
            let job = villagers::needed(&v, &living);
            let seed = v.id ^ (living.len() as u32 * 7919);
            villagers::spawn(&mut commands, &v, job, gate(&v, living.len()), seed);
        }
    }
}

/// Where the `i`th villager arrives: in the open beside the well.
fn gate(v: &Village, i: usize) -> Vec3 {
    v.center + Vec3::new(i as f32 % 4.0 - 1.5, 1.0, -3.5)
}

// --- What villages build --------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Well,
    Store,
    House,
    Farm,
}

/// Every building of a village, in the order it is built, with its middle at floor level
/// (in whole metres).
pub fn plan(terrain: &Terrain, center: Vec3) -> Vec<(Kind, IVec3)> {
    const SLOTS: [(Kind, i32, i32); 9] = [
        (Kind::Well, 0, 0),
        (Kind::Store, 7, 0),
        (Kind::House, 0, 8),
        (Kind::Farm, -11, 0),
        (Kind::House, 8, 8),
        (Kind::House, -8, 8),
        (Kind::House, 0, -9),
        (Kind::House, 8, -9),
        (Kind::House, -8, -9),
    ];
    let c = center.floor().as_ivec3();
    SLOTS
        .map(|(kind, x, z)| {
            let (x, z) = (c.x + x, c.z + z);
            let floor = terrain.surface_height(x as f32 + 0.5, z as f32 + 0.5).round() as i32;
            (kind, IVec3::new(x, floor, z))
        })
        .into()
}

/// A standing spot in front of a building.
pub fn spot(&(_, origin): &(Kind, IVec3)) -> Vec3 {
    origin.as_vec3() + Vec3::new(0.5, 0.1, 0.5)
}

pub fn houses_built(v: &Village) -> usize {
    (0..v.built.min(9)).filter(|&i| matches!(i, 2 | 4..=8)).count()
}

/// The farm's field, if it has been built: its middle and how far it reaches.
pub fn farm(terrain: &Terrain, v: &Village) -> Option<(Vec3, f32)> {
    (v.built > 3).then(|| (spot(&plan(terrain, v.center)[3]), 2.5))
}

/// The quarry's corner, out beyond the houses.
pub fn quarry(v: &Village) -> IVec3 {
    v.center.floor().as_ivec3() + IVec3::new(15, 0, -16)
}

/// The blocks of a building, a metre each: where (in metres) and of what. Air clears the way.
/// Laid bottom to top, clearing before building on each layer.
pub fn blocks(kind: Kind, origin: IVec3, center: Vec3) -> Vec<(IVec3, Material)> {
    let mut out = Vec::new();
    // The door faces the middle of the village.
    let to_center = center.floor().as_ivec3() - origin;
    let door = if to_center.z.abs() >= to_center.x.abs() {
        IVec2::new(0, to_center.z.signum())
    } else {
        IVec2::new(to_center.x.signum(), 0)
    };
    let mut put = |x: i32, y: i32, z: i32, m: Material| out.push((origin + IVec3::new(x, y, z), m));
    let walls = |put: &mut dyn FnMut(i32, i32, i32, Material), r: i32, high: i32, open: i32| {
        for y in 0..high {
            for x in -r..=r {
                for z in -r..=r {
                    let edge = x.abs() == r || z.abs() == r;
                    let corner = x.abs() == r && z.abs() == r;
                    let at_door = IVec2::new(x, z).dot(door) == r && (x * door.y + z * door.x).abs() <= open && y < 2;
                    let window =
                        y == 1 && !corner && (x * door.y + z * door.x).abs() == 0 && IVec2::new(x, z).dot(door) == -r;
                    let m = match () {
                        _ if !edge || at_door || window => Material::Air,
                        _ if corner => Material::Wood,
                        _ => Material::Planks,
                    };
                    put(x, y, z, m);
                }
            }
        }
    };
    match kind {
        Kind::Well => {
            for x in -1..=1 {
                for z in -1..=1 {
                    put(x, -1, z, Material::Stone);
                    put(x, 0, z, if x == 0 && z == 0 { Material::Ice } else { Material::Bricks });
                    for y in 1..3 {
                        put(x, y, z, if x != 0 && z != 0 { Material::Wood } else { Material::Air });
                    }
                    put(x, 3, z, Material::Planks);
                }
            }
        }
        Kind::House | Kind::Store => {
            for x in -2..=2 {
                for z in -2..=2 {
                    put(x, -1, z, Material::Planks);
                }
            }
            let open = if kind == Kind::Store { 1 } else { 0 };
            walls(&mut put, 2, 3, open);
            let roof = if kind == Kind::House { Material::Bricks } else { Material::Planks };
            for (y, r) in [(3, 2), (4, 1), (5, 0)].into_iter().take(if kind == Kind::House { 3 } else { 1 }) {
                for x in -r..=r {
                    for z in -r..=r {
                        put(x, y, z, roof);
                    }
                }
            }
        }
        Kind::Farm => {
            for x in -3i32..=3 {
                for z in -3i32..=3 {
                    let edge = x.abs() == 3 || z.abs() == 3;
                    let gate = IVec2::new(x, z).dot(door) == 3 && (x * door.y + z * door.x) == 0;
                    put(x, 0, z, if edge && !gate { Material::Wood } else { Material::Air });
                    put(x, 1, z, Material::Air);
                }
            }
        }
    }
    out.sort_by_key(|(p, m)| (p.y, *m != Material::Air));
    out
}

/// The cells of a one-metre block.
pub fn meter(m: IVec3, material: Material) -> impl Iterator<Item = (IVec3, Block)> {
    let n = (1.0 / CELL) as i32;
    let block = if material == Material::Air { Block::AIR } else { Block::solid(material) };
    (0..n * n * n).map(move |i| (m * n + IVec3::new(i % n, i / n % n, i / (n * n)), block))
}

/// The item a block of a building costs, if any.
pub fn cost(material: Material) -> Option<Item> {
    match material {
        Material::Planks => Some(Item::Planks),
        Material::Wood => Some(Item::Wood),
        Material::Bricks => Some(Item::Bricks),
        Material::Stone => Some(Item::Stone),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn village() -> Village {
        Village {
            id: 1,
            name: String::new(),
            center: Vec3::ZERO,
            stock: [0; GOODS.len()],
            coins: 100.0,
            built: 0,
            placed: 0,
            dug: 0,
        }
    }

    #[test]
    fn scarce_goods_cost_more_and_only_surplus_is_sold() {
        let mut v = village();
        v.add(Item::Planks, 10);
        let scarce = v.unit_price(Item::Planks);
        v.add(Item::Planks, 400);
        assert!(v.unit_price(Item::Planks) < scarce);
        let planks = good(Item::Planks).unwrap();
        assert!(v.sells(planks).is_some(), "a pile of planks is for sale");
        assert!(v.buys(planks).is_none(), "and no more are wanted");
        let bread = good(Item::Bread).unwrap();
        assert!(v.sells(bread).is_none() && v.buys(bread).is_some());
    }

    #[test]
    fn wages_never_empty_the_treasury() {
        let mut v = village();
        let paid: f32 = (0..1000).map(|_| v.pay(5.0)).sum();
        assert!(v.coins > 0.0 && paid > 50.0);
    }

    #[test]
    fn a_house_has_a_door_facing_the_well() {
        let origin = IVec3::new(0, 10, 8);
        let blocks = blocks(Kind::House, origin, Vec3::new(0.0, 10.0, 0.0));
        let at = |x, y, z| blocks.iter().rev().find(|(p, _)| *p == origin + IVec3::new(x, y, z)).map(|(_, m)| *m);
        assert_eq!(at(0, 0, -2), Some(Material::Air), "doorway towards the well");
        assert_eq!(at(0, 0, 2), Some(Material::Planks), "solid back wall");
        assert_eq!(at(2, 0, 2), Some(Material::Wood), "log corners");
    }
}
