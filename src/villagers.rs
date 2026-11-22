//! Villagers: each with a trade, a purse and an appetite, working for their village.
//!
//! A villager picks a job to do, walks there, does it, and brings what it earned to the store,
//! where the village pays for it. Hungry ones buy bread; at night they sleep at home. One who
//! goes hungry too long moves away.

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_replicon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    body::{Body, MoveBodies},
    falling::Removed,
    items::{Item, PropKind},
    mobs::rand01,
    net::{Avatar, Look},
    props::{self, Crop, Prop},
    sky::TimeOfDay,
    terrain::{CELL, Material, Terrain},
    villages::{self, Kind, Village},
};

/// Players never have this token, so villagers are never taken for one.
pub const TOKEN: u64 = u64::MAX;
const WALK: f32 = 2.4;
const ARRIVED: f32 = 1.4;
/// Someone who can't get somewhere in this long just goes there.
const STUCK_SECS: f32 = 25.0;
/// Hunger runs from 100 (full) down by one this often.
const HUNGER_SECS: f32 = 6.0;
const EAT_BELOW: f32 = 40.0;
/// A villager hungry this long leaves the village.
const STARVE_SECS: f32 = 120.0;
/// How far from the well woodcutters look for trees.
const FOREST: f32 = 96.0;
const FIELD_CROPS: usize = 16;

pub struct VillagersPlugin;

impl Plugin for VillagersPlugin {
    fn build(&self, app: &mut App) {
        app.replicate::<VillagerOf>().add_observer(shape).add_systems(
            Update,
            live.before(MoveBodies)
                .run_if(in_state(AppState::Game))
                .run_if(crate::net::authority)
                .run_if(|time: Res<Time<Virtual>>| !time.is_paused()),
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Job {
    Woodcutter,
    Miner,
    Farmer,
    Builder,
}

impl Job {
    fn name(self) -> &'static str {
        match self {
            Job::Woodcutter => "дровосек",
            Job::Miner => "шахтёр",
            Job::Farmer => "фермер",
            Job::Builder => "строитель",
        }
    }

    fn tool(self) -> Item {
        match self {
            Job::Woodcutter => Item::StoneAxe,
            Job::Miner => Item::StonePickaxe,
            Job::Farmer => Item::Seeds,
            Job::Builder => Item::Planks,
        }
    }
}

/// The job a newcomer should take: building first, then whatever the store is shortest of.
pub fn needed(v: &Village, living: &[Job]) -> Job {
    if !living.contains(&Job::Builder) {
        return Job::Builder;
    }
    let shortage = |item, target: f32| v.stock(item) as f32 / target;
    [
        (Job::Woodcutter, shortage(Item::Planks, 120.0)),
        (Job::Miner, shortage(Item::Stone, 80.0)),
        (Job::Farmer, shortage(Item::Bread, 24.0)),
    ]
    .into_iter()
    .map(|(job, short)| (job, short * (1 + living.iter().filter(|j| **j == job).count()) as f32))
    .min_by(|a, b| a.1.total_cmp(&b.1))
    .map_or(Job::Woodcutter, |(job, _)| job)
}

/// Which village someone belongs to; everyone sees it, so a player can trade with them.
#[derive(Component, Serialize, Deserialize, Clone, Copy)]
pub struct VillagerOf(pub u32);

#[derive(Component, Clone, Serialize, Deserialize)]
pub struct Villager {
    pub village: u32,
    pub job: Job,
    pub name: String,
    pub coins: f32,
    pub hunger: f32,
    skin: u8,
    carry: Option<(Item, u32)>,
    task: Task,
    /// Seconds spent on the current task.
    since: f32,
    starving: f32,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug)]
enum Task {
    /// Waiting this many more seconds before deciding what next.
    Idle(f32),
    Go {
        to: Vec3,
        then: Act,
    },
    Do {
        act: Act,
        left: f32,
    },
    Asleep,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug)]
enum Act {
    Chop(Vec3),
    /// A cubic metre of the quarry.
    Mine(IVec3),
    Plant(Vec3),
    Harvest(Vec3),
    Build,
    Deliver,
    Eat,
    Rest,
}

impl Act {
    fn seconds(self) -> f32 {
        match self {
            Act::Chop(_) => 6.0,
            Act::Mine(_) => 5.0,
            Act::Plant(_) | Act::Harvest(_) => 1.5,
            Act::Build => 1.2,
            Act::Deliver | Act::Eat | Act::Rest => 1.0,
        }
    }
}

const NAMES: [&str; 16] = [
    "Иван",
    "Марья",
    "Пётр",
    "Анна",
    "Фёдор",
    "Дарья",
    "Степан",
    "Ольга",
    "Тимофей",
    "Василиса",
    "Гордей",
    "Злата",
    "Семён",
    "Агата",
    "Матвей",
    "Ксения",
];

/// A villager as saved with the world.
#[derive(Clone, Serialize, Deserialize)]
pub struct SavedVillager {
    pub villager: Villager,
    pub at: Vec3,
}

pub fn spawn(commands: &mut Commands, village: &Village, job: Job, at: Vec3, seed: u32) {
    let villager = Villager {
        village: village.id,
        job,
        name: NAMES[(rand01(seed) * NAMES.len() as f32) as usize % NAMES.len()].into(),
        coins: 5.0,
        hunger: 80.0,
        skin: (rand01(seed ^ 0x51) * crate::avatars::SKINS.len() as f32) as u8,
        carry: None,
        task: Task::Idle(1.0),
        since: 0.0,
        starving: 0.0,
    };
    restore(commands, &SavedVillager { villager, at });
}

pub fn restore(commands: &mut Commands, saved: &SavedVillager) {
    let v = &saved.villager;
    commands.spawn((
        DespawnOnExit(AppState::Game),
        Replicated,
        Avatar {
            name: format!("{}, {}", v.name, v.job.name()),
            token: TOKEN,
            look: Look { skin: v.skin, held: Some(v.job.tool()), asleep: false },
        },
        VillagerOf(v.village),
        v.clone(),
        Body::new(0.35, 1.8),
        Transform::from_translation(saved.at),
    ));
}

/// Something to click on, here and on guests' screens.
fn shape(add: On<Add<VillagerOf>>, mut commands: Commands) {
    let body = Body::new(0.35, 1.8);
    let collider = Collider::compound(vec![(body.center_offset(), Quat::IDENTITY, body.collider())]);
    commands.entity(add.entity).insert((RigidBody::Kinematic, collider));
}

/// What the village around a villager offers to do next.
struct Surroundings<'a> {
    terrain: &'a Terrain,
    village: &'a Village,
    crops: &'a [(Entity, Vec3, bool)],
    night: bool,
}

impl Villager {
    fn home(&self, around: &Surroundings) -> Vec3 {
        let plan = villages::plan(around.terrain, around.village.center);
        let houses: Vec<_> = plan.iter().take(around.village.built).filter(|(k, _)| *k == Kind::House).collect();
        let pick = self.name.len() + self.job as usize;
        houses.get(pick % houses.len().max(1)).map_or(villages::spot(&plan[1]), |h| villages::spot(h))
    }

    fn decide(&self, here: Vec3, around: &Surroundings) -> Task {
        let v = around.village;
        let store = villages::spot(&villages::plan(around.terrain, v.center)[1]);
        let go = |to: Vec3, then| Task::Go { to, then };
        if around.night {
            return go(self.home(around), Act::Rest);
        }
        if self.carry.is_some() {
            return go(store, Act::Deliver);
        }
        if self.hunger < EAT_BELOW && v.stock(Item::Bread) > 0 {
            return go(store, Act::Eat);
        }
        let mine = || {
            let (cell, rim) = quarry_cell(around.terrain, v);
            go(rim, Act::Mine(cell))
        };
        // With no trees left about, a woodcutter makes do in the quarry.
        let chop = || {
            let trees = around.terrain.trees_near(v.center, FOREST);
            let standing = trees.into_iter().filter(|t| {
                around.terrain.block(((*t + Vec3::Y * 0.5) / CELL).floor().as_ivec3()).material == Material::Wood
            });
            match standing.min_by(|a, b| a.distance(here).total_cmp(&b.distance(here))) {
                Some(tree) => go(tree + (here - tree).with_y(0.0).normalize_or_zero() * 1.2, Act::Chop(tree)),
                None => mine(),
            }
        };
        match self.job {
            Job::Woodcutter => chop(),
            Job::Miner => mine(),
            Job::Farmer => {
                let Some((field, reach)) = villages::farm(around.terrain, v) else { return chop() };
                let in_field = |p: Vec3| (p - field).abs().max_element() <= reach + 0.5;
                if let Some(&(_, at, _)) = around.crops.iter().find(|(_, at, ripe)| *ripe && in_field(*at)) {
                    return go(at, Act::Harvest(at));
                }
                let planted: Vec<Vec3> = around.crops.iter().map(|c| c.1).filter(|p| in_field(*p)).collect();
                let free = (0..25)
                    .map(|i| field + Vec3::new((i % 5) as f32 - 2.0, 0.0, (i / 5) as f32 - 2.0))
                    .find(|p| !planted.iter().any(|c| c.xz().distance(p.xz()) < 0.5));
                match free {
                    Some(spot) if planted.len() < FIELD_CROPS && v.stock(Item::Seeds) > 0 => go(spot, Act::Plant(spot)),
                    _ => Task::Idle(4.0),
                }
            }
            Job::Builder => match next_block(around.terrain, v) {
                Some((m, material)) if villages::cost(material).is_none_or(|item| v.stock(item) > 0) => {
                    let at = m.as_vec3() + Vec3::new(0.5, 0.0, 0.5);
                    go(at + (here - at).with_y(0.0).normalize_or_zero() * 2.5, Act::Build)
                }
                // Short of something: the builder goes and gets it.
                Some((_, Material::Stone | Material::Bricks)) => mine(),
                Some(_) => chop(),
                None => chop(),
            },
        }
    }
}

/// The next block a village's builders lay, if any building is left to build.
fn next_block(terrain: &Terrain, v: &Village) -> Option<(IVec3, Material)> {
    let plan = villages::plan(terrain, v.center);
    let (kind, origin) = *plan.get(v.built)?;
    villages::blocks(kind, origin, v.center).get(v.placed).copied()
}

/// The quarry's next cubic metre and where to stand to dig it: three metres deep, then a step
/// further along.
fn quarry_cell(terrain: &Terrain, v: &Village) -> (IVec3, Vec3) {
    let q = villages::quarry(v);
    let n = v.dug as i32;
    let corner = q + IVec3::new(n / 27 * 3, 0, 0);
    let ground = terrain.surface_height(corner.x as f32 + 1.5, corner.z as f32 + 1.5).round() as i32;
    let cell = IVec3::new(corner.x + n % 3, ground - 1 - n / 9 % 3, corner.z + n / 3 % 3);
    (cell, Vec3::new(corner.x as f32 - 0.7, ground as f32 + 0.2, corner.z as f32 + 1.5))
}

#[allow(clippy::too_many_arguments)]
fn live(
    mut commands: Commands,
    time: Res<Time>,
    time_of_day: Res<TimeOfDay>,
    mut terrain: ResMut<Terrain>,
    mut villages: Query<&mut Village>,
    mut people: Query<(Entity, &mut Villager, &mut Body, &mut Transform, &mut Avatar)>,
    crops: Query<(Entity, &Prop, &Crop, &Transform), Without<Villager>>,
    mut removed: MessageWriter<Removed>,
) {
    let dt = time.delta_secs();
    let crop_list: Vec<(Entity, Vec3, bool)> = crops
        .iter()
        .filter(|(_, p, ..)| p.0 == PropKind::Wheat)
        .map(|(e, _, c, t)| (e, t.translation, c.ripe()))
        .collect();
    for (entity, mut person, mut body, mut transform, mut avatar) in &mut people {
        let Some(mut village) = villages.iter_mut().find(|v| v.id == person.village) else { continue };
        let here = transform.translation;
        person.hunger = (person.hunger - dt / HUNGER_SECS).max(0.0);
        person.starving = if person.hunger <= 0.0 { person.starving + dt } else { 0.0 };
        if person.starving > STARVE_SECS {
            info!("{} left {}, hungry", person.name, village.name);
            commands.entity(entity).despawn();
            continue;
        }
        person.since += dt;
        body.wish = Vec3::ZERO;
        body.jump = false;
        let night = time_of_day.is_night();
        let mut task = person.task;
        match task {
            Task::Idle(left) if left > dt => task = Task::Idle(left - dt),
            Task::Idle(_) => {
                let around = Surroundings { terrain: &terrain, village: &village, crops: &crop_list, night };
                task = person.decide(here, &around);
            }
            Task::Go { to, then } => {
                let flat = (to - here).with_y(0.0);
                if flat.length() < ARRIVED {
                    task = Task::Do { act: then, left: then.seconds() };
                } else if person.since > STUCK_SECS {
                    transform.translation = to + Vec3::Y * 0.5;
                    body.velocity = Vec3::ZERO;
                } else {
                    let speed = if person.hunger > 0.0 { WALK } else { WALK * 0.6 };
                    body.wish = flat.normalize() * speed;
                    body.jump = body.ground_normal.is_some() && body.velocity.with_y(0.0).length() < speed * 0.3;
                    let yaw = (-flat.x).atan2(-flat.z);
                    transform.rotation = transform.rotation.slerp(Quat::from_rotation_y(yaw), (8.0 * dt).min(1.0));
                }
            }
            Task::Do { act, left } if left > dt => task = Task::Do { act, left: left - dt },
            Task::Do { act, .. } => {
                task = Task::Idle(0.5);
                finish(&mut commands, act, &mut person, &mut village, &mut terrain, &crop_list, here, &mut removed);
                if matches!(act, Act::Rest) {
                    task = Task::Asleep;
                }
            }
            Task::Asleep if night => {}
            Task::Asleep => task = Task::Idle(0.0),
        }
        if std::mem::discriminant(&task) != std::mem::discriminant(&person.task) {
            person.since = 0.0;
        }
        person.task = task;
        let look = Look {
            skin: person.skin,
            held: person.carry.map(|(item, _)| item).or(Some(person.job.tool())),
            asleep: matches!(task, Task::Asleep),
        };
        if avatar.look != look {
            avatar.look = look;
        }
    }
}

/// Does what a villager came to do.
#[allow(clippy::too_many_arguments)]
fn finish(
    commands: &mut Commands,
    act: Act,
    person: &mut Villager,
    v: &mut Village,
    terrain: &mut Terrain,
    crops: &[(Entity, Vec3, bool)],
    here: Vec3,
    removed: &mut MessageWriter<Removed>,
) {
    match act {
        Act::Chop(tree) => {
            // The whole trunk comes down; the crown falls with nothing left to hold it.
            let base = (tree / CELL).floor().as_ivec3();
            let cells: Vec<IVec3> = (-4..48)
                .flat_map(|y| (-2..=2).flat_map(move |x| (-2..=2).map(move |z| base + IVec3::new(x, y, z))))
                .filter(|c| terrain.block(*c).material == Material::Wood)
                .collect();
            if cells.is_empty() {
                return;
            }
            terrain.set_blocks(cells.iter().map(|c| (*c, crate::terrain::Block::AIR)));
            person.carry = Some((Item::Wood, (cells.len() as u32 / 12).clamp(2, 8)));
            removed.write(Removed { cells, from: here });
        }
        Act::Mine(cell) => {
            let dug: Vec<_> =
                villages::meter(cell, Material::Air).map(|(c, _)| (c, terrain.block(c).material)).collect();
            let count = |m: Material| dug.iter().filter(|(_, mat)| *mat == m).count() as u32;
            terrain.set_blocks(villages::meter(cell, Material::Air));
            v.dug += 1;
            // Finds of coal and ore are told to the store at once; the stone is carried.
            v.add(Item::Coal, count(Material::CoalOre) / 8);
            v.add(Item::IronOre, count(Material::IronOre) / 8);
            let stone = count(Material::Stone) + count(Material::Dirt) / 2 + count(Material::Grass) / 2;
            person.carry = Some((Item::Stone, (stone / 16).max(1)));
        }
        Act::Plant(spot) => {
            if v.take(Item::Seeds, 1) {
                let ground = terrain.ground_below(spot + Vec3::Y);
                props::spawn(commands, PropKind::Wheat, Transform::from_translation(spot.with_y(ground)), 0.0);
            }
        }
        Act::Harvest(spot) => {
            if let Some(&(crop, ..)) = crops.iter().find(|(_, at, ripe)| *ripe && at.distance(spot) < 0.5) {
                commands.entity(crop).despawn();
                person.carry = Some((Item::Wheat, 2));
                v.add(Item::Seeds, 1);
            }
        }
        Act::Build => {
            let Some((m, material)) = next_block(terrain, v) else { return };
            if villages::cost(material).is_some_and(|item| !v.take(item, 1)) {
                return;
            }
            terrain.set_blocks(villages::meter(m, material));
            if material != Material::Air {
                person.coins += v.pay(v.unit_price(Item::Planks) * 3.0);
            }
            v.placed += 1;
            let plan = villages::plan(terrain, v.center);
            let (kind, origin) = plan[v.built];
            if v.placed >= villages::blocks(kind, origin, v.center).len() {
                v.built += 1;
                v.placed = 0;
                info!("{} built a {kind:?}", v.name);
            }
        }
        Act::Deliver => {
            if let Some((item, n)) = person.carry.take() {
                let wage = v.unit_price(item) * n as f32 * 0.6;
                v.add(item, n);
                person.coins += v.pay(wage);
            }
        }
        Act::Eat => {
            if v.take(Item::Bread, 1) {
                let price = v.unit_price(Item::Bread).min(person.coins);
                person.coins -= price;
                v.coins += price;
                person.hunger = (person.hunger + 50.0).min(100.0);
            }
        }
        Act::Rest => {}
    }
}
