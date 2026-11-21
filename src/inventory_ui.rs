//! The inventory screen: the player's items plus whatever is open beside them — the 2×2
//! crafting grid, a crafting table's 3×3 grid, a furnace or a chest — and the recipe book.

use bevy::{picking::pointer::PointerButton, prelude::*, window::PrimaryWindow};

use crate::{
    Creative, Mode,
    containers::{CHEST_SLOTS, CraftGrid, OpenContainer, SlotRef, Slots, SlotsMut},
    crafting::{self, Recipe},
    hud::{SLOT_SIZE, SlotView, Toast, spawn_slot},
    icons::ItemIcons,
    items::{HOTBAR, Inventory, Item, SLOTS, Stack},
    sound::{PlaySound, Sfx},
};

pub struct InventoryUiPlugin;

impl Plugin for InventoryUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Held>()
            .add_systems(Update, open.run_if(in_state(Mode::Playing)))
            .add_systems(OnEnter(Mode::Inventory), (prepare_grid, spawn_screen).chain())
            .add_systems(OnExit(Mode::Inventory), close_screen)
            .add_systems(
                Update,
                (close, refresh_recipes, follow_cursor, show_furnace).run_if(in_state(Mode::Inventory)),
            );
    }
}

/// The stack picked up with the mouse.
#[derive(Resource, Default)]
struct Held(Option<Stack>);

#[derive(Component)]
struct RecipeList;

#[derive(Component)]
struct RecipeButton(usize);

#[derive(Component)]
struct HeldIcon;

#[derive(Component)]
struct HoverText;

/// Furnace fire and progress bars.
#[derive(Component)]
enum FurnaceBar {
    Fire,
    Progress,
}

const PANEL_BG: Color = Color::srgba(0.1, 0.11, 0.13, 0.92);

fn open(keys: Res<ButtonInput<KeyCode>>, mut mode: ResMut<NextState<Mode>>, mut container: ResMut<OpenContainer>) {
    if keys.just_pressed(KeyCode::KeyE) {
        *container = OpenContainer::None;
        mode.set(Mode::Inventory);
    }
}

fn close(keys: Res<ButtonInput<KeyCode>>, mut mode: ResMut<NextState<Mode>>) {
    if keys.just_pressed(KeyCode::KeyE) || keys.just_pressed(KeyCode::Escape) {
        mode.set(Mode::Playing);
    }
}

fn prepare_grid(container: Res<OpenContainer>, mut grid: ResMut<CraftGrid>, mut sounds: MessageWriter<PlaySound>) {
    let width = if *container == OpenContainer::Workbench { 3 } else { 2 };
    *grid = CraftGrid { width, slots: vec![None; width * width] };
    sounds.write(PlaySound::ui(match *container {
        OpenContainer::Chest(_) => Sfx::ChestOpen,
        _ => Sfx::Inventory,
    }));
}

/// Puts everything held or left on the grid back into the inventory.
fn close_screen(
    mut held: ResMut<Held>,
    mut inventory: ResMut<Inventory>,
    mut grid: ResMut<CraftGrid>,
    mut container: ResMut<OpenContainer>,
    mut toast: ResMut<Toast>,
    mut sounds: MessageWriter<PlaySound>,
) {
    let leftovers: Vec<Stack> =
        held.0.take().into_iter().chain(grid.slots.iter_mut().filter_map(Option::take)).collect();
    let lost: u32 = leftovers.into_iter().map(|stack| inventory.add_stack(stack)).sum();
    if lost > 0 {
        toast.show("Инвентарь полон: часть предметов потеряна".into());
    }
    if matches!(*container, OpenContainer::Chest(_)) {
        sounds.write(PlaySound::ui(Sfx::ChestClose));
    }
    *container = OpenContainer::None;
}

fn panel() -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        padding: UiRect::all(px(12)),
        row_gap: px(4),
        border_radius: BorderRadius::all(px(10)),
        ..default()
    }
}

fn title(text: &str) -> impl Bundle {
    (
        Text::new(text),
        TextFont { font_size: FontSize::Px(20.0), ..default() },
        Node { margin: UiRect::bottom(px(6)), ..default() },
    )
}

fn row(commands: &mut Commands, parent: Entity) -> Entity {
    commands.spawn((Node { column_gap: px(4), align_items: AlignItems::Center, ..default() }, ChildOf(parent))).id()
}

fn slot(commands: &mut Commands, parent: Entity, source: SlotRef) -> Entity {
    let slot = spawn_slot(commands, parent, source);
    commands.entity(slot).observe(press_slot).observe(hover_slot).observe(unhover_slot);
    slot
}

fn arrow(commands: &mut Commands, parent: Entity) {
    commands.spawn((
        Text::new("→"),
        TextFont { font_size: FontSize::Px(30.0), ..default() },
        Node { margin: UiRect::horizontal(px(10)), ..default() },
        ChildOf(parent),
    ));
}

fn spawn_screen(
    mut commands: Commands,
    creative: Res<Creative>,
    icons: Res<ItemIcons>,
    container: Res<OpenContainer>,
    grid: Res<CraftGrid>,
) {
    let screen = commands
        .spawn((
            DespawnOnExit(Mode::Inventory),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                column_gap: px(16),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
        ))
        .id();

    let main = commands.spawn((panel(), BackgroundColor(PANEL_BG), ChildOf(screen))).id();
    spawn_container(&mut commands, main, *container, grid.width);

    commands.spawn((title("Инвентарь"), ChildOf(main)));
    // Backpack rows first, the hotbar row last, like on screen.
    for (n, r) in (1..SLOTS / HOTBAR).chain([0]).enumerate() {
        let line = row(&mut commands, main);
        if n == 3 {
            commands.entity(line).insert(Node {
                column_gap: px(4),
                align_items: AlignItems::Center,
                margin: UiRect::top(px(10)),
                ..default()
            });
        }
        for i in r * HOTBAR..(r + 1) * HOTBAR {
            slot(&mut commands, line, SlotRef::Inventory(i));
        }
    }
    commands.spawn((
        HoverText,
        Text::default(),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        Node { min_height: px(20), ..default() },
        ChildOf(main),
    ));
    commands.spawn((
        Text::new("ЛКМ — взять или положить, ПКМ — половину или по одному, Shift+ЛКМ — быстро переложить"),
        TextFont { font_size: FontSize::Px(13.0), ..default() },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.55)),
        ChildOf(main),
    ));

    let side = matches!(*container, OpenContainer::None | OpenContainer::Workbench) || creative.0;
    if side {
        let book = commands
            .spawn((
                Node { width: px(320), max_height: percent(85), overflow: Overflow::scroll_y(), ..panel() },
                BackgroundColor(PANEL_BG),
                ChildOf(screen),
            ))
            .id();
        if creative.0 {
            spawn_catalog(&mut commands, book, &icons);
        } else {
            commands.spawn((title("Книга рецептов"), ChildOf(book)));
            commands.spawn((
                Text::new("Нажмите — выложить на сетку, с Shift — сколько хватит"),
                TextFont { font_size: FontSize::Px(12.0), ..default() },
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.55)),
                ChildOf(book),
            ));
            commands.spawn((
                RecipeList,
                Node { flex_direction: FlexDirection::Column, row_gap: px(4), ..default() },
                ChildOf(book),
            ));
        }
    }

    commands.spawn((
        HeldIcon,
        DespawnOnExit(Mode::Inventory),
        ImageNode::default(),
        Node { position_type: PositionType::Absolute, width: px(40), height: px(40), ..default() },
        GlobalZIndex(10),
        Pickable::IGNORE,
        Visibility::Hidden,
    ));
}

/// The top of the panel: crafting grid, furnace or chest.
fn spawn_container(commands: &mut Commands, parent: Entity, container: OpenContainer, width: usize) {
    match container {
        OpenContainer::None | OpenContainer::Workbench => {
            let name = if width == 3 { "Верстак" } else { "Крафт" };
            commands.spawn((title(name), ChildOf(parent)));
            let line = row(commands, parent);
            let grid = commands
                .spawn((Node { flex_direction: FlexDirection::Column, row_gap: px(4), ..default() }, ChildOf(line)))
                .id();
            for y in 0..width {
                let cells = row(commands, grid);
                for x in 0..width {
                    slot(commands, cells, SlotRef::Grid(x + y * width));
                }
            }
            arrow(commands, line);
            slot(commands, line, SlotRef::Result);
        }
        OpenContainer::Furnace(_) => {
            commands.spawn((title("Печь"), ChildOf(parent)));
            let line = row(commands, parent);
            let column = commands
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: px(6),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    ChildOf(line),
                ))
                .id();
            slot(commands, column, SlotRef::FurnaceInput);
            bar(commands, column, FurnaceBar::Fire, Color::srgb(1.0, 0.55, 0.15));
            slot(commands, column, SlotRef::FurnaceFuel);
            let middle = commands
                .spawn((
                    Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Center, ..default() },
                    ChildOf(line),
                ))
                .id();
            arrow(commands, middle);
            bar(commands, middle, FurnaceBar::Progress, Color::WHITE);
            slot(commands, line, SlotRef::FurnaceOutput);
            commands.spawn((
                Text::new("Сверху — что плавить, снизу — топливо (уголь, дерево)"),
                TextFont { font_size: FontSize::Px(13.0), ..default() },
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.55)),
                ChildOf(parent),
            ));
        }
        OpenContainer::Chest(_) => {
            commands.spawn((title("Сундук"), ChildOf(parent)));
            for r in 0..CHEST_SLOTS / HOTBAR {
                let line = row(commands, parent);
                for i in r * HOTBAR..(r + 1) * HOTBAR {
                    slot(commands, line, SlotRef::Chest(i));
                }
            }
        }
    }
    commands.spawn((Node { height: px(10), ..default() }, ChildOf(parent)));
}

fn bar(commands: &mut Commands, parent: Entity, kind: FurnaceBar, color: Color) {
    commands
        .spawn((
            Node { width: px(44), height: px(6), border_radius: BorderRadius::all(px(3)), ..default() },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
            ChildOf(parent),
        ))
        .with_child((
            kind,
            Node { height: percent(100), width: percent(0), border_radius: BorderRadius::all(px(3)), ..default() },
            BackgroundColor(color),
        ));
}

fn show_furnace(slots: Slots, mut bars: Query<(&FurnaceBar, &mut Node)>) {
    let Some(furnace) = slots.furnace() else { return };
    for (bar, mut node) in &mut bars {
        let fraction = match bar {
            FurnaceBar::Fire if furnace.burn_total > 0.0 => furnace.burn_left.max(0.0) / furnace.burn_total,
            FurnaceBar::Fire => 0.0,
            FurnaceBar::Progress => furnace.progress,
        };
        node.width = percent(fraction * 100.0);
    }
}

/// Creative mode: every item, a click picks up a full stack.
fn spawn_catalog(commands: &mut Commands, parent: Entity, icons: &ItemIcons) {
    commands.spawn((title("Все предметы"), ChildOf(parent)));
    let grid = commands
        .spawn((Node { flex_wrap: FlexWrap::Wrap, column_gap: px(4), row_gap: px(4), ..default() }, ChildOf(parent)))
        .id();
    for item in Item::ALL {
        commands
            .spawn((
                Node {
                    width: px(SLOT_SIZE),
                    height: px(SLOT_SIZE),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(px(6)),
                    ..default()
                },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.06)),
                ChildOf(grid),
            ))
            .with_child((
                ImageNode::new(icons.get(item)),
                Node { width: px(40), height: px(40), ..default() },
                Pickable::IGNORE,
            ))
            .observe(move |_: On<PointerClick>, mut held: ResMut<Held>| {
                held.0 = Some(Stack::new(item, item.max_stack()))
            })
            .observe(move |_: On<PointerOver>, mut text: Single<&mut Text, With<HoverText>>| {
                text.0 = item.name().into()
            })
            .observe(|_: On<PointerOut>, mut text: Single<&mut Text, With<HoverText>>| text.0.clear());
    }
}

fn press_slot(
    press: On<PointerPress>,
    views: Query<&SlotView>,
    keys: Res<ButtonInput<KeyCode>>,
    mut slots: SlotsMut,
    mut held: ResMut<Held>,
    mut sounds: MessageWriter<PlaySound>,
) {
    let Ok(view) = views.get(press.event_target()) else { return };
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let primary = press.button == PointerButton::Primary;
    match view.source {
        SlotRef::Result => {
            if primary && take_result(&mut slots, &mut held.0, shift) {
                sounds.write(PlaySound { volume: 0.6, ..PlaySound::ui(Sfx::Click) });
            }
        }
        // Smelted items can only be taken out.
        SlotRef::FurnaceOutput => take_output(&mut slots, &mut held.0, shift),
        source if shift && primary => quick_move(&mut slots, source),
        source => {
            let Some(cell) = slots.slot(source) else { return };
            let (slot, hand) = if primary { move_all(*cell, held.0) } else { move_some(*cell, held.0) };
            *cell = slot;
            held.0 = hand;
        }
    }
}

fn take_output(slots: &mut SlotsMut, held: &mut Option<Stack>, into_inventory: bool) {
    let Some(stack) = slots.slot(SlotRef::FurnaceOutput).and_then(|s| *s) else { return };
    let rest = if into_inventory {
        let left = slots.inventory.add_stack(stack);
        (left > 0).then_some(Stack { count: left, ..stack })
    } else {
        match *held {
            None => {
                *held = Some(stack);
                None
            }
            Some(h) if h.item == stack.item && h.count + stack.count <= h.item.max_stack() => {
                *held = Some(Stack { count: h.count + stack.count, ..h });
                None
            }
            _ => Some(stack),
        }
    };
    if let Some(cell) = slots.slot(SlotRef::FurnaceOutput) {
        *cell = rest;
    }
}

/// Takes the crafting result into the hand, or with Shift crafts as many as fit into the inventory.
fn take_result(slots: &mut SlotsMut, held: &mut Option<Stack>, all: bool) -> bool {
    let mut crafted = false;
    while let Some(recipe) = slots.grid.recipe() {
        let made = Stack::new(recipe.output, recipe.count);
        if all {
            let mut after = slots.inventory.clone();
            if after.add_stack(made) > 0 {
                break;
            }
            *slots.inventory = after;
        } else {
            match *held {
                None => *held = Some(made),
                Some(h) if h.item == made.item && h.count + made.count <= h.item.max_stack() => {
                    *held = Some(Stack { count: h.count + made.count, ..h });
                }
                _ => break,
            }
        }
        slots.grid.consume();
        crafted = true;
        if !all {
            break;
        }
    }
    crafted
}

/// Shift-click: inventory into the open container (or between hotbar and backpack), and back.
fn quick_move(slots: &mut SlotsMut, source: SlotRef) {
    let Some(stack) = slots.slot(source).and_then(|s| s.take()) else { return };
    let targets: Vec<SlotRef> = match (source, *slots.open) {
        (SlotRef::Inventory(_), OpenContainer::Chest(_)) => (0..CHEST_SLOTS).map(SlotRef::Chest).collect(),
        (SlotRef::Inventory(_), OpenContainer::Furnace(_)) => {
            if crafting::burn(stack.item).is_some() && crafting::smelt(stack.item).is_none() {
                vec![SlotRef::FurnaceFuel]
            } else {
                vec![SlotRef::FurnaceInput]
            }
        }
        (SlotRef::Inventory(i), _) if i < HOTBAR => (HOTBAR..SLOTS).map(SlotRef::Inventory).collect(),
        (SlotRef::Inventory(_), _) => (0..HOTBAR).map(SlotRef::Inventory).collect(),
        _ => (0..SLOTS).map(SlotRef::Inventory).collect(),
    };
    let mut left = Some(stack);
    // Same items first, then empty slots.
    for pass in 0..2 {
        for &target in &targets {
            let Some(moving) = left else { break };
            let Some(cell) = slots.slot(target) else { continue };
            let fits = match cell {
                Some(s) => pass == 0 && s.item == moving.item && s.count < s.item.max_stack(),
                None => pass == 1,
            };
            if fits {
                let (placed, rest) = move_all(*cell, Some(moving));
                *cell = placed;
                left = rest;
            }
        }
    }
    if let Some(rest) = left
        && let Some(cell) = slots.slot(source)
    {
        *cell = Some(rest);
    }
}

/// Left click: put the held stack down (merging if it's the same item) or pick the slot up.
fn move_all(slot: Option<Stack>, hand: Option<Stack>) -> (Option<Stack>, Option<Stack>) {
    match (slot, hand) {
        (Some(s), Some(h)) if s.item == h.item && s.count < s.item.max_stack() => {
            let n = h.count.min(s.item.max_stack() - s.count);
            let rest = (h.count > n).then(|| Stack { count: h.count - n, ..h });
            (Some(Stack { count: s.count + n, ..s }), rest)
        }
        (slot, hand) => (hand, slot),
    }
}

/// Right click: with an empty hand take half the slot; otherwise put one item down.
fn move_some(slot: Option<Stack>, hand: Option<Stack>) -> (Option<Stack>, Option<Stack>) {
    match (slot, hand) {
        (Some(s), None) => {
            let take = s.count.div_ceil(2);
            let left = (s.count > take).then(|| Stack { count: s.count - take, ..s });
            (left, Some(Stack { count: take, ..s }))
        }
        (None, Some(h)) => (Some(Stack { count: 1, ..h }), (h.count > 1).then(|| Stack { count: h.count - 1, ..h })),
        (Some(s), Some(h)) if s.item == h.item && s.count < s.item.max_stack() => {
            (Some(Stack { count: s.count + 1, ..s }), (h.count > 1).then(|| Stack { count: h.count - 1, ..h }))
        }
        other => other,
    }
}

fn hover_slot(
    over: On<PointerOver>,
    views: Query<&SlotView>,
    slots: Slots,
    mut text: Single<&mut Text, With<HoverText>>,
) {
    let Ok(view) = views.get(over.event_target()) else { return };
    text.0 = slots
        .get(view.source)
        .map(|s| match s.item.tool() {
            Some(tool) => format!("{} (прочность {}/{})", s.item.name(), tool.durability - s.wear, tool.durability),
            None => s.item.name().to_string(),
        })
        .unwrap_or_default();
}

fn unhover_slot(_out: On<PointerOut>, mut text: Single<&mut Text, With<HoverText>>) {
    text.0.clear();
}

fn follow_cursor(
    held: Res<Held>,
    icons: Res<ItemIcons>,
    window: Single<&Window, With<PrimaryWindow>>,
    icon: Single<(&mut ImageNode, &mut Node, &mut Visibility), With<HeldIcon>>,
) {
    let (mut image, mut node, mut visibility) = icon.into_inner();
    match (held.0, window.cursor_position()) {
        (Some(stack), Some(cursor)) => {
            let handle = icons.get(stack.item);
            if image.image != handle {
                image.image = handle;
            }
            node.left = px(cursor.x - 20.0);
            node.top = px(cursor.y - 20.0);
            *visibility = Visibility::Inherited;
        }
        _ => *visibility = Visibility::Hidden,
    }
}

/// How many of `item` the player has, on the grid included.
fn available(inventory: &Inventory, grid: &CraftGrid, item: Item) -> u32 {
    inventory.count(item) + grid.slots.iter().flatten().filter(|s| s.item == item).map(|s| s.count).sum::<u32>()
}

fn craftable(recipe: &Recipe, inventory: &Inventory, grid: &CraftGrid) -> bool {
    let needed = recipe.ingredients();
    needed.iter().all(|item| available(inventory, grid, *item) >= needed.iter().filter(|i| *i == item).count() as u32)
}

/// Rebuilds the recipe book when what can be crafted may have changed.
fn refresh_recipes(
    mut commands: Commands,
    inventory: Res<Inventory>,
    grid: Res<CraftGrid>,
    icons: Res<ItemIcons>,
    list: Single<(Entity, Option<&Children>), With<RecipeList>>,
) {
    let (list, children) = list.into_inner();
    let fresh = children.is_none_or(|c| c.is_empty());
    if !fresh && !inventory.is_changed() && !grid.is_changed() {
        return;
    }
    commands.entity(list).despawn_children();

    let mut order: Vec<(usize, &Recipe, bool)> = crafting::recipes()
        .enumerate()
        .filter(|(_, r)| r.size() <= grid.width)
        .map(|(i, r)| (i, r, craftable(r, &inventory, &grid)))
        .collect();
    // What can be made comes first.
    order.sort_by_key(|(i, _, ok)| (!ok, *i));
    for (i, recipe, ready) in order {
        let alpha = if ready { 1.0 } else { 0.4 };
        let count = if recipe.count > 1 { format!(" ×{}", recipe.count) } else { String::new() };
        let mut needed: Vec<(Item, usize)> = Vec::new();
        for item in recipe.ingredients() {
            match needed.iter_mut().find(|(i, _)| *i == item) {
                Some((_, n)) => *n += 1,
                None => needed.push((item, 1)),
            }
        }
        let inputs = needed.iter().map(|(item, n)| format!("{n} {}", item.name())).collect::<Vec<_>>().join(", ");
        commands
            .spawn((
                RecipeButton(i),
                Node {
                    column_gap: px(8),
                    padding: UiRect::all(px(4)),
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(px(6)),
                    ..default()
                },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, if ready { 0.08 } else { 0.03 })),
                ChildOf(list),
            ))
            .with_children(|b| {
                b.spawn((
                    ImageNode::new(icons.get(recipe.output)).with_color(Color::WHITE.with_alpha(alpha)),
                    Node { width: px(SLOT_SIZE - 16.0), height: px(SLOT_SIZE - 16.0), flex_shrink: 0.0, ..default() },
                    Pickable::IGNORE,
                ));
                b.spawn((
                    Text::new(format!("{}{count}", recipe.output.name())),
                    TextFont { font_size: FontSize::Px(15.0), ..default() },
                    TextColor(Color::WHITE.with_alpha(alpha)),
                    Pickable::IGNORE,
                ))
                .with_child((
                    TextSpan::new(format!("\n{inputs}")),
                    TextFont { font_size: FontSize::Px(12.0), ..default() },
                    TextColor(Color::srgba(0.8, 0.85, 0.9, alpha * 0.8)),
                ));
            })
            .observe(fill_grid)
            .observe(|over: On<PointerOver>, mut colors: Query<&mut BackgroundColor>| {
                if let Ok(mut c) = colors.get_mut(over.event_target()) {
                    c.0 = c.0.with_alpha(c.0.alpha() + 0.08);
                }
            })
            .observe(|out: On<PointerOut>, mut colors: Query<&mut BackgroundColor>| {
                if let Ok(mut c) = colors.get_mut(out.event_target()) {
                    c.0 = c.0.with_alpha((c.0.alpha() - 0.08).max(0.0));
                }
            });
    }
}

/// Recipe book click: clears the grid and lays the recipe out from the inventory — one set,
/// or with Shift as many as there are materials for.
fn fill_grid(
    click: On<PointerClick>,
    buttons: Query<&RecipeButton>,
    keys: Res<ButtonInput<KeyCode>>,
    mut inventory: ResMut<Inventory>,
    mut grid: ResMut<CraftGrid>,
    mut toast: ResMut<Toast>,
    mut sounds: MessageWriter<PlaySound>,
) {
    let Ok(button) = buttons.get(click.event_target()) else { return };
    let Some(recipe) = crafting::recipes().nth(button.0) else { return };
    let returned: Vec<Stack> = grid.slots.iter_mut().filter_map(Option::take).collect();
    for stack in returned {
        inventory.add_stack(stack);
    }
    let layout = recipe.layout(grid.width);
    let per_set = |item: &Item| layout.iter().flatten().filter(|i| *i == item).count() as u32;
    let sets = layout.iter().flatten().map(|item| inventory.count(*item) / per_set(item)).min().unwrap_or(0);
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let sets = if shift { sets.min(64) } else { sets.min(1) };
    if sets == 0 {
        toast.show(format!("Не хватает материалов для «{}»", recipe.output.name()));
        return;
    }
    for (cell, item) in grid.slots.iter_mut().zip(layout) {
        if let Some(item) = item
            && inventory.remove(item, sets)
        {
            *cell = Some(Stack::new(item, sets));
        }
    }
    sounds.write(PlaySound { volume: 0.5, ..PlaySound::ui(Sfx::Page) });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_move_merge_and_split() {
        let dirt = |n| Some(Stack::new(Item::Dirt, n));
        assert_eq!(move_all(dirt(5), None), (None, dirt(5)));
        assert_eq!(move_all(dirt(995), dirt(10)), (dirt(999), dirt(6)));
        assert_eq!(move_all(Some(Stack::new(Item::Stone, 1)), dirt(3)), (dirt(3), Some(Stack::new(Item::Stone, 1))));
        assert_eq!(move_some(dirt(5), None), (dirt(2), dirt(3)));
        assert_eq!(move_some(None, dirt(3)), (dirt(1), dirt(2)));
        assert_eq!(move_some(dirt(1), dirt(1)), (dirt(2), None));
    }
}
