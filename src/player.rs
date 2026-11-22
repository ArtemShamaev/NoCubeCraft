use std::f32::consts::FRAC_PI_2;

use bevy::{
    anti_alias::fxaa::Fxaa,
    core_pipeline::prepass::DepthPrepass,
    input::mouse::AccumulatedMouseMotion,
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};

use crate::{
    AppState, Creative, Mode,
    body::{Body, MoveBodies},
    hud::Toast,
    save::LoadWorld,
    settings::Settings,
    terrain::{SHARDS_BELOW, SHARDS_Y, Terrain, TerrainAnchor},
};

pub const RADIUS: f32 = 0.35;
pub const HEIGHT: f32 = 1.8;
pub const EYE_HEIGHT: f32 = 1.62;

const WALK_SPEED: f32 = 4.5;
const SPRINT_SPEED: f32 = 7.5;
const FLY_SPEED: f32 = 11.0;
const FLY_SPRINT_SPEED: f32 = 24.0;
/// Radians per pixel of mouse movement at sensitivity 1.
const MOUSE_SENSITIVITY: f32 = 0.0022;
/// Two jump presses this close together toggle flying in creative mode.
const DOUBLE_TAP_SECS: f32 = 0.3;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::Game), spawn.after(LoadWorld))
            .add_systems(OnEnter(Mode::Playing), grab_cursor)
            .add_systems(OnExit(Mode::Playing), release_cursor)
            .add_systems(Update, (grab_on_click, pause).run_if(in_state(Mode::Playing)))
            .add_systems(Update, (look, steer).chain().before(MoveBodies).run_if(in_state(AppState::Game)))
            .add_systems(Update, (wait_for_ground, fall_out).after(MoveBodies).run_if(in_state(AppState::Game)));
    }
}

#[derive(Component)]
pub struct Player {
    pub yaw: f32,
    pub pitch: f32,
    /// Where the player comes back after dying.
    pub spawn: Vec3,
}

/// The player hovers here until the terrain under them exists; then they get a [`Body`].
#[derive(Component)]
pub struct Spawning;

#[derive(Component)]
pub struct PlayerCamera;

/// Where and how the player starts: a fresh world or a saved game.
#[derive(Resource, Clone)]
pub struct PlayerStart {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub spawn: Vec3,
}

fn spawn(mut commands: Commands, start: Res<PlayerStart>, settings: Res<Settings>) {
    commands
        .spawn((
            DespawnOnExit(AppState::Game),
            Player { yaw: start.yaw, pitch: start.pitch, spawn: start.spawn },
            Spawning,
            TerrainAnchor,
            Transform::from_translation(start.position),
            Visibility::default(),
        ))
        .with_child((
            PlayerCamera,
            Camera3d::default(),
            // Tuned for integrated GPUs: the depth prepass stops hidden hills from being
            // shaded, and FXAA replaces MSAA.
            DepthPrepass,
            Msaa::Off,
            Fxaa::default(),
            bevy::light::ShadowFilteringMethod::Hardware2x2,
            // A near plane well inside the body, so the view never pokes into a wall.
            Projection::Perspective(PerspectiveProjection { fov: settings.fov.to_radians(), near: 0.05, ..default() }),
            Transform::from_xyz(0.0, EYE_HEIGHT, 0.0),
        ));
}

pub fn body() -> Body {
    Body::new(RADIUS, HEIGHT)
}

fn wait_for_ground(
    mut commands: Commands,
    terrain: Res<Terrain>,
    player: Single<(Entity, &Transform), (With<Player>, With<Spawning>)>,
) {
    let (entity, transform) = player.into_inner();
    if terrain.is_ready_around(transform.translation) {
        commands.entity(entity).remove::<Spawning>().insert(body());
    }
}

pub fn in_shards(p: Vec3) -> bool {
    p.y > SHARDS_Y - SHARDS_BELOW - 200.0
}

/// Moves the player elsewhere; they hover there until the ground under them is ready.
pub fn teleport(commands: &mut Commands, player: Entity, transform: &mut Transform, to: Vec3) {
    transform.translation = to;
    commands.entity(player).remove::<Body>().insert(Spawning);
}

/// Falling off the Shards ends back home rather than in the void.
fn fall_out(
    mut commands: Commands,
    mut toast: ResMut<Toast>,
    player: Single<(Entity, &Player, &mut Transform), With<Body>>,
) {
    let (entity, player, mut transform) = player.into_inner();
    if in_shards(transform.translation) && transform.translation.y < SHARDS_Y - SHARDS_BELOW - 60.0 {
        let home = player.spawn;
        teleport(&mut commands, entity, &mut transform, home);
        toast.show("Пустота выбросила вас домой".into());
    }
}

fn grab_cursor(mut cursor: Single<&mut CursorOptions>) {
    cursor.grab_mode = CursorGrabMode::Locked;
    cursor.visible = false;
}

fn release_cursor(mut cursor: Single<&mut CursorOptions>) {
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
}

/// The cursor can get released by the window losing focus; clicking takes it back.
fn grab_on_click(mouse: Res<ButtonInput<MouseButton>>, cursor: Single<&mut CursorOptions>) {
    if mouse.just_pressed(MouseButton::Left) && !cursor_grabbed(&cursor) {
        grab_cursor(cursor);
    }
}

fn pause(keys: Res<ButtonInput<KeyCode>>, mut mode: ResMut<NextState<Mode>>) {
    if keys.just_pressed(KeyCode::Escape) {
        mode.set(Mode::Paused);
    }
}

pub fn cursor_grabbed(cursor: &CursorOptions) -> bool {
    cursor.grab_mode != CursorGrabMode::None
}

fn look(
    motion: Res<AccumulatedMouseMotion>,
    mode: Res<State<Mode>>,
    settings: Res<Settings>,
    cursor: Single<&CursorOptions>,
    mut player: Single<(&mut Player, &mut Transform), Without<PlayerCamera>>,
    mut camera: Single<&mut Transform, With<PlayerCamera>>,
) {
    let (player, transform) = &mut *player;
    if *mode == Mode::Playing && cursor_grabbed(&cursor) {
        let k = MOUSE_SENSITIVITY * settings.mouse_sensitivity;
        player.yaw -= motion.delta.x * k;
        player.pitch = (player.pitch - motion.delta.y * k).clamp(-FRAC_PI_2 + 0.01, FRAC_PI_2 - 0.01);
    }
    transform.rotation = Quat::from_rotation_y(player.yaw);
    camera.rotation = Quat::from_rotation_x(player.pitch);
}

fn steer(
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<State<Mode>>,
    creative: Res<Creative>,
    player: Single<(&mut Body, &Transform), With<Player>>,
    mut last_jump: Local<f32>,
) {
    let (mut body, transform) = player.into_inner();
    if !creative.0 {
        body.flying = false;
    }
    if *mode != Mode::Playing {
        body.wish = Vec3::ZERO;
        body.jump = false;
        body.descend = false;
        return;
    }
    if creative.0 && keys.just_pressed(KeyCode::Space) {
        let now = time.elapsed_secs();
        if now - *last_jump < DOUBLE_TAP_SECS {
            body.flying = !body.flying;
            body.velocity = Vec3::ZERO;
            *last_jump = 0.0;
        } else {
            *last_jump = now;
        }
    }
    let axis = |pos: KeyCode, neg: KeyCode| keys.pressed(pos) as i32 as f32 - keys.pressed(neg) as i32 as f32;
    let input = Vec3::new(axis(KeyCode::KeyD, KeyCode::KeyA), 0.0, axis(KeyCode::KeyS, KeyCode::KeyW));
    let sprint = keys.pressed(KeyCode::ShiftLeft);
    let speed = match (body.flying, sprint) {
        (true, true) => FLY_SPRINT_SPEED,
        (true, false) => FLY_SPEED,
        (false, true) => SPRINT_SPEED,
        (false, false) => WALK_SPEED,
    };
    body.wish = transform.rotation * input.normalize_or_zero() * speed;
    body.jump = keys.pressed(KeyCode::Space);
    body.descend = keys.pressed(KeyCode::ControlLeft);
}
