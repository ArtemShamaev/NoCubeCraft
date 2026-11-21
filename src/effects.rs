//! Debris that flies off dug blocks.

use std::collections::HashMap;

use bevy::{light::NotShadowCaster, prelude::*};

use crate::{AppState, terrain::Material};

const LIFETIME: f32 = 0.7;
const GRAVITY: f32 = 14.0;

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Debris>()
            .add_systems(Startup, setup)
            .add_systems(Update, (spawn_debris, fly).run_if(in_state(AppState::Game)));
    }
}

/// Bits of `material` bursting out of a dug region centred at `at`, `size` metres across.
#[derive(Message, Clone, Copy)]
pub struct Debris {
    pub at: Vec3,
    pub size: f32,
    pub material: Material,
}

#[derive(Component)]
struct Bit {
    velocity: Vec3,
    age: f32,
    scale: f32,
}

#[derive(Resource)]
struct DebrisAssets {
    cube: Handle<Mesh>,
    materials: HashMap<Material, Handle<StandardMaterial>>,
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let colors = Material::ALL.into_iter().filter(|m| *m != Material::Air).map(|m| {
        let [r, g, b] = crate::terrain::albedo(m);
        (
            m,
            materials.add(StandardMaterial {
                base_color: Color::linear_rgb(r, g, b),
                perceptual_roughness: 0.9,
                ..default()
            }),
        )
    });
    commands
        .insert_resource(DebrisAssets { cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)), materials: colors.collect() });
}

fn spawn_debris(
    mut commands: Commands,
    mut requests: MessageReader<Debris>,
    assets: Res<DebrisAssets>,
    mut seed: Local<u32>,
) {
    for debris in requests.read() {
        let Some(material) = assets.materials.get(&debris.material) else { continue };
        let count = (6.0 + debris.size * 8.0) as u32;
        for _ in 0..count {
            *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let r = |shift: u32| ((*seed >> shift) & 0xff) as f32 / 255.0 - 0.5;
            let offset = Vec3::new(r(0), r(8), r(16)) * debris.size;
            let velocity = Vec3::new(r(3) * 5.0, 2.5 + r(11).abs() * 4.0, r(19) * 5.0);
            let scale = 0.05 + (r(24) + 0.5) * 0.06;
            commands.spawn((
                DespawnOnExit(AppState::Game),
                Bit { velocity, age: 0.0, scale },
                Mesh3d(assets.cube.clone()),
                MeshMaterial3d(material.clone()),
                NotShadowCaster,
                Transform::from_translation(debris.at + offset).with_scale(Vec3::splat(scale)),
            ));
        }
    }
}

fn fly(mut commands: Commands, time: Res<Time>, mut bits: Query<(Entity, &mut Bit, &mut Transform)>) {
    let dt = time.delta_secs();
    for (entity, mut bit, mut transform) in &mut bits {
        bit.age += dt;
        if bit.age > LIFETIME {
            commands.entity(entity).despawn();
            continue;
        }
        bit.velocity.y -= GRAVITY * dt;
        transform.translation += bit.velocity * dt;
        transform.rotate_x(dt * 7.0);
        transform.scale = Vec3::splat(bit.scale * (1.0 - bit.age / LIFETIME).sqrt());
    }
}
