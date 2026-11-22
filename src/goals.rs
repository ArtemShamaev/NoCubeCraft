//! The way through the game, one goal at a time, from the first log to the Warden of the
//! Shards; and the Warden's health bar while it fights.

use std::f32::consts::TAU;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    hud::Toast,
    items::{Inventory, Item},
    mobs::{Mob, MobKind},
    player::{self, Player},
    props::Prop,
    save::LoadWorld,
    sound::{PlaySound, Sfx},
    terrain::ALTAR,
};

/// Shard cores the altar takes to call the Warden.
pub const ALTAR_CORES: u32 = 32;

pub struct GoalsPlugin;

impl Plugin for GoalsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::Game), spawn.after(LoadWorld))
            .add_systems(Update, (advance, show_goal, show_warden).chain().run_if(in_state(AppState::Game)));
    }
}

/// How far along the player is; saved with the world.
#[derive(Resource, Default, Clone, Copy, Serialize, Deserialize)]
pub struct Goals {
    pub stage: usize,
}

impl Goals {
    pub fn win(&mut self) {
        self.stage = GOALS.len();
    }
}

/// What a goal's check can look at.
struct Facts<'a> {
    inventory: &'a Inventory,
    position: Vec3,
    table: bool,
    warden: bool,
}

impl Facts<'_> {
    fn has(&self, items: &[Item]) -> bool {
        items.iter().any(|i| self.inventory.count(*i) > 0)
    }

    fn pickaxe(&self, tier: u8) -> bool {
        self.inventory
            .slots
            .iter()
            .flatten()
            .filter_map(|s| s.item.tool())
            .any(|t| t.tier >= tier && t.kind == crate::items::ToolKind::Pickaxe)
    }
}

struct Goal {
    text: &'static str,
    done: fn(&Facts) -> bool,
}

const GOALS: [Goal; 10] = [
    Goal { text: "Добудьте бревно", done: |f| f.has(&[Item::Wood, Item::Planks]) || f.table },
    Goal { text: "Сделайте верстак из досок и поставьте его", done: |f| f.table },
    Goal { text: "Сделайте каменную кирку", done: |f| f.pickaxe(1) },
    Goal {
        text: "Выплавьте железо: руда и уголь в печь", done: |f| f.has(&[Item::IronIngot]) || f.pickaxe(2)
    },
    Goal {
        text: "Найдите алмаз, глубоко под землёй", done: |f| f.has(&[Item::Diamond, Item::Activator, Item::Rift])
    },
    Goal {
        text: "Соберите физический активатор", done: |f| f.has(&[Item::Activator, Item::Rift])
    },
    Goal {
        text: "Соберите разлом, поставьте и войдите в Осколки", done: |f| player::in_shards(f.position)
    },
    Goal {
        text: "Добудьте 32 ядра осколка железной киркой", done: |f| f.inventory.count(Item::Core) >= ALTAR_CORES
    },
    Goal { text: "Найдите алтарь и отдайте ему ядра", done: |f| f.warden },
    // Only the Warden's death completes this one (`Goals::win`).
    Goal { text: "Победите Стража Осколков", done: |_| false },
];

#[derive(Component)]
struct GoalText;

#[derive(Component)]
struct WardenBar;

fn spawn(mut commands: Commands) {
    let shadow = TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.0, 0.6) };
    commands.spawn((
        GoalText,
        DespawnOnExit(AppState::Game),
        Text::default(),
        TextFont { font_size: FontSize::Px(16.0), ..default() },
        TextLayout::justify(Justify::Right),
        TextColor(Color::srgba(1.0, 0.95, 0.8, 0.9)),
        shadow,
        Node { position_type: PositionType::Absolute, right: px(14), top: px(10), max_width: px(380), ..default() },
        Pickable::IGNORE,
    ));
    let bar = commands
        .spawn((
            WardenBar,
            DespawnOnExit(AppState::Game),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                top: px(14),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(4),
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
        ))
        .id();
    commands.spawn((
        Text::new("Страж Осколков"),
        TextFont { font_size: FontSize::Px(18.0), ..default() },
        shadow,
        ChildOf(bar),
    ));
    commands
        .spawn((
            Node {
                width: px(420),
                height: px(12),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(4)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.25)),
            ChildOf(bar),
        ))
        .with_child((
            Node { width: percent(100), height: percent(100), border_radius: BorderRadius::all(px(3)), ..default() },
            BackgroundColor(Color::srgb(0.65, 0.3, 0.95)),
        ));
}

fn advance(
    mut goals: ResMut<Goals>,
    inventory: Res<Inventory>,
    mut toast: ResMut<Toast>,
    mut sounds: MessageWriter<PlaySound>,
    player: Single<&Transform, With<Player>>,
    props: Query<&Prop>,
    mobs: Query<&Mob>,
) {
    let facts = Facts {
        inventory: &inventory,
        position: player.translation,
        table: props.iter().any(|p| p.0 == crate::items::PropKind::CraftingTable),
        warden: mobs.iter().any(|m| m.kind == MobKind::Warden && m.alive()),
    };
    while let Some(goal) = GOALS.get(goals.stage).filter(|g| (g.done)(&facts)) {
        goals.stage += 1;
        toast.show(format!("Цель выполнена: {}", goal.text));
        sounds.write(PlaySound::ui(Sfx::Page));
    }
}

/// The current goal in the corner; the altar's goal points the way once in the Shards.
fn show_goal(goals: Res<Goals>, player: Single<&Transform, With<Player>>, mut text: Single<&mut Text, With<GoalText>>) {
    let Some(goal) = GOALS.get(goals.stage) else {
        text.0 = "Игра пройдена! Мир — ваш".into();
        return;
    };
    let mut line = format!("Цель {}/{}: {}", goals.stage + 1, GOALS.len(), goal.text);
    let p = player.translation;
    if goals.stage >= 8 && player::in_shards(p) {
        let to = Vec2::new(ALTAR.x - p.x, ALTAR.y - p.z);
        // The way relative to where the player faces: their forward is -Z turned by the yaw.
        let (yaw, ..) = player.rotation.to_euler(EulerRot::YXZ);
        let angle = (Vec2::new(-yaw.sin(), -yaw.cos()).angle_to(to) / TAU * 8.0).round() as i32;
        const ARROWS: [&str; 8] = ["↑", "↗", "→", "↘", "↓", "↙", "←", "↖"];
        line += &format!("\nАлтарь: {} {:.0} м", ARROWS[angle.rem_euclid(8) as usize], to.length());
    }
    if text.0 != line {
        text.0 = line;
    }
}

fn show_warden(
    mobs: Query<(&Mob, &Transform)>,
    player: Single<&Transform, With<Player>>,
    bar: Single<(&mut Visibility, &Children), With<WardenBar>>,
    children: Query<&Children>,
    mut fills: Query<&mut Node>,
) {
    let (mut visibility, parts) = bar.into_inner();
    let warden = mobs
        .iter()
        .find(|(m, t)| m.kind == MobKind::Warden && m.alive() && t.translation.distance(player.translation) < 80.0);
    *visibility = if warden.is_some() { Visibility::Inherited } else { Visibility::Hidden };
    let Some((mob, _)) = warden else { return };
    let fill = parts.get(1).and_then(|frame| children.get(*frame).ok()).and_then(|c| c.first().copied());
    if let Some(mut node) = fill.and_then(|f| fills.get_mut(f).ok()) {
        node.width = percent(100.0 * (mob.health / mob.kind.species().health).max(0.0));
    }
}
