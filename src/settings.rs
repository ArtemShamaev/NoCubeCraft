//! Player settings, kept between runs.

use std::fs;

use bevy::{
    prelude::*,
    window::{MonitorSelection, PresentMode, WindowMode},
};
use serde::{Deserialize, Serialize};

use crate::{player::PlayerCamera, terrain::ViewDistance, worlds::data_dir};

const FILE: &str = "settings.ron";

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Settings::load()).add_systems(Update, apply);
    }
}

#[derive(Resource, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Metres of terrain drawn.
    pub view_distance: f32,
    /// Vertical field of view, degrees.
    pub fov: f32,
    pub mouse_sensitivity: f32,
    pub fullscreen: bool,
    pub vsync: bool,
    pub shadows: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { view_distance: 300.0, fov: 75.0, mouse_sensitivity: 1.0, fullscreen: false, vsync: true, shadows: true }
    }
}

impl Settings {
    pub const VIEW_DISTANCES: [f32; 4] = [160.0, 240.0, 300.0, 400.0];

    fn load() -> Self {
        fs::read_to_string(data_dir().join(FILE)).ok().and_then(|t| ron::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let dir = data_dir();
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default()).expect("plain data");
        if let Err(e) = fs::create_dir_all(&dir).and_then(|_| fs::write(dir.join(FILE), text)) {
            error!("could not save settings: {e}");
        }
    }
}

fn apply(
    settings: Res<Settings>,
    mut view: ResMut<ViewDistance>,
    mut window: Single<&mut Window>,
    mut cameras: Query<&mut Projection, With<PlayerCamera>>,
    added: Query<(), Added<PlayerCamera>>,
    mut lights: Query<&mut DirectionalLight>,
) {
    if !settings.is_changed() && added.is_empty() {
        return;
    }
    if view.0 != settings.view_distance {
        view.0 = settings.view_distance;
    }
    window.mode = if settings.fullscreen {
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    };
    window.present_mode = if settings.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync };
    for mut projection in &mut cameras {
        if let Projection::Perspective(p) = &mut *projection {
            p.fov = settings.fov.to_radians();
        }
    }
    for mut light in &mut lights {
        light.shadow_maps_enabled = settings.shadows;
    }
}
