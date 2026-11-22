//! How players look to each other: a skin, the item in hand, a name overhead, walking and
//! lying in bed.

use bevy::{
    light::NotShadowCaster,
    prelude::*,
    world_serialization::{WorldAsset, WorldAssetRoot, WorldInstanceReady},
};

use crate::{
    AppState,
    drops::ItemLooks,
    items::Item,
    net::{Avatar, Me},
    player::PlayerCamera,
};

/// Model files in `assets/models/skins` and their names in the settings.
pub const SKINS: [(&str, &str); 8] = [
    ("adventurer", "Путешественник"),
    ("explorer", "Исследователь"),
    ("hooded", "В капюшоне"),
    ("hoodie", "В толстовке"),
    ("woman", "Путешественница"),
    ("worker", "Рабочий"),
    ("casual", "В футболке"),
    ("farmer", "Фермер"),
];
const SCALE: f32 = 0.9;
/// Names are read up to this far away.
const NAME_DISTANCE: f32 = 40.0;
/// Where an item is held, in the avatar's frame: right of the body, at hand height.
const HAND: Vec3 = Vec3::new(0.32, 0.9, -0.15);

pub struct AvatarsPlugin;

impl Plugin for AvatarsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load)
            .add_observer(name_tag)
            .add_systems(Update, (dress, pose, animate, hold, place_names).chain().run_if(in_state(AppState::Game)));
    }
}

/// Every skin's model and its idle, walk and run clips (all skins share one rig).
#[derive(Resource)]
struct Skins(Vec<(Handle<WorldAsset>, Handle<AnimationGraph>, [AnimationNodeIndex; 3])>);

fn load(mut commands: Commands, server: Res<AssetServer>, mut graphs: ResMut<Assets<AnimationGraph>>) {
    let skins = SKINS.map(|(file, _)| {
        let path = format!("models/skins/{file}.glb");
        let clips = [4, 22, 16].map(|i| server.load(GltfAssetLabel::Animation(i).from_asset(path.clone())));
        let (graph, nodes) = AnimationGraph::from_clips(clips);
        (server.load(GltfAssetLabel::Scene(0).from_asset(path)), graphs.add(graph), [nodes[0], nodes[1], nodes[2]])
    });
    commands.insert_resource(Skins(skins.into()));
}

/// The model an avatar wears now, and what's known about it once it has loaded.
#[derive(Component)]
struct Dressed {
    skin: u8,
    model: Entity,
    player: Option<Entity>,
    /// The item shown in hand, and its picture.
    held: Option<(Item, Entity)>,
    playing: usize,
    last: Vec3,
    /// Smoothed, since positions arrive in steps.
    speed: f32,
}

/// Puts on (or changes) the skin of every avatar but one's own.
fn dress(mut commands: Commands, me: Res<Me>, skins: Res<Skins>, avatars: Query<(Entity, &Avatar, Option<&Dressed>)>) {
    for (owner, avatar, dressed) in &avatars {
        let skin = avatar.look.skin.min(SKINS.len() as u8 - 1);
        if avatar.token == me.0 || dressed.is_some_and(|d| d.skin == skin) {
            continue;
        }
        if let Some(old) = dressed {
            commands.entity(old.model).despawn();
        }
        let (scene, graph, nodes) = skins.0[skin as usize].clone();
        let model = commands
            .spawn((WorldAssetRoot(scene), Transform::from_scale(Vec3::splat(SCALE)), ChildOf(owner)))
            .observe(
                move |ready: On<WorldInstanceReady>,
                      mut commands: Commands,
                      children: Query<&Children>,
                      mut players: Query<&mut AnimationPlayer>,
                      mut dressed: Query<&mut Dressed>| {
                    let Ok(mut dressed) = dressed.get_mut(owner) else { return };
                    for child in children.iter_descendants(ready.entity) {
                        if let Ok(mut player) = players.get_mut(child) {
                            player.play(nodes[0]).repeat();
                            commands.entity(child).insert(AnimationGraphHandle(graph.clone()));
                            dressed.player = Some(child);
                        }
                    }
                },
            )
            .id();
        commands.entity(owner).insert((
            Visibility::default(),
            Dressed { skin, model, player: None, held: None, playing: 0, last: Vec3::ZERO, speed: 0.0 },
        ));
    }
}

/// Upright, or lying on the back in bed.
fn pose(avatars: Query<(&Avatar, &Dressed)>, mut models: Query<&mut Transform>) {
    for (avatar, dressed) in &avatars {
        let Ok(mut transform) = models.get_mut(dressed.model) else { continue };
        // Models face +Z; players look towards -Z.
        let facing = Quat::from_rotation_y(std::f32::consts::PI);
        let (rotation, lift) = if avatar.look.asleep {
            (facing * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2), Vec3::new(0.0, -0.2, -0.9))
        } else {
            (facing, Vec3::ZERO)
        };
        transform.rotation = rotation;
        transform.translation = lift;
    }
}

fn animate(
    time: Res<Time>,
    skins: Res<Skins>,
    mut avatars: Query<(&Avatar, &Transform, &mut Dressed)>,
    mut players: Query<&mut AnimationPlayer>,
) {
    let dt = time.delta_secs().max(1e-3);
    for (avatar, transform, mut dressed) in &mut avatars {
        let speed = (transform.translation - dressed.last).with_y(0.0).length() / dt;
        dressed.last = transform.translation;
        dressed.speed += (speed - dressed.speed) * (1.0 - (-6.0 * dt).exp());
        let wanted = match dressed.speed {
            _ if avatar.look.asleep => 0,
            s if s > 5.0 => 2,
            s if s > 0.4 => 1,
            _ => 0,
        };
        let Some(Ok(mut player)) = dressed.player.map(|p| players.get_mut(p)) else { continue };
        if wanted == dressed.playing && player.is_playing_animation(skins.0[dressed.skin as usize].2[wanted]) {
            continue;
        }
        player.stop_all();
        player.play(skins.0[dressed.skin as usize].2[wanted]).repeat();
        dressed.playing = wanted;
    }
}

/// Shows the item in hand, held out in front of the right hand.
fn hold(mut commands: Commands, mut avatars: Query<(Entity, &Avatar, &mut Dressed)>, mut looks: ItemLooks) {
    for (owner, avatar, mut dressed) in &mut avatars {
        let wanted = avatar.look.held.filter(|_| !avatar.look.asleep);
        if dressed.held.map(|(item, _)| item) == wanted {
            continue;
        }
        if let Some((_, old)) = dressed.held.take() {
            commands.entity(old).despawn();
        }
        if let Some(item) = wanted {
            let (mesh, material) = looks.get(item);
            // The bones are scaled oddly in these models; the avatar's own frame is plainer.
            let part = commands
                .spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(material),
                    NotShadowCaster,
                    Transform::from_translation(HAND).with_rotation(Quat::from_rotation_y(-0.5)),
                    ChildOf(owner),
                ))
                .id();
            dressed.held = Some((item, part));
        }
    }
}

/// A player's name on screen, following their avatar.
#[derive(Component)]
struct NameTag(Entity);

fn name_tag(add: On<Add<Avatar>>, mut commands: Commands, avatars: Query<&Avatar>, me: Res<Me>) {
    let Ok(avatar) = avatars.get(add.entity) else { return };
    if avatar.token == me.0 {
        return;
    }
    commands.spawn((
        NameTag(add.entity),
        DespawnOnExit(AppState::Game),
        Text::new(avatar.name.clone()),
        TextFont { font_size: FontSize::Px(16.0), ..default() },
        TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.0, 0.7) },
        Node { position_type: PositionType::Absolute, ..default() },
        Visibility::Hidden,
        Pickable::IGNORE,
    ));
}

fn place_names(
    mut commands: Commands,
    mut tags: Query<(Entity, &NameTag, &mut Node, &mut Visibility, &ComputedNode)>,
    avatars: Query<(&Avatar, &GlobalTransform)>,
    camera: Single<(&Camera, &GlobalTransform), With<PlayerCamera>>,
) {
    let (camera, eye) = *camera;
    for (entity, tag, mut node, mut visibility, size) in &mut tags {
        let Ok((avatar, at)) = avatars.get(tag.0) else {
            commands.entity(entity).despawn();
            continue;
        };
        let height = if avatar.look.asleep { 1.0 } else { 1.95 };
        let head = at.translation() + Vec3::Y * height;
        let shown = head.distance(eye.translation()) < NAME_DISTANCE;
        match camera.world_to_viewport(eye, head).ok().filter(|_| shown) {
            Some(p) => {
                let half = size.size() * size.inverse_scale_factor() / 2.0;
                node.left = px(p.x - half.x);
                node.top = px(p.y - half.y * 2.0);
                visibility.set_if_neq(Visibility::Inherited);
            }
            None => {
                visibility.set_if_neq(Visibility::Hidden);
            }
        }
    }
}
