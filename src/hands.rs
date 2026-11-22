//! First-person view: the held item, its swing, and a gentle head bob while walking.

use std::collections::HashMap;

use bevy::{
    asset::RenderAssetUsages,
    image::{CompressedImageFormats, ImageSampler, ImageType},
    light::NotShadowCaster,
    prelude::*,
    world_serialization::{WorldAssetRoot, WorldInstanceReady},
};

use crate::{
    AppState, Mode,
    archery::Draw,
    body::Body,
    icons::{ItemIcons, head_color},
    items::{Inventory, Item, ToolKind},
    player::{EYE_HEIGHT, Player, PlayerCamera, SNEAK_DIP},
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
    /// Tool models recoloured for each tier.
    tiers: HashMap<u8, Handle<StandardMaterial>>,
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
    mut images: ResMut<Assets<Image>>,
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
        let tier = item.tool().map_or(0, |t| t.tier);
        let material = cache
            .tiers
            .entry(tier)
            .or_insert_with(|| {
                let texture = images.add(tool_colormap(head_color(item)));
                materials.add(StandardMaterial {
                    base_color_texture: Some(texture),
                    perceptual_roughness: 0.7,
                    ..default()
                })
            })
            .clone();
        commands.entity(entity).remove::<(Mesh3d, MeshMaterial3d<StandardMaterial>)>();
        commands
            .spawn((
                WorldAssetRoot(server.load(GltfAssetLabel::Scene(0).from_asset(path))),
                Transform::from_xyz(0.02, -0.1, 0.0).with_scale(Vec3::splat(0.85)),
                ChildOf(entity),
            ))
            .observe(
                move |ready: On<WorldInstanceReady>,
                      mut commands: Commands,
                      children: Query<&Children>,
                      meshes: Query<(), With<MeshMaterial3d<StandardMaterial>>>| {
                    for child in children.iter_descendants(ready.entity).filter(|c| meshes.contains(*c)) {
                        commands.entity(child).insert(MeshMaterial3d(material.clone()));
                    }
                },
            );
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

/// Kenney's palette with the grey strip that tool heads use tinted to `head`.
fn tool_colormap(head: [f32; 3]) -> Image {
    // Where the strip lies in the 512×512 palette, in pixels.
    const STRIP: (std::ops::Range<usize>, std::ops::Range<usize>) = (160..192, 384..512);
    let mut image = Image::from_buffer(
        include_bytes!("../assets/models/kenney/Textures/colormap.png"),
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::Default,
        RenderAssetUsages::RENDER_WORLD,
    )
    .expect("bundled palette decodes");
    let width = image.width() as usize;
    let data = image.data.as_mut().expect("decoded pixels");
    for y in STRIP.1 {
        for x in STRIP.0.clone() {
            let px = &mut data[(x + y * width) * 4..][..3];
            // Keep the strip's shading (it runs from about 125 to 195 grey), a little darker
            // than the colour itself, which sunlight washes out.
            let shade = px.iter().map(|v| *v as f32).sum::<f32>() / 3.0 / 230.0;
            for (v, c) in px.iter_mut().zip(head) {
                *v = (c * shade * 255.0).min(255.0) as u8;
            }
        }
    }
    image
}

fn swing(
    time: Res<Time>,
    mode: Res<State<Mode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    draw: Res<Draw>,
    held: Single<(&mut Held, &mut Transform)>,
) {
    let (mut held, mut transform) = held.into_inner();
    let bow = held.item == Some(Item::Bow);
    let using = *mode == Mode::Playing
        && (mouse.pressed(MouseButton::Left) || (mouse.just_pressed(MouseButton::Right) && !bow));
    if using && held.swing <= 0.0 {
        held.swing = SWING_SECS;
    }
    held.swing = (held.swing - time.delta_secs()).max(0.0);
    let t = 1.0 - held.swing / SWING_SECS;
    let arc = if held.swing > 0.0 { (t * std::f32::consts::PI).sin() } else { 0.0 };
    let tool = held.item.is_some_and(|i| i.material().is_none());
    // A drawn bow comes up to the middle of the view and back towards the eye.
    let pull = draw.pull();
    transform.translation = REST + Vec3::new(-0.08, 0.05, -0.12) * arc + Vec3::new(-0.2, 0.12, 0.14) * pull;
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
    mut dip: Local<f32>,
    mode: Option<Res<State<Mode>>>,
) {
    let speed = player.velocity.with_y(0.0).length();
    let walking = player.ground_normal.is_some() && !player.flying && speed > 0.5;
    let target = if walking { (speed / 4.5).min(1.5) } else { 0.0 };
    *amount += (target - *amount) * (1.0 - (-8.0 * time.delta_secs()).exp());
    *phase += speed * time.delta_secs() * 1.8;
    let crouch = match () {
        _ if mode.is_some_and(|m| **m == Mode::Sleeping) => EYE_HEIGHT - 0.3,
        _ if player.sneak => SNEAK_DIP,
        _ => 0.0,
    };
    *dip += (crouch - *dip) * (1.0 - (-14.0 * time.delta_secs()).exp());
    camera.translation = Vec3::new(
        phase.sin() * BOB_SWAY * *amount,
        EYE_HEIGHT - *dip + (*phase * 2.0).sin().abs() * BOB_HEIGHT * *amount,
        0.0,
    );
}
