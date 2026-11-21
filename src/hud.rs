use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    prelude::*,
};

use crate::{
    AppState,
    building::BuildTool,
    containers::{SlotRef, Slots},
    icons::ItemIcons,
    items::{HOTBAR, Inventory},
    player::Player,
    save::LoadWorld,
    survival::{MAX_HEALTH, MAX_HUNGER, Vitals},
    terrain::Terrain,
};

const TOAST_SECS: f32 = 2.5;
pub const SLOT_SIZE: f32 = 52.0;
const SLOT_BG: Color = Color::srgba(0.08, 0.08, 0.1, 0.6);
const SLOT_BORDER: Color = Color::srgba(1.0, 1.0, 1.0, 0.15);
const SELECTED_BORDER: Color = Color::srgba(1.0, 1.0, 1.0, 0.9);

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        // Bevy's built-in font has no Cyrillic; DejaVu Sans replaces it as the default.
        let font = Font::from_bytes(include_bytes!("../assets/fonts/DejaVuSans.ttf").to_vec());
        app.world_mut().resource_mut::<Assets<Font>>().insert(AssetId::default(), font).expect("default font slot");
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
            .init_resource::<Toast>()
            .init_resource::<Progress>()
            .add_systems(OnEnter(AppState::Game), spawn.after(LoadWorld))
            .add_systems(
                Update,
                (
                    toggle_debug,
                    update_debug,
                    update_slots,
                    update_vitals,
                    update_toast,
                    update_progress,
                    update_hurt,
                    update_build,
                )
                    .run_if(in_state(AppState::Game)),
            );
    }
}

/// Short messages shown above the hotbar.
#[derive(Resource, Default)]
pub struct Toast {
    lines: Vec<(String, f32)>,
}

impl Toast {
    pub fn show(&mut self, text: String) {
        debug!("toast: {text}");
        self.lines.retain(|(line, _)| *line != text);
        self.lines.push((text, TOAST_SECS));
        if self.lines.len() > 3 {
            self.lines.remove(0);
        }
    }
}

/// Progress of chopping or breaking something, shown under the crosshair.
#[derive(Resource, Default)]
pub struct Progress(pub Option<f32>);

#[derive(Component)]
struct DebugText;

#[derive(Component)]
struct ToastText;

/// Block size and shape, shown while a building material is in hand.
#[derive(Component)]
struct BuildText;

#[derive(Component)]
struct ProgressBar;

#[derive(Component)]
struct HurtOverlay;

#[derive(Component)]
enum VitalBar {
    Health,
    Hunger,
}

/// Shows one slot: of the inventory, the crafting grid, a furnace or a chest.
#[derive(Component)]
pub struct SlotView {
    pub source: SlotRef,
    icon: Entity,
    count: Entity,
    wear: Entity,
}

/// Spawns a slot widget under `parent`, showing `source`.
pub fn spawn_slot(commands: &mut Commands, parent: Entity, source: SlotRef) -> Entity {
    let icon = commands
        .spawn((
            ImageNode::default(),
            Node { width: px(40), height: px(40), ..default() },
            Visibility::Hidden,
            Pickable::IGNORE,
        ))
        .id();
    let count = commands
        .spawn((
            Text::default(),
            TextFont { font_size: FontSize::Px(14.0), ..default() },
            TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.0, 0.6) },
            Node { position_type: PositionType::Absolute, right: px(4), bottom: px(1), ..default() },
            Pickable::IGNORE,
        ))
        .id();
    let wear = commands
        .spawn((
            Node { position_type: PositionType::Absolute, left: px(6), bottom: px(4), height: px(3), ..default() },
            BackgroundColor(Color::srgb(0.3, 0.9, 0.3)),
            Visibility::Hidden,
            Pickable::IGNORE,
        ))
        .id();
    commands
        .spawn((
            SlotView { source, icon, count, wear },
            Node {
                width: px(SLOT_SIZE),
                height: px(SLOT_SIZE),
                border: UiRect::all(px(2)),
                border_radius: BorderRadius::all(px(6)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(SLOT_BG),
            BorderColor::all(SLOT_BORDER),
            ChildOf(parent),
        ))
        .add_children(&[icon, count, wear])
        .id()
}

fn spawn(mut commands: Commands) {
    let root = commands
        .spawn((
            DespawnOnExit(AppState::Game),
            Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
            Pickable::IGNORE,
        ))
        .id();
    let bar = |width: f32, height: f32| {
        (
            Node {
                position_type: PositionType::Absolute,
                left: percent(50),
                top: percent(50),
                width: px(width),
                height: px(height),
                margin: UiRect::new(px(-width / 2.0), px(0), px(-height / 2.0), px(0)),
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
            ChildOf(root),
        )
    };
    commands.spawn(bar(16.0, 2.0));
    commands.spawn(bar(2.0, 16.0));

    commands.spawn((
        HurtOverlay,
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
        BackgroundColor(Color::NONE),
        GlobalZIndex(-1),
        Pickable::IGNORE,
        ChildOf(root),
    ));

    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: percent(50),
            top: percent(50),
            width: px(60),
            height: px(5),
            margin: UiRect::new(px(-30), px(0), px(18), px(0)),
            ..default()
        })
        .insert((BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)), Visibility::Hidden, ProgressBar, ChildOf(root)))
        .with_child((Node { height: percent(100), ..default() }, BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.9))));

    commands.spawn((
        DebugText,
        Text::default(),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        Node { position_type: PositionType::Absolute, left: px(8), top: px(8), ..default() },
        Visibility::Hidden,
        ChildOf(root),
    ));

    let column = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                bottom: px(10),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(6),
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    commands.spawn((
        ToastText,
        Text::default(),
        TextFont { font_size: FontSize::Px(17.0), ..default() },
        TextLayout::justify(Justify::Center),
        TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.0, 0.6) },
        ChildOf(column),
    ));

    commands.spawn((
        BuildText,
        Text::default(),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
        TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.0, 0.6) },
        ChildOf(column),
    ));

    let vitals = commands.spawn((Node { column_gap: px(16), ..default() }, ChildOf(column))).id();
    for (kind, color) in
        [(VitalBar::Health, Color::srgb(0.85, 0.2, 0.18)), (VitalBar::Hunger, Color::srgb(0.85, 0.6, 0.2))]
    {
        commands
            .spawn((
                Node {
                    width: px(SLOT_SIZE * 4.5 - 8.0),
                    height: px(10),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(4)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
                BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.2)),
                ChildOf(vitals),
            ))
            .with_child((
                kind,
                Node { height: percent(100), border_radius: BorderRadius::all(px(3)), ..default() },
                BackgroundColor(color),
            ));
    }

    let hotbar = commands.spawn((Node { column_gap: px(4), ..default() }, ChildOf(column))).id();
    for i in 0..HOTBAR {
        spawn_slot(&mut commands, hotbar, SlotRef::Inventory(i));
    }
}

fn update_slots(
    inventory: Res<Inventory>,
    sources: Slots,
    icons: Res<ItemIcons>,
    mut slots: Query<(&SlotView, &mut BorderColor)>,
    mut texts: Query<&mut Text>,
    mut nodes: Query<(&mut Node, &mut Visibility, &mut BackgroundColor), (Without<SlotView>, Without<ImageNode>)>,
    mut icon_nodes: Query<(&mut ImageNode, &mut Visibility)>,
) {
    for (view, mut border) in &mut slots {
        let stack = sources.get(view.source);
        let selected = view.source == SlotRef::Inventory(inventory.selected);
        border.set_if_neq(BorderColor::all(if selected { SELECTED_BORDER } else { SLOT_BORDER }));

        if let Ok((mut image, mut visibility)) = icon_nodes.get_mut(view.icon) {
            match stack {
                Some(stack) => {
                    let handle = icons.get(stack.item);
                    if image.image != handle {
                        image.image = handle;
                    }
                    visibility.set_if_neq(Visibility::Inherited);
                }
                None => {
                    visibility.set_if_neq(Visibility::Hidden);
                }
            }
        }
        if let Ok(mut text) = texts.get_mut(view.count) {
            let shown = stack.filter(|s| s.count > 1).map(|s| s.count.to_string()).unwrap_or_default();
            if text.0 != shown {
                text.0 = shown;
            }
        }
        if let Ok((mut node, mut visibility, mut color)) = nodes.get_mut(view.wear) {
            let health = stack.and_then(|s| s.item.tool().map(|t| 1.0 - s.wear as f32 / t.durability as f32));
            match health {
                Some(h) if h < 1.0 => {
                    node.width = px((SLOT_SIZE - 16.0) * h);
                    color.0 = Color::srgb(1.0 - h, 0.3 + 0.6 * h, 0.2);
                    visibility.set_if_neq(Visibility::Inherited);
                }
                _ => {
                    visibility.set_if_neq(Visibility::Hidden);
                }
            }
        }
    }
}

fn update_vitals(vitals: Res<Vitals>, mut bars: Query<(&VitalBar, &mut Node)>) {
    if !vitals.is_changed() {
        return;
    }
    for (bar, mut node) in &mut bars {
        let fraction = match bar {
            VitalBar::Health => vitals.health / MAX_HEALTH,
            VitalBar::Hunger => vitals.hunger / MAX_HUNGER,
        };
        node.width = percent(fraction * 100.0);
    }
}

fn update_toast(time: Res<Time<Real>>, mut toast: ResMut<Toast>, mut text: Single<&mut Text, With<ToastText>>) {
    let dt = time.delta_secs();
    if toast.lines.is_empty() {
        return;
    }
    for (_, left) in &mut toast.lines {
        *left -= dt;
    }
    toast.lines.retain(|(_, left)| *left > 0.0);
    text.0 = toast.lines.iter().map(|(line, _)| line.as_str()).collect::<Vec<_>>().join("\n");
}

fn update_progress(
    progress: Res<Progress>,
    bar: Single<(&mut Visibility, &Children), With<ProgressBar>>,
    mut fill: Query<&mut Node>,
) {
    let (mut visibility, children) = bar.into_inner();
    match progress.0 {
        Some(p) => {
            *visibility = Visibility::Inherited;
            if let Ok(mut node) = fill.get_mut(children[0]) {
                node.width = percent(p.clamp(0.0, 1.0) * 100.0);
            }
        }
        None => *visibility = Visibility::Hidden,
    }
}

fn update_hurt(vitals: Res<Vitals>, mut overlay: Single<&mut BackgroundColor, With<HurtOverlay>>) {
    let low = 1.0 - (vitals.health / 6.0).min(1.0);
    overlay.0 = Color::srgba(0.7, 0.0, 0.0, (vitals.hurt * 0.45).max(low * 0.15));
}

fn toggle_debug(keys: Res<ButtonInput<KeyCode>>, mut debug: Single<&mut Visibility, With<DebugText>>) {
    if keys.just_pressed(KeyCode::F3) {
        debug.toggle_visible_hidden();
    }
}

fn update_debug(
    diagnostics: Res<DiagnosticsStore>,
    terrain: Res<Terrain>,
    player: Single<&Transform, With<Player>>,
    debug: Single<(&mut Text, &Visibility), With<DebugText>>,
) {
    let (mut text, visibility) = debug.into_inner();
    if visibility == Visibility::Hidden {
        return;
    }
    let fps = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS).and_then(|d| d.smoothed()).unwrap_or(0.0);
    let p = player.translation;
    let (meshed, pending, triangles) = terrain.chunk_counts();
    text.0 = format!(
        "{fps:.0} FPS\nXYZ {:.1} {:.1} {:.1}\nchunks {meshed}, building {pending}, {}k triangles",
        p.x,
        p.y,
        p.z,
        triangles / 1000
    );
}

fn update_build(tool: Res<BuildTool>, inventory: Res<Inventory>, mut text: Single<&mut Text, With<BuildText>>) {
    if !tool.is_changed() && !inventory.is_changed() {
        return;
    }
    let building = inventory.selected().is_some_and(|s| s.item.material().is_some());
    text.0 = if building {
        format!("Блок {} · {}    R — размер, F — форма", tool.size.name(), tool.shape.name())
    } else {
        format!("Копать по {}    R — размер", tool.size.name())
    };
}
