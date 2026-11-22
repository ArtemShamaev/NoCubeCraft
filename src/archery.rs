//! Bows and arrows: the player's bow, skeletons' shots, and arrows stuck where they landed.

use avian3d::prelude::*;
use bevy::{light::NotShadowCaster, prelude::*};
use bevy_replicon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    AppState, Creative, Mode,
    body::{Body, MoveBodies},
    hud::Toast,
    items::{Inventory, Item},
    mobs::Mob,
    net::AvatarOf,
    player::{self, Player, PlayerCamera},
    sound::{PlaySound, Sfx},
    survival::{Hurt, Vitals},
    terrain::{SHARDS_Y, Terrain},
};

/// Seconds to draw the bow fully.
const DRAW_SECS: f32 = 0.9;
/// Shorter pulls than this don't loose an arrow.
const MIN_DRAW_SECS: f32 = 0.15;
const GRAVITY: f32 = 20.0;
/// Seconds an arrow stays where it stuck.
const STUCK_SECS: f32 = 30.0;
/// The player picks up their own arrows this close.
const PICK_UP_RADIUS: f32 = 1.3;

pub struct ArcheryPlugin;

impl Plugin for ArcheryPlugin {
    fn build(&self, app: &mut App) {
        app.add_client_message::<Shoot>(Channel::Ordered)
            .replicate::<ArrowLook>()
            .add_observer(show_arrow)
            .init_resource::<Draw>()
            .add_systems(Startup, setup)
            .add_systems(Update, draw_bow.run_if(in_state(Mode::Playing)))
            .add_systems(OnExit(Mode::Playing), |mut draw: ResMut<Draw>| draw.0 = None)
            .add_systems(
                Update,
                (loose, fly).chain().after(MoveBodies).run_if(in_state(AppState::Game)).run_if(crate::net::authority),
            );
    }
}

/// An arrow leaving a bow.
#[derive(Message, Serialize, Deserialize)]
pub struct Shoot {
    pub from: Vec3,
    pub velocity: Vec3,
    pub damage: f32,
    /// The player's arrows hit creatures; everyone else's hit the player.
    pub by_player: bool,
}

/// Seconds the bow in hand has been drawn, while it is.
#[derive(Resource, Default)]
pub struct Draw(pub Option<f32>);

impl Draw {
    /// How far the string is pulled, 0..1.
    pub fn pull(&self) -> f32 {
        self.0.map_or(0.0, |t| (t / DRAW_SECS).min(1.0))
    }
}

#[derive(Component)]
struct Arrow {
    velocity: Vec3,
    damage: f32,
    by_player: bool,
    /// Seconds since it stuck in something.
    stuck: Option<f32>,
}

#[derive(Resource)]
struct ArrowAssets {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(ArrowAssets {
        mesh: meshes.add(Cuboid::new(0.03, 0.03, 0.6)),
        material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.55, 0.4, 0.25),
            perceptual_roughness: 0.8,
            ..default()
        }),
    });
}

/// Hold the right button to draw the bow, let go to shoot.
fn draw_bow(
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Single<&bevy::window::CursorOptions>,
    creative: Res<Creative>,
    mut inventory: ResMut<Inventory>,
    mut toast: ResMut<Toast>,
    mut draw: ResMut<Draw>,
    camera: Single<&GlobalTransform, With<PlayerCamera>>,
    alive: Query<(), (With<Player>, With<Body>)>,
    mut shots: MessageWriter<Shoot>,
) {
    let holding = inventory.selected().is_some_and(|s| s.item == Item::Bow);
    if !holding || alive.is_empty() {
        draw.0 = None;
        return;
    }
    if mouse.pressed(MouseButton::Right) && player::cursor_grabbed(&cursor) {
        if mouse.just_pressed(MouseButton::Right) && !creative.0 && inventory.count(Item::Arrow) == 0 {
            toast.show("Нет стрел".into());
        }
        draw.0 = Some(draw.0.unwrap_or(0.0) + time.delta_secs());
        return;
    }
    let pull = draw.pull();
    if draw.0.take().is_none_or(|t| t < MIN_DRAW_SECS) || !(creative.0 || inventory.remove(Item::Arrow, 1)) {
        return;
    }
    let forward = *camera.forward();
    shots.write(Shoot {
        from: camera.translation() + forward * 0.4,
        velocity: forward * (12.0 + 30.0 * pull),
        damage: 1.5 + 5.5 * pull,
        by_player: true,
    });
}

fn loose(mut commands: Commands, mut shots: MessageReader<FromClient<Shoot>>, mut sounds: MessageWriter<PlaySound>) {
    for shot in shots.read() {
        let shot = &shot.message;
        commands.spawn((
            DespawnOnExit(AppState::Game),
            Replicated,
            ArrowLook,
            Arrow { velocity: shot.velocity, damage: shot.damage, by_player: shot.by_player, stuck: None },
            Transform::from_translation(shot.from).looking_to(shot.velocity, Vec3::Y),
        ));
        sounds.write(PlaySound::at(Sfx::Bow, shot.from));
    }
}

/// Marks an arrow for everyone to see; its flight is the host's business.
#[derive(Component, Serialize, Deserialize)]
struct ArrowLook;

fn show_arrow(add: On<Add<ArrowLook>>, mut commands: Commands, assets: Res<ArrowAssets>) {
    commands.entity(add.entity).insert((
        Mesh3d(assets.mesh.clone()),
        MeshMaterial3d(assets.material.clone()),
        NotShadowCaster,
    ));
}

/// Moves arrows along their arcs; they stick in blocks and wound whatever they hit.
fn fly(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    spatial_query: SpatialQuery,
    (creative, mut vitals, mut inventory): (Res<Creative>, ResMut<Vitals>, ResMut<Inventory>),
    mut arrows: Query<(Entity, &mut Arrow, &mut Transform)>,
    mut mobs: Query<(&mut Mob, &mut Body, &Transform), (Without<Player>, Without<Arrow>)>,
    player: Single<(&Transform, Option<&mut Body>), (With<Player>, Without<Mob>, Without<Arrow>)>,
    guests: Query<(&Transform, &AvatarOf), (Without<Player>, Without<Mob>, Without<Arrow>)>,
    (mut sounds, mut hurts): (MessageWriter<PlaySound>, MessageWriter<ToClients<Hurt>>),
) {
    let dt = time.delta_secs();
    let (player_transform, mut player_body) = player.into_inner();
    let chest = player_transform.translation + Vec3::Y * player::HEIGHT * 0.5;
    for (entity, mut arrow, mut transform) in &mut arrows {
        if let Some(age) = &mut arrow.stuck {
            *age += dt;
            let near = transform.translation.distance(chest) < PICK_UP_RADIUS;
            if *age > STUCK_SECS || (near && arrow.by_player && inventory.add(Item::Arrow, 1) == 0) {
                commands.entity(entity).despawn();
            }
            continue;
        }
        let gravity = if transform.translation.y > SHARDS_Y - 500.0 { GRAVITY * 0.45 } else { GRAVITY };
        arrow.velocity.y -= gravity * dt;
        let from = transform.translation;
        let step = arrow.velocity * dt;
        let (Ok(dir), length) = (Dir3::new(step), step.length()) else { continue };
        let wall = terrain.raycast(from, *dir, length).map_or(f32::INFINITY, |h| h.distance);

        if arrow.by_player {
            let creature =
                spatial_query.cast_ray_predicate(from, dir, length, true, &SpatialQueryFilter::default(), &|e| {
                    mobs.get(e).is_ok_and(|(m, ..)| m.alive() && !m.tame)
                });
            if let Some(hit) = creature.filter(|h| h.distance < wall)
                && let Ok((mut mob, mut body, mob_transform)) = mobs.get_mut(hit.entity)
            {
                mob.hit(&mut body, arrow.damage, from - *dir, mob_transform.translation);
                sounds.write(PlaySound::at(Sfx::Hit, from));
                commands.entity(entity).despawn();
                continue;
            }
        } else {
            // Whether this step passes through a player standing with their middle at `middle`.
            let hits = |middle: Vec3| {
                let t = ((middle - from).dot(*dir)).clamp(0.0, length.min(wall));
                let off = from + *dir * t - middle;
                off.with_y(0.0).length() < player::RADIUS + 0.1 && off.y.abs() < player::HEIGHT * 0.5
            };
            let push = dir.with_y(0.0).normalize_or_zero() * 3.0;
            if let Some((_, guest)) = guests.iter().find(|(t, _)| hits(t.translation + Vec3::Y * player::HEIGHT * 0.5))
            {
                hurts.write(ToClients {
                    targets: SendTargets::Single(guest.0),
                    message: Hurt { amount: arrow.damage, push },
                });
                commands.entity(entity).despawn();
                continue;
            }
            if hits(chest) {
                if !creative.0 {
                    vitals.damage(arrow.damage);
                }
                if let Some(body) = &mut player_body {
                    body.velocity += push;
                }
                commands.entity(entity).despawn();
                continue;
            }
        }

        if wall.is_finite() {
            transform.translation = from + *dir * (wall - 0.05);
            arrow.stuck = Some(0.0);
            sounds.write(PlaySound::at(Sfx::Place, transform.translation));
        } else {
            transform.translation += step;
            transform.look_to(*dir, Vec3::Y);
        }
    }
}
