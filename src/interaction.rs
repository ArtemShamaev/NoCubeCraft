//! The player's hands: digging, building, chopping, fighting, using things.

use avian3d::prelude::*;
use bevy::{input::mouse::AccumulatedMouseScroll, prelude::*};

use crate::{
    AppState, Creative, Mode,
    body::Body,
    building::{self, BlockSize, BuildTool},
    containers::{ChestState, FurnaceState, OpenContainer},
    effects::Debris,
    falling::{Activate, Removed},
    goals::ALTAR_CORES,
    hud::{Progress, Toast},
    items::{HOTBAR, Inventory, Item, PropKind, Tool, ToolKind},
    mobs::{Mob, MobKind, Summon, rand01},
    player::{self, Player, PlayerCamera},
    props::{self, Crop, Prop, PropAssets},
    sky::TimeOfDay,
    sound::{PlaySound, Sfx},
    survival::Vitals,
    terrain::{ALTAR, Block, CELL, Material, SHARDS_Y, Shape, Terrain, TerrainChunk},
};

const REACH: f32 = 5.0;
const ATTACK_REACH: f32 = 3.5;
const BREAK_PROP_SECS: f32 = 0.5;
const ATTACK_SECS: f32 = 0.45;
/// Holding the button repeats placing (and creative digging) this often.
const REPEAT_SECS: f32 = 0.22;
/// Chance per grass block dug of finding seeds, and per leaf block of a stick.
const SEED_CHANCE: f32 = 0.02;
const STICK_CHANCE: f32 = 0.03;
/// Zombies this close keep the player awake.
const SLEEP_SAFE_RADIUS: f32 = 16.0;
const MORNING: f32 = 0.0;

pub struct InteractionPlugin;

impl Plugin for InteractionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BuildTool>()
            .add_systems(OnEnter(AppState::Game), spawn_ghost)
            .add_systems(Startup, |mut gizmos: ResMut<GizmoConfigStore>| {
                gizmos.config_mut::<DefaultGizmoConfigGroup>().0.line.width = 1.0;
            })
            .add_systems(
                Update,
                (select_slot, switch_tool, aim, (primary, secondary), show_target)
                    .chain()
                    .run_if(in_state(Mode::Playing)),
            )
            .add_systems(OnExit(Mode::Playing), (stop_progress, hide_ghost));
    }
}

fn select_slot(keys: Res<ButtonInput<KeyCode>>, scroll: Res<AccumulatedMouseScroll>, mut inventory: ResMut<Inventory>) {
    const KEYS: [KeyCode; HOTBAR] = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    if let Some(i) = KEYS.iter().position(|&k| keys.just_pressed(k)) {
        inventory.selected = i;
    }
    if scroll.delta.y != 0.0 {
        let steps = scroll.delta.y.signum() as i32;
        inventory.selected = (inventory.selected as i32 - steps).rem_euclid(HOTBAR as i32) as usize;
    }
}

fn switch_tool(keys: Res<ButtonInput<KeyCode>>, mut tool: ResMut<BuildTool>, mut toast: ResMut<Toast>) {
    if keys.just_pressed(KeyCode::KeyR) {
        tool.size = tool.size.next();
        toast.show(format!("Размер блока: {}", tool.size.name()));
    }
    if keys.just_pressed(KeyCode::KeyF) {
        tool.shape = tool.shape.next();
        toast.show(format!("Форма: {}", tool.shape.name()));
    }
}

fn stop_progress(mut progress: ResMut<Progress>) {
    progress.0 = None;
}

/// What the crosshair points at this frame.
#[derive(Default)]
enum Target {
    #[default]
    Nothing,
    Block {
        cell: IVec3,
        normal: IVec3,
        point: Vec3,
    },
    Entity {
        entity: Entity,
        distance: f32,
    },
}

#[derive(Resource, Default)]
struct Aim(Target);

fn aim(
    mut commands: Commands,
    terrain: Res<Terrain>,
    spatial_query: SpatialQuery,
    camera: Single<&GlobalTransform, With<PlayerCamera>>,
    player: Query<(), (With<Player>, With<Body>)>,
    chunks: Query<(), With<TerrainChunk>>,
) {
    if player.is_empty() {
        commands.insert_resource(Aim(Target::Nothing));
        return;
    }
    let origin = camera.translation();
    let dir = camera.forward();
    let block = terrain.raycast(origin, *dir, REACH);
    // Props and creatures are physics objects; chunk colliders are skipped, blocks come from the grid.
    let thing = spatial_query
        .cast_ray_predicate(origin, dir, REACH, true, &SpatialQueryFilter::default(), &|e| !chunks.contains(e));
    let target = match (block, thing) {
        (Some(b), Some(t)) if t.distance < b.distance => Target::Entity { entity: t.entity, distance: t.distance },
        (Some(b), _) => Target::Block { cell: b.cell, normal: b.normal, point: origin + dir * b.distance },
        (None, Some(t)) => Target::Entity { entity: t.entity, distance: t.distance },
        (None, None) => Target::Nothing,
    };
    commands.insert_resource(Aim(target));
}

/// Hardness: seconds to dig one 25 cm block by hand.
fn dig_seconds(material: Material) -> f32 {
    match material {
        Material::Air => 0.0,
        Material::Leaves => 0.004,
        Material::Grass | Material::Dirt | Material::Sand | Material::Snow | Material::Gravel | Material::Clay => 0.012,
        Material::Cactus => 0.02,
        Material::Ice | Material::Sandstone => 0.06,
        Material::Wood | Material::Planks | Material::Brace | Material::Cache => 0.04,
        Material::Activator => 0.08,
        Material::Stone | Material::Bricks | Material::CoalOre | Material::IronOre => 0.12,
        Material::DiamondOre | Material::Core => 0.2,
        Material::Shardstone => 0.1,
        Material::Rift => 0.3,
        // Never dug: see `primary`.
        Material::Altar => 1.0,
    }
}

fn tool_for(material: Material) -> ToolKind {
    match material {
        Material::Stone
        | Material::Bricks
        | Material::CoalOre
        | Material::IronOre
        | Material::DiamondOre
        | Material::Sandstone
        | Material::Ice
        | Material::Activator
        | Material::Shardstone
        | Material::Core
        | Material::Rift
        | Material::Altar => ToolKind::Pickaxe,
        Material::Wood | Material::Planks | Material::Leaves | Material::Cactus | Material::Brace | Material::Cache => {
            ToolKind::Axe
        }
        _ => ToolKind::Shovel,
    }
}

fn tool_speed(tool: Option<Tool>, material: Material) -> f32 {
    tool.filter(|t| t.kind == tool_for(material)).map_or(1.0, |t| t.speed)
}

/// Cells of the block-sized region holding `cell`, and the solid ones' blocks.
fn dig_region(terrain: &Terrain, cell: IVec3, size: BlockSize) -> Vec<(IVec3, Block)> {
    let n = size.cells();
    let origin = building::dig_origin(cell, n);
    let mut cells = Vec::new();
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let c = origin + IVec3::new(x, y, z);
                let block = terrain.block(c);
                if !block.is_air() {
                    cells.push((c, block));
                }
            }
        }
    }
    cells
}

/// Particles and sounds that confirm what the hands did.
#[derive(bevy::ecs::system::SystemParam)]
struct Feedback<'w> {
    debris: MessageWriter<'w, Debris>,
    sounds: MessageWriter<'w, PlaySound>,
    removed: MessageWriter<'w, Removed>,
}

impl Feedback<'_> {
    /// Dug-out blocks: a burst of their most common material and its sound.
    fn dug(&mut self, region: &[(IVec3, Block)], size: BlockSize, from: Vec3) {
        let Some((_, first)) = region.first() else { return };
        // Whatever this held up may now fall.
        self.removed.write(Removed { cells: region.iter().map(|(c, _)| *c).collect(), from });
        let material = region
            .iter()
            .map(|(_, b)| b.material)
            .max_by_key(|m| region.iter().filter(|(_, b)| b.material == *m).count())
            .unwrap_or(first.material);
        let n = size.cells();
        let center = building::dig_origin(region[0].0, n).as_vec3() * CELL + Vec3::splat(n as f32 * CELL / 2.0);
        self.debris.write(Debris { at: center, size: n as f32 * CELL, material });
        let sfx = match tool_for(material) {
            ToolKind::Pickaxe => Sfx::DigStone,
            ToolKind::Axe => Sfx::DigWood,
            ToolKind::Shovel | ToolKind::Sword => Sfx::DigSoft,
        };
        self.sounds.write(PlaySound::at(sfx, center));
    }
}

/// Work in progress on whatever the left button holds: a block region, a prop.
#[derive(Default)]
struct Work {
    key: Option<(IVec3, BlockSize, Option<Entity>)>,
    done: f32,
    repeat: f32,
}

/// Left button: dig, chop, break props, attack.
fn primary(
    mut commands: Commands,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Single<&bevy::window::CursorOptions>,
    aim: Res<Aim>,
    tool_settings: Res<BuildTool>,
    creative: Res<Creative>,
    player: Single<&Transform, (With<Player>, With<Body>)>,
    mut terrain: ResMut<Terrain>,
    mut inventory: ResMut<Inventory>,
    mut toast: ResMut<Toast>,
    mut progress: ResMut<Progress>,
    props: Query<(&Prop, Option<&Crop>, &Transform, Option<&FurnaceState>, Option<&ChestState>)>,
    mut mobs: Query<(&mut Mob, &mut Body, &Transform), Without<Player>>,
    (mut work, mut attack_cooldown): (Local<Work>, Local<f32>),
    mut feedback: Feedback,
) {
    let dt = time.delta_secs();
    *attack_cooldown -= dt;
    work.repeat -= dt;
    if !mouse.pressed(MouseButton::Left) || !player::cursor_grabbed(&cursor) {
        *work = Work::default();
        progress.0 = None;
        return;
    }
    let tool = inventory.selected_tool();

    match aim.0 {
        Target::Nothing => {
            work.key = None;
            progress.0 = None;
        }
        Target::Entity { entity, distance, .. } => {
            if let Ok((mut mob, mut body, transform)) = mobs.get_mut(entity) {
                work.key = None;
                progress.0 = None;
                if distance <= ATTACK_REACH && *attack_cooldown <= 0.0 {
                    *attack_cooldown = ATTACK_SECS;
                    let damage = tool.map_or(1.5, |t| t.damage);
                    mob.hit(&mut body, damage, player.translation, transform.translation);
                    inventory.wear_selected(1);
                    feedback.sounds.write(PlaySound::at(Sfx::Hit, transform.translation + Vec3::Y));
                }
                return;
            }
            let Ok((prop, crop, prop_transform, furnace, chest)) = props.get(entity) else { return };
            let key = (IVec3::ZERO, tool_settings.size, Some(entity));
            if work.key != Some(key) {
                *work = Work { key: Some(key), ..default() };
            }
            work.done += if creative.0 { 1.0 } else { dt / BREAK_PROP_SECS };
            progress.0 = Some(work.done);
            if work.done < 1.0 {
                return;
            }
            *work = Work::default();
            progress.0 = None;
            commands.entity(entity).despawn();
            feedback.sounds.write(PlaySound::at(Sfx::DigWood, prop_transform.translation));
            if creative.0 {
                return;
            }
            let gained: &[(Item, u32)] = match prop.0 {
                PropKind::Furnace => &[(Item::Furnace, 1)],
                PropKind::Torch => &[(Item::Torch, 1)],
                PropKind::Bed => &[(Item::Bed, 1)],
                PropKind::Wheat if crop.is_some_and(Crop::ripe) => &[(Item::Wheat, 1), (Item::Seeds, 2)],
                PropKind::Wheat => &[(Item::Seeds, 1)],
                PropKind::CraftingTable => &[(Item::CraftingTable, 1)],
                PropKind::Chest => &[(Item::Chest, 1)],
            };
            give(&mut inventory, &mut toast, gained);
            // Whatever was inside comes along.
            let inside = furnace
                .into_iter()
                .flat_map(FurnaceState::contents)
                .chain(chest.into_iter().flat_map(|c| c.slots.iter().flatten().copied()));
            for stack in inside {
                inventory.add_stack(stack);
            }
        }
        Target::Block { cell, .. } => {
            let size = tool_settings.size;
            let mut region = dig_region(&terrain, cell, size);
            // The altar stays, whatever is done to it.
            region.retain(|(_, b)| b.material != Material::Altar);
            if region.is_empty() {
                return;
            }
            let key = (building::dig_origin(cell, size.cells()), size, None);
            if work.key != Some(key) {
                *work = Work { key: Some(key), done: 0.0, repeat: work.repeat };
            }
            if creative.0 {
                // Instant, repeating while held.
                if work.repeat > 0.0 {
                    return;
                }
                work.repeat = REPEAT_SECS;
                terrain.set_blocks(region.iter().map(|(c, _)| (*c, Block::AIR)));
                feedback.dug(&region, size, player.translation);
                return;
            }
            let seconds: f32 = region.iter().map(|(_, b)| dig_seconds(b.material) / tool_speed(tool, b.material)).sum();
            work.done += dt / seconds.max(0.05);
            progress.0 = Some(work.done);
            if work.done < 1.0 {
                return;
            }
            *work = Work::default();
            progress.0 = None;
            feedback.dug(&region, size, player.translation);
            dig(&mut terrain, &mut inventory, &mut toast, &region, tool);
        }
    }
}

fn dig(
    terrain: &mut Terrain,
    inventory: &mut Inventory,
    toast: &mut Toast,
    region: &[(IVec3, Block)],
    tool: Option<Tool>,
) {
    terrain.set_blocks(region.iter().map(|(c, _)| (*c, Block::AIR)));
    let pickaxe = tool.filter(|t| t.kind == ToolKind::Pickaxe).map(|t| t.tier);
    let mut counts: Vec<(Item, u32)> = Vec::new();
    let mut add_n = |item: Item, count: u32| match counts.iter_mut().find(|(i, _)| *i == item) {
        Some((_, n)) => *n += count,
        None => counts.push((item, count)),
    };
    let mut used_tool = 0;
    let mut lost_ore = None;
    for (cell, block) in region {
        let mut add = |item: Item| add_n(item, 1);
        match Item::mined(block.material, pickaxe) {
            Some(item) => add(item),
            None => {
                if matches!(block.material, Material::IronOre | Material::DiamondOre) {
                    lost_ore = Some(block.material);
                }
            }
        }
        let roll = chance(*cell);
        if block.material == Material::Grass && roll < SEED_CHANCE {
            add(Item::Seeds);
        }
        if block.material == Material::Leaves && roll < STICK_CHANCE {
            add(Item::Stick);
        }
        if block.material == Material::Cache {
            let (item, count) = loot(roll);
            add_n(item, count);
        }
        if tool.is_some_and(|t| t.kind == tool_for(block.material)) {
            used_tool += 1;
        }
    }
    inventory.wear_selected(used_tool);
    match lost_ore {
        Some(Material::IronOre) => toast.show("Железо добывается каменной киркой или лучше".into()),
        Some(_) => toast.show("Алмазы добываются железной киркой или лучше".into()),
        None => {}
    }
    give(inventory, toast, &counts);
}

/// What one cell of a ruin's cache holds.
fn loot(roll: f32) -> (Item, u32) {
    const TABLE: [(f32, Item, u32); 7] = [
        (0.28, Item::Arrow, 4),
        (0.46, Item::IronIngot, 2),
        (0.60, Item::Bone, 2),
        (0.70, Item::Diamond, 1),
        (0.78, Item::Bow, 1),
        (0.90, Item::Bread, 2),
        (1.00, Item::Coal, 4),
    ];
    TABLE.iter().find(|(p, ..)| roll < *p).map_or((Item::Coal, 1), |&(_, item, n)| (item, n))
}

/// A repeatable pseudo-random number in [0, 1) for a cell.
fn chance(cell: IVec3) -> f32 {
    let mut h = (cell.x as u32).wrapping_mul(73_856_093)
        ^ (cell.y as u32).wrapping_mul(19_349_663)
        ^ (cell.z as u32).wrapping_mul(83_492_791);
    h = h.wrapping_mul(2_654_435_761);
    (h >> 8) as f32 / 16_777_216.0
}

fn give(inventory: &mut Inventory, toast: &mut Toast, items: &[(Item, u32)]) {
    let mut lost = 0;
    for &(item, count) in items.iter().filter(|(_, n)| *n > 0) {
        lost += inventory.add(item, count);
        toast.show(format!("+{count} {}", item.name()));
    }
    if lost > 0 {
        toast.show("Инвентарь полон".into());
    }
}

/// Where a right click would build: the block's corner cell and the empty cells it fills
/// (relative to that corner), with their shapes.
fn placement(
    aim: &Aim,
    tool: &BuildTool,
    camera: &GlobalTransform,
    terrain: &Terrain,
) -> Option<(IVec3, Vec<(IVec3, Shape)>)> {
    let Target::Block { cell, normal, .. } = aim.0 else { return None };
    let n = tool.size.cells();
    let origin = building::place_origin(cell, normal, n);
    let dir = building::facing(*camera.forward());
    let free = building::compose(tool.shape, n, dir)
        .into_iter()
        .filter(|(p, _)| terrain.block(origin + *p).is_air())
        .collect();
    Some((origin, free))
}

/// Whether a cell overlaps the player's capsule (approximated by its box).
fn blocks_player(cell: IVec3, feet: Vec3) -> bool {
    let lo = cell.as_vec3() * CELL;
    let hi = lo + CELL;
    let r = player::RADIUS - 0.02;
    lo.x < feet.x + r
        && hi.x > feet.x - r
        && lo.z < feet.z + r
        && hi.z > feet.z - r
        && lo.y < feet.y + player::HEIGHT
        && hi.y > feet.y + 0.02
}

/// Right button: build, place props, eat, sleep, open the furnace.
fn secondary(
    mut commands: Commands,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Single<&bevy::window::CursorOptions>,
    aim: Res<Aim>,
    (tool, creative): (Res<BuildTool>, Res<Creative>),
    camera: Single<&GlobalTransform, With<PlayerCamera>>,
    player: Single<(Entity, &mut Player, &mut Transform), With<Body>>,
    mut terrain: ResMut<Terrain>,
    mut inventory: ResMut<Inventory>,
    (mut vitals, mut toast): (ResMut<Vitals>, ResMut<Toast>),
    (mut time_of_day, mut mode, mut open): (ResMut<TimeOfDay>, ResMut<NextState<Mode>>, ResMut<OpenContainer>),
    prop_assets: Res<PropAssets>,
    props: Query<&Prop>,
    mut mobs: Query<(&mut Mob, &Transform), Without<Player>>,
    (mut repeat, mut sounds, mut activate, mut summons): (
        Local<f32>,
        MessageWriter<PlaySound>,
        MessageWriter<Activate>,
        MessageWriter<Summon>,
    ),
) {
    *repeat -= time.delta_secs();
    let pressed = mouse.just_pressed(MouseButton::Right);
    if !mouse.pressed(MouseButton::Right) || !player::cursor_grabbed(&cursor) {
        *repeat = 0.0;
        return;
    }
    let (entity, mut player, mut transform) = player.into_inner();
    let feet = transform.translation;
    let selected = inventory.selected();

    if pressed {
        // Beds and furnaces work whatever is in hand.
        if let Target::Entity { entity, .. } = aim.0
            && let Ok(prop) = props.get(entity)
        {
            match prop.0 {
                PropKind::Furnace | PropKind::CraftingTable | PropKind::Chest => {
                    *open = match prop.0 {
                        PropKind::Furnace => OpenContainer::Furnace(entity),
                        PropKind::Chest => OpenContainer::Chest(entity),
                        _ => OpenContainer::Workbench,
                    };
                    mode.set(Mode::Inventory);
                    return;
                }
                PropKind::Bed => {
                    player.spawn = feet;
                    if !time_of_day.is_night() {
                        toast.show("Точка возрождения сохранена. Спать можно только ночью".into());
                    } else if mobs.iter().any(|(m, t)| m.hostile() && t.translation.distance(feet) < SLEEP_SAFE_RADIUS)
                    {
                        toast.show("Рядом зомби, уснуть не получится".into());
                    } else {
                        time_of_day.0 = MORNING;
                        toast.show("Доброе утро! Точка возрождения сохранена".into());
                    }
                    return;
                }
                _ => {}
            }
        }

        // Bones win wolves over; a dog sits or follows when clicked.
        if let Target::Entity { entity, .. } = aim.0
            && let Ok((mut mob, _)) = mobs.get_mut(entity)
        {
            if mob.tame {
                mob.sitting = !mob.sitting;
                toast.show(if mob.sitting { "Пёс сидит" } else { "Пёс идёт за вами" }.into());
                return;
            }
            if mob.kind == MobKind::Wolf && selected.is_some_and(|s| s.item == Item::Bone) {
                if !creative.0 {
                    inventory.consume_selected(1);
                }
                let roll = rand01(entity.to_bits() as u32 ^ (time.elapsed_secs() * 1000.0) as u32);
                toast.show(if mob.tame_with_bone(roll) {
                    "Волк приручён! Теперь это ваш пёс".into()
                } else {
                    "Волк не доверяет вам. Ещё кость?".into()
                });
                return;
            }
        }

        if let Target::Block { cell, .. } = aim.0 {
            match terrain.block(cell).material {
                Material::Altar => {
                    if mobs.iter().any(|(m, _)| m.kind == MobKind::Warden && m.alive()) {
                        toast.show("Страж уже здесь".into());
                    } else if !creative.0 && !inventory.remove(Item::Core, ALTAR_CORES) {
                        toast.show(format!("Алтарю нужно {ALTAR_CORES} ядер осколка"));
                    } else {
                        // Beyond the altar, as seen from the way here.
                        let at = Vec3::new(ALTAR.x, SHARDS_Y + 3.1, ALTAR.y - 5.0);
                        summons.write(Summon { kind: MobKind::Warden, at, yaw: std::f32::consts::PI });
                        sounds.write(PlaySound::at(Sfx::Rattle, at));
                        toast.show("Страж Осколков пробудился!".into());
                    }
                    return;
                }
                Material::Activator => {
                    activate.write(Activate { cell, from: feet });
                    return;
                }
                Material::Rift => {
                    let (to, message) = if player::in_shards(feet) {
                        (player.spawn, "Разлом вернул вас домой")
                    } else {
                        (terrain.shards_arrival(), "Осколки. Под ногами — пустота")
                    };
                    player::teleport(&mut commands, entity, &mut transform, to);
                    sounds.write(PlaySound::ui(Sfx::Place));
                    toast.show(message.into());
                    return;
                }
                _ => {}
            }
        }

        let Some(stack) = selected else { return };
        if stack.item == Item::Glue {
            // Glue smears over a whole block-sized region; on glued blocks it wipes off.
            let Target::Block { cell, .. } = aim.0 else { return };
            let glue = !terrain.block(cell).glued;
            let region = dig_region(&terrain, cell, tool.size);
            let at = (cell.as_vec3() + 0.5) * CELL;
            terrain.set_blocks(region.into_iter().map(|(c, b)| (c, Block { glued: glue, ..b })));
            sounds.write(PlaySound::at(Sfx::DigSoft, at));
            if glue && !creative.0 {
                inventory.consume_selected(1);
            }
            return;
        }
        if let Some((hunger, health)) = stack.item.food() {
            if vitals.eat(hunger, health) {
                inventory.consume_selected(1);
            }
            return;
        }
        if let Some(kind) = stack.item.prop() {
            let Target::Block { cell, normal, point } = aim.0 else { return };
            if normal != IVec3::Y {
                toast.show("Ставится только сверху на блок".into());
                return;
            }
            let ground = terrain.block(cell).material;
            if kind == PropKind::Wheat && !matches!(ground, Material::Grass | Material::Dirt) {
                toast.show("Семена сажают в землю или траву".into());
                return;
            }
            let solid = matches!(kind, PropKind::Furnace | PropKind::Bed);
            if solid && point.with_y(feet.y).distance(feet) < player::RADIUS + 0.9 {
                toast.show("Слишком близко к себе".into());
                return;
            }
            // Face the player, standing exactly on top of the clicked block.
            let to_player = (feet - point).with_y(0.0);
            let yaw = (-to_player.x).atan2(-to_player.z);
            let at = point.with_y((cell.y + 1) as f32 * CELL);
            props::spawn(
                &mut commands,
                &prop_assets,
                kind,
                Transform::from_translation(at).with_rotation(Quat::from_rotation_y(yaw)),
                0.0,
            );
            sounds.write(PlaySound::at(Sfx::Place, at));
            if !creative.0 {
                inventory.consume_selected(1);
            }
            return;
        }
    }

    // Building with blocks, repeating while the button is held.
    let Some(stack) = selected else { return };
    let Some(material) = stack.item.material() else { return };
    if *repeat > 0.0 {
        return;
    }
    *repeat = REPEAT_SECS;
    let Some((origin, cells)) = placement(&aim, &tool, &camera, &terrain) else { return };
    if cells.is_empty() {
        return;
    }
    if cells.iter().any(|(p, _)| blocks_player(origin + *p, feet)) {
        toast.show("Мешаете сами себе".into());
        return;
    }
    let needed = cells.len() as u32;
    if !creative.0 && stack.count < needed {
        toast.show(format!("Нужно {needed} × «{}», есть {}", stack.item.name(), stack.count));
        return;
    }
    sounds.write(PlaySound::at(Sfx::Place, (origin + cells[0].0).as_vec3() * CELL));
    terrain.set_blocks(cells.into_iter().map(|(p, shape)| (origin + p, Block::new(material, shape))));
    if !creative.0 {
        inventory.consume_selected(needed);
    }
}

#[derive(Component)]
struct Ghost;

fn spawn_ghost(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    commands.spawn((
        Ghost,
        DespawnOnExit(AppState::Game),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.85, 0.92, 1.0, 0.14),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            cull_mode: None,
            ..default()
        })),
        bevy::light::NotShadowCaster,
        Transform::default(),
        Visibility::Hidden,
    ));
}

fn hide_ghost(mut ghost: Query<&mut Visibility, With<Ghost>>) {
    for mut v in &mut ghost {
        *v = Visibility::Hidden;
    }
}

/// Outlines what digging would take and previews what building would add.
fn show_target(
    mut commands: Commands,
    mut gizmos: Gizmos,
    aim: Res<Aim>,
    tool: Res<BuildTool>,
    inventory: Res<Inventory>,
    terrain: Res<Terrain>,
    progress: Res<Progress>,
    camera: Single<&GlobalTransform, With<PlayerCamera>>,
    ghost: Single<(Entity, &mut Transform, &mut Visibility), With<Ghost>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut shown: Local<Option<Vec<(IVec3, Shape)>>>,
) {
    let (ghost_entity, mut ghost_transform, mut ghost_visibility) = ghost.into_inner();
    let building = inventory.selected().is_some_and(|s| s.item.material().is_some());
    match aim.0 {
        Target::Block { cell, .. } => {
            let n = tool.size.cells();
            let origin = building::dig_origin(cell, n);
            let size = Vec3::splat(n as f32 * CELL + 0.01);
            let center = origin.as_vec3() * CELL + size / 2.0 - 0.005;
            let k = progress.0.unwrap_or(0.0).clamp(0.0, 1.0);
            // A faint dark outline that warms up while digging, so it never hides the block.
            let color = Color::srgba(0.05 + 0.6 * k, 0.05 + 0.2 * k, 0.05, 0.22 + 0.4 * k);
            gizmos.cube(Transform::from_translation(center).with_scale(size), color);
        }
        Target::Entity { .. } | Target::Nothing => {}
    }

    let preview = building.then(|| placement(&aim, &tool, &camera, &terrain)).flatten();
    match preview {
        Some((origin, cells)) if !cells.is_empty() => {
            if shown.as_ref() != Some(&cells) {
                commands.entity(ghost_entity).insert(Mesh3d(meshes.add(building::ghost_mesh(&cells))));
                *shown = Some(cells);
            }
            ghost_transform.translation = origin.as_vec3() * CELL;
            *ghost_visibility = Visibility::Inherited;
        }
        _ => *ghost_visibility = Visibility::Hidden,
    }
}
