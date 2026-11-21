//! Sound effects and ambience: recordings from `assets/sounds` (CC0, see CREDITS.md),
//! plus wind, splashes and clicks made in code.

use std::{collections::HashMap, f32::consts::TAU, sync::Arc};

use bevy::{
    audio::{PlaybackMode, SpatialScale, Volume},
    prelude::*,
};

use crate::{
    AppState,
    body::Body,
    mobs::{Mob, MobKind},
    player::{Player, PlayerCamera},
    sky::TimeOfDay,
    survival::Vitals,
    terrain::{CELL, Material, Terrain},
};

const RATE: u32 = 22_050;
/// Metres walked per footstep.
const STRIDE: f32 = 1.7;

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PlaySound>()
            .add_systems(Startup, setup)
            .add_systems(OnEnter(AppState::Game), start_ambience)
            .add_systems(Update, (play, add_listener))
            .add_systems(Update, (footsteps, hurt, creatures, ambience).run_if(in_state(AppState::Game)));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Sfx {
    DigSoft,
    DigStone,
    DigWood,
    Place,
    StepGrass,
    StepSnow,
    StepWood,
    StepStone,
    Hit,
    Hurt,
    Groan,
    Baa,
    ChestOpen,
    ChestClose,
    Page,
    Inventory,
    Splash,
    Click,
}

impl Sfx {
    /// Recorded takes (in `assets/sounds`); a few per sound so repeats don't sound mechanical.
    fn files(self) -> Vec<String> {
        let numbered = |name: &str, n: usize| (0..n).map(|i| format!("sounds/{name}_{i:03}.ogg")).collect();
        match self {
            Sfx::DigSoft => numbered("impactSoft_medium", 5),
            Sfx::DigStone => numbered("impactMining", 5),
            Sfx::DigWood => numbered("impactWood_medium", 5),
            Sfx::Place => numbered("impactPlank_medium", 3),
            Sfx::StepGrass => numbered("footstep_grass", 5),
            Sfx::StepSnow => numbered("footstep_snow", 5),
            Sfx::StepWood => numbered("footstep_wood", 5),
            Sfx::StepStone => numbered("footstep_concrete", 5),
            Sfx::Hit => numbered("impactPunch_medium", 5),
            Sfx::Hurt => numbered("impactPunch_heavy", 3),
            Sfx::Groan => {
                ["undead-1", "undead-2", "undead-3", "zombienoise2"].map(|n| format!("sounds/{n}.ogg")).into()
            }
            Sfx::Baa => vec!["sounds/sheep_baa.ogg".into()],
            Sfx::ChestOpen => vec!["sounds/doorOpen_1.ogg".into()],
            Sfx::ChestClose => vec!["sounds/doorClose_1.ogg".into()],
            Sfx::Page => vec!["sounds/bookFlip1.ogg".into()],
            Sfx::Inventory => vec!["sounds/cloth1.ogg".into()],
            Sfx::Splash | Sfx::Click => Vec::new(),
        }
    }
}

/// Ask for a sound; `at` places it in the world, otherwise it plays as-is.
#[derive(Message, Clone, Copy)]
pub struct PlaySound {
    pub sfx: Sfx,
    pub at: Option<Vec3>,
    pub volume: f32,
}

impl PlaySound {
    pub fn ui(sfx: Sfx) -> Self {
        Self { sfx, at: None, volume: 1.0 }
    }

    pub fn at(sfx: Sfx, at: Vec3) -> Self {
        Self { sfx, at: Some(at), volume: 1.0 }
    }
}

#[derive(Resource)]
struct Sounds {
    effects: HashMap<Sfx, Vec<Handle<AudioSource>>>,
    wind: Handle<AudioSource>,
    birds: Handle<AudioSource>,
    crickets: Handle<AudioSource>,
}

fn setup(mut commands: Commands, server: Res<AssetServer>, mut sources: ResMut<Assets<AudioSource>>) {
    let mut synth = |samples: Vec<f32>| sources.add(AudioSource { bytes: Arc::from(wav(&samples)) });
    let mut effects: HashMap<Sfx, Vec<Handle<AudioSource>>> = HashMap::new();
    for sfx in ALL {
        let files = sfx.files();
        let takes = if files.is_empty() {
            (0..3).map(|seed| synth(synthesize(sfx, seed))).collect()
        } else {
            files.into_iter().map(|f| server.load(f)).collect()
        };
        effects.insert(sfx, takes);
    }
    let wind = synth(wind());
    commands.insert_resource(Sounds {
        effects,
        wind,
        birds: server.load("sounds/birds.ogg"),
        crickets: server.load("sounds/crickets.ogg"),
    });
}

const ALL: [Sfx; 18] = [
    Sfx::DigSoft,
    Sfx::DigStone,
    Sfx::DigWood,
    Sfx::Place,
    Sfx::StepGrass,
    Sfx::StepSnow,
    Sfx::StepWood,
    Sfx::StepStone,
    Sfx::Hit,
    Sfx::Hurt,
    Sfx::Groan,
    Sfx::Baa,
    Sfx::ChestOpen,
    Sfx::ChestClose,
    Sfx::Page,
    Sfx::Inventory,
    Sfx::Splash,
    Sfx::Click,
];

fn add_listener(mut commands: Commands, cameras: Query<Entity, (With<PlayerCamera>, Without<SpatialListener>)>) {
    for camera in &cameras {
        commands.entity(camera).insert(SpatialListener::new(0.3));
    }
}

fn play(mut commands: Commands, mut requests: MessageReader<PlaySound>, sounds: Res<Sounds>, mut counter: Local<u32>) {
    for request in requests.read() {
        let Some(takes) = sounds.effects.get(&request.sfx) else { continue };
        *counter = counter.wrapping_add(1);
        let take = takes[*counter as usize % takes.len()].clone();
        let pitch = 0.92 + (counter.wrapping_mul(2_654_435_761) >> 24) as f32 / 255.0 * 0.16;
        let settings = PlaybackSettings {
            mode: PlaybackMode::Despawn,
            volume: Volume::Linear(request.volume),
            speed: pitch,
            spatial: request.at.is_some(),
            spatial_scale: Some(SpatialScale::new(0.15)),
            ..PlaybackSettings::DESPAWN
        };
        let mut sound = commands.spawn((AudioPlayer(take), settings));
        if let Some(at) = request.at {
            sound.insert(Transform::from_translation(at));
        }
    }
}

#[derive(Component)]
enum Ambience {
    Wind,
    Birds,
    Crickets,
}

fn start_ambience(mut commands: Commands, sounds: Res<Sounds>) {
    for (kind, source) in [
        (Ambience::Wind, sounds.wind.clone()),
        (Ambience::Birds, sounds.birds.clone()),
        (Ambience::Crickets, sounds.crickets.clone()),
    ] {
        commands.spawn((
            DespawnOnExit(AppState::Game),
            kind,
            AudioPlayer(source),
            PlaybackSettings { volume: Volume::Linear(0.0), ..PlaybackSettings::LOOP },
        ));
    }
}

/// Wind always, birds by day, crickets by night.
fn ambience(time_of_day: Option<Res<TimeOfDay>>, mut loops: Query<(&Ambience, &mut AudioSink)>) {
    let Some(time_of_day) = time_of_day else { return };
    let night = if time_of_day.is_night() { 1.0 } else { 0.0 };
    for (kind, mut sink) in &mut loops {
        let volume = match kind {
            Ambience::Wind => 0.18 - 0.08 * night,
            Ambience::Birds => 0.35 * (1.0 - night),
            Ambience::Crickets => 0.3 * night,
        };
        sink.set_volume(Volume::Linear(volume));
    }
}

fn footsteps(
    time: Res<Time>,
    terrain: Res<Terrain>,
    player: Single<(&Body, &Transform), With<Player>>,
    mut sounds: MessageWriter<PlaySound>,
    mut walked: Local<f32>,
    mut was_in_water: Local<bool>,
) {
    let (body, transform) = player.into_inner();
    if body.in_water && !*was_in_water && body.velocity.y < -2.0 {
        sounds.write(PlaySound { volume: 0.8, ..PlaySound::at(Sfx::Splash, transform.translation) });
    }
    *was_in_water = body.in_water;
    if body.ground_normal.is_none() || body.flying {
        return;
    }
    *walked += body.velocity.with_y(0.0).length() * time.delta_secs();
    if *walked > STRIDE {
        *walked = 0.0;
        let under = terrain.block(((transform.translation - Vec3::Y * 0.1) / CELL).floor().as_ivec3());
        let sfx = match under.material {
            Material::Snow => Sfx::StepSnow,
            Material::Planks | Material::Wood => Sfx::StepWood,
            Material::Stone | Material::Bricks | Material::CoalOre | Material::IronOre | Material::DiamondOre => {
                Sfx::StepStone
            }
            _ => Sfx::StepGrass,
        };
        sounds.write(PlaySound { volume: 0.5, ..PlaySound::ui(sfx) });
    }
}

fn hurt(vitals: Res<Vitals>, mut sounds: MessageWriter<PlaySound>, mut last: Local<f32>) {
    if vitals.health < *last {
        sounds.write(PlaySound::ui(Sfx::Hurt));
    }
    *last = vitals.health;
}

/// Zombies groan and sheep bleat now and then, about every eight seconds each.
fn creatures(
    time: Res<Time>,
    mobs: Query<(Entity, &Mob, &Transform)>,
    player: Single<&Transform, With<Player>>,
    mut sounds: MessageWriter<PlaySound>,
    mut frame: Local<u32>,
) {
    *frame = frame.wrapping_add(1);
    for (entity, mob, transform) in &mobs {
        if transform.translation.distance(player.translation) > 30.0 {
            continue;
        }
        let mut rng = Rng((entity.to_bits() as u32) ^ frame.wrapping_mul(0x9e37_79b9) | 1);
        if rng.noise() * 0.5 + 0.5 < time.delta_secs() / 8.0 {
            let sfx = match mob.kind {
                MobKind::Zombie => Sfx::Groan,
                MobKind::Sheep => Sfx::Baa,
            };
            sounds.write(PlaySound::at(sfx, transform.translation + Vec3::Y));
        }
    }
}

/// Mono 16-bit PCM WAV.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = samples.len() as u32 * 2;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend(b"RIFF");
    out.extend((36 + data_len).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(RATE.to_le_bytes());
    out.extend((RATE * 2).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data_len.to_le_bytes());
    for s in samples {
        out.extend(((s.clamp(-1.0, 1.0) * 32_000.0) as i16).to_le_bytes());
    }
    out
}

struct Rng(u32);

impl Rng {
    fn noise(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

/// One-pole low-pass filter; `k` near 0 is muffled, 1 is open.
struct LowPass(f32);

impl LowPass {
    fn run(&mut self, x: f32, k: f32) -> f32 {
        self.0 += (x - self.0) * k;
        self.0
    }
}

fn render(seconds: f32, mut f: impl FnMut(f32) -> f32) -> Vec<f32> {
    (0..(seconds * RATE as f32) as usize).map(|i| f(i as f32 / RATE as f32)).collect()
}

fn sine(hz: f32, t: f32) -> f32 {
    (TAU * hz * t).sin()
}

/// The few sounds made in code rather than recorded.
fn synthesize(sfx: Sfx, seed: u32) -> Vec<f32> {
    let mut rng = Rng(0x9e37_79b9 ^ (seed + 1).wrapping_mul(0x85eb_ca6b) ^ sfx as u32);
    let mut lp = LowPass(0.0);
    match sfx {
        Sfx::Splash => render(0.5, |t| {
            let raw = rng.noise();
            (raw - lp.run(raw, 0.3)) * 1.2 * (t * 80.0).min(1.0) * (-t * 7.0).exp()
        }),
        _ => render(0.03, |t| sine(1200.0 + seed as f32 * 60.0, t) * (-t * 160.0).exp() * 0.5),
    }
}

/// Eight seconds of breeze; the gusts repeat exactly, so the loop has no seam.
fn wind() -> Vec<f32> {
    const SECONDS: f32 = 8.0;
    let mut rng = Rng(12345);
    let mut lp = LowPass(0.0);
    let mut samples = render(SECONDS, |t| {
        let gust = 0.55 + 0.3 * sine(1.0 / SECONDS, t) + 0.15 * sine(3.0 / SECONDS, t);
        lp.run(rng.noise(), 0.02 + 0.02 * gust) * 6.0 * gust
    });
    crossfade_loop(&mut samples);
    samples
}

/// Blends the end into the start so a looping sample doesn't click.
fn crossfade_loop(samples: &mut [f32]) {
    let fade = (RATE as usize) / 2;
    let n = samples.len();
    for i in 0..fade {
        let w = i as f32 / fade as f32;
        samples[i] = samples[i] * w + samples[n - fade + i] * (1.0 - w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_are_valid_and_audible() {
        for sfx in [Sfx::Splash, Sfx::Click] {
            let samples = synthesize(sfx, 0);
            assert!(!samples.is_empty());
            let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.05 && peak.is_finite(), "{sfx:?} peak {peak}");
        }
        let bytes = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 6);
    }
}
