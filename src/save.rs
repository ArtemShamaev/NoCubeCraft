//! Loading a world when the game starts and saving it while playing and on the way out.

use std::{fs, path::Path};

use bevy::{ecs::system::SystemParam, prelude::*, tasks::IoTaskPool};
use bevy_replicon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    AppState, Creative,
    body::Body,
    containers::{ChestState, FurnaceState},
    drops::{self, Drop, Dropped, SavedDrop},
    goals::Goals,
    items::{Inventory, PropKind},
    mobs::{Mob, SavedDog, SavedDogs},
    net::{Guests, Joining, Welcome},
    player::{Player, PlayerStart, Spawning},
    props::{self, Crop, Prop},
    sky::{START_TIME, TimeOfDay},
    survival::Vitals,
    terrain::{EditsSnapshot, Terrain},
    villagers::{self, SavedVillager, Villager},
    villages::{Founded, Village},
    worlds::CurrentWorld,
};

const AUTOSAVE_SECS: f32 = 30.0;
const STATE_FILE: &str = "state.ron";
const EDITS_FILE: &str = "terrain.bin";

pub struct SavePlugin;

/// Runs when entering the game, before anything that needs the world.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoadWorld;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::Game), load.in_set(LoadWorld))
            .add_systems(OnExit(AppState::Game), close)
            .add_systems(Update, autosave.run_if(in_state(AppState::Game)))
            .add_systems(Last, save_on_exit.after(bevy::window::ExitSystems).run_if(in_state(AppState::Game)));
    }
}

#[derive(Serialize, Deserialize)]
struct State {
    position: Vec3,
    yaw: f32,
    pitch: f32,
    spawn: Vec3,
    inventory: Inventory,
    vitals: Vitals,
    time_of_day: f32,
    props: Vec<SavedProp>,
    #[serde(default)]
    dogs: Vec<SavedDog>,
    #[serde(default)]
    goals: Goals,
    #[serde(default)]
    drops: Vec<SavedDrop>,
    #[serde(default)]
    guests: Guests,
    #[serde(default)]
    villages: Vec<Village>,
    #[serde(default)]
    villagers: Vec<SavedVillager>,
    #[serde(default)]
    founded: Founded,
}

/// A player who visits this world over the network: where they are and what they carry.
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedPlayer {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub spawn: Vec3,
    pub inventory: Inventory,
    pub vitals: Vitals,
    #[serde(default)]
    pub goals: Goals,
}

impl SavedPlayer {
    pub fn of(player: &Player, transform: &Transform, inventory: &Inventory, vitals: &Vitals, goals: &Goals) -> Self {
        Self {
            position: transform.translation,
            yaw: player.yaw,
            pitch: player.pitch,
            spawn: player.spawn,
            inventory: inventory.clone(),
            vitals: vitals.clone(),
            goals: *goals,
        }
    }
}

/// Sets a guest up from the host's welcome.
fn enter_as_guest(commands: &mut Commands, welcome: &Welcome) -> Terrain {
    let mut terrain = Terrain::new(welcome.seed);
    if let Err(error) = terrain.restore(&welcome.edits) {
        error!("the host's world did not load: {error}");
    }
    commands.insert_resource(Creative(welcome.creative));
    commands.insert_resource(TimeOfDay(welcome.time));
    match welcome.player.clone() {
        Some(p) => {
            commands.insert_resource(PlayerStart { position: p.position, yaw: p.yaw, pitch: p.pitch, spawn: p.spawn });
            commands.insert_resource(p.inventory);
            commands.insert_resource(if p.vitals.health > 0.0 { p.vitals } else { Vitals::default() });
            commands.insert_resource(p.goals);
        }
        None => {
            let spawn = welcome.spawn;
            commands.insert_resource(PlayerStart { position: spawn, yaw: 0.0, pitch: 0.0, spawn });
            commands.insert_resource(Inventory::default());
            commands.insert_resource(Vitals::default());
            commands.insert_resource(Goals::default());
        }
    }
    terrain
}

#[derive(Serialize, Deserialize)]
struct SavedProp {
    kind: PropKind,
    position: Vec3,
    yaw: f32,
    /// Growth of crops, in seconds.
    age: f32,
    #[serde(default)]
    furnace: Option<FurnaceState>,
    #[serde(default)]
    chest: Option<ChestState>,
}

fn load(
    mut commands: Commands,
    world: Option<ResMut<CurrentWorld>>,
    joining: Option<Res<Joining>>,
    mut drop_writer: MessageWriter<Drop>,
) {
    if let Some(joining) = joining {
        let terrain = enter_as_guest(&mut commands, &joining.0);
        commands.insert_resource(terrain);
        return;
    }
    let Some(mut world) = world else { return };
    world.touch();
    let mut terrain = Terrain::new(world.info.seed);
    commands.insert_resource(Creative(world.info.creative));
    match read(&world.dir, &mut terrain) {
        Ok(Some(mut state)) => {
            info!("loaded {}", world.dir.display());
            // Saved while dead: come back at the spawn point, as the death screen would.
            if state.vitals.health <= 0.0 {
                state.vitals = Vitals::default();
                state.position = state.spawn;
            }
            for p in &state.props {
                let transform = Transform::from_translation(p.position).with_rotation(Quat::from_rotation_y(p.yaw));
                let entity = props::spawn(&mut commands, p.kind, transform, p.age);
                if let Some(furnace) = &p.furnace {
                    commands.entity(entity).insert(furnace.clone());
                }
                if let Some(chest) = &p.chest {
                    commands.entity(entity).insert(chest.clone());
                }
            }
            commands.insert_resource(PlayerStart {
                position: state.position,
                yaw: state.yaw,
                pitch: state.pitch,
                spawn: state.spawn,
            });
            commands.insert_resource(state.inventory);
            commands.insert_resource(state.vitals);
            commands.insert_resource(TimeOfDay(state.time_of_day));
            commands.insert_resource(SavedDogs(state.dogs));
            commands.insert_resource(state.goals);
            drops::restore(&state.drops, &mut drop_writer);
            commands.insert_resource(state.guests);
            commands.insert_resource(state.founded);
            for village in state.villages {
                commands.spawn((DespawnOnExit(AppState::Game), Replicated, village));
            }
            for villager in &state.villagers {
                villagers::restore(&mut commands, villager);
            }
        }
        result => {
            if let Err(error) = result {
                // Keep the broken files around rather than overwriting them.
                error!("could not load {}: {error}", world.dir.display());
                for name in [STATE_FILE, EDITS_FILE] {
                    let _ = fs::rename(world.dir.join(name), world.dir.join(name).with_extension("broken"));
                }
                terrain = Terrain::new(world.info.seed);
            }
            let spawn = terrain.find_spawn();
            commands.insert_resource(PlayerStart { position: spawn, yaw: 0.0, pitch: 0.0, spawn });
            commands.insert_resource(Inventory::default());
            commands.insert_resource(Vitals::default());
            commands.insert_resource(TimeOfDay(START_TIME));
            commands.insert_resource(Goals::default());
            commands.insert_resource(Guests::default());
        }
    }
    commands.insert_resource(terrain);
}

fn read(dir: &Path, terrain: &mut Terrain) -> Result<Option<State>, String> {
    let state = match fs::read_to_string(dir.join(STATE_FILE)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let state: State = ron::from_str(&state).map_err(|e| e.to_string())?;
    let edits = fs::read(dir.join(EDITS_FILE)).map_err(|e| e.to_string())?;
    terrain.restore(&edits)?;
    Ok(Some(state))
}

/// Everything needed to write a save, gathered on the main thread.
struct Snapshot {
    state: State,
    edits: EditsSnapshot,
}

fn write(dir: &Path, snapshot: &Snapshot) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let state = ron::ser::to_string_pretty(&snapshot.state, ron::ser::PrettyConfig::default())
        .map_err(std::io::Error::other)?;
    // Write to temporary files first so a crash mid-save can't corrupt the world.
    let files = [(EDITS_FILE, snapshot.edits.encode()), (STATE_FILE, state.into_bytes())];
    for (name, bytes) in &files {
        fs::write(dir.join(name).with_extension("tmp"), bytes)?;
    }
    for (name, _) in &files {
        fs::rename(dir.join(name).with_extension("tmp"), dir.join(name))?;
    }
    Ok(())
}

fn save(dir: &Path, snapshot: &Snapshot) {
    match write(dir, snapshot) {
        Ok(()) => info!("saved {}", dir.display()),
        Err(e) => error!("could not save {}: {e}", dir.display()),
    }
}

/// Only a player who has finished spawning is saved, so a half-loaded world isn't written.
type Ready = (With<Body>, Without<Spawning>);

#[derive(SystemParam)]
pub struct SaveParams<'w, 's> {
    world: Option<Res<'w, CurrentWorld>>,
    /// Guests don't save; the host keeps their things.
    joining: Option<Res<'w, Joining>>,
    guests: Option<Res<'w, Guests>>,
    terrain: Option<Res<'w, Terrain>>,
    player: Option<Single<'w, 's, (&'static Player, &'static Transform), Ready>>,
    inventory: Option<Res<'w, Inventory>>,
    vitals: Option<Res<'w, Vitals>>,
    time_of_day: Option<Res<'w, TimeOfDay>>,
    goals: Option<Res<'w, Goals>>,
    mobs: Query<'w, 's, (&'static Mob, &'static Transform)>,
    drops: Query<'w, 's, (&'static Dropped, &'static Transform)>,
    villages: Query<'w, 's, &'static Village>,
    villagers: Query<'w, 's, (&'static Villager, &'static Transform)>,
    founded: Option<Res<'w, Founded>>,
    props: Query<
        'w,
        's,
        (
            &'static Prop,
            &'static Transform,
            Option<&'static Crop>,
            Option<&'static FurnaceState>,
            Option<&'static ChestState>,
        ),
    >,
}

impl SaveParams<'_, '_> {
    fn snapshot(&self) -> Option<Snapshot> {
        if self.joining.is_some() || self.world.is_none() {
            return None;
        }
        let (player, transform) = **self.player.as_ref()?;
        let state = State {
            position: transform.translation,
            yaw: player.yaw,
            pitch: player.pitch,
            spawn: player.spawn,
            inventory: (**self.inventory.as_ref()?).clone(),
            vitals: (**self.vitals.as_ref()?).clone(),
            time_of_day: self.time_of_day.as_ref()?.0,
            props: self
                .props
                .iter()
                .map(|(prop, t, crop, furnace, chest)| SavedProp {
                    kind: prop.0,
                    position: t.translation,
                    yaw: t.rotation.to_euler(EulerRot::YXZ).0,
                    age: crop.map_or(0.0, |c| c.age),
                    furnace: furnace.cloned(),
                    chest: chest.cloned(),
                })
                .collect(),
            dogs: self.mobs.iter().filter_map(|(mob, t)| SavedDog::of(mob, t)).collect(),
            goals: *self.goals.as_deref()?,
            drops: self.drops.iter().map(|(d, t)| d.saved(t)).collect(),
            guests: (**self.guests.as_ref()?).clone(),
            villages: self.villages.iter().cloned().collect(),
            villagers: self
                .villagers
                .iter()
                .map(|(v, t)| SavedVillager { villager: v.clone(), at: t.translation })
                .collect(),
            founded: (**self.founded.as_ref()?).clone(),
        };
        Some(Snapshot { state, edits: self.terrain.as_ref()?.edits_snapshot() })
    }
}

fn autosave(time: Res<Time<Real>>, params: SaveParams, mut since: Local<f32>) {
    *since += time.delta_secs();
    if *since < AUTOSAVE_SECS {
        return;
    }
    *since = 0.0;
    if let Some(snapshot) = params.snapshot() {
        let Some(dir) = params.world.as_ref().map(|w| w.dir.clone()) else { return };
        IoTaskPool::get().spawn(async move { save(&dir, &snapshot) }).detach();
    }
}

fn save_on_exit(mut exit: MessageReader<AppExit>, params: SaveParams) {
    if exit.read().next().is_some()
        && let Some(snapshot) = params.snapshot()
        && let Some(world) = &params.world
    {
        save(&world.dir, &snapshot);
    }
}

/// Saves right away; run it before leaving the game, while the player still exists.
pub fn save_now(params: SaveParams) {
    if let Some(snapshot) = params.snapshot()
        && let Some(world) = &params.world
    {
        save(&world.dir, &snapshot);
    }
}

/// Leaving to the menu: drop the world's resources (entities go by themselves).
fn close(mut commands: Commands) {
    commands.remove_resource::<Terrain>();
    commands.remove_resource::<Inventory>();
    commands.remove_resource::<Vitals>();
    commands.remove_resource::<TimeOfDay>();
    commands.remove_resource::<PlayerStart>();
    commands.remove_resource::<Goals>();
    commands.remove_resource::<SavedDogs>();
    commands.remove_resource::<Guests>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::Item;

    #[test]
    fn state_round_trips_through_ron() {
        let mut inventory = Inventory::default();
        inventory.add(Item::IronPickaxe, 1);
        inventory.add(Item::Planks, 40);
        let state = State {
            position: Vec3::new(1.0, 2.0, 3.0),
            yaw: 0.5,
            pitch: -0.2,
            spawn: Vec3::ONE,
            inventory,
            vitals: Vitals::default(),
            time_of_day: 0.7,
            props: vec![SavedProp {
                kind: PropKind::Chest,
                position: Vec3::ZERO,
                yaw: 1.0,
                age: 12.0,
                furnace: None,
                chest: Some(ChestState { slots: vec![Some(crate::items::Stack::new(Item::Diamond, 3)); 2] }),
            }],
            dogs: vec![SavedDog { position: Vec3::X, health: 7.0, sitting: true }],
            goals: Goals { stage: 4 },
            drops: Vec::new(),
            guests: Guests::default(),
            villages: Vec::new(),
            villagers: Vec::new(),
            founded: Founded::default(),
        };
        let text = ron::to_string(&state).unwrap();
        let back: State = ron::from_str(&text).unwrap();
        assert_eq!(back.inventory.count(Item::Planks), 40);
        assert_eq!(back.props[0].age, 12.0);
        assert_eq!(back.props[0].chest.as_ref().unwrap().slots[1].unwrap().count, 3);
        assert_eq!(back.position, state.position);
        assert!(back.dogs[0].sitting);
        assert_eq!(back.goals.stage, 4);
    }
}
