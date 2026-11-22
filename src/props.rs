//! Objects placed in the world: furnaces, torches, beds and crops.

use avian3d::prelude::*;
use bevy::{
    asset::RenderAssetUsages,
    ecs::entity::{EntityMapper, MapEntities},
    light::NotShadowCaster,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    world_serialization::{WorldAsset, WorldAssetRoot, WorldInstanceReady},
};
use bevy_replicon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    body::Layer,
    containers::{ChestState, FurnaceState},
    drops::Drop,
    items::{Item, PropKind, Stack},
    sound::{PlaySound, Sfx},
};

/// Seconds for wheat to grow from seed to harvest.
pub const WHEAT_GROW_SECS: f32 = 90.0;
const WHEAT_STAGES: usize = 4;
/// Torches keep zombies from appearing this close.
pub const TORCH_SAFE_RADIUS: f32 = 12.0;

pub struct PropsPlugin;

impl Plugin for PropsPlugin {
    fn build(&self, app: &mut App) {
        app.add_client_message::<Place>(Channel::Ordered)
            .add_mapped_client_message::<Break>(Channel::Ordered)
            .replicate::<Prop>()
            .replicate::<Crop>()
            .replicate::<Lid>()
            .add_mapped_client_message::<Peek>(Channel::Ordered)
            .add_observer(dress)
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (place, break_props, peek, grow).run_if(in_state(AppState::Game)).run_if(crate::net::authority),
            )
            .add_systems(Update, (show_crops, swing_lids).run_if(in_state(AppState::Game)));
    }
}

#[derive(Component, Clone, Copy, Serialize, Deserialize)]
pub struct Prop(pub PropKind);

/// A player putting a prop down: wherever props are kept, it appears.
#[derive(Message, Serialize, Deserialize)]
pub struct Place {
    pub kind: PropKind,
    pub at: Vec3,
    pub yaw: f32,
}

/// A player breaking a prop; it drops itself and whatever it held.
#[derive(Message, Serialize, Deserialize, Clone, Copy)]
pub struct Break(pub Entity);

impl MapEntities for Break {
    fn map_entities<M: EntityMapper>(&mut self, mapper: &mut M) {
        self.0 = mapper.get_mapped(self.0);
    }
}

/// How many players have a chest open: its lid is up while anyone does.
#[derive(Component, Serialize, Deserialize, PartialEq)]
pub struct Lid(u8);

/// A player opened or closed a chest.
#[derive(Message, Serialize, Deserialize, Clone, Copy)]
pub struct Peek {
    pub chest: Entity,
    pub open: bool,
}

impl MapEntities for Peek {
    fn map_entities<M: EntityMapper>(&mut self, mapper: &mut M) {
        self.chest = mapper.get_mapped(self.chest);
    }
}

fn peek(mut peeks: MessageReader<FromClient<Peek>>, mut lids: Query<&mut Lid>) {
    for Peek { chest, open } in peeks.read().map(|p| p.message) {
        if let Ok(mut lid) = lids.get_mut(chest) {
            lid.0 = if open { lid.0.saturating_add(1) } else { lid.0.saturating_sub(1) };
        }
    }
}

/// The animation player of a chest's model, and which way its lid went last.
#[derive(Component)]
struct LidPlayer {
    player: Entity,
    open: bool,
    nodes: [AnimationNodeIndex; 2],
}

fn swing_lids(
    mut chests: Query<(&Lid, &mut LidPlayer, &Transform)>,
    mut players: Query<&mut AnimationPlayer>,
    mut sounds: MessageWriter<PlaySound>,
) {
    for (lid, mut chest, transform) in &mut chests {
        let open = lid.0 > 0;
        if open == chest.open {
            continue;
        }
        chest.open = open;
        let Ok(mut player) = players.get_mut(chest.player) else { continue };
        player.stop_all();
        player.play(chest.nodes[if open { 0 } else { 1 }]);
        let sfx = if open { Sfx::ChestOpen } else { Sfx::ChestClose };
        sounds.write(PlaySound::at(sfx, transform.translation + Vec3::Y * 0.5));
    }
}

fn place(mut commands: Commands, mut places: MessageReader<FromClient<Place>>) {
    for p in places.read() {
        let p = &p.message;
        spawn(
            &mut commands,
            p.kind,
            Transform::from_translation(p.at).with_rotation(Quat::from_rotation_y(p.yaw)),
            0.0,
        );
    }
}

fn break_props(
    mut commands: Commands,
    mut breaks: MessageReader<FromClient<Break>>,
    creative: Res<crate::Creative>,
    props: Query<(&Prop, &Transform, Option<&Crop>, Option<&FurnaceState>, Option<&ChestState>)>,
    mut drops: MessageWriter<Drop>,
) {
    for Break(entity) in breaks.read().map(|b| &b.message) {
        let Ok((prop, transform, crop, furnace, chest)) = props.get(*entity) else { continue };
        commands.entity(*entity).despawn();
        if creative.0 {
            continue;
        }
        let gained: &[(Item, u32)] = match prop.0 {
            PropKind::Furnace => &[(Item::Furnace, 1)],
            PropKind::Torch => &[(Item::Torch, 1)],
            PropKind::Bed => &[(Item::Bed, 1)],
            PropKind::Wheat if crop.is_some_and(Crop::ripe) => &[(Item::Wheat, 1), (Item::Seeds, 2)],
            PropKind::Wheat => &[(Item::Seeds, 1)],
            PropKind::CraftingTable => &[(Item::CraftingTable, 1)],
            PropKind::Chest => &[(Item::Chest, 1)],
        };
        let at = transform.translation + Vec3::Y * 0.3;
        // Whatever was inside spills out.
        let inside = furnace
            .into_iter()
            .flat_map(FurnaceState::contents)
            .chain(chest.into_iter().flat_map(|c| c.slots.iter().flatten().copied()));
        let stacks = gained.iter().map(|&(item, n)| Stack::new(item, n)).chain(inside);
        for (i, stack) in stacks.enumerate() {
            drops.write(Drop { stack, ..Drop::pop(stack.item, stack.count, at, i as u32 ^ entity.index_u32()) });
        }
    }
}

/// A planted crop; grows while loaded.
#[derive(Component, Serialize, Deserialize)]
pub struct Crop {
    pub age: f32,
    stage: usize,
}

impl Crop {
    pub fn ripe(&self) -> bool {
        self.age >= WHEAT_GROW_SECS
    }
}

#[derive(Resource)]
pub struct PropAssets {
    stone: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
    glow: Handle<StandardMaterial>,
    wood: Handle<StandardMaterial>,
    cube: Handle<Mesh>,
    stick: Handle<Mesh>,
    flame: Handle<Mesh>,
    crop: Handle<Mesh>,
    wheat: [Handle<StandardMaterial>; WHEAT_STAGES],
    table: Handle<WorldAsset>,
    chest: Handle<WorldAsset>,
    /// The chest's lid opening and closing.
    lid: (Handle<AnimationGraph>, [AnimationNodeIndex; 2]),
    bedroll: Handle<WorldAsset>,
}

fn setup(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    let (lid, nodes) = AnimationGraph::from_clips(
        [0, 1].map(|i| server.load(GltfAssetLabel::Animation(i).from_asset("models/kenney/chest.glb"))),
    );
    let model = |name: &str| server.load(GltfAssetLabel::Scene(0).from_asset(format!("models/kenney/{name}.glb")));
    let mut color = |r, g, b| {
        materials.add(StandardMaterial { base_color: Color::srgb(r, g, b), perceptual_roughness: 0.85, ..default() })
    };
    let stone = color(0.42, 0.41, 0.40);
    let dark = color(0.05, 0.04, 0.04);
    let wood = color(0.55, 0.38, 0.22);
    let wheat_colors = [(0.30, 0.55, 0.18), (0.42, 0.60, 0.20), (0.62, 0.62, 0.22), (0.86, 0.72, 0.30)];
    let wheat = wheat_colors.map(|(r, g, b)| {
        materials.add(StandardMaterial {
            base_color: Color::srgb(r, g, b),
            perceptual_roughness: 0.9,
            cull_mode: None,
            double_sided: true,
            ..default()
        })
    });
    let glow = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.6, 0.2),
        emissive: LinearRgba::rgb(12.0, 5.0, 1.0),
        ..default()
    });
    commands.insert_resource(PropAssets {
        stone,
        dark,
        glow,
        wood,
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        stick: meshes.add(Cylinder::new(0.04, 0.6).mesh().resolution(6)),
        flame: meshes.add(Sphere::new(0.08).mesh().ico(1).expect("small subdivision count")),
        crop: meshes.add(crop_mesh()),
        wheat,
        table: model("workbench"),
        chest: model("chest"),
        lid: (graphs.add(lid), [nodes[0], nodes[1]]),
        bedroll: model("bedroll"),
    });
}

/// A prop as the world's owner keeps it; its looks and collider come from [`dress`].
pub fn spawn(commands: &mut Commands, kind: PropKind, transform: Transform, age: f32) -> Entity {
    let mut entity = commands.spawn((DespawnOnExit(AppState::Game), Replicated, Prop(kind), transform));
    match kind {
        PropKind::Furnace => {
            entity.insert(FurnaceState::default());
        }
        PropKind::Chest => {
            entity.insert((ChestState::default(), Lid(0)));
        }
        PropKind::Wheat => {
            entity.insert(Crop { age, stage: stage(age) });
        }
        _ => {}
    }
    entity.id()
}

/// Looks and collider of a prop, on the host and on guests alike.
fn dress(add: On<Add<Prop>>, mut commands: Commands, props: Query<(&Prop, Option<&Crop>)>, assets: Res<PropAssets>) {
    let Ok((&Prop(kind), crop)) = props.get(add.entity) else { return };
    let assets = &*assets;
    let cube = |material: &Handle<StandardMaterial>, size: Vec3, at: Vec3| {
        (
            Mesh3d(assets.cube.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(at).with_scale(size),
        )
    };
    let walkthrough = CollisionLayers::new(Layer::Walkthrough, LayerMask::NONE);
    let mut entity = commands.entity(add.entity);
    entity.insert((RigidBody::Static, Visibility::default()));
    match kind {
        PropKind::Furnace => {
            entity.insert(Collider::compound(vec![(Vec3::Y * 0.45, Quat::IDENTITY, Collider::cuboid(0.9, 0.9, 0.9))]));
            entity.with_children(|p| {
                p.spawn(cube(&assets.stone, Vec3::splat(0.9), Vec3::Y * 0.45));
                p.spawn(cube(&assets.dark, Vec3::new(0.5, 0.35, 0.05), Vec3::new(0.0, 0.3, -0.44)));
                p.spawn((cube(&assets.glow, Vec3::new(0.4, 0.08, 0.05), Vec3::new(0.0, 0.16, -0.45)), NotShadowCaster));
            });
        }
        PropKind::Torch => {
            entity.insert((
                Collider::compound(vec![(Vec3::Y * 0.35, Quat::IDENTITY, Collider::cylinder(0.12, 0.7))]),
                walkthrough,
            ));
            entity.with_children(|p| {
                p.spawn((
                    Mesh3d(assets.stick.clone()),
                    MeshMaterial3d(assets.wood.clone()),
                    Transform::from_xyz(0.0, 0.3, 0.0),
                ));
                p.spawn((
                    Mesh3d(assets.flame.clone()),
                    MeshMaterial3d(assets.glow.clone()),
                    Transform::from_xyz(0.0, 0.65, 0.0),
                    NotShadowCaster,
                ));
                p.spawn((torch_light(), Transform::from_xyz(0.0, 0.8, 0.0)));
            });
        }
        PropKind::Bed => {
            entity.insert(Collider::compound(vec![(Vec3::Y * 0.2, Quat::IDENTITY, Collider::cuboid(1.0, 0.4, 2.0))]));
            entity.with_child(model(&assets.bedroll, 3.3));
        }
        PropKind::CraftingTable => {
            entity.insert(Collider::compound(vec![(Vec3::Y * 0.36, Quat::IDENTITY, Collider::cuboid(0.9, 0.72, 0.9))]));
            entity.with_child(model(&assets.table, 3.0));
        }
        PropKind::Chest => {
            entity.insert(Collider::compound(vec![(Vec3::Y * 0.35, Quat::IDENTITY, Collider::cuboid(0.9, 0.7, 0.9))]));
            let owner = add.entity;
            let (graph, nodes) = assets.lid.clone();
            let lid = commands.spawn((model(&assets.chest, 3.4), ChildOf(owner))).id();
            commands.entity(lid).observe(
                move |ready: On<WorldInstanceReady>,
                      mut commands: Commands,
                      children: Query<&Children>,
                      players: Query<(), With<AnimationPlayer>>| {
                    if let Some(player) = children.iter_descendants(ready.entity).find(|c| players.contains(*c)) {
                        commands.entity(player).insert(AnimationGraphHandle(graph.clone()));
                        commands.entity(owner).insert(LidPlayer { player, open: false, nodes });
                    }
                },
            );
        }
        PropKind::Wheat => {
            let stage = crop.map_or(0, |c| c.stage);
            entity.insert((
                Collider::compound(vec![(Vec3::Y * 0.3, Quat::IDENTITY, Collider::cuboid(0.8, 0.6, 0.8))]),
                walkthrough,
            ));
            entity.with_children(|p| {
                p.spawn((
                    Mesh3d(assets.crop.clone()),
                    MeshMaterial3d(assets.wheat[stage].clone()),
                    Transform::from_scale(crop_scale(stage)),
                    NotShadowCaster,
                ));
            });
        }
    }
}

/// A downloaded model standing at the prop's base, turned to face the player who placed it.
fn model(scene: &Handle<WorldAsset>, scale: f32) -> impl Bundle {
    (
        WorldAssetRoot(scene.clone()),
        Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI)).with_scale(Vec3::splat(scale)),
    )
}

pub fn torch_light() -> PointLight {
    PointLight { color: Color::srgb(1.0, 0.7, 0.4), intensity: 300_000.0, range: 16.0, ..default() }
}

fn stage(age: f32) -> usize {
    ((age / WHEAT_GROW_SECS * (WHEAT_STAGES - 1) as f32) as usize).min(WHEAT_STAGES - 1)
}

fn crop_scale(stage: usize) -> Vec3 {
    Vec3::new(1.0, 0.35 + 0.65 * stage as f32 / (WHEAT_STAGES - 1) as f32, 1.0)
}

fn grow(time: Res<Time>, mut crops: Query<&mut Crop>) {
    for mut crop in &mut crops {
        crop.bypass_change_detection().age += time.delta_secs();
        let stage = stage(crop.age);
        // Only a new stage is news for guests.
        if stage != crop.stage {
            crop.stage = stage;
        }
    }
}

fn show_crops(
    assets: Res<PropAssets>,
    crops: Query<(&Crop, &Children), Changed<Crop>>,
    mut parts: Query<(&mut Transform, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    for (crop, children) in &crops {
        let stage = crop.stage;
        for &child in children {
            if let Ok((mut transform, mut material)) = parts.get_mut(child) {
                transform.scale = crop_scale(stage);
                material.0 = assets.wheat[stage].clone();
            }
        }
    }
}

/// A tuft of crossed blades, about 0.8 m tall at full size.
fn crop_mesh() -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for i in 0..9 {
        let offset = Vec3::new((i % 3) as f32 - 1.0, 0.0, (i / 3) as f32 - 1.0) * 0.25;
        let angle = i as f32 * 1.3;
        let side = Vec3::new(angle.cos(), 0.0, angle.sin()) * 0.12;
        let normal = Vec3::new(-angle.sin(), 0.2, angle.cos()).normalize();
        let base = positions.len() as u32;
        positions.extend([
            offset - side,
            offset + side,
            offset + Vec3::Y * 0.8 + side * 0.3,
            offset + Vec3::Y * 0.8 - side * 0.3,
        ]);
        normals.extend([normal; 4]);
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}
