//! Creatures: farm animals graze by day, monsters hunt the player at night; which ones
//! depends on the biome.

use std::f32::consts::TAU;

use avian3d::prelude::*;
use bevy::{
    prelude::*,
    world_serialization::{WorldAsset, WorldAssetRoot, WorldInstanceReady},
};

use crate::{
    AppState, Creative, Mode,
    body::{Body, Layer, MoveBodies},
    hud::Toast,
    items::{Inventory, Item, PropKind},
    player::Player,
    props::{Prop, TORCH_SAFE_RADIUS},
    sky::TimeOfDay,
    survival::Vitals,
    terrain::{Biome, CELL, Material, SEA_LEVEL, Terrain},
};

const MAX_ANIMALS: usize = 8;
const MAX_MONSTERS: usize = 5;
const ANIMAL_SPAWN_SECS: f32 = 4.0;
const MONSTER_SPAWN_SECS: f32 = 10.0;
const SPAWN_MIN_DISTANCE: f32 = 16.0;
const SPAWN_MAX_DISTANCE: f32 = 26.0;
const DESPAWN_DISTANCE: f32 = 110.0;

const SIGHT: f32 = 40.0;
const ATTACK_SECS: f32 = 1.2;
/// How long a body lies after dying.
const DEATH_SECS: f32 = 1.4;
/// Health per second zombies lose in daylight.
const SUNBURN: f32 = 2.0;

pub struct MobsPlugin;

impl Plugin for MobsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Summon>()
            .add_systems(Startup, setup)
            .add_systems(Update, summon.run_if(in_state(AppState::Game)))
            .add_systems(
                Update,
                (spawn_mobs, think).chain().before(MoveBodies).run_if(|mode: Option<Res<State<Mode>>>| {
                    mode.is_some_and(|m| !matches!(**m, Mode::Paused | Mode::Settings))
                }),
            )
            .add_systems(Update, (die, animate).after(MoveBodies).run_if(in_state(AppState::Game)));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MobKind {
    Sheep,
    Zombie,
    Cow,
    Pig,
    Chicken,
    Spider,
    Wolf,
}

/// What makes each kind of creature different.
#[derive(Clone, Copy)]
pub struct Species {
    health: f32,
    radius: f32,
    height: f32,
    walk: f32,
    run: f32,
    /// Damage per bite; zero for animals, which flee instead.
    pub damage: f32,
    /// Items dropped: item, at least, at most.
    drops: &'static [(Item, u32, u32)],
    model: &'static str,
    /// Clip indices in the glTF file: idle, walk, run, attack, hurt, death.
    clips: [usize; 6],
    scale: f32,
}

/// A creature to put into the world right away (the `summon` cheat).
#[derive(Message)]
pub struct Summon {
    pub kind: MobKind,
    pub at: Vec3,
    pub yaw: f32,
}

fn summon(mut commands: Commands, mut summons: MessageReader<Summon>, models: Res<MobModels>) {
    for s in summons.read() {
        spawn_mob(&mut commands, &models, s.kind, s.at, s.yaw);
    }
}

impl MobKind {
    pub fn named(name: &str) -> Option<MobKind> {
        MobKind::ALL.into_iter().find(|k| format!("{k:?}").eq_ignore_ascii_case(name))
    }

    pub const ALL: [MobKind; 7] =
        [MobKind::Sheep, MobKind::Zombie, MobKind::Cow, MobKind::Pig, MobKind::Chicken, MobKind::Spider, MobKind::Wolf];

    pub fn species(self) -> &'static Species {
        match self {
            MobKind::Sheep => &Species {
                health: 8.0,
                radius: 0.45,
                height: 1.1,
                walk: 1.3,
                run: 4.5,
                damage: 0.0,
                drops: &[(Item::Mutton, 1, 2), (Item::Wool, 1, 2)],
                model: "models/sheep.glb",
                clips: [3, 7, 6, 1, 1, 0],
                scale: 0.55,
            },
            MobKind::Cow => &Species {
                health: 10.0,
                radius: 0.55,
                height: 1.4,
                walk: 1.1,
                run: 4.0,
                damage: 0.0,
                drops: &[(Item::Beef, 1, 3)],
                model: "models/cow.glb",
                clips: [12, 9, 4, 0, 6, 2],
                scale: 0.3,
            },
            MobKind::Pig => &Species {
                health: 10.0,
                radius: 0.4,
                height: 0.9,
                walk: 1.2,
                run: 4.5,
                damage: 0.0,
                drops: &[(Item::Porkchop, 1, 3)],
                model: "models/pig.glb",
                clips: [2, 7, 6, 1, 1, 0],
                scale: 0.55,
            },
            MobKind::Chicken => &Species {
                health: 4.0,
                radius: 0.25,
                height: 0.6,
                walk: 1.0,
                run: 3.5,
                damage: 0.0,
                drops: &[(Item::Chicken, 1, 1)],
                model: "models/chicken.glb",
                clips: [3, 4, 4, 0, 0, 1],
                scale: 0.48,
            },
            MobKind::Zombie => &Species {
                health: 20.0,
                radius: 0.35,
                height: 1.8,
                walk: 1.2,
                run: 3.0,
                damage: 2.0,
                drops: &[],
                model: "models/zombie.glb",
                clips: [3, 6, 5, 0, 2, 1],
                scale: 0.62,
            },
            MobKind::Spider => &Species {
                health: 14.0,
                radius: 0.6,
                height: 0.8,
                walk: 1.5,
                run: 4.2,
                damage: 2.0,
                drops: &[],
                model: "models/spider.glb",
                clips: [2, 4, 4, 0, 0, 1],
                scale: 0.42,
            },
            MobKind::Wolf => &Species {
                health: 12.0,
                radius: 0.4,
                height: 1.0,
                walk: 1.4,
                run: 5.0,
                damage: 3.0,
                drops: &[],
                model: "models/wolf.glb",
                clips: [11, 8, 3, 0, 5, 1],
                scale: 0.36,
            },
        }
    }

    pub fn hostile(self) -> bool {
        self.species().damage > 0.0
    }
}

#[derive(Component)]
pub struct Mob {
    pub kind: MobKind,
    pub health: f32,
    /// Seconds left of the red flash after being hit.
    pub flash: f32,
    /// Seconds left of running away (animals).
    pub scared: f32,
    /// Seconds until the next wander decision or attack.
    timer: f32,
    heading: Vec3,
    /// Seconds since this mob last struck the player.
    since_attack: f32,
    /// Counts down while the death animation plays.
    dying: Option<f32>,
}

impl Mob {
    fn new(kind: MobKind) -> Self {
        Self {
            kind,
            health: kind.species().health,
            flash: 0.0,
            scared: 0.0,
            timer: 0.0,
            heading: Vec3::ZERO,
            since_attack: 9.0,
            dying: None,
        }
    }

    pub fn alive(&self) -> bool {
        self.dying.is_none()
    }

    /// Takes a hit from the player standing at `from`.
    pub fn hit(&mut self, body: &mut Body, damage: f32, from: Vec3, at: Vec3) {
        if !self.alive() {
            return;
        }
        self.health -= damage;
        self.flash = 0.25;
        self.scared = 6.0;
        let away = (at - from).with_y(0.0).normalize_or_zero();
        body.velocity = away * 6.0 + Vec3::Y * 4.5;
        body.ground_normal = None;
    }
}

/// A creature's model and its animation clips, loaded from `assets/models`.
struct Model {
    scene: Handle<WorldAsset>,
    graph: Handle<AnimationGraph>,
    idle: AnimationNodeIndex,
    walk: AnimationNodeIndex,
    run: AnimationNodeIndex,
    attack: AnimationNodeIndex,
    hurt: AnimationNodeIndex,
    death: AnimationNodeIndex,
    scale: f32,
}

/// One model per kind, in `MobKind::ALL` order.
#[derive(Resource)]
struct MobModels(Vec<Model>);

impl MobModels {
    fn get(&self, kind: MobKind) -> &Model {
        &self.0[kind as usize]
    }
}

fn setup(mut commands: Commands, server: Res<AssetServer>, mut graphs: ResMut<Assets<AnimationGraph>>) {
    let mut load = |kind: MobKind| {
        let Species { model: path, clips, scale, .. } = *kind.species();
        let (graph, nodes) =
            AnimationGraph::from_clips(clips.map(|i| server.load(GltfAssetLabel::Animation(i).from_asset(path))));
        Model {
            scene: server.load(GltfAssetLabel::Scene(0).from_asset(path)),
            graph: graphs.add(graph),
            idle: nodes[0],
            walk: nodes[1],
            run: nodes[2],
            attack: nodes[3],
            hurt: nodes[4],
            death: nodes[5],
            scale,
        }
    };
    commands.insert_resource(MobModels(MobKind::ALL.map(&mut load).into()));
}

/// Links a mob to the animation player inside its model, once the model has spawned.
#[derive(Component)]
struct Animated {
    player: Entity,
    playing: AnimationNodeIndex,
}

fn spawn_mob(commands: &mut Commands, models: &MobModels, kind: MobKind, at: Vec3, yaw: f32) {
    let body = Body::new(kind.species().radius, kind.species().height);
    let collider = Collider::compound(vec![(body.center_offset(), Quat::IDENTITY, body.collider())]);
    let model = models.get(kind);
    let mob = commands
        .spawn((
            DespawnOnExit(AppState::Game),
            Mob::new(kind),
            body,
            RigidBody::Kinematic,
            collider,
            Transform::from_translation(at).with_rotation(Quat::from_rotation_y(yaw)),
            Visibility::default(),
        ))
        .id();
    // Models face +Z; mobs walk towards -Z.
    commands
        .spawn((
            WorldAssetRoot(model.scene.clone()),
            Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI)).with_scale(Vec3::splat(model.scale)),
            ChildOf(mob),
        ))
        .observe(
            move |ready: On<WorldInstanceReady>,
                  mut commands: Commands,
                  models: Res<MobModels>,
                  children: Query<&Children>,
                  mut players: Query<&mut AnimationPlayer>| {
                let model = models.get(kind);
                for child in children.iter_descendants(ready.entity) {
                    if let Ok(mut player) = players.get_mut(child) {
                        player.play(model.idle).repeat();
                        commands.entity(child).insert(AnimationGraphHandle(model.graph.clone()));
                        commands.entity(mob).insert(Animated { player: child, playing: model.idle });
                        break;
                    }
                }
            },
        );
}

fn spawn_mobs(
    mut commands: Commands,
    time: Res<Time>,
    models: Res<MobModels>,
    terrain: Res<Terrain>,
    time_of_day: Res<TimeOfDay>,
    spatial_query: SpatialQuery,
    player: Single<&Transform, (With<Player>, With<Body>)>,
    mobs: Query<&Mob>,
    props: Query<(&Prop, &Transform)>,
    mut animal_timer: Local<f32>,
    mut monster_timer: Local<f32>,
) {
    let dt = time.delta_secs();
    let night = time_of_day.is_night();
    let (timer, period, max) = if night {
        (&mut *monster_timer, MONSTER_SPAWN_SECS, MAX_MONSTERS)
    } else {
        (&mut *animal_timer, ANIMAL_SPAWN_SECS, MAX_ANIMALS)
    };
    *timer += dt;
    if *timer < period || mobs.iter().filter(|m| m.kind.hostile() == night).count() >= max {
        return;
    }
    *timer = 0.0;

    let seed = (time.elapsed_secs() * 1000.0) as u32;
    let angle = rand01(seed) * TAU;
    let distance = SPAWN_MIN_DISTANCE + rand01(seed ^ 0x9e37) * (SPAWN_MAX_DISTANCE - SPAWN_MIN_DISTANCE);
    let xz = player.translation + Vec3::new(angle.cos(), 0.0, angle.sin()) * distance;
    // Each biome has its own creatures.
    let pick = |kinds: &[MobKind]| kinds.get((rand01(seed ^ 0x3c6e) * kinds.len() as f32) as usize).copied();
    let kind = match (night, terrain.biome(xz.x, xz.z)) {
        (false, Biome::Plains | Biome::Forest) => pick(&[MobKind::Sheep, MobKind::Cow, MobKind::Pig, MobKind::Chicken]),
        (false, Biome::Savanna) => pick(&[MobKind::Cow, MobKind::Sheep]),
        (false, Biome::Swamp) => pick(&[MobKind::Pig, MobKind::Chicken]),
        (false, Biome::Taiga | Biome::Snowy) => pick(&[MobKind::Sheep]),
        (false, Biome::Desert) => None,
        (true, Biome::Taiga | Biome::Snowy) => pick(&[MobKind::Wolf, MobKind::Wolf, MobKind::Zombie]),
        (true, _) => pick(&[MobKind::Zombie, MobKind::Zombie, MobKind::Spider]),
    };
    let Some(kind) = kind else { return };
    let top = terrain.surface_height(xz.x, xz.z) + 20.0;
    if !terrain.is_ready_around(xz.with_y(top - 20.0)) {
        return;
    }
    // Land on whatever is actually there now, edits included.
    let filter = SpatialQueryFilter::from_mask(Layer::Solid);
    let Some(hit) = spatial_query.cast_ray(xz.with_y(top), Dir3::NEG_Y, 60.0, true, &filter) else { return };
    let ground = xz.with_y(top - hit.distance);
    if hit.normal.y < 0.7 || ground.y < SEA_LEVEL + 0.5 {
        return;
    }
    if !kind.hostile()
        && terrain.block(((ground - Vec3::Y * 0.1) / CELL).floor().as_ivec3()).material != Material::Grass
    {
        return;
    }
    let lit = props.iter().any(|(p, t)| p.0 == PropKind::Torch && t.translation.distance(ground) < TORCH_SAFE_RADIUS);
    if kind.hostile() && lit {
        return;
    }
    spawn_mob(&mut commands, &models, kind, ground + Vec3::Y * 0.05, angle);
}

fn think(
    mut commands: Commands,
    time: Res<Time>,
    time_of_day: Res<TimeOfDay>,
    (mut vitals, creative): (ResMut<Vitals>, Res<Creative>),
    mut mobs: Query<(Entity, &mut Mob, &mut Body, &mut Transform), Without<Player>>,
    mut player: Single<(&Transform, Option<&mut Body>), With<Player>>,
) {
    let dt = time.delta_secs();
    let (player_transform, player_body) = &mut *player;
    let target = player_transform.translation;
    let alive = player_body.is_some() && !creative.0;

    for (entity, mut mob, mut body, mut transform) in &mut mobs {
        let to_player = target - transform.translation;
        if to_player.length() > DESPAWN_DISTANCE {
            commands.entity(entity).despawn();
            continue;
        }
        mob.flash = (mob.flash - dt).max(0.0);
        mob.scared = (mob.scared - dt).max(0.0);
        mob.timer -= dt;
        mob.since_attack += dt;
        if !mob.alive() {
            body.wish = Vec3::ZERO;
            body.jump = false;
            continue;
        }
        let seed = (entity.to_bits() as u32).wrapping_mul(2_654_435_761) ^ (time.elapsed_secs() * 10.0) as u32;

        let species = mob.kind.species();
        let mut wish = mob.heading;
        match mob.kind.hostile() {
            false if mob.scared > 0.0 => {
                wish = (-to_player).with_y(0.0).normalize_or_zero() * species.run;
            }
            true if alive && to_player.length() < SIGHT => {
                let flat = to_player.with_y(0.0);
                wish = flat.normalize_or_zero() * species.run;
                if flat.length() < species.radius + 1.15 && to_player.y.abs() < 2.0 {
                    wish = Vec3::ZERO;
                    if mob.timer <= 0.0 {
                        mob.timer = ATTACK_SECS;
                        mob.since_attack = 0.0;
                        vitals.damage(species.damage);
                        if let Some(body) = player_body {
                            body.velocity += flat.normalize_or_zero() * 5.0 + Vec3::Y * 3.0;
                            body.ground_normal = None;
                        }
                    }
                }
            }
            _ if mob.timer <= 0.0 => {
                // Wander: pick a direction or stand still for a few seconds.
                mob.timer = 2.0 + rand01(seed) * 5.0;
                let angle = rand01(seed ^ 0x51ed) * TAU;
                mob.heading = if rand01(seed ^ 0x2545) < 0.4 {
                    Vec3::ZERO
                } else {
                    Vec3::new(angle.cos(), 0.0, angle.sin()) * species.walk
                };
                wish = mob.heading;
            }
            _ => {}
        }

        if mob.kind == MobKind::Zombie && !time_of_day.is_night() {
            mob.health -= SUNBURN * dt;
            mob.flash = mob.flash.max(0.05);
        }

        // Hop when walking into something.
        let blocked = wish.length() > 0.5 && body.velocity.with_y(0.0).length() < wish.length() * 0.3;
        body.jump = blocked && body.ground_normal.is_some();
        body.wish = wish;
        if wish.length() > 0.1 {
            let yaw = (-wish.x).atan2(-wish.z);
            let target = Quat::from_rotation_y(yaw);
            transform.rotation = transform.rotation.slerp(target, 1.0 - (-8.0 * dt).exp());
        }
    }
}

/// Starts the death animation (and gives loot), then removes the body once it has played.
fn die(
    mut commands: Commands,
    time: Res<Time>,
    mut inventory: ResMut<Inventory>,
    mut toast: ResMut<Toast>,
    mut mobs: Query<(Entity, &mut Mob, &Transform)>,
    player: Single<&Transform, With<Player>>,
) {
    for (entity, mut mob, transform) in &mut mobs {
        if let Some(left) = &mut mob.dying {
            *left -= time.delta_secs();
            if *left <= 0.0 {
                commands.entity(entity).despawn();
            }
            continue;
        }
        if mob.health > 0.0 {
            continue;
        }
        mob.dying = Some(DEATH_SECS);
        // Sunburnt zombies far away just vanish; only the player's kills give loot.
        let drops = mob.kind.species().drops;
        if drops.is_empty() || transform.translation.distance(player.translation) > 8.0 {
            continue;
        }
        let seed = (entity.to_bits() as u32) ^ (time.elapsed_secs() * 1000.0) as u32;
        let mut lost = 0;
        let mut gained = Vec::new();
        for (i, &(item, least, most)) in drops.iter().enumerate() {
            let count = least + (rand01(seed ^ (i as u32 * 0x77)) * (most - least + 1) as f32) as u32;
            lost += inventory.add(item, count.min(most));
            gained.push(format!("+{} {}", count.min(most), item.name()));
        }
        toast.show(gained.join(", "));
        if lost > 0 {
            toast.show("Инвентарь полон".into());
        }
    }
}

/// Picks the clip that fits what each mob is doing and blends into it.
fn animate(
    models: Res<MobModels>,
    mut mobs: Query<(&Mob, &Body, &mut Animated)>,
    mut players: Query<&mut AnimationPlayer>,
) {
    for (mob, body, mut animated) in &mut mobs {
        let model = models.get(mob.kind);
        let speed = body.velocity.with_y(0.0).length();
        let wanted = if !mob.alive() {
            model.death
        } else if mob.since_attack < 0.6 {
            model.attack
        } else if mob.flash > 0.1 && mob.kind.hostile() {
            model.hurt
        } else if speed > 2.6 {
            model.run
        } else if speed > 0.3 {
            model.walk
        } else {
            model.idle
        };
        if wanted == animated.playing {
            continue;
        }
        let Ok(mut player) = players.get_mut(animated.player) else { continue };
        player.stop_all();
        let active = player.play(wanted);
        if wanted != model.death {
            active.repeat();
        }
        animated.playing = wanted;
    }
}

/// Cheap hash to [0, 1); mob behaviour doesn't need a real random generator.
fn rand01(seed: u32) -> f32 {
    let mut h = seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    h = ((h >> ((h >> 28) + 4)) ^ h).wrapping_mul(277_803_737);
    h = (h >> 22) ^ h;
    (h >> 8) as f32 / 16_777_216.0
}
