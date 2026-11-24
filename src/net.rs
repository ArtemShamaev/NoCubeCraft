//! Playing together: one player opens their world to the network and others join it.
//!
//! The host runs the world as in singleplayer: creatures, items, physics and saving. Guests
//! generate the same terrain from the seed, receive everything else replicated, and send what
//! they do. Game logic that only the world's owner may run is gated by [`authority`].

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    time::{Duration, SystemTime},
};

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::{
    RenetChannelsExt, RenetClient, RenetServer, RepliconRenetPlugins,
    netcode::{
        ClientAuthentication, NetcodeClientTransport, NetcodeServerTransport, ServerAuthentication, ServerConfig,
    },
    renet::ConnectionConfig,
};
use serde::{Deserialize, Serialize};

use crate::{
    AppState, Mode,
    body::Body,
    falling::Removed,
    goals::Goals,
    hud::Toast,
    items::{Inventory, Item},
    player::Player,
    save::SavedPlayer,
    settings::Settings,
    sky::TimeOfDay,
    survival::Vitals,
    terrain::{Block, Terrain},
};

pub const PORT: u16 = 25_575;
/// Hosts answer on this port when someone on the home network looks for worlds.
const DISCOVERY_PORT: u16 = PORT + 1;
const PROBE: &[u8] = b"NOCUBECRAFT?";
const ANSWER: &[u8] = b"NOCUBECRAFT!";
/// Players with different versions of the game can't play together.
const PROTOCOL: u64 = 3;
const MAX_GUESTS: usize = 7;
const POSE_SECS: f32 = 0.05;
const TIME_SYNC_SECS: f32 = 2.0;
const SAVE_SECS: f32 = 10.0;

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((RepliconPlugins, RepliconRenetPlugins))
            .init_resource::<Guests>()
            .insert_resource(Me(0))
            .replicate::<Transform>()
            .replicate::<Avatar>()
            .add_client_message::<Hello>(Channel::Ordered)
            .add_client_message::<Pose>(Channel::Unreliable)
            .add_client_message::<EditRequest>(Channel::Ordered)
            .add_client_message::<SavePlayer>(Channel::Ordered)
            .add_server_message::<Welcome>(Channel::Ordered)
            .add_server_message::<Edits>(Channel::Ordered)
            .add_server_message::<TimeSync>(Channel::Unreliable)
            .add_server_message::<Notice>(Channel::Ordered)
            .add_systems(Update, show_notices)
            .add_observer(forget_guest)
            .add_systems(OnEnter(ClientState::Connected), say_hello)
            .add_systems(OnExit(ClientState::Connected), lost_host)
            .add_systems(Update, (enter_world, search).run_if(in_state(AppState::Menu)))
            .add_systems(Update, answer_probes.run_if(resource_exists::<Beacon>))
            .add_systems(OnExit(AppState::Game), close)
            .add_systems(
                Update,
                (send_edits, (greet, take_requests, move_avatars, host_avatar, sync_time, keep_guests))
                    .chain()
                    .run_if(in_state(ServerState::Running))
                    .run_if(in_state(AppState::Game)),
            )
            .add_systems(
                Update,
                (send_edits, take_edits, take_time, send_pose, send_save)
                    .run_if(in_state(ClientState::Connected))
                    .run_if(in_state(AppState::Game)),
            );
    }
}

/// Runs where the world is owned: singleplayer and the host, never a guest.
pub fn authority(state: Res<State<ClientState>>) -> bool {
    *state == ClientState::Disconnected
}

/// Whether this game is a guest in someone else's world.
pub fn is_guest(state: &State<ClientState>) -> bool {
    *state != ClientState::Disconnected
}

/// This player's token: 0 for the host (and singleplayer), random for a guest.
#[derive(Resource, Clone, Copy, PartialEq)]
pub struct Me(pub u64);

/// A player as the others see them.
#[derive(Component, Serialize, Deserialize, Clone, PartialEq)]
pub struct Avatar {
    pub name: String,
    pub token: u64,
    pub look: Look,
}

/// What can be seen of a player besides where they are.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default)]
pub struct Look {
    pub skin: u8,
    pub held: Option<Item>,
    pub asleep: bool,
}

impl Look {
    pub fn of(settings: &Settings, inventory: &Inventory, mode: Option<&State<Mode>>) -> Self {
        Self {
            skin: settings.skin,
            held: inventory.selected().map(|s| s.item),
            asleep: mode.is_some_and(|m| **m == Mode::Sleeping),
        }
    }
}

/// On the host: which connected client a guest's avatar stands for.
#[derive(Component, Clone, Copy)]
pub struct AvatarOf(pub ClientId);

/// A short message for one player, shown as a toast.
#[derive(Message, Serialize, Deserialize)]
pub struct Notice(pub String);

/// On the host: the connected client a guest's avatar belongs to, kept on the client entity.
#[derive(Component)]
struct GuestOf {
    avatar: Entity,
    name: String,
}

/// Saved guests of this world, by name; saved with it.
#[derive(Resource, Default, Clone, Serialize, Deserialize)]
pub struct Guests(pub HashMap<String, SavedPlayer>);

#[derive(Message, Serialize, Deserialize)]
struct Hello {
    name: String,
    token: u64,
    skin: u8,
}

/// Everything a guest needs to enter the world.
#[derive(Message, Serialize, Deserialize, Clone)]
pub struct Welcome {
    pub seed: u32,
    pub creative: bool,
    pub time: f32,
    pub spawn: Vec3,
    /// The host's edits, as they would be saved.
    pub edits: Vec<u8>,
    /// Where this guest was last time, with their things.
    pub player: Option<SavedPlayer>,
}

/// A guest's welcome, kept until the world has loaded from it.
#[derive(Resource)]
pub struct Joining(pub Welcome);

#[derive(Message, Serialize, Deserialize)]
struct Pose {
    at: Vec3,
    yaw: f32,
    look: Look,
}

#[derive(Message, Serialize, Deserialize)]
struct EditRequest(Vec<(IVec3, u16)>);

#[derive(Message, Serialize, Deserialize)]
struct Edits(Vec<(IVec3, u16)>);

#[derive(Message, Serialize, Deserialize)]
struct TimeSync(f32);

#[derive(Message, Serialize, Deserialize)]
struct SavePlayer(SavedPlayer);

fn encode(changes: &[(IVec3, Block)]) -> Vec<(IVec3, u16)> {
    changes.iter().map(|(c, b)| (*c, b.to_bits())).collect()
}

fn decode(changes: &[(IVec3, u16)]) -> impl Iterator<Item = (IVec3, Block)> + '_ {
    changes.iter().filter_map(|(c, bits)| Some((*c, Block::from_bits(*bits)?)))
}

fn connection(channels: &RepliconChannels) -> ConnectionConfig {
    let mut server = channels.server_configs();
    // The welcome carries the whole edited world.
    for channel in &mut server {
        channel.max_memory_usage_bytes = 64 * 1024 * 1024;
    }
    ConnectionConfig { server_channels_config: server, client_channels_config: channels.client_configs(), ..default() }
}

fn now() -> Duration {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default()
}

/// Opens the world being played to the network.
pub fn host(commands: &mut Commands, channels: &RepliconChannels) -> std::io::Result<()> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, PORT))?;
    let config = ServerConfig {
        current_time: now(),
        max_clients: MAX_GUESTS,
        protocol_id: PROTOCOL,
        authentication: ServerAuthentication::Unsecure,
        public_addresses: Vec::new(),
    };
    commands.insert_resource(RenetServer::new(connection(channels)));
    commands.insert_resource(NetcodeServerTransport::new(config, socket)?);
    // Not finding the world on the home network is no reason not to host it.
    match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)).and_then(|s| {
        s.set_nonblocking(true)?;
        s.set_broadcast(true)?;
        Ok(s)
    }) {
        Ok(socket) => commands.insert_resource(Beacon { socket, since: f32::INFINITY }),
        Err(e) => warn!("no discovery on the home network: {e}"),
    }
    Ok(())
}

/// This computer's address on the home network, to tell the other players.
pub fn local_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    // A VPN may route internet traffic through a different interface than the home network.
    #[cfg(target_os = "linux")]
    if let Some((gateway, _)) = linux_lan_route() {
        if socket.connect((gateway, 80)).is_ok() {
            return Some(socket.local_addr().ok()?.ip());
        }
    }
    // Connecting a UDP socket sends nothing; it only picks the outgoing interface.
    socket.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// The default gateway and its subnet broadcast, read without invoking system commands.
#[cfg(target_os = "linux")]
fn linux_lan_route() -> Option<(Ipv4Addr, Ipv4Addr)> {
    linux_lan_route_from(&std::fs::read_to_string("/proc/net/route").ok()?)
}

#[cfg(target_os = "linux")]
fn linux_lan_route_from(table: &str) -> Option<(Ipv4Addr, Ipv4Addr)> {
    let routes: Vec<_> = table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            Some((
                *fields.first()?,
                u32::from_str_radix(fields.get(1)?, 16).ok()?,
                u32::from_str_radix(fields.get(2)?, 16).ok()?,
                u32::from_str_radix(fields.get(3)?, 16).ok()?,
                fields.get(6)?.parse::<u32>().ok()?,
                u32::from_str_radix(fields.get(7)?, 16).ok()?,
            ))
        })
        .collect();
    let (interface, _, gateway, _, _, _) = routes
        .iter()
        .filter(|(_, destination, gateway, flags, _, _)| *destination == 0 && *gateway != 0 && flags & 3 == 3)
        .min_by_key(|(_, _, _, _, metric, _)| *metric)?;
    let (_, destination, _, _, _, mask) = routes.iter().find(|(name, destination, via, flags, _, mask)| {
        name == interface && *via == 0 && flags & 1 != 0 && *mask != 0 && gateway & mask == *destination
    })?;
    Some((Ipv4Addr::from(gateway.to_le_bytes()), Ipv4Addr::from((destination | !mask).to_le_bytes())))
}

/// The host's side of discovery: answers probes with the host's name.
#[derive(Resource)]
struct Beacon {
    socket: UdpSocket,
    since: f32,
}

fn answer_probes(mut beacon: ResMut<Beacon>, time: Res<Time<Real>>, settings: Res<Settings>) {
    let answer = [ANSWER, settings.player_name.as_bytes()].concat();
    beacon.since += time.delta_secs();
    if beacon.since >= 1.0 {
        beacon.since = 0.0;
        // Announce from the LAN interface so clients can find us even if their
        // own broadcast probes leave through a VPN.
        #[cfg(target_os = "linux")]
        let broadcast = linux_lan_route().map_or(Ipv4Addr::BROADCAST, |(_, broadcast)| broadcast);
        #[cfg(not(target_os = "linux"))]
        let broadcast = Ipv4Addr::BROADCAST;
        let _ = beacon.socket.send_to(&answer, (broadcast, DISCOVERY_PORT));
    }
    let mut buf = [0; 64];
    while let Ok((n, from)) = beacon.socket.recv_from(&mut buf) {
        if &buf[..n] == PROBE {
            let _ = beacon.socket.send_to(&answer, from);
        }
    }
}

/// Worlds found on the home network while the join page is open.
#[derive(Resource)]
pub struct Search {
    socket: UdpSocket,
    since: f32,
    pub found: Vec<(SocketAddr, String)>,
}

impl Search {
    pub fn start() -> Option<Self> {
        // Listening on the discovery port also receives unsolicited host announcements.
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT))
            .or_else(|_| UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)))
            .ok()?;
        socket.set_broadcast(true).ok()?;
        socket.set_nonblocking(true).ok()?;
        Some(Self { socket, since: f32::INFINITY, found: Vec::new() })
    }
}

fn search(time: Res<Time<Real>>, search: Option<ResMut<Search>>) {
    let Some(mut search) = search else { return };
    search.since += time.delta_secs();
    if search.since > 1.0 {
        search.since = 0.0;
        let _ = search.socket.send_to(PROBE, (Ipv4Addr::BROADCAST, DISCOVERY_PORT));
        // Limited broadcast can leave through a VPN. The subnet address reaches the LAN.
        #[cfg(target_os = "linux")]
        if let Some((_, broadcast)) = linux_lan_route() {
            let _ = search.socket.send_to(PROBE, (broadcast, DISCOVERY_PORT));
        }
    }
    let mut buf = [0; 128];
    while let Ok((n, from)) = search.socket.recv_from(&mut buf) {
        let Some(name) = buf[..n].strip_prefix(ANSWER) else { continue };
        let host = SocketAddr::new(from.ip(), PORT);
        if !search.found.iter().any(|(a, _)| *a == host) {
            search.found.push((host, String::from_utf8_lossy(name).into_owned()));
        }
    }
}

/// Starts connecting to a host; the world opens once its welcome arrives.
pub fn join(commands: &mut Commands, channels: &RepliconChannels, host: SocketAddr) -> std::io::Result<()> {
    let token = now().as_nanos() as u64 | 1;
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    let authentication =
        ClientAuthentication::Unsecure { client_id: token, protocol_id: PROTOCOL, server_addr: host, user_data: None };
    commands.insert_resource(RenetClient::new(connection(channels)));
    let transport = NetcodeClientTransport::new(now(), authentication, socket).map_err(std::io::Error::other)?;
    commands.insert_resource(transport);
    commands.insert_resource(Me(token));
    Ok(())
}

fn show_notices(mut notices: MessageReader<Notice>, mut toast: ResMut<Toast>) {
    for notice in notices.read() {
        toast.show(notice.0.clone());
    }
}

/// Tells one player something: the host directly, a guest over the network.
pub fn notify(notices: &mut MessageWriter<ToClients<Notice>>, to: ClientId, text: impl Into<String>) {
    notices.write(ToClients { targets: SendTargets::Single(to), message: Notice(text.into()) });
}

fn close(mut commands: Commands, remote: Query<Entity, With<Remote>>) {
    // What the host had replicated here goes with the connection.
    for entity in &remote {
        commands.entity(entity).try_despawn();
    }
    commands.remove_resource::<RenetServer>();
    commands.remove_resource::<NetcodeServerTransport>();
    commands.remove_resource::<Beacon>();
    commands.remove_resource::<RenetClient>();
    commands.remove_resource::<NetcodeClientTransport>();
    commands.remove_resource::<Joining>();
    commands.insert_resource(Me(0));
}

fn say_hello(me: Res<Me>, settings: Res<Settings>, mut hello: MessageWriter<Hello>) {
    hello.write(Hello { name: settings.player_name.clone(), token: me.0, skin: settings.skin });
}

fn enter_world(mut commands: Commands, mut welcomes: MessageReader<Welcome>, mut state: ResMut<NextState<AppState>>) {
    if let Some(welcome) = welcomes.read().last() {
        commands.remove_resource::<crate::worlds::CurrentWorld>();
        commands.insert_resource(Joining(welcome.clone()));
        state.set(AppState::Game);
    }
}

/// The host went away: back to the menu.
fn lost_host(state: Res<State<AppState>>, mut next: ResMut<NextState<AppState>>, mut toast: ResMut<Toast>) {
    if *state == AppState::Game {
        toast.show("Связь с хозяином мира потеряна".into());
        next.set(AppState::Menu);
    }
}

/// Lets a new guest in: their avatar, and the world as it is.
fn greet(
    mut commands: Commands,
    mut hellos: MessageReader<FromClient<Hello>>,
    terrain: Res<Terrain>,
    (creative, time, guests): (Res<crate::Creative>, Res<TimeOfDay>, Res<Guests>),
    host: Single<&Player>,
    mut welcomes: MessageWriter<ToClients<Welcome>>,
) {
    for hello in hellos.read() {
        let Some(client) = hello.client_id.entity() else { continue };
        let saved = guests.0.get(&hello.name).cloned();
        let at = saved.as_ref().map_or(host.spawn, |s| s.position);
        let avatar = commands
            .spawn((
                Replicated,
                DespawnOnExit(AppState::Game),
                Avatar { name: hello.name.clone(), token: hello.token, look: Look { skin: hello.skin, ..default() } },
                AvatarOf(hello.client_id),
                Transform::from_translation(at),
            ))
            .id();
        commands.entity(client).insert(GuestOf { avatar, name: hello.name.clone() });
        welcomes.write(ToClients {
            targets: SendTargets::Single(hello.client_id),
            message: Welcome {
                seed: terrain.seed(),
                creative: creative.0,
                time: time.0,
                spawn: host.spawn,
                edits: terrain.edits_snapshot().encode(),
                player: saved,
            },
        });
        info!("{} joined", hello.name);
    }
}

fn forget_guest(remove: On<Remove<GuestOf>>, mut commands: Commands, guests: Query<&GuestOf>) {
    if let Ok(guest) = guests.get(remove.entity) {
        info!("{} left", guest.name);
        commands.entity(guest.avatar).try_despawn();
    }
}

/// Edits made here go to everyone else.
fn send_edits(
    mut terrain: ResMut<Terrain>,
    client: Res<State<ClientState>>,
    mut requests: MessageWriter<EditRequest>,
    mut edits: MessageWriter<ToClients<Edits>>,
) {
    let journal = terrain.take_journal();
    if journal.is_empty() {
        return;
    }
    if is_guest(&client) {
        requests.write(EditRequest(encode(&journal)));
    } else {
        edits.write(ToClients { targets: SendTargets::CLIENTS_ONLY, message: Edits(encode(&journal)) });
    }
}

/// A guest's edits: applied here, passed on to the others, and checked for what they loosen.
fn take_requests(
    mut requests: MessageReader<FromClient<EditRequest>>,
    mut terrain: ResMut<Terrain>,
    mut edits: MessageWriter<ToClients<Edits>>,
    mut removed: MessageWriter<Removed>,
    guests: Query<&GuestOf>,
    avatars: Query<&Transform, With<Avatar>>,
) {
    for request in requests.read() {
        terrain.apply(decode(&request.message.0));
        let from = request
            .client_id
            .entity()
            .and_then(|c| guests.get(c).ok())
            .and_then(|g| avatars.get(g.avatar).ok())
            .map_or(Vec3::ZERO, |t| t.translation);
        let cells = decode(&request.message.0).filter(|(_, b)| b.is_air()).map(|(c, _)| c).collect();
        removed.write(Removed { cells, from });
        let targets = SendTargets::AllExcept(request.client_id);
        edits.write(ToClients { targets, message: Edits(request.message.0.clone()) });
    }
}

fn take_edits(mut edits: MessageReader<Edits>, mut terrain: ResMut<Terrain>) {
    for edit in edits.read() {
        terrain.apply(decode(&edit.0));
    }
}

fn send_pose(
    time: Res<Time<Real>>,
    player: Single<(&Player, &Transform)>,
    (settings, inventory, mode): (Res<Settings>, Res<Inventory>, Option<Res<State<Mode>>>),
    mut poses: MessageWriter<Pose>,
    mut since: Local<f32>,
) {
    *since += time.delta_secs();
    if *since < POSE_SECS {
        return;
    }
    *since = 0.0;
    let (player, transform) = *player;
    let look = Look::of(&settings, &inventory, mode.as_deref());
    poses.write(Pose { at: transform.translation, yaw: player.yaw, look });
}

fn move_avatars(
    mut poses: MessageReader<FromClient<Pose>>,
    guests: Query<&GuestOf>,
    mut avatars: Query<(&mut Avatar, &mut Transform)>,
) {
    for pose in poses.read() {
        let Some(guest) = pose.client_id.entity().and_then(|c| guests.get(c).ok()) else { continue };
        if let Ok((mut avatar, mut transform)) = avatars.get_mut(guest.avatar) {
            transform.translation = pose.message.at;
            transform.rotation = Quat::from_rotation_y(pose.message.yaw);
            // Only real changes go out to the others.
            if avatar.look != pose.message.look {
                avatar.look = pose.message.look;
            }
        }
    }
}

/// The host is seen by guests too.
fn host_avatar(
    mut commands: Commands,
    (settings, inventory, mode): (Res<Settings>, Res<Inventory>, Option<Res<State<Mode>>>),
    player: Single<(&Player, &Transform), Without<Avatar>>,
    mut avatars: Query<(&mut Avatar, &mut Transform)>,
) {
    let (player, at) = *player;
    let look = Look::of(&settings, &inventory, mode.as_deref());
    match avatars.iter_mut().find(|(a, _)| a.token == 0) {
        Some((mut avatar, mut transform)) => {
            transform.translation = at.translation;
            transform.rotation = Quat::from_rotation_y(player.yaw);
            if avatar.look != look {
                avatar.look = look;
            }
        }
        None => {
            commands.spawn((
                Replicated,
                DespawnOnExit(AppState::Game),
                Avatar { name: settings.player_name.clone(), token: 0, look },
                Transform::from_translation(at.translation),
            ));
        }
    }
}

fn sync_time(
    time: Res<Time<Real>>,
    time_of_day: Res<TimeOfDay>,
    mut syncs: MessageWriter<ToClients<TimeSync>>,
    mut since: Local<f32>,
) {
    *since += time.delta_secs();
    if *since >= TIME_SYNC_SECS {
        *since = 0.0;
        syncs.write(ToClients { targets: SendTargets::CLIENTS_ONLY, message: TimeSync(time_of_day.0) });
    }
}

fn take_time(mut syncs: MessageReader<TimeSync>, mut time_of_day: ResMut<TimeOfDay>) {
    if let Some(sync) = syncs.read().last() {
        time_of_day.0 = sync.0;
    }
}

/// A guest's things and whereabouts are kept in the host's save.
fn send_save(
    time: Res<Time<Real>>,
    player: Single<(&Player, &Transform), With<Body>>,
    (inventory, vitals, goals): (Res<Inventory>, Res<Vitals>, Res<Goals>),
    mut saves: MessageWriter<SavePlayer>,
    mut since: Local<f32>,
) {
    *since += time.delta_secs();
    if *since < SAVE_SECS {
        return;
    }
    *since = 0.0;
    let (player, transform) = *player;
    saves.write(SavePlayer(SavedPlayer::of(player, transform, &inventory, &vitals, &goals)));
}

fn keep_guests(mut saves: MessageReader<FromClient<SavePlayer>>, mut guests: ResMut<Guests>, names: Query<&GuestOf>) {
    for save in saves.read() {
        if let Some(guest) = save.client_id.entity().and_then(|c| names.get(c).ok()) {
            guests.0.insert(guest.name.clone(), save.message.0.clone());
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn finds_lan_broadcast_despite_vpn_route() {
        let table = "Iface Destination Gateway Flags RefCnt Use Metric Mask MTU Window IRTT\n\
                     wlan0 00000000 0101A8C0 0003 0 0 600 00000000 0 0 0\n\
                     tun0 000012AC 00000000 0001 0 0 0 FCFFFFFF 0 0 0\n\
                     wlan0 0001A8C0 00000000 0001 0 0 600 00FFFFFF 0 0 0\n";
        assert_eq!(linux_lan_route_from(table), Some((Ipv4Addr::new(192, 168, 1, 1), Ipv4Addr::new(192, 168, 1, 255))));
    }
}
