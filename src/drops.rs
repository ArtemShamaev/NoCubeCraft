//! Items lying in the world: what digging, breaking and killing leave behind, and what the
//! player throws away. They spin where they lie and fly to a player who comes close.

use std::collections::HashMap;

use bevy::{light::NotShadowCaster, prelude::*};
use bevy_replicon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    AppState, Mode,
    body::Body,
    icons::ItemIcons,
    items::{Inventory, Item, Stack},
    net::AvatarOf,
    player::{self, Player, PlayerCamera},
    sound::{PlaySound, Sfx},
    terrain::{SHARDS_Y, Terrain},
};

const GRAVITY: f32 = 16.0;
/// Items drift towards a player this close, and are taken this close.
const ATTRACT_RADIUS: f32 = 2.2;
const TAKE_RADIUS: f32 = 0.6;
/// Thrown items can't be picked up again straight away.
const THROW_DELAY: f32 = 1.5;
/// Seconds before an item left lying vanishes.
const LIFETIME: f32 = 300.0;
const SIZE: f32 = 0.22;

pub struct DropsPlugin;

impl Plugin for DropsPlugin {
    fn build(&self, app: &mut App) {
        app.add_client_message::<Drop>(Channel::Ordered)
            .add_server_message::<Give>(Channel::Ordered)
            .replicate::<DropLook>()
            .init_resource::<Looks>()
            .add_observer(show)
            .add_systems(Update, throw.run_if(in_state(Mode::Playing)))
            .add_systems(
                Update,
                (spawn, fall, pick_up).chain().run_if(in_state(AppState::Game)).run_if(crate::net::authority),
            )
            .add_systems(Update, (spin, take_gifts).run_if(in_state(AppState::Game)));
    }
}

/// Puts items into the world at `at`, moving at `velocity`.
#[derive(Message, Clone, Copy, Serialize, Deserialize)]
pub struct Drop {
    pub stack: Stack,
    pub at: Vec3,
    pub velocity: Vec3,
    /// Seconds before it can be picked up.
    pub delay: f32,
}

impl Drop {
    /// Items popping out of something at `at`, with a little random hop.
    pub fn pop(item: Item, count: u32, at: Vec3, seed: u32) -> Self {
        let r = |shift: u32| ((seed.wrapping_mul(2_654_435_761) >> shift) & 0xff) as f32 / 255.0 - 0.5;
        Self { stack: Stack::new(item, count), at, velocity: Vec3::new(r(0) * 2.0, 3.0, r(8) * 2.0), delay: 0.3 }
    }
}

/// An item lying around, as saved with the world.
#[derive(Clone, Serialize, Deserialize)]
pub struct SavedDrop {
    pub stack: Stack,
    pub at: Vec3,
    pub age: f32,
}

#[derive(Component)]
pub struct Dropped {
    pub stack: Stack,
    velocity: Vec3,
    resting: bool,
    pub age: f32,
    delay: f32,
}

impl Dropped {
    pub fn saved(&self, transform: &Transform) -> SavedDrop {
        SavedDrop { stack: self.stack, at: transform.translation, age: self.age }
    }
}

/// What a dropped item is, for everyone to see; [`Dropped`] lives only with the world's owner.
#[derive(Component, Serialize, Deserialize)]
struct DropLook(Item);

/// Items picked up by a guest, sent to them.
#[derive(Message, Serialize, Deserialize)]
struct Give(Stack);

/// The spinning picture of a dropped item, a child of it.
#[derive(Component)]
struct Spinner;

#[derive(Resource, Default)]
pub struct Looks {
    cube: Option<Handle<Mesh>>,
    quad: Option<Handle<Mesh>>,
    materials: HashMap<Item, Handle<StandardMaterial>>,
}

fn spawn(mut commands: Commands, mut drops: MessageReader<FromClient<Drop>>) {
    for drop in drops.read() {
        let drop = &drop.message;
        commands.spawn((
            DespawnOnExit(AppState::Game),
            Replicated,
            DropLook(drop.stack.item),
            Dropped { stack: drop.stack, velocity: drop.velocity, resting: false, age: 0.0, delay: drop.delay },
            Transform::from_translation(drop.at),
        ));
    }
}

/// The picture of a dropped item, on the host and on guests alike.
fn show(add: On<Add<DropLook>>, mut commands: Commands, drops: Query<&DropLook>, mut looks: ItemLooks) {
    let Ok(&DropLook(item)) = drops.get(add.entity) else { return };
    let (mesh, material) = looks.get(item);
    commands.entity(add.entity).insert(Visibility::default()).with_child((
        Spinner,
        Mesh3d(mesh),
        MeshMaterial3d(material),
        NotShadowCaster,
        Transform::default(),
    ));
}

/// Small pictures of items in the world: a cube of a block's colour, or the item's icon.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ItemLooks<'w> {
    icons: Res<'w, ItemIcons>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    looks: ResMut<'w, Looks>,
}

impl ItemLooks<'_> {
    /// About [`SIZE`] across.
    pub fn get(&mut self, item: Item) -> (Handle<Mesh>, Handle<StandardMaterial>) {
        let block = item.material();
        let (meshes, looks) = (&mut self.meshes, &mut *self.looks);
        let mesh = if block.is_some() {
            looks.cube.get_or_insert_with(|| meshes.add(Cuboid::from_length(SIZE))).clone()
        } else {
            looks.quad.get_or_insert_with(|| meshes.add(Rectangle::from_length(SIZE * 1.6))).clone()
        };
        let material = looks
            .materials
            .entry(item)
            .or_insert_with(|| {
                self.materials.add(match block {
                    Some(m) => {
                        let [r, g, b] = crate::terrain::albedo(m);
                        StandardMaterial {
                            base_color: Color::linear_rgb(r, g, b),
                            perceptual_roughness: 0.9,
                            ..default()
                        }
                    }
                    None => StandardMaterial {
                        base_color_texture: Some(self.icons.get(item)),
                        alpha_mode: AlphaMode::Mask(0.5),
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    },
                })
            })
            .clone();
        (mesh, material)
    }
}

/// Spawns items saved with the world.
pub fn restore(drops: &[SavedDrop], writer: &mut MessageWriter<Drop>) {
    for d in drops {
        writer.write(Drop { stack: d.stack, at: d.at, velocity: Vec3::ZERO, delay: 0.0 });
    }
}

/// Falls until it lands on a block, and again if the block goes.
fn fall(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut drops: Query<(Entity, &mut Dropped, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (entity, mut drop, mut transform) in &mut drops {
        drop.age += dt;
        drop.delay -= dt;
        if drop.age > LIFETIME {
            commands.entity(entity).despawn();
            continue;
        }
        let at = transform.translation;
        if drop.resting {
            if terrain.raycast(at, Vec3::NEG_Y, SIZE * 0.6).is_some() {
                continue;
            }
            drop.resting = false;
        }
        let gravity = if at.y > SHARDS_Y - 500.0 { GRAVITY * 0.45 } else { GRAVITY };
        drop.velocity.y = (drop.velocity.y - gravity * dt).max(-30.0);
        let step = drop.velocity * dt;
        let Ok(dir) = Dir3::new(step) else { continue };
        match terrain.raycast(at, *dir, step.length() + SIZE * 0.5) {
            Some(hit) => {
                transform.translation = at + *dir * (hit.distance - SIZE * 0.5).max(0.0);
                if hit.normal.y > 0 {
                    drop.resting = true;
                    drop.velocity = Vec3::ZERO;
                } else {
                    // Off a wall or ceiling: lose the speed into it and keep falling.
                    let n = hit.normal.as_vec3();
                    let into = n * drop.velocity.dot(n);
                    drop.velocity -= into;
                }
            }
            None => transform.translation += step,
        }
    }
}

/// Drifts nearby items to the player and puts them in the inventory.
fn pick_up(
    mut commands: Commands,
    time: Res<Time>,
    mut inventory: ResMut<Inventory>,
    mut drops: Query<(Entity, &mut Dropped, &mut Transform), (Without<Player>, Without<AvatarOf>)>,
    player: Query<&Transform, (With<Player>, With<Body>)>,
    guests: Query<(&Transform, &AvatarOf), Without<Player>>,
    mut sounds: MessageWriter<PlaySound>,
    mut gifts: MessageWriter<ToClients<Give>>,
) {
    let chest = |t: &Transform| t.translation + Vec3::Y * player::HEIGHT * 0.4;
    let host = player.single().ok().map(|t| (chest(t), None));
    let everyone: Vec<(Vec3, Option<ClientId>)> =
        guests.iter().map(|(t, a)| (chest(t), Some(a.0))).chain(host).collect();
    for (entity, mut drop, mut transform) in &mut drops {
        if drop.delay > 0.0 {
            continue;
        }
        let here = transform.translation;
        let Some(&(chest, who)) = everyone.iter().min_by(|a, b| a.0.distance(here).total_cmp(&b.0.distance(here)))
        else {
            return;
        };
        let to = chest - here;
        // Guests sort out their own full pockets.
        if to.length() > ATTRACT_RADIUS || (who.is_none() && !inventory.has_room(drop.stack.item)) {
            continue;
        }
        if to.length() > TAKE_RADIUS {
            transform.translation += to.normalize() * (8.0 * time.delta_secs()).min(to.length());
            drop.resting = false;
            drop.velocity = Vec3::ZERO;
            continue;
        }
        if let Some(guest) = who {
            gifts.write(ToClients { targets: SendTargets::Single(guest), message: Give(drop.stack) });
            commands.entity(entity).despawn();
            continue;
        }
        let left = inventory.add_stack(drop.stack);
        if left == drop.stack.count {
            continue;
        }
        sounds.write(PlaySound::at(Sfx::Pickup, chest));
        if left == 0 {
            commands.entity(entity).despawn();
        } else {
            drop.stack.count = left;
        }
    }
}

fn spin(
    time: Res<Time>,
    drops: Query<(Entity, &Children), With<DropLook>>,
    mut spinners: Query<&mut Transform, With<Spinner>>,
) {
    for (entity, children) in &drops {
        // Each item turns out of step with its neighbours.
        let t = time.elapsed_secs() + (entity.index_u32() % 97) as f32 * 0.37;
        for child in children {
            if let Ok(mut transform) = spinners.get_mut(*child) {
                transform.translation.y = SIZE * 0.5 + (t * 3.0).sin() * 0.04;
                transform.rotation = Quat::from_rotation_y(t * 2.0);
            }
        }
    }
}

/// Items a guest picked up in the host's world; what doesn't fit is thrown back down.
fn take_gifts(
    mut gifts: MessageReader<Give>,
    mut inventory: ResMut<Inventory>,
    player: Query<&Transform, With<Player>>,
    mut drops: MessageWriter<Drop>,
    mut sounds: MessageWriter<PlaySound>,
) {
    for Give(stack) in gifts.read() {
        let left = inventory.add_stack(*stack);
        sounds.write(PlaySound::ui(Sfx::Pickup));
        if left > 0
            && let Ok(at) = player.single()
        {
            drops.write(Drop {
                stack: Stack { count: left, ..*stack },
                at: at.translation + Vec3::Y,
                velocity: Vec3::Y,
                delay: THROW_DELAY,
            });
        }
    }
}

/// Q throws one of the selected items; Ctrl+Q the whole stack.
fn throw(
    keys: Res<ButtonInput<KeyCode>>,
    mut inventory: ResMut<Inventory>,
    camera: Single<&GlobalTransform, With<PlayerCamera>>,
    alive: Query<(), (With<Player>, With<Body>)>,
    mut drops: MessageWriter<Drop>,
) {
    if !keys.just_pressed(KeyCode::KeyQ) || alive.is_empty() {
        return;
    }
    let Some(stack) = inventory.selected() else { return };
    let count = if keys.pressed(KeyCode::ControlLeft) { stack.count } else { 1 };
    inventory.consume_selected(count);
    let forward = *camera.forward();
    drops.write(Drop {
        stack: Stack { count, ..stack },
        at: camera.translation() + forward * 0.4 - Vec3::Y * 0.3,
        velocity: forward * 6.0 + Vec3::Y * 2.0,
        delay: THROW_DELAY,
    });
}
