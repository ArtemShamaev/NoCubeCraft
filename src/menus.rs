//! Title screen, world list, new world form, settings, pause and death screens.

use bevy::prelude::*;

use crate::{
    AppState, MenuPage, Mode,
    body::Body,
    player::{Player, Spawning},
    save,
    settings::Settings,
    sky::TimeOfDay,
    survival::Vitals,
    terrain::{Terrain, TerrainAnchor},
    ui::{self, ACCENT, TextField},
    worlds::{self, CurrentWorld},
};

pub struct MenusPlugin;

impl Plugin for MenusPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::Menu), spawn_scenery)
            .add_systems(OnExit(AppState::Menu), clear_scenery)
            .add_systems(Update, fly_around.run_if(in_state(AppState::Menu)))
            .add_systems(OnEnter(MenuPage::Main), spawn_main)
            .add_systems(OnEnter(MenuPage::Worlds), spawn_worlds)
            .add_systems(OnEnter(MenuPage::Create), spawn_create)
            .add_systems(OnEnter(MenuPage::Settings), |commands: Commands, settings: Res<Settings>| {
                spawn_settings(commands, settings, (DespawnOnExit(MenuPage::Settings), backdrop()))
            })
            .add_systems(OnEnter(Mode::Settings), |commands: Commands, settings: Res<Settings>| {
                spawn_settings(commands, settings, DespawnOnExit(Mode::Settings))
            })
            .add_systems(OnEnter(Mode::Paused), spawn_pause)
            .add_systems(OnEnter(Mode::Dead), spawn_death)
            .add_systems(OnEnter(Mode::Victory), spawn_victory)
            .add_systems(OnEnter(AppState::Game), spawn_loading)
            .add_systems(Update, (pause_time, hide_loading))
            .add_systems(Update, back_on_escape)
            .add_systems(Update, respawn_on_enter.run_if(in_state(Mode::Dead)));
    }
}

/// The title screen's backdrop: a world of its own, seen from a camera slowly circling it.
const SCENERY_SEED: u32 = 1337;
/// Morning light, with long shadows.
const SCENERY_TIME: f32 = 0.06;
const ORBIT_RADIUS: f32 = 70.0;
/// Radians per second.
const ORBIT_SPEED: f32 = 0.015;

#[derive(Component)]
struct MenuCamera {
    center: Vec3,
    angle: f32,
}

fn spawn_scenery(mut commands: Commands) {
    let terrain = Terrain::new(SCENERY_SEED);
    let center = terrain.find_spawn();
    commands.insert_resource(terrain);
    commands.insert_resource(TimeOfDay(SCENERY_TIME));
    commands.spawn((
        DespawnOnExit(AppState::Menu),
        MenuCamera { center, angle: 0.0 },
        TerrainAnchor,
        Camera3d::default(),
        Msaa::Off,
        Transform::from_translation(center),
    ));
}

fn clear_scenery(mut commands: Commands) {
    commands.remove_resource::<Terrain>();
    commands.remove_resource::<TimeOfDay>();
}

fn fly_around(time: Res<Time<Real>>, terrain: Res<Terrain>, camera: Single<(&mut MenuCamera, &mut Transform)>) {
    let (mut camera, mut transform) = camera.into_inner();
    camera.angle += ORBIT_SPEED * time.delta_secs();
    let eye = camera.center + Vec3::new(camera.angle.cos(), 0.0, camera.angle.sin()) * ORBIT_RADIUS;
    let ground = terrain.surface_height(eye.x, eye.z).max(camera.center.y);
    let eye = eye.with_y(ground + 22.0);
    *transform = Transform::from_translation(eye).looking_at(camera.center + Vec3::Y * 16.0, Vec3::Y);
}

/// The game world stands still while a menu is over it.
/// Menus stop the world, unless others are playing in it too.
fn pause_time(
    mode: Option<Res<State<Mode>>>,
    mut time: ResMut<Time<Virtual>>,
    server: Res<State<bevy_replicon::prelude::ServerState>>,
    client: Res<State<bevy_replicon::prelude::ClientState>>,
) {
    let alone = *server == bevy_replicon::prelude::ServerState::Stopped && !crate::net::is_guest(&client);
    let paused =
        alone && mode.is_some_and(|m| matches!(**m, Mode::Paused | Mode::Settings | Mode::Credits | Mode::Victory));
    if paused != time.is_paused() {
        if paused { time.pause() } else { time.unpause() }
    }
}

fn back_on_escape(
    keys: Res<ButtonInput<KeyCode>>,
    page: Option<Res<State<MenuPage>>>,
    mode: Option<Res<State<Mode>>>,
    mut next_page: ResMut<NextState<MenuPage>>,
    mut next_mode: ResMut<NextState<Mode>>,
) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    if let Some(page) = page
        && **page != MenuPage::Main
    {
        next_page.set(MenuPage::Main);
    }
    match mode.as_deref().map(|m| (**m).clone()) {
        Some(Mode::Paused) => next_mode.set(Mode::Playing),
        Some(Mode::Settings) => next_mode.set(Mode::Paused),
        _ => {}
    }
}

/// Darkens the world behind a menu enough to read over it.
fn backdrop() -> BackgroundGradient {
    BackgroundGradient::from(LinearGradient::to_right(vec![
        ColorStop::auto(Color::srgba(0.02, 0.03, 0.05, 0.85)),
        ColorStop::auto(Color::srgba(0.02, 0.03, 0.05, 0.35)),
    ]))
}

pub fn menu_screen(commands: &mut Commands, page: MenuPage) -> Entity {
    let screen = ui::screen(commands, DespawnOnExit(page), Color::NONE);
    commands.entity(screen).insert(backdrop());
    screen
}

fn spawn_main(mut commands: Commands) {
    let screen = menu_screen(&mut commands, MenuPage::Main);
    // A column down the left, over the darker side of the backdrop.
    commands.entity(screen).insert(Node {
        position_type: PositionType::Absolute,
        width: percent(100),
        height: percent(100),
        flex_direction: FlexDirection::Column,
        justify_content: JustifyContent::Center,
        align_items: AlignItems::FlexStart,
        padding: UiRect::left(px(90)),
        row_gap: px(6),
        ..default()
    });
    let title = ui::title(&mut commands, screen, "NOCUBECRAFT", 76.0);
    commands.entity(title).insert(Node { align_self: AlignSelf::FlexStart, ..default() });
    ui::label(&mut commands, screen, "Выживание в мире из маленьких блоков", 18.0, Color::srgba(1.0, 1.0, 1.0, 0.75));
    let panel = commands
        .spawn((
            Node {
                width: px(300),
                flex_direction: FlexDirection::Column,
                row_gap: px(10),
                margin: UiRect::top(px(40)),
                ..default()
            },
            ChildOf(screen),
        ))
        .id();
    let play = ui::button(&mut commands, panel, "Играть");
    commands
        .entity(play)
        .observe(|_: On<PointerClick>, mut page: ResMut<NextState<MenuPage>>| page.set(MenuPage::Worlds));
    let together = ui::button(&mut commands, panel, "Сетевая игра");
    commands
        .entity(together)
        .observe(|_: On<PointerClick>, mut page: ResMut<NextState<MenuPage>>| page.set(MenuPage::Join));
    let settings = ui::button(&mut commands, panel, "Настройки");
    commands
        .entity(settings)
        .observe(|_: On<PointerClick>, mut page: ResMut<NextState<MenuPage>>| page.set(MenuPage::Settings));
    let credits = ui::button(&mut commands, panel, "Титры");
    commands
        .entity(credits)
        .observe(|_: On<PointerClick>, mut page: ResMut<NextState<MenuPage>>| page.set(MenuPage::Credits));
    let quit = ui::button(&mut commands, panel, "Выход");
    commands.entity(quit).observe(|_: On<PointerClick>, mut exit: MessageWriter<AppExit>| {
        exit.write(AppExit::Success);
    });
    let version = ui::label(
        &mut commands,
        screen,
        concat!("v", env!("CARGO_PKG_VERSION")),
        14.0,
        Color::srgba(1.0, 1.0, 1.0, 0.5),
    );
    commands.entity(version).insert(Node {
        position_type: PositionType::Absolute,
        left: px(16),
        bottom: px(12),
        ..default()
    });
}

/// A world in the list; clicking "delete" once arms it, twice deletes.
#[derive(Component)]
struct WorldRow {
    world: CurrentWorld,
    armed: bool,
}

fn spawn_worlds(mut commands: Commands) {
    let screen = menu_screen(&mut commands, MenuPage::Worlds);
    let panel = ui::panel(&mut commands, screen, 560.0);
    ui::title(&mut commands, panel, "Миры", 32.0);

    let worlds = worlds::list();
    if worlds.is_empty() {
        ui::label(
            &mut commands,
            panel,
            "Пока ни одного мира. Создайте первый!",
            17.0,
            Color::srgba(1.0, 1.0, 1.0, 0.7),
        );
    }
    let list = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                max_height: px(420),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            ChildOf(panel),
        ))
        .id();
    for world in worlds {
        let row = commands
            .spawn((
                Node {
                    padding: UiRect::all(px(10)),
                    column_gap: px(8),
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(px(8)),
                    ..default()
                },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.05)),
                ChildOf(list),
            ))
            .id();
        let info = commands
            .spawn((Node { flex_direction: FlexDirection::Column, flex_grow: 1.0, ..default() }, ChildOf(row)))
            .id();
        ui::label(&mut commands, info, &world.info.name, 19.0, Color::WHITE);
        let mode = if world.info.creative { "творческий" } else { "выживание" };
        let details = format!("{mode} · сид {} · {}", world.info.seed, ago(world.info.last_played));
        ui::label(&mut commands, info, &details, 13.0, Color::srgba(1.0, 1.0, 1.0, 0.55));

        let play = ui::button(&mut commands, row, "Играть");
        let chosen = world.clone();
        commands.entity(play).observe(
            move |_: On<PointerClick>, mut commands: Commands, mut state: ResMut<NextState<AppState>>| {
                commands.insert_resource(chosen.clone());
                state.set(AppState::Game);
            },
        );
        let delete = ui::button(&mut commands, row, "Удалить");
        commands.entity(delete).insert(WorldRow { world, armed: false }).observe(
            |click: On<PointerClick>,
             mut rows: Query<&mut WorldRow>,
             children: Query<&Children>,
             mut texts: Query<&mut Text>,
             mut page: ResMut<NextState<MenuPage>>| {
                let Ok(mut row) = rows.get_mut(click.event_target()) else { return };
                if row.armed {
                    if let Err(e) = worlds::delete(&row.world.dir) {
                        error!("could not delete {}: {e}", row.world.dir.display());
                    }
                    // Rebuild the list.
                    page.set(MenuPage::Worlds);
                    return;
                }
                row.armed = true;
                if let Some(mut text) = children.get(click.event_target()).ok().and_then(|c| texts.get_mut(c[0]).ok()) {
                    text.0 = "Точно?".into();
                }
            },
        );
    }

    let buttons = ui::row(&mut commands, panel);
    let create = ui::button(&mut commands, buttons, "Новый мир");
    commands
        .entity(create)
        .observe(|_: On<PointerClick>, mut page: ResMut<NextState<MenuPage>>| page.set(MenuPage::Create));
    let back = ui::button(&mut commands, buttons, "Назад");
    commands
        .entity(back)
        .observe(|_: On<PointerClick>, mut page: ResMut<NextState<MenuPage>>| page.set(MenuPage::Main));
}

fn ago(timestamp: u64) -> String {
    let minutes = worlds::now().saturating_sub(timestamp) / 60;
    match minutes {
        0 => "только что".into(),
        m if m < 60 => format!("{m} мин назад"),
        m if m < 60 * 24 => format!("{} ч назад", m / 60),
        m => format!("{} дн назад", m / 60 / 24),
    }
}

#[derive(Component)]
struct NameField;

#[derive(Component)]
struct SeedField;

#[derive(Component)]
struct ModeToggle(bool);

fn spawn_create(mut commands: Commands) {
    let screen = menu_screen(&mut commands, MenuPage::Create);
    let panel = ui::panel(&mut commands, screen, 440.0);
    ui::title(&mut commands, panel, "Новый мир", 32.0);
    ui::label(&mut commands, panel, "Название", 15.0, Color::srgba(1.0, 1.0, 1.0, 0.7));
    let name = ui::text_field(&mut commands, panel, "Новый мир", true);
    commands.entity(name).insert(NameField);
    ui::label(&mut commands, panel, "Сид (необязательно: число или слово)", 15.0, Color::srgba(1.0, 1.0, 1.0, 0.7));
    let seed = ui::text_field(&mut commands, panel, "случайный", false);
    commands.entity(seed).insert(SeedField);

    let mode = ui::button(&mut commands, panel, "Режим: выживание");
    commands.entity(mode).insert(ModeToggle(false)).observe(
        |click: On<PointerClick>, mut toggles: Query<(&mut ModeToggle, &Children)>, mut texts: Query<&mut Text>| {
            let Ok((mut toggle, children)) = toggles.get_mut(click.event_target()) else { return };
            toggle.0 = !toggle.0;
            if let Ok(mut text) = texts.get_mut(children[0]) {
                text.0 = if toggle.0 { "Режим: творческий" } else { "Режим: выживание" }.into();
            }
        },
    );

    let buttons = ui::row(&mut commands, panel);
    let create = ui::button(&mut commands, buttons, "Создать");
    commands.entity(create).observe(
        |_: On<PointerClick>,
         mut commands: Commands,
         name: Single<&TextField, With<NameField>>,
         seed: Single<&TextField, With<SeedField>>,
         mode: Single<&ModeToggle>,
         mut state: ResMut<NextState<AppState>>| match worlds::create(&name.value, &seed.value, mode.0) {
            Ok(world) => {
                commands.insert_resource(world);
                state.set(AppState::Game);
            }
            Err(e) => error!("could not create a world: {e}"),
        },
    );
    let back = ui::button(&mut commands, buttons, "Назад");
    commands
        .entity(back)
        .observe(|_: On<PointerClick>, mut page: ResMut<NextState<MenuPage>>| page.set(MenuPage::Worlds));
}

/// One adjustable setting: shows its value and steps it with the arrows.
#[derive(Component, Clone, Copy)]
enum Setting {
    Skin,
    ViewDistance,
    Fov,
    Sensitivity,
    Fullscreen,
    Vsync,
    Shadows,
}

impl Setting {
    const ALL: [Setting; 7] = [
        Setting::Skin,
        Setting::ViewDistance,
        Setting::Fov,
        Setting::Sensitivity,
        Setting::Fullscreen,
        Setting::Vsync,
        Setting::Shadows,
    ];

    fn name(self) -> &'static str {
        match self {
            Setting::Skin => "Внешность",
            Setting::ViewDistance => "Дальность прорисовки",
            Setting::Fov => "Поле зрения",
            Setting::Sensitivity => "Чувствительность мыши",
            Setting::Fullscreen => "Полный экран",
            Setting::Vsync => "Вертикальная синхронизация",
            Setting::Shadows => "Тени",
        }
    }

    fn value(self, s: &Settings) -> String {
        let on_off = |b: bool| if b { "вкл" } else { "выкл" }.to_string();
        match self {
            Setting::Skin => crate::avatars::SKINS[s.skin as usize % crate::avatars::SKINS.len()].1.into(),
            Setting::ViewDistance => format!("{:.0} м", s.view_distance),
            Setting::Fov => format!("{:.0}°", s.fov),
            Setting::Sensitivity => format!("{:.1}", s.mouse_sensitivity),
            Setting::Fullscreen => on_off(s.fullscreen),
            Setting::Vsync => on_off(s.vsync),
            Setting::Shadows => on_off(s.shadows),
        }
    }

    fn step(self, s: &mut Settings, dir: i32) {
        match self {
            Setting::Skin => {
                let n = crate::avatars::SKINS.len() as i32;
                s.skin = (s.skin as i32 + dir).rem_euclid(n) as u8;
            }
            Setting::ViewDistance => {
                let all = Settings::VIEW_DISTANCES;
                let i = all.iter().position(|d| *d >= s.view_distance).unwrap_or(all.len() - 1) as i32;
                s.view_distance = all[(i + dir).clamp(0, all.len() as i32 - 1) as usize];
            }
            Setting::Fov => s.fov = (s.fov + 5.0 * dir as f32).clamp(60.0, 110.0),
            Setting::Sensitivity => {
                s.mouse_sensitivity = ((s.mouse_sensitivity + 0.1 * dir as f32) * 10.0).round().clamp(2.0, 30.0) / 10.0
            }
            Setting::Fullscreen => s.fullscreen = !s.fullscreen,
            Setting::Vsync => s.vsync = !s.vsync,
            Setting::Shadows => s.shadows = !s.shadows,
        }
    }
}

#[derive(Component)]
struct OptionValue(Setting);

#[derive(Component)]
struct OptionStep(Setting, i32);

fn spawn_settings(mut commands: Commands, settings: Res<Settings>, scope: impl Bundle) {
    let screen = ui::screen(&mut commands, scope, Color::srgba(0.0, 0.0, 0.0, 0.55));
    commands.entity(screen).insert(GlobalZIndex(6));
    let panel = ui::panel(&mut commands, screen, 480.0);
    ui::title(&mut commands, panel, "Настройки", 32.0);
    for option in Setting::ALL {
        let row = ui::row(&mut commands, panel);
        let name = ui::label(&mut commands, row, option.name(), 17.0, Color::WHITE);
        commands.entity(name).insert(Node { flex_grow: 1.0, ..default() });
        let left = ui::button(&mut commands, row, "‹");
        commands.entity(left).insert(OptionStep(option, -1)).observe(step_option);
        commands.spawn((
            OptionValue(option),
            Text::new(option.value(&settings)),
            TextFont { font_size: FontSize::Px(17.0), ..default() },
            TextColor(ACCENT),
            TextLayout::justify(Justify::Center),
            Node { width: px(90), ..default() },
            ChildOf(row),
        ));
        let right = ui::button(&mut commands, row, "›");
        commands.entity(right).insert(OptionStep(option, 1)).observe(step_option);
    }
    let back = ui::button(&mut commands, panel, "Готово");
    commands.entity(back).observe(
        |_: On<PointerClick>,
         state: Res<State<AppState>>,
         mut page: ResMut<NextState<MenuPage>>,
         mut mode: ResMut<NextState<Mode>>| match **state {
            AppState::Menu => page.set(MenuPage::Main),
            AppState::Game => mode.set(Mode::Paused),
        },
    );
}

fn step_option(
    click: On<PointerClick>,
    steps: Query<&OptionStep>,
    mut settings: ResMut<Settings>,
    mut values: Query<(&OptionValue, &mut Text)>,
) {
    let Ok(&OptionStep(option, dir)) = steps.get(click.event_target()) else { return };
    option.step(&mut settings, dir);
    settings.save();
    for (value, mut text) in &mut values {
        text.0 = value.0.value(&settings);
    }
}

/// The pause menu's column of buttons, which others add to.
#[derive(Component)]
pub struct PausePanel;

pub fn spawn_pause(mut commands: Commands) {
    let screen = ui::screen(&mut commands, DespawnOnExit(Mode::Paused), Color::srgba(0.0, 0.0, 0.0, 0.5));
    ui::title(&mut commands, screen, "Пауза", 44.0);
    let panel = ui::panel(&mut commands, screen, 320.0);
    commands.entity(panel).insert(PausePanel);
    let resume = ui::button(&mut commands, panel, "Продолжить");
    commands.entity(resume).observe(|_: On<PointerClick>, mut mode: ResMut<NextState<Mode>>| mode.set(Mode::Playing));
    let settings = ui::button(&mut commands, panel, "Настройки");
    commands
        .entity(settings)
        .observe(|_: On<PointerClick>, mut mode: ResMut<NextState<Mode>>| mode.set(Mode::Settings));
    let to_menu = ui::button(&mut commands, panel, "Сохранить и выйти в меню");
    commands.entity(to_menu).observe(
        |_: On<PointerClick>, mut commands: Commands, mut state: ResMut<NextState<AppState>>| {
            commands.run_system_cached(save::save_now);
            state.set(AppState::Menu);
        },
    );
    let quit = ui::button(&mut commands, panel, "Сохранить и выйти из игры");
    commands.entity(quit).observe(|_: On<PointerClick>, mut exit: MessageWriter<AppExit>| {
        exit.write(AppExit::Success);
    });
}

fn spawn_death(mut commands: Commands) {
    let screen = ui::screen(&mut commands, DespawnOnExit(Mode::Dead), Color::srgba(0.35, 0.0, 0.0, 0.6));
    ui::title(&mut commands, screen, "Вы погибли", 48.0);
    let panel = ui::panel(&mut commands, screen, 320.0);
    let again = ui::button(&mut commands, panel, "Возродиться (Enter)");
    commands.entity(again).observe(|_: On<PointerClick>, mut commands: Commands| commands.run_system_cached(respawn));
}

fn spawn_victory(mut commands: Commands) {
    let screen = ui::screen(&mut commands, DespawnOnExit(Mode::Victory), Color::srgba(0.12, 0.04, 0.22, 0.7));
    ui::title(&mut commands, screen, "Игра пройдена!", 52.0);
    let text = "Страж Осколков повержен, его сердце — ваше.\nМир остаётся с вами: стройте, исследуйте, живите дальше.";
    let label = ui::label(&mut commands, screen, text, 19.0, Color::srgba(1.0, 0.95, 1.0, 0.85));
    commands.entity(label).insert(TextLayout::justify(Justify::Center));
    let panel = ui::panel(&mut commands, screen, 320.0);
    let resume = ui::button(&mut commands, panel, "Продолжить");
    commands.entity(resume).observe(|_: On<PointerClick>, mut mode: ResMut<NextState<Mode>>| mode.set(Mode::Playing));
    let credits = ui::button(&mut commands, panel, "Титры");
    commands.entity(credits).observe(|_: On<PointerClick>, mut mode: ResMut<NextState<Mode>>| mode.set(Mode::Credits));
    let to_menu = ui::button(&mut commands, panel, "Сохранить и выйти в меню");
    commands.entity(to_menu).observe(
        |_: On<PointerClick>, mut commands: Commands, mut state: ResMut<NextState<AppState>>| {
            commands.run_system_cached(save::save_now);
            state.set(AppState::Menu);
        },
    );
}

fn respawn_on_enter(keys: Res<ButtonInput<KeyCode>>, mut commands: Commands) {
    if keys.just_pressed(KeyCode::Enter) {
        commands.run_system_cached(respawn);
    }
}

/// Back to the spawn point with full health; the inventory is kept.
fn respawn(
    mut commands: Commands,
    mut vitals: ResMut<Vitals>,
    mut mode: ResMut<NextState<Mode>>,
    player: Single<(Entity, &Player, &mut Transform)>,
) {
    let (entity, player, mut transform) = player.into_inner();
    transform.translation = player.spawn;
    commands.entity(entity).remove::<Body>().insert(Spawning);
    *vitals = Vitals::default();
    mode.set(Mode::Playing);
}

#[derive(Component)]
struct LoadingScreen;

fn spawn_loading(mut commands: Commands, world: Option<Res<CurrentWorld>>) {
    let name = world.map_or_else(|| "Сетевая игра".to_string(), |w| w.info.name.clone());
    let screen = ui::screen(&mut commands, (LoadingScreen, DespawnOnExit(AppState::Game)), Color::NONE);
    commands.entity(screen).insert((backdrop(), GlobalZIndex(20)));
    ui::title(&mut commands, screen, &name, 40.0);
    ui::label(&mut commands, screen, "Загрузка мира…", 18.0, Color::srgba(1.0, 1.0, 1.0, 0.75));
}

fn hide_loading(
    mut commands: Commands,
    screens: Query<Entity, With<LoadingScreen>>,
    player: Query<(), (With<Player>, With<Body>)>,
) {
    if !player.is_empty() {
        for screen in &screens {
            commands.entity(screen).despawn();
        }
    }
}
