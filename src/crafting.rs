//! Recipes in the spirit of Minecraft: patterns laid out on a 2×2 or 3×3 grid, smelting and fuel.

use crate::items::Item;

pub enum Pattern {
    /// Rows of symbols; a space is an empty cell. The layout may sit anywhere on the grid,
    /// and may be mirrored left to right.
    Shaped(&'static [&'static str], &'static [(char, Item)]),
    /// These items in any cells, one each.
    Shapeless(&'static [Item]),
}

pub struct Recipe {
    pub output: Item,
    pub count: u32,
    pub pattern: Pattern,
}

const fn shaped(output: Item, count: u32, rows: &'static [&'static str], key: &'static [(char, Item)]) -> Recipe {
    Recipe { output, count, pattern: Pattern::Shaped(rows, key) }
}

const fn shapeless(output: Item, count: u32, items: &'static [Item]) -> Recipe {
    Recipe { output, count, pattern: Pattern::Shapeless(items) }
}

use Item::*;

const P: (char, Item) = ('P', Planks);
const S: (char, Item) = ('|', Stick);

/// Tools of one tier: pickaxe, axe, shovel and sword with heads of material `$m`.
macro_rules! tools {
    ($m:expr, $pick:expr, $axe:expr, $shovel:expr, $sword:expr) => {
        [
            shaped($pick, 1, &["MMM", " | ", " | "], &[('M', $m), S]),
            shaped($axe, 1, &["MM", "M|", " |"], &[('M', $m), S]),
            shaped($shovel, 1, &["M", "|", "|"], &[('M', $m), S]),
            shaped($sword, 1, &["M", "M", "|"], &[('M', $m), S]),
        ]
    };
}

const WOOD_TOOLS: [Recipe; 4] = tools!(Planks, WoodenPickaxe, WoodenAxe, WoodenShovel, WoodenSword);
const STONE_TOOLS: [Recipe; 4] = tools!(Stone, StonePickaxe, StoneAxe, StoneShovel, StoneSword);
const IRON_TOOLS: [Recipe; 4] = tools!(IronIngot, IronPickaxe, IronAxe, IronShovel, IronSword);
const DIAMOND_TOOLS: [Recipe; 4] = tools!(Diamond, DiamondPickaxe, DiamondAxe, DiamondShovel, DiamondSword);

const BASICS: [Recipe; 11] = [
    shapeless(Planks, 4, &[Wood]),
    shaped(Stick, 4, &["P", "P"], &[P]),
    shaped(CraftingTable, 1, &["PP", "PP"], &[P]),
    shaped(Torch, 4, &["C", "|"], &[('C', Coal), S]),
    shaped(Torch, 4, &["C", "|"], &[('C', Charcoal), S]),
    shaped(Chest, 1, &["PPP", "P P", "PPP"], &[P]),
    shaped(Furnace, 1, &["SSS", "S S", "SSS"], &[('S', Stone)]),
    shaped(Bed, 1, &["WWW", "PPP"], &[('W', Wool), P]),
    shaped(Bread, 1, &["WWW"], &[('W', Wheat)]),
    shaped(Bricks, 4, &["SS", "SS"], &[('S', Stone)]),
    shapeless(Seeds, 1, &[Wheat]),
];

/// Every crafting recipe, in recipe-book order.
pub fn recipes() -> impl Iterator<Item = &'static Recipe> {
    BASICS.iter().chain(&WOOD_TOOLS).chain(&STONE_TOOLS).chain(&IRON_TOOLS).chain(&DIAMOND_TOOLS)
}

impl Recipe {
    /// Edge of the smallest square grid it fits on.
    pub fn size(&self) -> usize {
        match &self.pattern {
            Pattern::Shaped(rows, _) => rows.len().max(rows.iter().map(|r| r.len()).max().unwrap_or(0)),
            Pattern::Shapeless(items) => {
                if items.len() <= 4 {
                    2
                } else {
                    3
                }
            }
        }
    }

    /// Items needed, one per used cell.
    pub fn ingredients(&self) -> Vec<Item> {
        match &self.pattern {
            Pattern::Shaped(rows, key) => rows.iter().flat_map(|r| r.chars()).filter_map(|c| lookup(key, c)).collect(),
            Pattern::Shapeless(items) => items.to_vec(),
        }
    }

    /// Where each ingredient goes on a grid `width` cells wide, from the top left.
    pub fn layout(&self, width: usize) -> Vec<Option<Item>> {
        let mut grid = vec![None; width * width];
        match &self.pattern {
            Pattern::Shaped(rows, key) => {
                for (y, row) in rows.iter().enumerate() {
                    for (x, c) in row.chars().enumerate() {
                        grid[x + y * width] = lookup(key, c);
                    }
                }
            }
            Pattern::Shapeless(items) => {
                for (cell, item) in grid.iter_mut().zip(items.iter()) {
                    *cell = Some(*item);
                }
            }
        }
        grid
    }

    fn matches(&self, grid: &[Option<Item>], width: usize) -> bool {
        match &self.pattern {
            Pattern::Shaped(rows, key) => {
                let Some((x0, y0, w, h)) = bounds(grid, width) else { return false };
                let pw = rows.iter().map(|r| r.len()).max().unwrap_or(0);
                if w != pw || h != rows.len() {
                    return false;
                }
                let fits = |mirror: bool| {
                    (0..h).all(|y| {
                        (0..w).all(|x| {
                            let px = if mirror { w - 1 - x } else { x };
                            let wanted = rows[y].chars().nth(px).and_then(|c| lookup(key, c));
                            grid[x0 + x + (y0 + y) * width] == wanted
                        })
                    })
                };
                fits(false) || fits(true)
            }
            Pattern::Shapeless(items) => {
                let mut have: Vec<Item> = grid.iter().flatten().copied().collect();
                if have.len() != items.len() {
                    return false;
                }
                items.iter().all(|item| match have.iter().position(|h| h == item) {
                    Some(i) => {
                        have.swap_remove(i);
                        true
                    }
                    None => false,
                })
            }
        }
    }
}

fn lookup(key: &[(char, Item)], c: char) -> Option<Item> {
    key.iter().find(|(k, _)| *k == c).map(|(_, item)| *item)
}

/// Smallest rectangle holding every filled cell: x, y, width, height.
fn bounds(grid: &[Option<Item>], width: usize) -> Option<(usize, usize, usize, usize)> {
    let filled: Vec<(usize, usize)> =
        grid.iter().enumerate().filter(|(_, c)| c.is_some()).map(|(i, _)| (i % width, i / width)).collect();
    let x0 = filled.iter().map(|p| p.0).min()?;
    let y0 = filled.iter().map(|p| p.1).min()?;
    let x1 = filled.iter().map(|p| p.0).max()?;
    let y1 = filled.iter().map(|p| p.1).max()?;
    Some((x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

/// The recipe laid out on a `width`×`width` grid, if any.
pub fn find(grid: &[Option<Item>], width: usize) -> Option<&'static Recipe> {
    recipes().find(|r| r.size() <= width && r.matches(grid, width))
}

/// What a furnace turns an item into.
pub fn smelt(item: Item) -> Option<Item> {
    match item {
        IronOre => Some(IronIngot),
        Mutton => Some(CookedMutton),
        Wood => Some(Charcoal),
        _ => None,
    }
}

/// How many items one piece of fuel smelts.
pub fn burn(item: Item) -> Option<f32> {
    match item {
        Coal | Charcoal => Some(8.0),
        Wood | Planks | CraftingTable | Chest => Some(1.5),
        Stick => Some(0.5),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaped_recipes_match_anywhere_and_mirrored() {
        let (p, s, n) = (Some(Planks), Some(Stick), None);
        // A wooden axe on a 3×3 grid, mirrored.
        let axe = [p, p, n, s, p, n, s, n, n];
        assert_eq!(find(&axe, 3).map(|r| r.output), Some(WoodenAxe));
        // Sticks in the right column of a 2×2 grid.
        assert_eq!(find(&[n, p, n, p], 2).map(|r| (r.output, r.count)), Some((Stick, 4)));
        // A pickaxe needs the 3×3 grid of a crafting table.
        let pick = [p, p, p, n, s, n, n, s, n];
        assert_eq!(find(&pick, 3).map(|r| r.output), Some(WoodenPickaxe));
        assert!(find(&[p, p, n, n], 2).is_none());
    }

    #[test]
    fn layouts_match_their_recipe() {
        assert_eq!(find(&[None, None, Some(Wood), None], 2).map(|r| r.output), Some(Planks));
        for recipe in recipes() {
            let width = recipe.size().max(2);
            let found = find(&recipe.layout(width), width).expect("layout matches");
            assert_eq!((found.output, found.count), (recipe.output, recipe.count));
            assert_eq!(recipe.ingredients().len(), recipe.layout(width).iter().flatten().count());
        }
    }

    #[test]
    fn every_tool_tier_is_craftable_and_ores_smelt() {
        for tool in [WoodenPickaxe, StoneSword, IronAxe, DiamondShovel] {
            assert!(recipes().any(|r| r.output == tool), "{tool:?}");
        }
        assert_eq!(smelt(IronOre), Some(IronIngot));
        assert!(burn(Coal).unwrap() > burn(Stick).unwrap());
    }
}
