//! Objects placed in the world: furnaces, torches, beds and crops.

use avian3d::prelude::*;
use bevy::{
    asset::RenderAssetUsages,
    light::NotShadowCaster,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    world_serialization::{WorldAsset, WorldAssetRoot},
};

use crate::{
    AppState,
    body::Layer,
    containers::{ChestState, FurnaceState},
    items::PropKind,
};

/// Seconds for wheat to grow from seed to harvest.
pub const WHEAT_GROW_SECS: f32 = 90.0;
const WHEAT_STAGES: usize = 4;
/// Torches keep zombies from appearing this close.
pub const TORCH_SAFE_RADIUS: f32 = 12.0;

pub struct PropsPlugin;

impl Plugin for PropsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(Update, grow.run_if(in_state(AppState::Game)));
    }
}

#[derive(Component, Clone, Copy)]
pub struct Prop(pub PropKind);

/// A planted crop; grows while loaded.
#[derive(Component)]
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
    bedroll: Handle<WorldAsset>,
}

fn setup(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
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
        bedroll: model("bedroll"),
    });
}

/// Spawns a prop standing at `transform` (its base on the ground).
pub fn spawn(commands: &mut Commands, assets: &PropAssets, kind: PropKind, transform: Transform, age: f32) -> Entity {
    let cube = |material: &Handle<StandardMaterial>, size: Vec3, at: Vec3| {
        (
            Mesh3d(assets.cube.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(at).with_scale(size),
        )
    };
    let walkthrough = CollisionLayers::new(Layer::Walkthrough, LayerMask::NONE);
    let mut entity = commands.spawn((
        DespawnOnExit(AppState::Game),
        Prop(kind),
        RigidBody::Static,
        transform,
        Visibility::default(),
    ));
    match kind {
        PropKind::Furnace => {
            entity.insert(FurnaceState::default());
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
            entity.insert((
                ChestState::default(),
                Collider::compound(vec![(Vec3::Y * 0.35, Quat::IDENTITY, Collider::cuboid(0.9, 0.7, 0.9))]),
            ));
            entity.with_child(model(&assets.chest, 3.4));
        }
        PropKind::Wheat => {
            let stage = stage(age);
            entity.insert((
                Crop { age, stage },
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
    entity.id()
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

fn grow(
    time: Res<Time>,
    assets: Res<PropAssets>,
    mut crops: Query<(&mut Crop, &Children)>,
    mut parts: Query<(&mut Transform, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    for (mut crop, children) in &mut crops {
        crop.age += time.delta_secs();
        let stage = stage(crop.age);
        if stage == crop.stage {
            continue;
        }
        crop.stage = stage;
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
