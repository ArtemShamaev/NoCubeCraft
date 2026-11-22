//! Menus for playing together: the join page, and opening a world from the pause menu.

use std::net::{SocketAddr, ToSocketAddrs};

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::RenetClient;

use crate::{
    MenuPage, Mode,
    hud::Toast,
    net::{self, PORT, Search},
    settings::Settings,
    ui::{self, TextField},
};

/// Seconds to wait for a host before giving up.
const CONNECT_SECS: f32 = 10.0;

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(MenuPage::Join), spawn_join)
            .add_systems(OnExit(MenuPage::Join), |mut commands: Commands| commands.remove_resource::<Search>())
            .add_systems(Update, (list_worlds, show_status).run_if(in_state(MenuPage::Join)))
            .add_systems(OnEnter(Mode::Paused), spawn_host_button.after(crate::menus::spawn_pause));
    }
}

#[derive(Component)]
struct NameField;

#[derive(Component)]
struct AddressField;

#[derive(Component)]
struct WorldList;

#[derive(Component)]
struct Status;

fn set_value(commands: &mut Commands, field: Entity, value: String) {
    commands.queue(move |world: &mut World| {
        if let Some(mut field) = world.get_mut::<TextField>(field) {
            field.value = value;
        }
    });
}

fn spawn_join(mut commands: Commands, settings: Res<Settings>) {
    if let Some(search) = Search::start() {
        commands.insert_resource(search);
    }
    let screen = crate::menus::menu_screen(&mut commands, MenuPage::Join);
    let panel = ui::panel(&mut commands, screen, 460.0);
    let dim = Color::srgba(1.0, 1.0, 1.0, 0.7);
    ui::title(&mut commands, panel, "Сетевая игра", 32.0);
    ui::label(&mut commands, panel, "Ваше имя", 15.0, dim);
    let name = ui::text_field(&mut commands, panel, "Игрок", false);
    commands.entity(name).insert(NameField);
    set_value(&mut commands, name, settings.player_name.clone());
    ui::label(&mut commands, panel, "Миры в домашней сети", 15.0, dim);
    let list = commands
        .spawn((WorldList, Node { flex_direction: FlexDirection::Column, row_gap: px(6), ..default() }, ChildOf(panel)))
        .id();
    ui::label(&mut commands, list, "Ищем…", 16.0, dim);
    ui::label(&mut commands, panel, "Или адрес хозяина", 15.0, dim);
    let address = ui::text_field(&mut commands, panel, "например, 192.168.1.5", false);
    commands.entity(address).insert(AddressField);
    set_value(&mut commands, address, settings.last_address.clone());
    let status = ui::label(&mut commands, panel, "", 15.0, Color::srgb(1.0, 0.8, 0.5));
    commands.entity(status).insert(Status);

    let buttons = ui::row(&mut commands, panel);
    let join = ui::button(&mut commands, buttons, "Подключиться");
    commands.entity(join).observe(
        |_: On<PointerClick>,
         mut commands: Commands,
         address: Single<&TextField, With<AddressField>>,
         name: Single<&TextField, With<NameField>>,
         channels: Res<RepliconChannels>,
         mut settings: ResMut<Settings>| {
            let target = address.value.trim().to_string();
            connect(&mut commands, &channels, &mut settings, &name.value, &target);
        },
    );
    let back = ui::button(&mut commands, buttons, "Назад");
    commands.entity(back).observe(
        |_: On<PointerClick>, mut commands: Commands, mut page: ResMut<NextState<MenuPage>>| {
            commands.remove_resource::<RenetClient>();
            commands.remove_resource::<bevy_replicon_renet::netcode::NetcodeClientTransport>();
            page.set(MenuPage::Main);
        },
    );
}

/// Starts joining `address` (an IP, a name, with or without a port) as `name`.
fn connect(commands: &mut Commands, channels: &RepliconChannels, settings: &mut Settings, name: &str, address: &str) {
    let host = if address.contains(':') { address.to_socket_addrs() } else { (address, PORT).to_socket_addrs() }
        .ok()
        .and_then(|mut a| a.find(SocketAddr::is_ipv4));
    let name = name.trim();
    if !name.is_empty() {
        settings.player_name = name.to_string();
    }
    let Some(host) = host else {
        commands.queue(|world: &mut World| set_status(world, "Не понимаю этот адрес"));
        return;
    };
    settings.last_address = address.to_string();
    settings.save();
    if let Err(e) = net::join(commands, channels, host) {
        error!("could not join {host}: {e}");
        commands.queue(|world: &mut World| set_status(world, "Не удалось подключиться"));
    }
}

fn set_status(world: &mut World, text: &str) {
    let mut statuses = world.query_filtered::<&mut Text, With<Status>>();
    for mut status in statuses.iter_mut(world) {
        status.0 = text.into();
    }
}

/// Keeps the list of found worlds up to date.
fn list_worlds(
    mut commands: Commands,
    search: Option<Res<Search>>,
    list: Single<(Entity, Option<&Children>), With<WorldList>>,
    mut shown: Local<usize>,
) {
    let Some(search) = search else { return };
    if search.found.is_empty() || search.found.len() == *shown {
        return;
    }
    *shown = search.found.len();
    let (list, children) = *list;
    for child in children.into_iter().flatten() {
        commands.entity(*child).despawn();
    }
    for (host, name) in &search.found {
        let button = ui::button(&mut commands, list, &format!("{name} — {}", host.ip()));
        let address = host.ip().to_string();
        commands.entity(button).observe(
            move |_: On<PointerClick>,
                  mut commands: Commands,
                  name: Single<&TextField, With<NameField>>,
                  channels: Res<RepliconChannels>,
                  mut settings: ResMut<Settings>| {
                connect(&mut commands, &channels, &mut settings, &name.value, &address);
            },
        );
    }
}

/// Says how joining goes, and gives up on a host that doesn't answer.
fn show_status(
    mut commands: Commands,
    time: Res<Time<Real>>,
    state: Res<State<ClientState>>,
    client: Option<Res<RenetClient>>,
    mut status: Single<&mut Text, With<Status>>,
    mut waited: Local<f32>,
) {
    let Some(client) = client else {
        *waited = 0.0;
        return;
    };
    *waited += time.delta_secs();
    let text = if client.is_disconnected() || *waited > CONNECT_SECS {
        commands.remove_resource::<RenetClient>();
        commands.remove_resource::<bevy_replicon_renet::netcode::NetcodeClientTransport>();
        "Не удалось подключиться. Хозяин открыл мир для сети?"
    } else if *state == ClientState::Connected {
        "Загружаем мир хозяина…"
    } else {
        "Подключение…"
    };
    if status.0 != text {
        status.0 = text.into();
    }
}

/// In the pause menu: open the world to the network, or show where it is open.
fn spawn_host_button(
    mut commands: Commands,
    panel: Single<Entity, With<crate::menus::PausePanel>>,
    server: Option<Res<bevy_replicon_renet::RenetServer>>,
    client: Res<State<ClientState>>,
) {
    let dim = Color::srgba(1.0, 1.0, 1.0, 0.75);
    if net::is_guest(&client) {
        ui::label(&mut commands, *panel, "Вы в гостях", 15.0, dim);
        return;
    }
    if server.is_some() {
        let ip = net::local_ip().map_or_else(|| "?".into(), |ip| ip.to_string());
        ui::label(&mut commands, *panel, &format!("Мир открыт для сети: {ip}"), 15.0, dim);
        return;
    }
    let open = ui::button(&mut commands, *panel, "Открыть для сети");
    commands.entity(open).observe(
        |_: On<PointerClick>,
         mut commands: Commands,
         channels: Res<RepliconChannels>,
         mut toast: ResMut<Toast>,
         mut mode: ResMut<NextState<Mode>>| {
            match net::host(&mut commands, &channels) {
                Ok(()) => {
                    let ip = net::local_ip().map_or_else(|| "?".into(), |ip| ip.to_string());
                    toast.show(format!("Мир открыт для сети. Адрес для друга: {ip}"));
                }
                Err(e) => toast.show(format!("Не удалось открыть мир: {e}")),
            }
            mode.set(Mode::Playing);
        },
    );
}
