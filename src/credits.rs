//! End credits: they roll after the Warden falls, and from the title screen.

use bevy::prelude::*;

use crate::{MenuPage, Mode};

/// Pixels per second the credits rise.
const SPEED: f32 = 45.0;

pub struct CreditsPlugin;

impl Plugin for CreditsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(Mode::Credits), |commands: Commands| spawn(commands, DespawnOnExit(Mode::Credits)))
            .add_systems(OnEnter(MenuPage::Credits), |commands: Commands| {
                spawn(commands, DespawnOnExit(MenuPage::Credits))
            })
            .add_systems(Update, roll);
    }
}

/// Heading, then lines under it.
const SECTIONS: &[(&str, &[&str])] = &[
    ("Автор игры", &["Артём Шамаев", "github.com/ArtemShamaev"]),
    ("Модели", &["Quaternius — животные, зомби, скелет", "Kenney — инструменты, сундук, верстак"]),
    ("Текстуры", &["ambientCG"]),
    ("Звуки", &["Kenney", "Vinrax — свинья (CC-BY 3.0)", "bonebrah, qubodup и другие авторы OpenGameArt"]),
    ("Шрифт", &["DejaVu Fonts"]),
    ("Движок", &["Bevy", "Avian Physics"]),
    ("Спасибо", &["Вам — за то, что дошли до конца"]),
];

/// The rolling column, and how far it has risen.
#[derive(Component)]
struct Roll(f32);

fn spawn(mut commands: Commands, scope: impl Bundle) {
    let screen = commands
        .spawn((
            scope,
            Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
            BackgroundColor(Color::srgb(0.03, 0.01, 0.06)),
            GlobalZIndex(10),
        ))
        .id();
    let column = commands
        .spawn((
            Roll(0.0),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(6),
                ..default()
            },
            ChildOf(screen),
        ))
        .id();
    let mut line = |text: &str, size: f32, color: Color, gap: f32| {
        commands.spawn((
            Text::new(text),
            TextFont { font_size: FontSize::Px(size), ..default() },
            TextColor(color),
            Node { margin: UiRect::top(px(gap)), ..default() },
            ChildOf(column),
        ));
    };
    let dim = Color::srgba(0.8, 0.7, 1.0, 0.7);
    line("NOCUBECRAFT", 64.0, Color::WHITE, 0.0);
    line(concat!("версия ", env!("CARGO_PKG_VERSION")), 16.0, dim, 0.0);
    for (heading, names) in SECTIONS {
        line(heading, 16.0, dim, 48.0);
        for name in *names {
            line(name, 24.0, Color::WHITE, 0.0);
        }
    }
    line("Made with Anthropic Claude Opus 5.5", 20.0, Color::srgba(0.85, 0.78, 0.7, 1.0), 120.0);
    let skip = commands
        .spawn((
            Text::new("Esc — пропустить"),
            TextFont { font_size: FontSize::Px(14.0), ..default() },
            TextColor(Color::srgba(1.0, 1.0, 1.0, 0.4)),
            Node { position_type: PositionType::Absolute, right: px(16), bottom: px(12), ..default() },
        ))
        .id();
    commands.entity(screen).add_child(skip);
}

/// Rises from below the screen; once it has passed (or on Esc) the credits end.
fn roll(
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    mut rolls: Query<(&mut Roll, &mut Node, &ComputedNode)>,
    mode: Option<Res<State<Mode>>>,
    mut next_mode: ResMut<NextState<Mode>>,
    mut next_page: ResMut<NextState<MenuPage>>,
) {
    let Ok((mut roll, mut node, computed)) = rolls.single_mut() else { return };
    roll.0 += SPEED * time.delta_secs();
    let height = window.height();
    node.top = px(height - roll.0);
    let tall = computed.size().y * computed.inverse_scale_factor();
    if roll.0 < height + tall && !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    if mode.is_some() {
        next_mode.set(Mode::Victory);
    } else {
        next_page.set(MenuPage::Main);
    }
}
