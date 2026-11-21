//! First-person view: the held item, its swing, and a gentle head bob while walking.

use std::collections::HashMap;

use bevy::{light::NotShadowCaster, prelude::*, world_serialization::WorldAssetRoot};

use crate::{
    AppState, Mode,
    body::Body,
    icons::ItemIcons,
    items::{Inventory, Item, ToolKind},
    player::{EYE_HEIGHT, Player, PlayerCamera},
};

const SWING_SECS: f32 = 0.28;
/// Where the item rests, relative to the eye.
const REST: Vec3 = Vec3::new(0.36, -0.32, -0.62);
const BOB_HEIGHT: f32 = 0.035;
const BOB_SWAY: f32 = 0.02;

pub struct HandsPlugin;

impl Plugin for HandsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (attach, show_item, swing, bob).chain().run_if(in_state(AppState::Game)));
    }
}

#[derive(Component)]
struct Held {
    item: Option<Item>,
    swing: f32,
}

#[derive(Default)]
struct Cache {
    quad: Option<Handle<Mesh>>,
    cube: Option<Handle<Mesh>>,
    materials: HashMap<Item, Handle<StandardMaterial>>,
}

fn attach(mut commands: Commands, cameras: Query<Entity, Added<PlayerCamera>>) {
    for camera in &cameras {
        commands.spawn((
            Held { item: None, swing: 0.0 },
            NotShadowCaster,
            Transform::from_translation(REST),
            Visibility::Hidden,
            ChildOf(camera),
        ));
    }
}

/// Blocks are held as little cubes, everything else as its icon, like a cut-out.
fn show_item(
    mut commands: Commands,
    inventory: Res<Inventory>,
    icons: Res<ItemIcons>,
    held: Single<(Entity, &mut Held, &mut Visibility)>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cache: Local<Cache>,
) {
    let (entity, mut held, mut visibility) = held.into_inner();
    let item = inventory.selected().map(|s| s.item);
    if item == held.item {
        return;
    }
    held.item = item;
    commands.entity(entity).despawn_children();
    let Some(item) = item else {
        *visibility = Visibility::Hidden;
        return;
    };
    *visibility = Visibility::Inherited;
    // Tools are held as their 3D models (Kenney, CC0).
    if let Some(path) = tool_model(item) {
        commands.entity(entity).remove::<(Mesh3d, MeshMaterial3d<StandardMaterial>)>();
        commands.spawn((
            WorldAssetRoot(server.load(GltfAssetLabel::Scene(0).from_asset(path))),
            Transform::from_xyz(0.02, -0.1, 0.0).with_scale(Vec3::splat(0.85)),
            ChildOf(entity),
        ));
        return;
    }
    let block = item.material();
    let mesh = if block.is_some() {
        cache.cube.get_or_insert_with(|| meshes.add(Cuboid::new(0.11, 0.11, 0.11))).clone()
    } else {
        cache.quad.get_or_insert_with(|| meshes.add(Rectangle::new(0.3, 0.3))).clone()
    };
    let material = cache
        .materials
        .entry(item)
        .or_insert_with(|| {
            materials.add(match block {
                Some(m) => {
                    let [r, g, b] = crate::terrain::albedo(m);
                    StandardMaterial { base_color: Color::linear_rgb(r, g, b), perceptual_roughness: 0.9, ..default() }
                }
                None => StandardMaterial {
                    base_color_texture: Some(icons.get(item)),
                    alpha_mode: AlphaMode::Mask(0.5),
                    double_sided: true,
                    cull_mode: None,
                    perceptual_roughness: 0.8,
                    ..default()
                },
            })
        })
        .clone();
    commands.entity(entity).insert((Mesh3d(mesh), MeshMaterial3d(material)));
}

/// The model for a tool: plain for wood and stone, the upgraded look for iron and diamond.
fn tool_model(item: Item) -> Option<String> {
    let tool = item.tool()?;
    let name = match tool.kind {
        ToolKind::Pickaxe => "tool-pickaxe",
        ToolKind::Axe => "tool-axe",
        ToolKind::Shovel => "tool-shovel",
        ToolKind::Sword => return None,
    };
    let upgraded = if tool.tier >= 2 { "-upgraded" } else { "" };
    Some(format!("models/kenney/{name}{upgraded}.glb"))
}

fn swing(
    time: Res<Time>,
    mode: Res<State<Mode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    held: Single<(&mut Held, &mut Transform)>,
) {
    let (mut held, mut transform) = held.into_inner();
    let using = *mode == Mode::Playing && (mouse.pressed(MouseButton::Left) || mouse.just_pressed(MouseButton::Right));
    if using && held.swing <= 0.0 {
        held.swing = SWING_SECS;
    }
    held.swing = (held.swing - time.delta_secs()).max(0.0);
    let t = 1.0 - held.swing / SWING_SECS;
    let arc = if held.swing > 0.0 { (t * std::f32::consts::PI).sin() } else { 0.0 };
    let tool = held.item.is_some_and(|i| i.material().is_none());
    transform.translation = REST + Vec3::new(-0.08, 0.05, -0.12) * arc;
    transform.rotation = if tool {
        // Tools tilt forward like a chop.
        Quat::from_rotation_z(0.5) * Quat::from_rotation_x(-1.1 * arc) * Quat::from_rotation_y(-0.3)
    } else {
        Quat::from_rotation_y(0.6) * Quat::from_rotation_x(0.35 - 0.8 * arc)
    };
}

fn bob(
    time: Res<Time>,
    player: Single<&Body, With<Player>>,
    mut camera: Single<&mut Transform, With<PlayerCamera>>,
    mut phase: Local<f32>,
    mut amount: Local<f32>,
) {
    let speed = player.velocity.with_y(0.0).length();
    let walking = player.ground_normal.is_some() && !player.flying && speed > 0.5;
    let target = if walking { (speed / 4.5).min(1.5) } else { 0.0 };
    *amount += (target - *amount) * (1.0 - (-8.0 * time.delta_secs()).exp());
    *phase += speed * time.delta_secs() * 1.8;
    camera.translation = Vec3::new(
        phase.sin() * BOB_SWAY * *amount,
        EYE_HEIGHT + (*phase * 2.0).sin().abs() * BOB_HEIGHT * *amount,
        0.0,
    );
}
