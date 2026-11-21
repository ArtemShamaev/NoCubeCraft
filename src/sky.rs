use std::f32::consts::TAU;

use bevy::{
    asset::embedded_asset,
    light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap, NotShadowCaster, NotShadowReceiver},
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError},
    shader::ShaderRef,
};

use crate::{
    AppState,
    player::PlayerCamera,
    settings::Settings,
    terrain::{Terrain, ViewDistance},
};

const DAY_LENGTH_SECS: f32 = 600.0;
/// 0 is sunrise, 0.25 noon, 0.5 sunset.
pub const START_TIME: f32 = 0.08;

const SUN_LUX: f32 = 12_000.0;
const MOON_LUX: f32 = 600.0;
const DAY_AMBIENT: f32 = 500.0;
const NIGHT_AMBIENT: f32 = 120.0;

const DAY_SKY: Srgba = Srgba::rgb(0.62, 0.77, 0.93);
const DAY_ZENITH: Srgba = Srgba::rgb(0.24, 0.46, 0.84);
const DUSK_SKY: Srgba = Srgba::rgb(0.98, 0.6, 0.38);
const NIGHT_SKY: Srgba = Srgba::rgb(0.025, 0.035, 0.08);
const NIGHT_ZENITH: Srgba = Srgba::rgb(0.005, 0.01, 0.03);
/// The sky dome sits inside the camera's far plane but beyond any terrain.
const DOME_RADIUS: f32 = 850.0;

/// Fog starts at this fraction of the view distance and is thick just before its end.
const FOG_START: f32 = 0.4;
const FOG_END: f32 = 0.92;
const WATER_FOG: Srgba = Srgba::rgb(0.05, 0.2, 0.26);
const WATER_FOG_END: f32 = 24.0;
const CAVE_FOG: Srgba = Srgba::rgb(0.02, 0.02, 0.025);
const CAVE_FOG_END: f32 = 45.0;

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/sky.wesl");
        app.add_plugins(MaterialPlugin::<SkyMaterial>::default())
            .insert_resource(DirectionalLightShadowMap { size: 1024 })
            .add_systems(OnEnter(AppState::Game), (spawn_sun, spawn_dome))
            .add_systems(Update, (add_fog, advance, follow_camera).chain().run_if(resource_exists::<TimeOfDay>));
    }
}

/// Fraction of the current day, in [0, 1).
#[derive(Resource)]
pub struct TimeOfDay(pub f32);

impl TimeOfDay {
    /// Between sunset and sunrise, with a little margin for dusk.
    pub fn is_night(&self) -> bool {
        (0.52..0.98).contains(&self.0)
    }
}

fn spawn_sun(mut commands: Commands, settings: Res<Settings>) {
    commands.spawn((
        DespawnOnExit(AppState::Game),
        DirectionalLight { shadow_maps_enabled: settings.shadows, ..default() },
        CascadeShadowConfigBuilder {
            num_cascades: 2,
            first_cascade_far_bound: 20.0,
            maximum_distance: 70.0,
            ..default()
        }
        .build(),
    ));
}

/// Gradient, sun, moon, stars and clouds, painted on a dome that follows the camera.
#[derive(Asset, AsBindGroup, TypePath, Clone, Default)]
pub struct SkyMaterial {
    #[uniform(0)]
    params: SkyParams,
}

#[derive(ShaderType, Clone, Copy, Default)]
struct SkyParams {
    zenith: Vec4,
    horizon: Vec4,
    sun: Vec4,
    sun_color: Vec4,
    misc: Vec4,
}

impl Material for SkyMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://barnocraft/shaders/sky.wesl".into()
    }

    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Seen from inside.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

#[derive(Component)]
struct Dome(Handle<SkyMaterial>);

fn spawn_dome(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<SkyMaterial>>) {
    let material = materials.add(SkyMaterial::default());
    commands.spawn((
        DespawnOnExit(AppState::Game),
        Dome(material.clone()),
        Mesh3d(meshes.add(Sphere::new(DOME_RADIUS).mesh().uv(32, 18))),
        MeshMaterial3d(material),
        NotShadowCaster,
        NotShadowReceiver,
        Transform::default(),
    ));
}

fn follow_camera(camera: Single<&GlobalTransform, With<PlayerCamera>>, mut dome: Single<&mut Transform, With<Dome>>) {
    dome.translation = camera.translation();
}

fn add_fog(mut commands: Commands, cameras: Query<Entity, Added<Camera3d>>) {
    for camera in &cameras {
        commands
            .entity(camera)
            .insert(DistanceFog { falloff: FogFalloff::Linear { start: FOG_START, end: FOG_END }, ..default() });
    }
}

fn advance(
    time: Res<Time>,
    mut time_of_day: ResMut<TimeOfDay>,
    mut clear: ResMut<ClearColor>,
    mut ambient: ResMut<GlobalAmbientLight>,
    sun: Single<(&mut DirectionalLight, &mut Transform)>,
    terrain: Res<Terrain>,
    view: Res<ViewDistance>,
    mut fog: Query<(&mut DistanceFog, &GlobalTransform), With<PlayerCamera>>,
    dome: Single<&Dome>,
    mut sky_materials: ResMut<Assets<SkyMaterial>>,
    real: Res<Time<Real>>,
) {
    time_of_day.0 = (time_of_day.0 + time.delta_secs() / DAY_LENGTH_SECS).fract();
    let angle = time_of_day.0 * TAU;
    // Tilted orbit so the sun never passes exactly overhead and shadows keep some direction.
    let sun_dir = Vec3::new(angle.cos(), angle.sin(), 0.35).normalize();
    let elevation = sun_dir.y;

    // 0 at night, 1 in full day, with a soft transition around the horizon.
    let day = smoothstep(-0.12, 0.25, elevation);
    let dusk = (1.0 - (elevation / 0.3).abs()).clamp(0.0, 1.0) * smoothstep(-0.2, 0.0, elevation);

    let sky = NIGHT_SKY.mix(&DAY_SKY, day).mix(&DUSK_SKY, dusk * 0.6);
    let sunlight = Srgba::WHITE.mix(&Srgba::rgb(1.0, 0.62, 0.38), dusk);

    let (mut light, mut transform) = sun.into_inner();
    let (light_dir, lux, color) = if elevation > -0.05 {
        (sun_dir, SUN_LUX * smoothstep(-0.05, 0.15, elevation), sunlight)
    } else {
        (-sun_dir, MOON_LUX * smoothstep(0.0, 0.2, -elevation), Srgba::rgb(0.62, 0.7, 1.0))
    };
    *transform = Transform::default().looking_to(-light_dir, Vec3::Y);
    // Deep underground the sky's light doesn't reach; only torches help there.
    let eye = fog.iter().next().map(|(_, g)| g.translation());
    let under = eye.map_or(0.0, |e| smoothstep(3.0, 10.0, terrain.surface_height(e.x, e.z) - e.y));
    light.illuminance = lux * (1.0 - under);
    light.color = color.into();

    clear.0 = sky.into();
    if let Some(mut material) = sky_materials.get_mut(&dome.0) {
        let zenith = NIGHT_ZENITH.mix(&DAY_ZENITH, day);
        let linear = |c: Srgba| LinearRgba::from(c).to_vec4();
        material.params = SkyParams {
            zenith: linear(zenith),
            horizon: linear(sky),
            sun: sun_dir.extend(day),
            sun_color: LinearRgba::from(sunlight).to_vec3().extend(dusk),
            misc: Vec4::new(real.elapsed_secs() % 10_000.0, 0.0, 0.0, 0.0),
        };
    }
    ambient.color = Srgba::rgb(0.75, 0.82, 1.0).mix(&Srgba::WHITE, day).into();
    ambient.brightness = (NIGHT_AMBIENT + (DAY_AMBIENT - NIGHT_AMBIENT) * day) * (1.0 - 0.8 * under);

    for (mut fog, camera) in &mut fog {
        if terrain.is_water(camera.translation()) {
            fog.color = WATER_FOG.mix(&NIGHT_SKY, 1.0 - day).into();
            fog.directional_light_color = Color::NONE;
            fog.falloff = FogFalloff::Linear { start: 0.0, end: WATER_FOG_END };
        } else if under > 0.0 {
            fog.color = sky.mix(&CAVE_FOG, under).into();
            fog.directional_light_color = Color::NONE;
            let end = view.0 * FOG_END * (1.0 - under) + CAVE_FOG_END * under;
            fog.falloff = FogFalloff::Linear { start: end * 0.3, end };
        } else {
            fog.color = sky.into();
            fog.directional_light_color = sunlight.with_alpha(0.4 * day).into();
            fog.falloff = FogFalloff::Linear { start: view.0 * FOG_START, end: view.0 * FOG_END };
        }
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
