//! Creatures: farm animals graze by day, monsters hunt the player at night; which ones
//! depends on the biome. Wolves fed bones become dogs, and the Warden guards the Shards.

use std::f32::consts::TAU;

use avian3d::prelude::*;
use bevy::ecs::entity::{EntityMapper, MapEntities};
use bevy::{
    prelude::*,
    world_serialization::{WorldAsset, WorldAssetRoot, WorldInstanceReady},
};
use bevy_replicon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    AppState, Creative, Mode,
    archery::Shoot,
    body::{Body, Layer, MoveBodies},
    drops::Drop,
    goals::Goals,
    hud::Toast,
    items::{Item, PropKind},
    net::{self, AvatarOf, Notice},
    player::{self, Player},
    props::{Prop, TORCH_SAFE_RADIUS},
    sky::TimeOfDay,
    survival::{Hurt, Vitals},
    terrain::{Biome, CELL, Material, SEA_LEVEL, SHARDS_ABOVE, SHARDS_Y, Terrain},
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
/// Health per second zombies and skeletons lose in daylight.
const SUNBURN: f32 = 2.0;
/// Archers keep about this far from the player.
const SHOOT_NEAR: f32 = 7.0;
const SHOOT_FAR: f32 = 14.0;
const ARROW_SPEED: f32 = 24.0;
/// Dogs walk up when the player is further than this, and catch up from beyond the second.
const DOG_HEEL: f32 = 3.0;
const DOG_CATCH_UP: f32 = 28.0;
/// Dogs go for monsters this close to the player.
const DOG_GUARD: f32 = 12.0;
/// Chance that a bone wins a wolf over.
const TAME_CHANCE: f32 = 0.34;

pub struct MobsPlugin;

impl Plugin for MobsPlugin {
    fn build(&self, app: &mut App) {
        app.add_client_message::<Summon>(Channel::Ordered)
            .add_mapped_client_message::<Attack>(Channel::Ordered)
            .add_mapped_client_message::<Order>(Channel::Ordered)
            .add_server_message::<Won>(Channel::Ordered)
            .replicate::<Mob>()
            .add_observer(dress_mob)
            .add_systems(Startup, setup)
            .add_systems(Update, celebrate.run_if(in_state(AppState::Game)))
            .add_systems(
                Update,
                (summon, bring_dogs, take_orders).run_if(in_state(AppState::Game)).run_if(crate::net::authority),
            )
            .add_systems(
                Update,
                (spawn_mobs, think)
                    .chain()
                    .before(MoveBodies)
                    .run_if(in_state(AppState::Game))
                    .run_if(|time: Res<Time<Virtual>>| !time.is_paused())
                    .run_if(crate::net::authority),
            )
            .add_systems(Update, die.after(MoveBodies).run_if(in_state(AppState::Game)).run_if(crate::net::authority))
            .add_systems(Update, animate.after(MoveBodies).run_if(in_state(AppState::Game)));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum MobKind {
    Sheep,
    Zombie,
    Cow,
    Pig,
    Chicken,
    Spider,
    Wolf,
    Skeleton,
    Warden,
}

/// What makes each kind of creature different.
#[derive(Clone, Copy)]
pub struct Species {
    pub health: f32,
    radius: f32,
    height: f32,
    walk: f32,
    run: f32,
    /// Damage per bite or arrow; zero for animals, which flee instead.
    pub damage: f32,
    /// Items dropped: item, at least, at most.
    drops: &'static [(Item, u32, u32)],
    model: &'static str,
    /// Clip indices in the glTF file: idle, walk, run, attack, hurt, death.
    clips: [usize; 6],
    scale: f32,
}

/// A creature to put into the world right away (the `summon` cheat, the altar).
#[derive(Message, Serialize, Deserialize)]
pub struct Summon {
    pub kind: MobKind,
    pub at: Vec3,
    pub yaw: f32,
}

fn summon(mut commands: Commands, mut summons: MessageReader<FromClient<Summon>>) {
    for s in summons.read() {
        spawn_mob(&mut commands, Mob::new(s.message.kind), s.message.at, s.message.yaw);
    }
}

/// The Warden has fallen: everyone in the world has won.
#[derive(Message, Serialize, Deserialize)]
pub struct Won;

fn celebrate(mut won: MessageReader<Won>, mut goals: ResMut<Goals>, mut mode: ResMut<NextState<Mode>>) {
    if won.read().last().is_some() {
        goals.win();
        mode.set(Mode::Credits);
    }
}

/// A player hitting a creature, sent to wherever creatures live.
#[derive(Message, Serialize, Deserialize, Clone, Copy)]
pub struct Attack {
    pub mob: Entity,
    pub damage: f32,
    pub from: Vec3,
}

impl MapEntities for Attack {
    fn map_entities<M: EntityMapper>(&mut self, mapper: &mut M) {
        self.mob = mapper.get_mapped(self.mob);
    }
}

/// What a player tells a creature: a bone to a wolf, or "sit" to a dog.
#[derive(Message, Serialize, Deserialize, Clone, Copy)]
pub struct Order {
    pub mob: Entity,
    pub order: OrderKind,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub enum OrderKind {
    /// Fed a bone; the roll decides if it takes.
    Tame(f32),
    Sit,
}

impl MapEntities for Order {
    fn map_entities<M: EntityMapper>(&mut self, mapper: &mut M) {
        self.mob = mapper.get_mapped(self.mob);
    }
}

fn take_orders(
    mut attacks: MessageReader<FromClient<Attack>>,
    mut orders: MessageReader<FromClient<Order>>,
    mut mobs: Query<(&mut Mob, &mut Body, &Transform)>,
    mut notices: MessageWriter<ToClients<Notice>>,
) {
    for attack in attacks.read() {
        if let Ok((mut mob, mut body, transform)) = mobs.get_mut(attack.message.mob) {
            mob.hit(&mut body, attack.message.damage, attack.message.from, transform.translation);
        }
    }
    for order in orders.read() {
        let Ok((mut mob, ..)) = mobs.get_mut(order.message.mob) else { continue };
        let text = match order.message.order {
            OrderKind::Sit if mob.tame => {
                mob.sitting = !mob.sitting;
                if mob.sitting { "Пёс сидит" } else { "Пёс идёт за вами" }
            }
            OrderKind::Sit => continue,
            OrderKind::Tame(roll) if mob.tame_with_bone(roll) => "Волк приручён! Теперь это ваш пёс",
            OrderKind::Tame(_) => "Волк не доверяет вам. Ещё кость?",
        };
        net::notify(&mut notices, order.client_id, text);
    }
}

impl MobKind {
    pub fn named(name: &str) -> Option<MobKind> {
        MobKind::ALL.into_iter().find(|k| format!("{k:?}").eq_ignore_ascii_case(name))
    }

    pub const ALL: [MobKind; 9] = [
        MobKind::Sheep,
        MobKind::Zombie,
        MobKind::Cow,
        MobKind::Pig,
        MobKind::Chicken,
        MobKind::Spider,
        MobKind::Wolf,
        MobKind::Skeleton,
        MobKind::Warden,
    ];

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
                drops: &[(Item::Chicken, 1, 1), (Item::Feather, 1, 3)],
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
                drops: &[(Item::Thread, 0, 2)],
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
            MobKind::Skeleton => &Species {
                health: 16.0,
                radius: 0.35,
                height: 1.8,
                walk: 1.2,
                run: 2.8,
                damage: 2.5,
                drops: &[(Item::Bone, 0, 2), (Item::Arrow, 0, 3)],
                model: "models/skeleton.glb",
                clips: [3, 6, 5, 0, 2, 1],
                scale: 0.62,
            },
            MobKind::Warden => &Species {
                health: 220.0,
                radius: 0.8,
                height: 4.0,
                walk: 1.6,
                run: 3.4,
                damage: 5.0,
                drops: &[(Item::Heart, 1, 1), (Item::Bone, 6, 10)],
                model: "models/skeleton.glb",
                clips: [3, 6, 5, 0, 2, 1],
                scale: 1.4,
            },
        }
    }

    pub fn hostile(self) -> bool {
        self.species().damage > 0.0
    }

    /// Shoots arrows rather than biting (the Warden does both).
    fn archer(self) -> bool {
        matches!(self, MobKind::Skeleton | MobKind::Warden)
    }
}

#[derive(Component, Serialize, Deserialize)]
pub struct Mob {
    pub kind: MobKind,
    pub health: f32,
    /// Seconds left of the red flash after being hit.
    pub flash: f32,
    /// Seconds left of running away (animals).
    pub scared: f32,
    /// A wolf won over with bones: follows the player and fights for them.
    pub tame: bool,
    /// A dog told to stay put.
    pub sitting: bool,
    /// Hurt by the player or their dog, so it drops loot when it dies.
    by_player: bool,
    /// Seconds until the next wander decision or attack.
    timer: f32,
    heading: Vec3,
    /// Seconds since this mob last struck.
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
            tame: false,
            sitting: false,
            by_player: false,
            timer: 0.0,
            heading: Vec3::ZERO,
            since_attack: 9.0,
            dying: None,
        }
    }

    pub fn alive(&self) -> bool {
        self.dying.is_none()
    }

    /// Whether it attacks the player.
    pub fn hostile(&self) -> bool {
        self.kind.hostile() && !self.tame
    }

    /// Takes a hit from the player (or their dog, or arrow) at `from`.
    pub fn hit(&mut self, body: &mut Body, damage: f32, from: Vec3, at: Vec3) {
        if !self.alive() {
            return;
        }
        self.health -= damage;
        self.flash = 0.25;
        self.scared = 6.0;
        self.by_player = true;
        // The Warden is too heavy to knock about.
        let knock = if self.kind == MobKind::Warden { 0.15 } else { 1.0 };
        let away = (at - from).with_y(0.0).normalize_or_zero();
        body.velocity = (away * 6.0 + Vec3::Y * 4.5) * knock;
        body.ground_normal = None;
    }

    /// Feeds a wolf a bone; true if it is now a dog.
    pub fn tame_with_bone(&mut self, roll: f32) -> bool {
        if self.kind != MobKind::Wolf || !self.alive() || roll >= TAME_CHANCE {
            return false;
        }
        self.tame = true;
        self.health = self.kind.species().health;
        true
    }
}

/// A dog, as saved with the world.
#[derive(Clone, Serialize, Deserialize)]
pub struct SavedDog {
    pub position: Vec3,
    pub health: f32,
    pub sitting: bool,
}

impl SavedDog {
    pub fn of(mob: &Mob, transform: &Transform) -> Option<Self> {
        (mob.tame && mob.alive()).then_some(Self {
            position: transform.translation,
            health: mob.health,
            sitting: mob.sitting,
        })
    }
}

/// Dogs from a save, waiting for the ground under the player to load.
#[derive(Resource)]
pub struct SavedDogs(pub Vec<SavedDog>);

fn bring_dogs(
    mut commands: Commands,
    dogs: Option<Res<SavedDogs>>,
    player: Query<&Transform, (With<Player>, With<Body>)>,
) {
    let (Some(dogs), Ok(player)) = (dogs, player.single()) else { return };
    for (i, dog) in dogs.0.iter().enumerate() {
        // A dog left far away (or in another world) comes along anyway.
        let near = dog.position.distance(player.translation) < DOG_CATCH_UP;
        let at = if near { dog.position } else { player.translation + Vec3::new(i as f32 * 0.6 - 1.0, 0.3, 1.0) };
        let mob = Mob { tame: true, sitting: dog.sitting && near, health: dog.health, ..Mob::new(MobKind::Wolf) };
        spawn_mob(&mut commands, mob, at, 0.0);
    }
    commands.remove_resource::<SavedDogs>();
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
    /// Where it was last frame, and how fast it has been moving: guests see only positions.
    last: Vec3,
    speed: f32,
}

/// A creature that thinks: only where the world is owned. Its looks come from [`dress_mob`].
fn spawn_mob(commands: &mut Commands, mob: Mob, at: Vec3, yaw: f32) {
    let body = Body::new(mob.kind.species().radius, mob.kind.species().height);
    commands.spawn((
        DespawnOnExit(AppState::Game),
        Replicated,
        mob,
        body,
        Transform::from_translation(at).with_rotation(Quat::from_rotation_y(yaw)),
    ));
}

/// Model and collider for a creature, on the host and on every guest.
fn dress_mob(add: On<Add<Mob>>, mut commands: Commands, mobs: Query<&Mob>, models: Res<MobModels>) {
    let Ok(mob) = mobs.get(add.entity) else { return };
    let kind = mob.kind;
    let shape = Body::new(kind.species().radius, kind.species().height);
    let collider = Collider::compound(vec![(shape.center_offset(), Quat::IDENTITY, shape.collider())]);
    let model = models.get(kind);
    let mob = add.entity;
    commands.entity(mob).insert((RigidBody::Kinematic, collider, Visibility::default()));
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
                        commands.entity(mob).insert(Animated {
                            player: child,
                            playing: model.idle,
                            last: Vec3::ZERO,
                            speed: 0.0,
                        });
                        break;
                    }
                }
            },
        );
}

fn spawn_mobs(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    time_of_day: Res<TimeOfDay>,
    spatial_query: SpatialQuery,
    player: Single<&Transform, (With<Player>, With<Body>)>,
    guests: Query<&Transform, With<AvatarOf>>,
    mobs: Query<&Mob>,
    props: Query<(&Prop, &Transform)>,
    mut animal_timer: Local<f32>,
    mut monster_timer: Local<f32>,
) {
    let dt = time.delta_secs();
    let seed = (time.elapsed_secs() * 1000.0) as u32;
    // Creatures come to life around one of the players, a different one each time.
    let around: Vec<Vec3> = guests.iter().map(|t| t.translation).chain([player.translation]).collect();
    let center = around[(rand01(seed ^ 0x7f4a) * around.len() as f32) as usize % around.len()];
    // Only skeletons roam the Shards, by day and night alike.
    let shards = player::in_shards(center);
    let night = time_of_day.is_night() || shards;
    let (timer, period, max) = if night {
        (&mut *monster_timer, MONSTER_SPAWN_SECS, MAX_MONSTERS)
    } else {
        (&mut *animal_timer, ANIMAL_SPAWN_SECS, MAX_ANIMALS)
    };
    *timer += dt;
    if *timer < period || mobs.iter().filter(|m| !m.tame && m.hostile() == night).count() >= max {
        return;
    }
    *timer = 0.0;

    let angle = rand01(seed) * TAU;
    let distance = SPAWN_MIN_DISTANCE + rand01(seed ^ 0x9e37) * (SPAWN_MAX_DISTANCE - SPAWN_MIN_DISTANCE);
    let xz = center + Vec3::new(angle.cos(), 0.0, angle.sin()) * distance;
    // Each biome has its own creatures.
    let pick = |kinds: &[MobKind]| kinds.get((rand01(seed ^ 0x3c6e) * kinds.len() as f32) as usize).copied();
    let kind = match (night, terrain.biome(xz.x, xz.z)) {
        _ if shards => Some(MobKind::Skeleton),
        (false, Biome::Plains | Biome::Forest) => pick(&[MobKind::Sheep, MobKind::Cow, MobKind::Pig, MobKind::Chicken]),
        (false, Biome::Savanna) => pick(&[MobKind::Cow, MobKind::Sheep]),
        (false, Biome::Swamp) => pick(&[MobKind::Pig, MobKind::Chicken]),
        (false, Biome::Taiga | Biome::Snowy) => pick(&[MobKind::Sheep]),
        (false, Biome::Desert) => None,
        (true, Biome::Taiga | Biome::Snowy) => {
            pick(&[MobKind::Wolf, MobKind::Wolf, MobKind::Zombie, MobKind::Skeleton])
        }
        (true, _) => pick(&[MobKind::Zombie, MobKind::Zombie, MobKind::Spider, MobKind::Skeleton]),
    };
    let Some(kind) = kind else { return };
    let top = if shards { SHARDS_Y + SHARDS_ABOVE } else { terrain.surface_height(xz.x, xz.z) + 20.0 };
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
    spawn_mob(&mut commands, Mob::new(kind), ground + Vec3::Y * 0.05, angle);
}

/// Picks a direction to stroll in, or stands still, for a few seconds.
fn wander(mob: &mut Mob, seed: u32) {
    mob.timer = 2.0 + rand01(seed) * 5.0;
    let angle = rand01(seed ^ 0x51ed) * TAU;
    mob.heading = if rand01(seed ^ 0x2545) < 0.4 {
        Vec3::ZERO
    } else {
        Vec3::new(angle.cos(), 0.0, angle.sin()) * mob.kind.species().walk
    };
}

fn think(
    mut commands: Commands,
    time: Res<Time>,
    time_of_day: Res<TimeOfDay>,
    terrain: Res<Terrain>,
    (mut vitals, creative): (ResMut<Vitals>, Res<Creative>),
    mut mobs: Query<(Entity, &mut Mob, &mut Body, &mut Transform), Without<Player>>,
    mut player: Single<(&Transform, Option<&mut Body>), With<Player>>,
    guests: Query<(&Transform, &AvatarOf), (Without<Player>, Without<Mob>)>,
    (mut shots, mut hurts): (MessageWriter<Shoot>, MessageWriter<ToClients<Hurt>>),
) {
    let dt = time.delta_secs();
    let (player_transform, player_body) = &mut *player;
    let target = player_transform.translation;
    let ready = player_body.is_some();
    // Everyone a monster may go for: guests are `Some` of how to reach them.
    let mut prey: Vec<(Vec3, Option<ClientId>)> = guests.iter().map(|(t, a)| (t.translation, Some(a.0))).collect();
    if ready {
        prey.push((target, None));
    }
    if creative.0 {
        prey.clear();
    }
    let everyone: Vec<Vec3> = guests.iter().map(|(t, _)| t.translation).chain([target]).collect();
    let enemies: Vec<(Entity, Vec3)> = mobs
        .iter()
        .filter(|(_, m, ..)| m.alive() && m.hostile() && m.kind != MobKind::Warden)
        .map(|(e, _, _, t)| (e, t.translation))
        .collect();
    let mut bites = Vec::new();

    for (entity, mut mob, mut body, mut transform) in &mut mobs {
        let here = transform.translation;
        let nearest = prey.iter().min_by(|a, b| a.0.distance(here).total_cmp(&b.0.distance(here)));
        let (to_player, who) = nearest.map_or((target - here, None), |&(at, who)| (at - here, Some(who)));
        let alive = who.is_some();
        let far = everyone.iter().all(|p| p.distance(here) > DESPAWN_DISTANCE);
        if far && !mob.tame && mob.kind != MobKind::Warden {
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
        let flat = to_player.with_y(0.0);
        let mut wish = mob.heading;
        if mob.tame {
            wish = Vec3::ZERO;
            mob.health = (mob.health + 0.5 * dt).min(species.health);
            let prey = enemies
                .iter()
                .filter(|(_, at)| at.distance(target) < DOG_GUARD)
                .min_by(|a, b| a.1.distance(here).total_cmp(&b.1.distance(here)));
            if mob.sitting {
                // Stays put until told otherwise.
            } else if (target - here).length() > DOG_CATCH_UP && ready {
                transform.translation = target + Vec3::new(1.0, 0.3, 1.0);
                body.velocity = Vec3::ZERO;
            } else if let Some(&(prey, at)) = prey {
                let to_prey = (at - here).with_y(0.0);
                wish = to_prey.normalize_or_zero() * species.run;
                if to_prey.length() < species.radius + 1.0 {
                    wish = Vec3::ZERO;
                    if mob.timer <= 0.0 {
                        mob.timer = ATTACK_SECS;
                        mob.since_attack = 0.0;
                        bites.push((prey, species.damage, here));
                    }
                }
            } else if (target - here).with_y(0.0).length() > DOG_HEEL {
                let heel = (target - here).with_y(0.0);
                let speed = if heel.length() > 8.0 { species.run } else { species.walk * 1.5 };
                wish = heel.normalize_or_zero() * speed;
            }
        } else if !mob.hostile() {
            if mob.scared > 0.0 {
                wish = -flat.normalize_or_zero() * species.run;
            } else if mob.timer <= 0.0 {
                wander(&mut mob, seed);
                wish = mob.heading;
            }
        } else if alive && to_player.length() < SIGHT {
            let toward = flat.normalize_or_zero();
            let distance = flat.length();
            if mob.kind.archer() && (mob.kind == MobKind::Skeleton || distance > 5.0) {
                wish = if distance > SHOOT_FAR {
                    toward * species.run
                } else if distance < SHOOT_NEAR && mob.kind == MobKind::Skeleton {
                    -toward * species.walk
                } else {
                    Vec3::ZERO
                };
                let eye = here + Vec3::Y * species.height * 0.85;
                let chest = here + to_player + Vec3::Y * player::HEIGHT * 0.6;
                let line = chest - eye;
                if mob.timer <= 0.0 && terrain.raycast(eye, line, line.length()).is_none() {
                    let warden = mob.kind == MobKind::Warden;
                    mob.timer = if warden { 1.6 } else { 2.2 };
                    mob.since_attack = 0.0;
                    // Aim a little high: the arrow drops on its way.
                    let gravity = if here.y > SHARDS_Y - 500.0 { 9.0 } else { 20.0 };
                    let flight = line.length() / ARROW_SPEED;
                    let velocity = line / flight + Vec3::Y * 0.5 * gravity * flight;
                    let spread: &[f32] = if warden { &[-0.12, 0.0, 0.12] } else { &[0.0] };
                    for turn in spread {
                        let velocity = Quat::from_rotation_y(*turn) * velocity;
                        shots.write(Shoot { from: eye, velocity, damage: species.damage * 0.7, by_player: false });
                    }
                }
            } else {
                wish = toward * species.run;
                if distance < species.radius + 1.15 && to_player.y.abs() < 2.0 + species.height * 0.5 {
                    wish = Vec3::ZERO;
                    if mob.timer <= 0.0 {
                        mob.timer = ATTACK_SECS;
                        mob.since_attack = 0.0;
                        let push = toward * 5.0 + Vec3::Y * 3.0;
                        match who.flatten() {
                            Some(client) => {
                                hurts.write(ToClients {
                                    targets: SendTargets::Single(client),
                                    message: Hurt { amount: species.damage, push },
                                });
                            }
                            None => {
                                vitals.damage(species.damage);
                                if let Some(body) = player_body {
                                    body.velocity += push;
                                    body.ground_normal = None;
                                }
                            }
                        }
                    }
                }
            }
        } else if mob.timer <= 0.0 {
            wander(&mut mob, seed);
            wish = mob.heading;
        }

        let undead = matches!(mob.kind, MobKind::Zombie | MobKind::Skeleton);
        if undead && !time_of_day.is_night() && !player::in_shards(here) {
            mob.health -= SUNBURN * dt;
            mob.flash = mob.flash.max(0.05);
        }

        // Over the void, nobody walks off an island's edge.
        if here.y > SHARDS_Y - 500.0 && wish.length() > 0.1 {
            let ahead = here + wish.normalize() * (species.radius + 0.5) + Vec3::Y * 0.5;
            if terrain.raycast(ahead, Vec3::NEG_Y, 3.0).is_none() {
                wish = Vec3::ZERO;
            }
        }

        // Hop when walking into something.
        let blocked = wish.length() > 0.5 && body.velocity.with_y(0.0).length() < wish.length() * 0.3;
        body.jump = blocked && body.ground_normal.is_some();
        body.wish = wish;
        // Monsters keep their eyes on the player, archers above all.
        let facing = if mob.hostile() && alive && to_player.length() < SIGHT { flat } else { wish };
        if facing.length() > 0.1 {
            let yaw = (-facing.x).atan2(-facing.z);
            let target = Quat::from_rotation_y(yaw);
            transform.rotation = transform.rotation.slerp(target, 1.0 - (-8.0 * dt).exp());
        }
    }

    for (prey, damage, from) in bites {
        if let Ok((_, mut mob, mut body, transform)) = mobs.get_mut(prey) {
            mob.hit(&mut body, damage, from, transform.translation);
        }
    }
}

/// Starts the death animation (and gives loot), then removes the body once it has played.
fn die(
    mut commands: Commands,
    time: Res<Time>,
    mut toast: ResMut<Toast>,
    mut won: MessageWriter<ToClients<Won>>,
    mut drops: MessageWriter<Drop>,
    mut mobs: Query<(Entity, &mut Mob, &Transform)>,
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
        if mob.tame {
            toast.show("Ваш пёс погиб".into());
        }
        if mob.kind == MobKind::Warden {
            won.write(ToClients { targets: SendTargets::All, message: Won });
        }
        // Sunburnt monsters just crumble; only the player's kills give loot.
        if !mob.by_player {
            continue;
        }
        let seed = (entity.to_bits() as u32) ^ (time.elapsed_secs() * 1000.0) as u32;
        let at = transform.translation + Vec3::Y * 0.5;
        for (i, &(item, least, most)) in mob.kind.species().drops.iter().enumerate() {
            let count = (least + (rand01(seed ^ (i as u32 * 0x77)) * (most - least + 1) as f32) as u32).min(most);
            if count > 0 {
                drops.write(Drop::pop(item, count, at, seed ^ i as u32));
            }
        }
    }
}

/// Picks the clip that fits what each mob is doing and blends into it.
fn animate(
    time: Res<Time>,
    models: Res<MobModels>,
    mut mobs: Query<(&Mob, &Transform, &mut Animated)>,
    mut players: Query<&mut AnimationPlayer>,
) {
    let dt = time.delta_secs().max(1e-3);
    for (mob, transform, mut animated) in &mut mobs {
        let model = models.get(mob.kind);
        let moved = (transform.translation - animated.last).with_y(0.0).length() / dt;
        animated.last = transform.translation;
        animated.speed += (moved - animated.speed) * (1.0 - (-8.0 * dt).exp());
        let speed = animated.speed;
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
pub fn rand01(seed: u32) -> f32 {
    let mut h = seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    h = ((h >> ((h >> 28) + 4)) ^ h).wrapping_mul(277_803_737);
    h = (h >> 22) ^ h;
    (h >> 8) as f32 / 16_777_216.0
}
