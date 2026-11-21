//! Everything that holds items besides the player: the crafting grid, furnaces and chests,
//! and a common way to address any slot.

use bevy::{ecs::system::SystemParam, prelude::*};
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    crafting::{self, Recipe},
    items::{Inventory, Item, Stack},
};

/// Seconds to smelt one item.
pub const SMELT_SECS: f32 = 5.0;
pub const CHEST_SLOTS: usize = 27;

pub struct ContainersPlugin;

impl Plugin for ContainersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OpenContainer>()
            .init_resource::<CraftGrid>()
            .add_systems(Update, smelt.run_if(in_state(AppState::Game)));
    }
}

/// What the inventory screen shows beside the player's items.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpenContainer {
    /// The player's own 2×2 crafting grid.
    #[default]
    None,
    /// A crafting table: 3×3 grid.
    Workbench,
    Furnace(Entity),
    Chest(Entity),
}

/// Items laid out for crafting; emptied back into the inventory when the screen closes.
#[derive(Resource)]
pub struct CraftGrid {
    pub width: usize,
    pub slots: Vec<Option<Stack>>,
}

impl Default for CraftGrid {
    fn default() -> Self {
        Self { width: 2, slots: vec![None; 4] }
    }
}

impl CraftGrid {
    pub fn recipe(&self) -> Option<&'static Recipe> {
        let items: Vec<Option<Item>> = self.slots.iter().map(|s| s.map(|s| s.item)).collect();
        crafting::find(&items, self.width)
    }

    /// Uses up one item from every filled cell.
    pub fn consume(&mut self) {
        for slot in &mut self.slots {
            if let Some(stack) = slot {
                stack.count -= 1;
                if stack.count == 0 {
                    *slot = None;
                }
            }
        }
    }
}

#[derive(Component, Default, Clone, Serialize, Deserialize)]
pub struct FurnaceState {
    pub input: Option<Stack>,
    pub fuel: Option<Stack>,
    pub output: Option<Stack>,
    /// Seconds of fire left, and how long the current piece of fuel lasts in total.
    pub burn_left: f32,
    pub burn_total: f32,
    /// Of the item being smelted, 0..1.
    pub progress: f32,
}

#[derive(Component, Clone, Serialize, Deserialize)]
pub struct ChestState {
    pub slots: Vec<Option<Stack>>,
}

impl Default for ChestState {
    fn default() -> Self {
        Self { slots: vec![None; CHEST_SLOTS] }
    }
}

impl FurnaceState {
    /// Whether the input can be smelted into the output slot right now.
    fn can_smelt(&self) -> bool {
        let Some(result) = self.input.and_then(|s| crafting::smelt(s.item)) else { return false };
        self.output.is_none_or(|o| o.item == result && o.count < result.max_stack())
    }

    /// Everything inside, for when the furnace is broken.
    pub fn contents(&self) -> impl Iterator<Item = Stack> {
        [self.input, self.fuel, self.output].into_iter().flatten()
    }
}

fn smelt(time: Res<Time>, mut furnaces: Query<&mut FurnaceState>) {
    let dt = time.delta_secs();
    for mut furnace in &mut furnaces {
        let can = furnace.can_smelt();
        if furnace.burn_left <= 0.0
            && can
            && let Some(per_piece) = furnace.fuel.and_then(|f| crafting::burn(f.item))
        {
            furnace.burn_left = per_piece * SMELT_SECS;
            furnace.burn_total = furnace.burn_left;
            take_one(&mut furnace.fuel);
        }
        if furnace.burn_left <= 0.0 {
            furnace.progress = 0.0;
            continue;
        }
        furnace.burn_left -= dt;
        if !can {
            furnace.progress = 0.0;
            continue;
        }
        furnace.progress += dt / SMELT_SECS;
        if furnace.progress >= 1.0 {
            furnace.progress = 0.0;
            let result = furnace.input.and_then(|s| crafting::smelt(s.item)).expect("checked by can_smelt");
            take_one(&mut furnace.input);
            match &mut furnace.output {
                Some(out) => out.count += 1,
                None => furnace.output = Some(Stack::new(result, 1)),
            }
        }
    }
}

fn take_one(slot: &mut Option<Stack>) {
    if let Some(stack) = slot {
        stack.count -= 1;
        if stack.count == 0 {
            *slot = None;
        }
    }
}

/// A slot anywhere on the inventory screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlotRef {
    Inventory(usize),
    Grid(usize),
    /// The crafting result; taking from it crafts.
    Result,
    FurnaceInput,
    FurnaceFuel,
    FurnaceOutput,
    Chest(usize),
}

/// Read access to every slot, for drawing.
#[derive(SystemParam)]
pub struct Slots<'w, 's> {
    inventory: Option<Res<'w, Inventory>>,
    grid: Res<'w, CraftGrid>,
    open: Res<'w, OpenContainer>,
    furnaces: Query<'w, 's, &'static FurnaceState>,
    chests: Query<'w, 's, &'static ChestState>,
}

impl Slots<'_, '_> {
    pub fn get(&self, slot: SlotRef) -> Option<Stack> {
        match (slot, *self.open) {
            (SlotRef::Inventory(i), _) => self.inventory.as_ref()?.slots[i],
            (SlotRef::Grid(i), _) => self.grid.slots.get(i).copied().flatten(),
            (SlotRef::Result, _) => self.grid.recipe().map(|r| Stack::new(r.output, r.count)),
            (SlotRef::FurnaceInput, OpenContainer::Furnace(e)) => self.furnaces.get(e).ok()?.input,
            (SlotRef::FurnaceFuel, OpenContainer::Furnace(e)) => self.furnaces.get(e).ok()?.fuel,
            (SlotRef::FurnaceOutput, OpenContainer::Furnace(e)) => self.furnaces.get(e).ok()?.output,
            (SlotRef::Chest(i), OpenContainer::Chest(e)) => self.chests.get(e).ok()?.slots[i],
            _ => None,
        }
    }

    pub fn furnace(&self) -> Option<&FurnaceState> {
        match *self.open {
            OpenContainer::Furnace(e) => self.furnaces.get(e).ok(),
            _ => None,
        }
    }
}

/// Write access to every slot, for moving items around.
#[derive(SystemParam)]
pub struct SlotsMut<'w, 's> {
    pub inventory: ResMut<'w, Inventory>,
    pub grid: ResMut<'w, CraftGrid>,
    pub open: Res<'w, OpenContainer>,
    furnaces: Query<'w, 's, &'static mut FurnaceState>,
    chests: Query<'w, 's, &'static mut ChestState>,
}

impl SlotsMut<'_, '_> {
    /// The slot's storage, if it exists right now. The crafting result has none.
    pub fn slot(&mut self, slot: SlotRef) -> Option<&mut Option<Stack>> {
        match (slot, *self.open) {
            (SlotRef::Inventory(i), _) => self.inventory.slots.get_mut(i),
            (SlotRef::Grid(i), _) => self.grid.slots.get_mut(i),
            (SlotRef::FurnaceInput, OpenContainer::Furnace(e)) => {
                Some(&mut self.furnaces.get_mut(e).ok()?.into_inner().input)
            }
            (SlotRef::FurnaceFuel, OpenContainer::Furnace(e)) => {
                Some(&mut self.furnaces.get_mut(e).ok()?.into_inner().fuel)
            }
            (SlotRef::FurnaceOutput, OpenContainer::Furnace(e)) => {
                Some(&mut self.furnaces.get_mut(e).ok()?.into_inner().output)
            }
            (SlotRef::Chest(i), OpenContainer::Chest(e)) => self.chests.get_mut(e).ok()?.into_inner().slots.get_mut(i),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn furnace_smelts_with_fuel_over_time() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_systems(Update, smelt);
        let furnace = app
            .world_mut()
            .spawn(FurnaceState {
                input: Some(Stack::new(Item::IronOre, 3)),
                fuel: Some(Stack::new(Item::Stick, 2)),
                ..default()
            })
            .id();
        // Advance a fixed amount of time per update.
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_millis(100)));
        for _ in 0..200 {
            app.update();
        }
        let state = app.world().get::<FurnaceState>(furnace).unwrap();
        // Two sticks burn long enough for one ingot.
        assert_eq!(state.output.map(|s| (s.item, s.count)), Some((Item::IronIngot, 1)));
        assert_eq!(state.input.map(|s| s.count), Some(2));
        assert!(state.fuel.is_none());
    }

    #[test]
    fn grid_crafts_and_consumes() {
        let mut grid = CraftGrid::default();
        grid.slots[1] = Some(Stack::new(Item::Planks, 3));
        grid.slots[3] = Some(Stack::new(Item::Planks, 1));
        assert_eq!(grid.recipe().map(|r| r.output), Some(Item::Stick));
        grid.consume();
        assert_eq!(grid.slots[1].map(|s| s.count), Some(2));
        assert!(grid.slots[3].is_none());
        assert!(grid.recipe().is_none());
    }
}
