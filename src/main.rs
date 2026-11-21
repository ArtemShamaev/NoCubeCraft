mod body;
mod building;
mod console;
mod containers;
mod crafting;
mod effects;
mod hands;
mod hud;
mod icons;
mod interaction;
mod inventory_ui;
mod items;
mod menus;
mod mobs;
mod player;
mod props;
mod save;
mod settings;
mod sky;
mod sound;
mod survival;
mod terrain;
mod ui;
mod worlds;

use avian3d::prelude::*;
use bevy::prelude::*;

/// Title screen or a world being played.
#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum AppState {
    #[default]
    Menu,
    Game,
}

#[derive(SubStates, Default, Debug, Clone, PartialEq, Eq, Hash)]
#[source(AppState = AppState::Menu)]
pub enum MenuPage {
    #[default]
    Main,
    Worlds,
    Create,
    Settings,
}

/// What the player is doing in the game; decides which input is live.
#[derive(SubStates, Default, Debug, Clone, PartialEq, Eq, Hash)]
#[source(AppState = AppState::Game)]
pub enum Mode {
    #[default]
    Playing,
    Inventory,
    Paused,
    Settings,
    Dead,
    Console,
}

/// Creative mode: flying, endless blocks, instant digging, no harm.
#[derive(Resource, Default)]
pub struct Creative(pub bool);

/// `assets` next to the executable when shipped, the source tree's otherwise.
fn asset_root() -> String {
    let beside_exe = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join("assets")));
    match beside_exe {
        Some(dir) if dir.is_dir() => dir.to_string_lossy().into_owned(),
        _ => concat!(env!("CARGO_MANIFEST_DIR"), "/assets").to_string(),
    }
}

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window { title: "Barnocraft".into(), ..default() }),
                    ..default()
                })
                .set(AssetPlugin { file_path: asset_root(), ..default() }),
            PhysicsPlugins::default(),
        ))
        .init_state::<AppState>()
        .add_sub_state::<MenuPage>()
        .add_sub_state::<Mode>()
        .init_resource::<Creative>()
        .add_plugins((
            terrain::TerrainPlugin,
            body::BodyPlugin,
            player::PlayerPlugin,
            props::PropsPlugin,
            save::SavePlugin,
            interaction::InteractionPlugin,
            survival::SurvivalPlugin,
            mobs::MobsPlugin,
            sky::SkyPlugin,
            icons::IconsPlugin,
            hud::HudPlugin,
            inventory_ui::InventoryUiPlugin,
            menus::MenusPlugin,
        ))
        .add_plugins((
            ui::UiPlugin,
            settings::SettingsPlugin,
            console::ConsolePlugin,
            containers::ContainersPlugin,
            effects::EffectsPlugin,
            hands::HandsPlugin,
            sound::SoundPlugin,
        ))
        .run();
}
