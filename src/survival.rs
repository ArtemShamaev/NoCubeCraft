//! Health, hunger, falling and dying.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    Creative, Mode,
    body::{Body, MoveBodies},
    player::Player,
};

pub const MAX_HEALTH: f32 = 20.0;
pub const MAX_HUNGER: f32 = 20.0;
/// Seconds per point of hunger.
const HUNGER_SECS: f32 = 35.0;
/// While starving, a point of damage this often.
const STARVE_SECS: f32 = 5.0;
/// While well fed, a point of health this often.
const REGEN_SECS: f32 = 4.0;
const WELL_FED: f32 = 16.0;
/// Falls shorter than this are free.
const SAFE_FALL: f32 = 3.5;
const GRAVITY: f32 = 24.0;

pub struct SurvivalPlugin;

impl Plugin for SurvivalPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (fall_damage.after(MoveBodies), metabolism, die).chain().run_if(|mode: Option<Res<State<Mode>>>| {
                mode.is_some_and(|m| matches!(**m, Mode::Playing | Mode::Inventory | Mode::Console))
            }),
        );
    }
}

#[derive(Resource, Clone, Serialize, Deserialize)]
pub struct Vitals {
    pub health: f32,
    pub hunger: f32,
    /// Seconds left of the red flash after taking damage.
    #[serde(skip)]
    pub hurt: f32,
    #[serde(skip)]
    clock: f32,
}

impl Default for Vitals {
    fn default() -> Self {
        Self { health: MAX_HEALTH, hunger: MAX_HUNGER, hurt: 0.0, clock: 0.0 }
    }
}

impl Vitals {
    pub fn damage(&mut self, amount: f32) {
        self.health = (self.health - amount).max(0.0);
        self.hurt = 0.4;
    }

    /// Returns false if the player isn't hungry or hurt, so food isn't wasted.
    pub fn eat(&mut self, hunger: f32, health: f32) -> bool {
        if self.hunger >= MAX_HUNGER && self.health >= MAX_HEALTH {
            return false;
        }
        self.hunger = (self.hunger + hunger).min(MAX_HUNGER);
        self.health = (self.health + health).min(MAX_HEALTH);
        true
    }
}

/// `_alive` keeps hunger from ticking while the player waits to (re)spawn.
fn metabolism(
    time: Res<Time>,
    creative: Res<Creative>,
    mut vitals: ResMut<Vitals>,
    _alive: Single<(), (With<Player>, With<Body>)>,
) {
    let dt = time.delta_secs();
    vitals.hurt = (vitals.hurt - dt).max(0.0);
    if vitals.health <= 0.0 || creative.0 {
        return;
    }
    vitals.hunger = (vitals.hunger - dt / HUNGER_SECS).max(0.0);
    vitals.clock += dt;
    if vitals.hunger <= 0.0 && vitals.clock >= STARVE_SECS {
        vitals.clock = 0.0;
        vitals.damage(1.0);
    } else if vitals.hunger >= WELL_FED && vitals.health < MAX_HEALTH && vitals.clock >= REGEN_SECS {
        vitals.clock = 0.0;
        vitals.health = (vitals.health + 1.0).min(MAX_HEALTH);
    }
}

fn fall_damage(creative: Res<Creative>, mut vitals: ResMut<Vitals>, player: Single<&Body, With<Player>>) {
    if creative.0 {
        return;
    }
    let height = player.impact * player.impact / (2.0 * GRAVITY);
    if height > SAFE_FALL {
        vitals.damage((height - SAFE_FALL).ceil());
    }
}

fn die(vitals: Res<Vitals>, mut mode: ResMut<NextState<Mode>>) {
    if vitals.health <= 0.0 {
        mode.set(Mode::Dead);
    }
}
