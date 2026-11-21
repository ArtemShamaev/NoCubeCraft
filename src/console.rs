//! Cheat console: T or / opens it, Enter runs a command, Esc closes.

use std::f32::consts::PI;

use bevy::prelude::*;

use crate::{
    AppState, Creative, Mode,
    body::Body,
    items::{Inventory, Item},
    mobs::{MobKind, Summon},
    player::Player,
    sky::TimeOfDay,
    survival::{MAX_HEALTH, MAX_HUNGER, Vitals},
    terrain::Terrain,
    ui::{self, TextField},
    worlds::CurrentWorld,
};

const HISTORY_LINES: usize = 10;

pub struct ConsolePlugin;

impl Plugin for ConsolePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ConsoleLog>()
            .add_systems(Update, open.run_if(in_state(Mode::Playing)))
            .add_systems(OnEnter(Mode::Console), spawn)
            .add_systems(Update, (submit, recall, close).run_if(in_state(Mode::Console)))
            .add_systems(OnExit(AppState::Game), |mut log: ResMut<ConsoleLog>| *log = ConsoleLog::default());
    }
}

#[derive(Resource, Default)]
struct ConsoleLog {
    lines: Vec<String>,
    commands: Vec<String>,
    recalled: Option<usize>,
}

#[derive(Component)]
struct LogText;

fn open(keys: Res<ButtonInput<KeyCode>>, mut mode: ResMut<NextState<Mode>>) {
    if keys.any_just_pressed([KeyCode::KeyT, KeyCode::Slash]) {
        mode.set(Mode::Console);
    }
}

fn close(keys: Res<ButtonInput<KeyCode>>, mut mode: ResMut<NextState<Mode>>) {
    if keys.just_pressed(KeyCode::Escape) {
        mode.set(Mode::Playing);
    }
}

fn spawn(mut commands: Commands, log: Res<ConsoleLog>) {
    let root = commands
        .spawn((
            DespawnOnExit(Mode::Console),
            Node {
                position_type: PositionType::Absolute,
                left: px(12),
                bottom: px(150),
                width: px(620),
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                padding: UiRect::all(px(10)),
                border_radius: BorderRadius::all(px(8)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            GlobalZIndex(8),
        ))
        .id();
    let help = if log.lines.is_empty() {
        "Введите help — список команд".to_string()
    } else {
        log.lines.join("\n")
    };
    commands.spawn((
        LogText,
        Text::new(help),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
        ChildOf(root),
    ));
    ui::text_field(&mut commands, root, "/команда", true);
}

fn submit(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut field: Single<&mut TextField>,
    mut log: ResMut<ConsoleLog>,
) {
    if !keys.just_pressed(KeyCode::Enter) {
        return;
    }
    let line = field.value.trim().trim_start_matches('/').to_string();
    field.value.clear();
    if line.is_empty() {
        return;
    }
    log.commands.push(line.clone());
    log.recalled = None;
    commands.run_system_cached_with(run, line);
}

/// Up and down arrows walk through earlier commands.
fn recall(keys: Res<ButtonInput<KeyCode>>, mut log: ResMut<ConsoleLog>, mut field: Single<&mut TextField>) {
    let len = log.commands.len();
    if len == 0 {
        return;
    }
    let index = if keys.just_pressed(KeyCode::ArrowUp) {
        Some(log.recalled.map_or(len - 1, |i| i.saturating_sub(1)))
    } else if keys.just_pressed(KeyCode::ArrowDown) {
        log.recalled.map(|i| (i + 1).min(len - 1))
    } else {
        return;
    };
    if let Some(i) = index {
        log.recalled = Some(i);
        field.value = log.commands[i].clone();
    }
}

/// Everything commands can touch.
#[derive(bevy::ecs::system::SystemParam)]
struct World<'w, 's> {
    creative: ResMut<'w, Creative>,
    current: ResMut<'w, CurrentWorld>,
    inventory: ResMut<'w, Inventory>,
    vitals: ResMut<'w, Vitals>,
    time_of_day: ResMut<'w, TimeOfDay>,
    terrain: Res<'w, Terrain>,
    player: Single<'w, 's, (&'static mut Player, &'static mut Transform, Option<&'static mut Body>)>,
    log: ResMut<'w, ConsoleLog>,
    summons: MessageWriter<'w, Summon>,
    text: Single<'w, 's, &'static mut Text, With<LogText>>,
}

fn run(In(line): In<String>, mut world: World) {
    let reply = execute(&line, &mut world);
    let log = &mut world.log;
    log.lines.push(format!("> {line}"));
    log.lines.extend(reply.lines().map(String::from));
    let excess = log.lines.len().saturating_sub(HISTORY_LINES);
    log.lines.drain(..excess);
    world.text.0 = log.lines.join("\n");
}

const HELP: &str = "help · gamemode creative|survival · give <предмет> [кол-во] · items
time set day|night|noon|midnight|0..1 · tp <x> <y> <z> · tp spawn
heal · feed · kill · clear · seed · summon <моб>";

fn execute(line: &str, w: &mut World) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["help" | "помощь"] => HELP.into(),
        ["gamemode" | "gm", mode] => {
            let creative = match *mode {
                "creative" | "c" | "1" | "творческий" => true,
                "survival" | "s" | "0" | "выживание" => false,
                _ => return "Режимы: creative, survival".into(),
            };
            w.creative.0 = creative;
            w.current.info.creative = creative;
            w.current.touch();
            if creative {
                "Творческий режим: двойной пробел — полёт, Ctrl — вниз"
            } else {
                "Режим выживания"
            }
            .into()
        }
        ["items"] => Item::ALL.iter().map(|i| i.id()).collect::<Vec<_>>().join(", "),
        ["give", name, rest @ ..] => {
            let Some(item) =
                Item::ALL.into_iter().find(|i| i.id() == *name || i.name().to_lowercase() == name.to_lowercase())
            else {
                return format!("Нет предмета «{name}». Список: items");
            };
            let count = match rest {
                [] => item.max_stack(),
                [n] => match n.parse::<u32>() {
                    Ok(n) if n > 0 => n.min(item.max_stack() * 36),
                    _ => return "Количество — положительное число".into(),
                },
                _ => return "give <предмет> [кол-во]".into(),
            };
            let lost = w.inventory.add(item, count);
            format!("Выдано: {} × {}{}", count - lost, item.name(), if lost > 0 { " (не всё влезло)" } else { "" })
        }
        ["time", "set", value] | ["time", value] => {
            let t = match *value {
                "day" | "день" => 0.08,
                "noon" | "полдень" => 0.25,
                "night" | "ночь" => 0.6,
                "midnight" | "полночь" => 0.75,
                v => match v.parse::<f32>() {
                    Ok(t) if (0.0..=1.0).contains(&t) => t,
                    _ => return "time set day|night|noon|midnight или число от 0 до 1".into(),
                },
            };
            w.time_of_day.0 = t;
            format!("Время: {t:.2}")
        }
        ["tp", "spawn"] => {
            let spawn = w.player.0.spawn;
            teleport(w, spawn)
        }
        ["tp", x, y, z] => match (x.parse::<f32>(), y.parse::<f32>(), z.parse::<f32>()) {
            (Ok(x), Ok(y), Ok(z)) => teleport(w, Vec3::new(x, y, z)),
            _ => "tp <x> <y> <z>".into(),
        },
        ["summon", name] => match MobKind::named(name) {
            Some(kind) => {
                let (player, transform, _) = &*w.player;
                let ahead = Quat::from_rotation_y(player.yaw) * Vec3::NEG_Z * 4.0;
                w.summons.write(Summon { kind, at: transform.translation + ahead + Vec3::Y, yaw: player.yaw + PI });
                format!("{kind:?}")
            }
            None => format!("Мобы: {}", MobKind::ALL.map(|k| format!("{k:?}").to_lowercase()).join(", ")),
        },
        ["heal"] => {
            w.vitals.health = MAX_HEALTH;
            "Здоровье восстановлено".into()
        }
        ["feed"] => {
            w.vitals.hunger = MAX_HUNGER;
            "Голод утолён".into()
        }
        ["kill"] => {
            w.vitals.damage(MAX_HEALTH * 10.0);
            "…".into()
        }
        ["clear"] => {
            *w.inventory = Inventory::default();
            "Инвентарь очищен".into()
        }
        ["seed"] => format!("Сид мира: {}", w.terrain.seed()),
        _ => format!("Неизвестная команда «{line}». help — список команд"),
    }
}

fn teleport(w: &mut World, to: Vec3) -> String {
    let (_, transform, body) = &mut *w.player;
    transform.translation = to;
    if let Some(body) = body {
        body.velocity = Vec3::ZERO;
    }
    format!("Перемещение в {:.0} {:.0} {:.0}", to.x, to.y, to.z)
}
