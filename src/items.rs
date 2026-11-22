use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::terrain::Material;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Item {
    Dirt,
    Stone,
    Sand,
    Planks,
    Coal,
    IronOre,
    IronIngot,
    Wood,
    Bricks,
    Snow,
    Leaves,
    Stick,
    StonePickaxe,
    StoneShovel,
    StoneAxe,
    IronPickaxe,
    IronShovel,
    IronAxe,
    Furnace,
    Torch,
    Bed,
    Mutton,
    CookedMutton,
    Wool,
    Seeds,
    Wheat,
    Bread,
    WoodenPickaxe,
    WoodenShovel,
    WoodenAxe,
    WoodenSword,
    StoneSword,
    IronSword,
    DiamondPickaxe,
    DiamondShovel,
    DiamondAxe,
    DiamondSword,
    Diamond,
    Charcoal,
    CraftingTable,
    Chest,
    Sandstone,
    Gravel,
    Clay,
    Ice,
    Cactus,
    Beef,
    CookedBeef,
    Porkchop,
    CookedPorkchop,
    Chicken,
    CookedChicken,
    Glue,
    Brace,
    Activator,
    Shardstone,
    Core,
    Rift,
    Bow,
    Arrow,
    Bone,
    Thread,
    Feather,
    Heart,
    Coin,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolKind {
    Pickaxe,
    Shovel,
    Axe,
    Sword,
}

#[derive(Clone, Copy, Debug)]
pub struct Tool {
    pub kind: ToolKind,
    /// 0 wood, 1 stone, 2 iron, 3 diamond.
    pub tier: u8,
    /// Speed multiplier on the materials the tool is made for.
    pub speed: f32,
    /// Blocks (25 cm) dug before the tool breaks.
    pub durability: u32,
    /// Damage per hit.
    pub damage: f32,
}

/// Things that stand in the world as objects rather than voxels.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum PropKind {
    Furnace,
    Torch,
    Bed,
    Wheat,
    CraftingTable,
    Chest,
}

impl Item {
    pub fn name(self) -> &'static str {
        match self {
            Item::Dirt => "Земля",
            Item::Stone => "Камень",
            Item::Sand => "Песок",
            Item::Planks => "Доски",
            Item::Coal => "Уголь",
            Item::IronOre => "Железная руда",
            Item::IronIngot => "Железный слиток",
            Item::Wood => "Бревно",
            Item::Bricks => "Каменный кирпич",
            Item::Snow => "Снег",
            Item::Leaves => "Листва",
            Item::Stick => "Палка",
            Item::WoodenPickaxe => "Деревянная кирка",
            Item::WoodenShovel => "Деревянная лопата",
            Item::WoodenAxe => "Деревянный топор",
            Item::WoodenSword => "Деревянный меч",
            Item::StonePickaxe => "Каменная кирка",
            Item::StoneShovel => "Каменная лопата",
            Item::StoneAxe => "Каменный топор",
            Item::StoneSword => "Каменный меч",
            Item::IronPickaxe => "Железная кирка",
            Item::IronShovel => "Железная лопата",
            Item::IronAxe => "Железный топор",
            Item::IronSword => "Железный меч",
            Item::DiamondPickaxe => "Алмазная кирка",
            Item::DiamondShovel => "Алмазная лопата",
            Item::DiamondAxe => "Алмазный топор",
            Item::DiamondSword => "Алмазный меч",
            Item::Diamond => "Алмаз",
            Item::Charcoal => "Древесный уголь",
            Item::CraftingTable => "Верстак",
            Item::Chest => "Сундук",
            Item::Furnace => "Печь",
            Item::Torch => "Факел",
            Item::Bed => "Кровать",
            Item::Mutton => "Сырая баранина",
            Item::CookedMutton => "Жареная баранина",
            Item::Wool => "Шерсть",
            Item::Seeds => "Семена пшеницы",
            Item::Wheat => "Пшеница",
            Item::Bread => "Хлеб",
            Item::Sandstone => "Песчаник",
            Item::Gravel => "Гравий",
            Item::Clay => "Глина",
            Item::Ice => "Лёд",
            Item::Cactus => "Кактус",
            Item::Beef => "Сырая говядина",
            Item::CookedBeef => "Стейк",
            Item::Porkchop => "Сырая свинина",
            Item::CookedPorkchop => "Жареная свинина",
            Item::Chicken => "Сырая курица",
            Item::CookedChicken => "Жареная курица",
            Item::Glue => "Клей",
            Item::Brace => "Скоба",
            Item::Activator => "Физический активатор",
            Item::Shardstone => "Осколочный камень",
            Item::Core => "Ядро осколка",
            Item::Rift => "Разлом",
            Item::Bow => "Лук",
            Item::Arrow => "Стрела",
            Item::Bone => "Кость",
            Item::Thread => "Нить",
            Item::Feather => "Перо",
            Item::Heart => "Сердце Осколков",
            Item::Coin => "Монета",
        }
    }

    pub fn max_stack(self) -> u32 {
        if self.tool().is_some() || matches!(self, Item::Bow | Item::Heart) { 1 } else { 999 }
    }

    pub fn tool(self) -> Option<Tool> {
        use ToolKind::*;
        let (kind, tier) = match self {
            Item::WoodenPickaxe => (Pickaxe, 0),
            Item::WoodenShovel => (Shovel, 0),
            Item::WoodenAxe => (Axe, 0),
            Item::WoodenSword => (Sword, 0),
            Item::StonePickaxe => (Pickaxe, 1),
            Item::StoneShovel => (Shovel, 1),
            Item::StoneAxe => (Axe, 1),
            Item::StoneSword => (Sword, 1),
            Item::IronPickaxe => (Pickaxe, 2),
            Item::IronShovel => (Shovel, 2),
            Item::IronAxe => (Axe, 2),
            Item::IronSword => (Sword, 2),
            Item::DiamondPickaxe => (Pickaxe, 3),
            Item::DiamondShovel => (Shovel, 3),
            Item::DiamondAxe => (Axe, 3),
            Item::DiamondSword => (Sword, 3),
            _ => return None,
        };
        let speed = [2.0, 3.0, 5.0, 7.0][tier as usize];
        let durability = [1500, 4000, 15000, 50000][tier as usize];
        let damage = match kind {
            Sword => 4.0 + tier as f32,
            Axe => 3.0 + tier as f32,
            _ => 2.0 + tier as f32 * 0.5,
        };
        Some(Tool { kind, tier, speed, durability, damage })
    }

    /// The voxel material this item builds with.
    pub fn material(self) -> Option<Material> {
        match self {
            Item::Dirt => Some(Material::Dirt),
            Item::Stone => Some(Material::Stone),
            Item::Sand => Some(Material::Sand),
            Item::Planks => Some(Material::Planks),
            Item::Wood => Some(Material::Wood),
            Item::Bricks => Some(Material::Bricks),
            Item::Snow => Some(Material::Snow),
            Item::Leaves => Some(Material::Leaves),
            Item::Sandstone => Some(Material::Sandstone),
            Item::Gravel => Some(Material::Gravel),
            Item::Clay => Some(Material::Clay),
            Item::Ice => Some(Material::Ice),
            Item::Cactus => Some(Material::Cactus),
            Item::Brace => Some(Material::Brace),
            Item::Activator => Some(Material::Activator),
            Item::Shardstone => Some(Material::Shardstone),
            Item::Rift => Some(Material::Rift),
            _ => None,
        }
    }

    /// The item a dug-out block turns into, given the pickaxe tier in hand (if any):
    /// iron needs stone or better, diamond needs iron or better.
    pub fn mined(material: Material, pickaxe: Option<u8>) -> Option<Item> {
        match material {
            Material::Air => None,
            Material::Grass | Material::Dirt => Some(Item::Dirt),
            Material::Stone => Some(Item::Stone),
            Material::Sand => Some(Item::Sand),
            Material::Planks => Some(Item::Planks),
            Material::Wood => Some(Item::Wood),
            Material::Bricks => Some(Item::Bricks),
            Material::Snow => Some(Item::Snow),
            Material::Leaves => Some(Item::Leaves),
            Material::CoalOre => Some(Item::Coal),
            Material::IronOre => pickaxe.is_some_and(|t| t >= 1).then_some(Item::IronOre),
            Material::DiamondOre => pickaxe.is_some_and(|t| t >= 2).then_some(Item::Diamond),
            Material::Sandstone => Some(Item::Sandstone),
            Material::Gravel => Some(Item::Gravel),
            Material::Clay => Some(Item::Clay),
            Material::Ice => Some(Item::Ice),
            Material::Cactus => Some(Item::Cactus),
            Material::Brace => Some(Item::Brace),
            Material::Activator => Some(Item::Activator),
            Material::Shardstone => Some(Item::Shardstone),
            Material::Core => pickaxe.is_some_and(|t| t >= 2).then_some(Item::Core),
            Material::Rift => Some(Item::Rift),
            Material::Cache | Material::Altar => None,
        }
    }

    /// Every item, for the creative catalogue and commands.
    pub const ALL: [Item; 65] = [
        Item::Dirt,
        Item::Stone,
        Item::Sand,
        Item::Planks,
        Item::Wood,
        Item::Bricks,
        Item::Snow,
        Item::Leaves,
        Item::Sandstone,
        Item::Gravel,
        Item::Clay,
        Item::Ice,
        Item::Cactus,
        Item::Coal,
        Item::Charcoal,
        Item::IronOre,
        Item::IronIngot,
        Item::Diamond,
        Item::Stick,
        Item::CraftingTable,
        Item::Furnace,
        Item::Chest,
        Item::Glue,
        Item::Brace,
        Item::Activator,
        Item::Rift,
        Item::Shardstone,
        Item::Core,
        Item::Torch,
        Item::Bed,
        Item::WoodenPickaxe,
        Item::WoodenShovel,
        Item::WoodenAxe,
        Item::WoodenSword,
        Item::StonePickaxe,
        Item::StoneShovel,
        Item::StoneAxe,
        Item::StoneSword,
        Item::IronPickaxe,
        Item::IronShovel,
        Item::IronAxe,
        Item::IronSword,
        Item::DiamondPickaxe,
        Item::DiamondShovel,
        Item::DiamondAxe,
        Item::DiamondSword,
        Item::Bow,
        Item::Arrow,
        Item::Mutton,
        Item::CookedMutton,
        Item::Beef,
        Item::CookedBeef,
        Item::Porkchop,
        Item::CookedPorkchop,
        Item::Chicken,
        Item::CookedChicken,
        Item::Wool,
        Item::Thread,
        Item::Feather,
        Item::Bone,
        Item::Seeds,
        Item::Wheat,
        Item::Bread,
        Item::Heart,
        Item::Coin,
    ];

    /// Name used by console commands.
    pub fn id(self) -> String {
        let debug = format!("{self:?}");
        let mut id = String::new();
        for (i, c) in debug.chars().enumerate() {
            if c.is_uppercase() && i > 0 {
                id.push('_');
            }
            id.push(c.to_ascii_lowercase());
        }
        id
    }

    /// Hunger restored and health healed.
    pub fn food(self) -> Option<(f32, f32)> {
        match self {
            Item::Mutton => Some((3.0, 1.0)),
            Item::CookedMutton => Some((8.0, 4.0)),
            Item::Bread => Some((6.0, 2.0)),
            Item::Beef | Item::Porkchop => Some((3.0, 1.0)),
            Item::CookedBeef | Item::CookedPorkchop => Some((9.0, 5.0)),
            Item::Chicken => Some((2.0, 0.0)),
            Item::CookedChicken => Some((7.0, 3.0)),
            _ => None,
        }
    }

    pub fn prop(self) -> Option<PropKind> {
        match self {
            Item::Furnace => Some(PropKind::Furnace),
            Item::Torch => Some(PropKind::Torch),
            Item::Bed => Some(PropKind::Bed),
            Item::Seeds => Some(PropKind::Wheat),
            Item::CraftingTable => Some(PropKind::CraftingTable),
            Item::Chest => Some(PropKind::Chest),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Stack {
    pub item: Item,
    pub count: u32,
    /// Durability used up, for tools.
    pub wear: u32,
}

impl Stack {
    pub fn new(item: Item, count: u32) -> Self {
        Self { item, count, wear: 0 }
    }
}

pub const HOTBAR: usize = 9;
pub const SLOTS: usize = 36;

/// The player's items. The first [`HOTBAR`] slots are the hotbar.
#[derive(Resource, Clone, Serialize, Deserialize)]
pub struct Inventory {
    pub slots: Vec<Option<Stack>>,
    pub selected: usize,
}

impl Default for Inventory {
    fn default() -> Self {
        Self { slots: vec![None; SLOTS], selected: 0 }
    }
}

impl Inventory {
    /// Adds items, filling existing stacks first. Returns how many didn't fit.
    pub fn add(&mut self, item: Item, mut count: u32) -> u32 {
        let max = item.max_stack();
        for slot in self.slots.iter_mut().flatten().filter(|s| s.item == item) {
            let n = count.min(max - slot.count.min(max));
            slot.count += n;
            count -= n;
        }
        // Empty hotbar slots come first, so new things are at hand.
        for slot in self.slots.iter_mut().filter(|s| s.is_none()) {
            if count == 0 {
                break;
            }
            let n = count.min(max);
            *slot = Some(Stack::new(item, n));
            count -= n;
        }
        count
    }

    /// Adds a stack as is, keeping a tool's wear. Returns how many didn't fit.
    pub fn add_stack(&mut self, stack: Stack) -> u32 {
        if stack.item.tool().is_none() {
            return self.add(stack.item, stack.count);
        }
        match self.slots.iter_mut().find(|s| s.is_none()) {
            Some(slot) => {
                *slot = Some(stack);
                0
            }
            None => stack.count,
        }
    }

    /// Whether at least one more `item` fits.
    pub fn has_room(&self, item: Item) -> bool {
        self.slots.iter().any(|s| s.is_none_or(|s| s.item == item && s.count < item.max_stack()))
    }

    pub fn count(&self, item: Item) -> u32 {
        self.slots.iter().flatten().filter(|s| s.item == item).map(|s| s.count).sum()
    }

    /// Removes `count` items if there are that many; otherwise removes nothing.
    pub fn remove(&mut self, item: Item, mut count: u32) -> bool {
        if self.count(item) < count {
            return false;
        }
        // Take from the back so the hotbar keeps its stacks the longest.
        for slot in self.slots.iter_mut().rev() {
            let Some(stack) = slot.filter(|s| s.item == item) else { continue };
            let n = count.min(stack.count);
            count -= n;
            *slot = (stack.count > n).then(|| Stack { count: stack.count - n, ..stack });
            if count == 0 {
                break;
            }
        }
        true
    }

    pub fn selected(&self) -> Option<Stack> {
        self.slots[self.selected]
    }

    /// Uses up `amount` of the selected stack.
    pub fn consume_selected(&mut self, amount: u32) {
        let slot = &mut self.slots[self.selected];
        if let Some(stack) = slot {
            stack.count = stack.count.saturating_sub(amount);
            if stack.count == 0 {
                *slot = None;
            }
        }
    }

    /// Wears the selected tool; it breaks when worn out.
    pub fn wear_selected(&mut self, amount: u32) {
        let slot = &mut self.slots[self.selected];
        let Some(stack) = slot else { return };
        let Some(tool) = stack.item.tool() else { return };
        stack.wear += amount;
        if stack.wear >= tool.durability {
            *slot = None;
        }
    }

    pub fn selected_tool(&self) -> Option<Tool> {
        self.selected().and_then(|s| s.item.tool())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_stacks_then_overflows() {
        let mut inventory = Inventory::default();
        assert_eq!(inventory.add(Item::Dirt, 1500), 0);
        assert_eq!(inventory.slots[0].unwrap().count, 999);
        assert_eq!(inventory.slots[1].unwrap().count, 501);
        assert_eq!(inventory.add(Item::Dirt, 10), 0);
        assert_eq!(inventory.slots[1].unwrap().count, 511);

        assert_eq!(inventory.add(Item::StoneAxe, 2), 0);
        assert_eq!(inventory.count(Item::StoneAxe), 2);
        assert_eq!(inventory.slots[3].unwrap().count, 1);

        let mut full = Inventory::default();
        assert_eq!(full.add(Item::Stone, 999 * SLOTS as u32 + 5), 5);
    }

    #[test]
    fn remove_is_all_or_nothing() {
        let mut inventory = Inventory::default();
        inventory.add(Item::Stick, 1020);
        assert!(!inventory.remove(Item::Stick, 1021));
        assert_eq!(inventory.count(Item::Stick), 1020);
        assert!(inventory.remove(Item::Stick, 1000));
        assert_eq!(inventory.count(Item::Stick), 20);
        assert!(inventory.remove(Item::Stick, 20));
        assert!(inventory.slots.iter().all(Option::is_none));
    }

    #[test]
    fn ids_are_snake_case() {
        assert_eq!(Item::IronPickaxe.id(), "iron_pickaxe");
        assert_eq!(Item::Dirt.id(), "dirt");
    }

    #[test]
    fn worn_out_tool_breaks() {
        let mut inventory = Inventory::default();
        inventory.add(Item::StoneShovel, 1);
        inventory.wear_selected(3999);
        assert!(inventory.selected().is_some());
        inventory.wear_selected(1);
        assert!(inventory.selected().is_none());
    }
}
