//! Structural physics: pieces of the world that lose their support fall as rigid bodies —
//! a tree cut at the trunk topples, sand slides down — and turn back into blocks where they land.
//! Only trees break loose this way: rock, earth and buildings stay put however they are dug.
//! Leaves break where they hit something, and fall away once no wood holds them.

use std::collections::{HashSet, VecDeque};

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    AppState,
    body::{Body, Layer},
    effects::Debris,
    sound::{PlaySound, Sfx},
    terrain::{Block, CELL, MAX_PIECE, Material, Shape, Terrain, TerrainAssets, mesh_blocks},
};

/// Pieces bigger than this many cells count as part of the ground.
const MAX_CELLS: usize = 9000;
/// A body slower than this for `SETTLE_SECS` has come to rest.
const REST_SPEED: f32 = 0.35;
const SETTLE_SECS: f32 = 0.6;
/// Bodies freeze back into blocks after this long whatever happens.
const MAX_FALL_SECS: f32 = 12.0;
/// Crumbly materials fall when there is nothing right under them.
fn loose(material: Material) -> bool {
    matches!(material, Material::Sand | Material::Snow | Material::Gravel)
}

pub struct FallingPlugin;

impl Plugin for FallingPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Removed>().add_message::<Activate>().add_systems(
            Update,
            (activate, check_support, release, crash, settle).chain().run_if(in_state(AppState::Game)),
        );
    }
}

/// Cells just emptied, and where the player stood: pieces tip away from them.
#[derive(Message, Clone)]
pub struct Removed {
    pub cells: Vec<IVec3>,
    pub from: Vec3,
}

/// A physics activator was used: its structure comes loose as one body.
#[derive(Message, Clone)]
pub struct Activate {
    pub cell: IVec3,
    pub from: Vec3,
}

#[derive(Component)]
struct Piece {
    /// Blocks relative to the body's origin.
    cells: Vec<(IVec3, Block)>,
    /// Linear and angular velocity to start falling with, once the ground's colliders no
    /// longer hold the blocks this piece came from.
    launch: Option<(Vec3, Vec3)>,
    age: f32,
    resting: f32,
    /// The lowest row it came loose from: wood below is its own stump.
    base: i32,
}

impl Piece {
    /// Where a cell's centre is now.
    fn world(&self, transform: &GlobalTransform, cell: IVec3) -> Vec3 {
        transform.transform_point((cell.as_vec3() + 0.5) * CELL)
    }
}

fn is_leaves(block: Block) -> bool {
    block.material == Material::Leaves
}

fn neighbours(cell: IVec3) -> [IVec3; 6] {
    [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z].map(|d| cell + d)
}

/// Leaves hold nothing up: a cut trunk falls even if its crown touches a neighbour's.
fn supports(block: Block) -> bool {
    !block.is_air() && !is_leaves(block)
}

/// The wood connected to `start`, plus its own crown, unless it rests on anything else
/// or is too big to be loose.
fn piece(terrain: &Terrain, start: IVec3, seen: &mut HashSet<IVec3>) -> Option<Vec<(IVec3, Block)>> {
    let mut cells = Vec::new();
    let mut queue = VecDeque::from([start]);
    let mut local: HashSet<IVec3> = HashSet::from([start]);
    let (mut lo, mut hi) = (start, start);
    while let Some(cell) = queue.pop_front() {
        let block = terrain.block(cell);
        cells.push((cell, block));
        lo = lo.min(cell);
        hi = hi.max(cell);
        if cells.len() > MAX_CELLS || (hi - lo).max_element() >= MAX_PIECE {
            seen.extend(local);
            return None;
        }
        for next in neighbours(cell) {
            let block = terrain.block(next);
            if supports(block) && block.material != Material::Wood {
                seen.extend(local);
                return None;
            }
            if local.insert(next) && supports(block) {
                queue.push_back(next);
            }
        }
    }
    seen.extend(local.iter().copied());

    // The crown: leaves reachable from the piece that don't touch some other trunk.
    let own: HashSet<IVec3> = cells.iter().map(|(c, _)| *c).collect();
    let mut leaves = VecDeque::new();
    for (cell, _) in &cells {
        leaves.extend(neighbours(*cell).into_iter().filter(|n| terrain.block(*n).material == Material::Leaves));
    }
    let reach = (crate::terrain::TREE_REACH / CELL) as i32;
    let mut crown: HashSet<IVec3> = HashSet::new();
    while let Some(leaf) = leaves.pop_front() {
        if crown.contains(&leaf) || crown.len() > MAX_CELLS {
            continue;
        }
        let near = leaf.x >= lo.x - reach && leaf.x <= hi.x + reach && leaf.z >= lo.z - reach && leaf.z <= hi.z + reach;
        let foreign = neighbours(leaf).iter().any(|n| !own.contains(n) && terrain.block(*n).material == Material::Wood);
        if !near || foreign {
            continue;
        }
        crown.insert(leaf);
        leaves.extend(neighbours(leaf).into_iter().filter(|n| terrain.block(*n).material == Material::Leaves));
    }
    cells.extend(crown.into_iter().map(|c| (c, terrain.block(c))));
    let (lo, hi) = cells.iter().fold((IVec3::MAX, IVec3::MIN), |(l, h), (c, _)| (l.min(*c), h.max(*c)));
    ((hi - lo).max_element() < MAX_PIECE).then_some(cells)
}

/// A column of crumbly blocks resting on nothing, from `cell` upwards.
fn loose_column(terrain: &Terrain, cell: IVec3) -> Option<Vec<(IVec3, Block)>> {
    if !terrain.block(cell - IVec3::Y).is_air() {
        return None;
    }
    let mut column = Vec::new();
    let mut c = cell;
    loop {
        let block = terrain.block(c);
        if block.is_air() || !loose(block.material) || column.len() > 64 {
            break;
        }
        column.push((c, block));
        c.y += 1;
    }
    (!column.is_empty()).then_some(column)
}

/// The structure an activator holds together: itself, glued blocks and braces joined to it
/// or to each other, and whatever touches a brace. `None` if it is too big to move.
fn structure(terrain: &Terrain, activator: IVec3) -> Option<Vec<(IVec3, Block)>> {
    let binds = |b: Block| b.glued || matches!(b.material, Material::Brace | Material::Activator);
    let mut cells = vec![(activator, terrain.block(activator))];
    let mut seen = HashSet::from([activator]);
    let mut queue = VecDeque::from([activator]);
    while let Some(cell) = queue.pop_front() {
        // Braces (and the activator) grab every block they touch; glue only sticks to glue.
        let holds_all = matches!(terrain.block(cell).material, Material::Brace | Material::Activator);
        for next in neighbours(cell) {
            let block = terrain.block(next);
            if block.is_air() || seen.contains(&next) || !(holds_all || binds(block)) {
                continue;
            }
            seen.insert(next);
            cells.push((next, block));
            if binds(block) {
                queue.push_back(next);
            }
            if cells.len() > MAX_CELLS {
                return None;
            }
        }
    }
    let (lo, hi) = bounds(&cells);
    ((hi - lo).max_element() < MAX_PIECE).then_some(cells)
}

/// Leaves connected to `start` that no wood holds up any more, unless there are too many to tell.
fn orphan_leaves(terrain: &Terrain, start: IVec3, seen: &mut HashSet<IVec3>) -> Vec<IVec3> {
    let mut cluster = Vec::new();
    let mut queue = VecDeque::from([start]);
    seen.insert(start);
    let mut held = false;
    while let Some(cell) = queue.pop_front() {
        cluster.push(cell);
        for next in neighbours(cell) {
            let block = terrain.block(next);
            held |= block.material == Material::Wood;
            if is_leaves(block) && seen.insert(next) {
                queue.push_back(next);
            }
        }
        // Keep flooding even once held, so the rest of the crown isn't checked again.
        if cluster.len() > MAX_CELLS {
            held = true;
        }
    }
    if held { Vec::new() } else { cluster }
}

/// A few bursts of leaves spread over `cells`.
fn shed(cells: &[Vec3], debris: &mut MessageWriter<Debris>, sounds: &mut MessageWriter<PlaySound>) {
    let Some(first) = cells.first() else { return };
    for at in cells.iter().step_by(cells.len().div_ceil(12)) {
        debris.write(Debris { at: *at, size: 0.6, material: Material::Leaves });
    }
    sounds.write(PlaySound::at(Sfx::DigSoft, *first));
}

/// A rigid body for blocks: the mesh shows them all, the collider is the hull of all but
/// leaves, so a crown sinks into the ground instead of propping its tree up.
fn body_parts(cells: &[(IVec3, Block)]) -> Option<(Mesh, Collider)> {
    let (mesh, _) = mesh_blocks(cells)?;
    let solid: Vec<_> = cells.iter().filter(|(_, b)| !is_leaves(*b)).copied().collect();
    let (_, points) = mesh_blocks(&solid)?;
    Some((mesh, Collider::convex_hull(&points)?))
}

fn bounds(cells: &[(IVec3, Block)]) -> (IVec3, IVec3) {
    cells.iter().fold((IVec3::MAX, IVec3::MIN), |(l, h), (c, _)| (l.min(*c), h.max(*c)))
}

/// Turns blocks into one body that starts falling with `launch` (linear and angular velocity)
/// once the ground has been rebuilt without them. `straight` ones only drop, without turning.
fn loosen(
    commands: &mut Commands,
    terrain: &mut Terrain,
    meshes: &mut Assets<Mesh>,
    assets: &TerrainAssets,
    cells: &[(IVec3, Block)],
    launch: (Vec3, Vec3),
    straight: bool,
) -> bool {
    let (lo, _) = bounds(cells);
    let local: Vec<(IVec3, Block)> = cells.iter().map(|(c, b)| (*c - lo, Block { shape: Shape::CUBE, ..*b })).collect();
    let Some((mesh, collider)) = body_parts(&local) else { return false };
    terrain.set_blocks(cells.iter().map(|(c, _)| (*c, Block::AIR)));
    let mut piece = commands.spawn((
        DespawnOnExit(AppState::Game),
        Piece { cells: local, launch: Some(launch), age: 0.0, resting: 0.0, base: lo.y },
        RigidBody::Static,
        collider,
        // Bodies walk through falling pieces rather than get shoved into the ground.
        CollisionLayers::new(Layer::Falling, Layer::Solid),
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(assets.material.clone()),
        Transform::from_translation(lo.as_vec3() * CELL),
    ));
    if straight {
        piece.insert(LockedAxes::ROTATION_LOCKED.lock_translation_x().lock_translation_z());
    }
    true
}

/// An activated structure comes loose; whatever it held up may fall in turn.
fn activate(
    mut commands: Commands,
    mut events: MessageReader<Activate>,
    mut terrain: ResMut<Terrain>,
    assets: Res<TerrainAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut removed: MessageWriter<Removed>,
    mut sounds: MessageWriter<PlaySound>,
) {
    for event in events.read() {
        let Some(cells) = structure(&terrain, event.cell) else { continue };
        if loosen(&mut commands, &mut terrain, &mut meshes, &assets, &cells, (Vec3::ZERO, Vec3::ZERO), false) {
            sounds.write(PlaySound::at(Sfx::Place, (event.cell.as_vec3() + 0.5) * CELL));
            removed.write(Removed { cells: cells.iter().map(|(c, _)| *c).collect(), from: event.from });
        }
    }
}

fn check_support(
    mut commands: Commands,
    mut removed: MessageReader<Removed>,
    mut terrain: ResMut<Terrain>,
    assets: Res<TerrainAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut sounds: MessageWriter<PlaySound>,
    mut debris: MessageWriter<Debris>,
) {
    for event in removed.read() {
        let mut seen = HashSet::new();
        let mut pieces = Vec::new();
        for &cell in &event.cells {
            // Loose material right above falls on its own.
            if let Some(column) = loose_column(&terrain, cell + IVec3::Y) {
                seen.extend(column.iter().map(|(c, _)| *c));
                pieces.push((column, false));
            }
            for next in neighbours(cell) {
                if seen.contains(&next) || terrain.block(next).material != Material::Wood {
                    continue;
                }
                if let Some(cells) = piece(&terrain, next, &mut seen) {
                    pieces.push((cells, true));
                }
            }
        }
        // Leaves around whatever went away may have lost their tree.
        let mut around: Vec<IVec3> = event.cells.clone();
        for (cells, tip) in pieces {
            let (lo, hi) = bounds(&cells);
            let center = (lo + hi + 1).as_vec3() * CELL / 2.0;
            // Tall pieces (trees) tip over, away from whoever cut them; sand drops straight down.
            let away = (center - event.from).with_y(0.0).normalize_or(Vec3::X);
            let size = hi - lo;
            let spin = if tip && size.y > size.x.max(size.z) { away.cross(Vec3::Y) * -0.9 } else { Vec3::ZERO };
            let launch = (away * if tip { 0.6 } else { 0.0 }, spin);
            if loosen(&mut commands, &mut terrain, &mut meshes, &assets, &cells, launch, !tip) {
                around.extend(cells.iter().map(|(c, _)| *c));
                sounds.write(PlaySound::at(if tip { Sfx::DigWood } else { Sfx::DigSoft }, center));
            }
        }
        let mut checked = HashSet::new();
        let mut orphans = Vec::new();
        for cell in around {
            for next in neighbours(cell) {
                if !checked.contains(&next) && is_leaves(terrain.block(next)) {
                    orphans.extend(orphan_leaves(&terrain, next, &mut checked));
                }
            }
        }
        terrain.set_blocks(orphans.iter().map(|c| (*c, Block::AIR)));
        let at: Vec<Vec3> = orphans.iter().map(|c| (c.as_vec3() + 0.5) * CELL).collect();
        shed(&at, &mut debris, &mut sounds);
    }
}

/// Pieces start falling once the ground they broke off from has been rebuilt without them.
fn release(mut commands: Commands, terrain: Res<Terrain>, mut pieces: Query<(Entity, &mut Piece, &Transform)>) {
    for (entity, mut piece, transform) in &mut pieces {
        let Some((linear, angular)) = piece.launch else { continue };
        let size = piece.cells.iter().fold(IVec3::ZERO, |m, (c, _)| m.max(*c + 1)).as_vec3() * CELL;
        if !terrain.is_current(transform.translation - CELL, transform.translation + size + CELL) {
            continue;
        }
        piece.launch = None;
        commands.entity(entity).insert((RigidBody::Dynamic, LinearVelocity(linear), AngularVelocity(angular)));
    }
}

/// Trunk cells moving faster than this snap the trunks they hit.
const CRASH_SPEED: f32 = 2.0;

/// A falling tree crashes through what stands in its way: its own leaves break where they hit
/// something, leaves it sweeps through break, and a trunk it hits snaps at the foot and falls too.
fn crash(
    mut terrain: ResMut<Terrain>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut pieces: Query<(
        &mut Piece,
        &GlobalTransform,
        &LinearVelocity,
        &AngularVelocity,
        &ComputedCenterOfMass,
        &mut Mesh3d,
    )>,
    mut removed: MessageWriter<Removed>,
    mut debris: MessageWriter<Debris>,
    mut sounds: MessageWriter<PlaySound>,
) {
    for (mut piece, transform, linear, angular, com, mut mesh) in &mut pieces {
        if piece.launch.is_some() {
            continue;
        }
        let center = transform.transform_point(com.0);
        // Leaning on something and stopped: whatever trunk props it up gives way.
        let stalled = piece.resting > 0.2 && transform.rotation().mul_vec3(Vec3::Y).y < 0.97;
        let base = piece.base;
        let foreign = |c: IVec3| c.y > base + 4 && terrain.block(c).material == Material::Wood;
        let mut broken = Vec::new();
        let mut swept = Vec::new();
        let mut hit = None;
        piece.cells.retain(|(cell, block)| {
            let at = transform.transform_point((cell.as_vec3() + 0.5) * CELL);
            let inside = terrain.block((at / CELL).floor().as_ivec3());
            if is_leaves(*block) {
                let lost = !inside.is_air();
                if lost {
                    broken.push(at);
                }
                return !lost;
            }
            let velocity = linear.0 + angular.0.cross(at - center);
            // Leaves never hold a trunk up: it sweeps through any around it.
            let near = (at / CELL).floor().as_ivec3();
            for x in -1..=1 {
                for y in -1..=1 {
                    for z in -1..=1 {
                        let c = near + IVec3::new(x, y, z);
                        if is_leaves(terrain.block(c)) {
                            swept.push(c);
                        } else if stalled && foreign(c) {
                            hit = hit.or(Some(c));
                        }
                    }
                }
            }
            // Where this cell is about to be.
            let ahead = ((at + velocity.normalize_or_zero() * CELL) / CELL).floor().as_ivec3();
            if velocity.length() > CRASH_SPEED && foreign(ahead) {
                hit = hit.or(Some(ahead));
            }
            true
        });
        if !broken.is_empty() {
            if let Some((rest, _)) = mesh_blocks(&piece.cells) {
                mesh.0 = meshes.add(rest);
            }
            shed(&broken, &mut debris, &mut sounds);
        }
        if !swept.is_empty() {
            swept.sort_unstable_by_key(|c| c.to_array());
            swept.dedup();
            terrain.set_blocks(swept.iter().map(|c| (*c, Block::AIR)));
            let at: Vec<Vec3> = swept.iter().map(|c| (c.as_vec3() + 0.5) * CELL).collect();
            shed(&at, &mut debris, &mut sounds);
        }
        if let Some(cell) = hit {
            let cells = foot(&terrain, cell);
            terrain.set_blocks(cells.iter().map(|c| (*c, Block::AIR)));
            sounds.write(PlaySound::at(Sfx::DigWood, (cell.as_vec3() + 0.5) * CELL));
            removed.write(Removed { cells, from: center });
        }
    }
}

/// The slice of trunk just above the foot of the tree that `cell` (trunk or branch) belongs
/// to, where it snaps.
fn foot(terrain: &Terrain, cell: IVec3) -> Vec<IVec3> {
    let is_wood = |c: IVec3| terrain.block(c).material == Material::Wood;
    // The lowest wood connected to `cell`.
    let mut base = cell;
    let mut seen = HashSet::from([cell]);
    let mut queue = VecDeque::from([cell]);
    while let Some(c) = queue.pop_front() {
        if c.y < base.y {
            base = c;
        }
        for next in neighbours(c) {
            if seen.len() < MAX_CELLS && is_wood(next) && seen.insert(next) {
                queue.push_back(next);
            }
        }
    }
    let start = base + IVec3::Y;
    if !is_wood(start) {
        return vec![cell];
    }
    let mut slice = vec![start];
    let mut queue = VecDeque::from([start]);
    while let Some(c) = queue.pop_front() {
        for next in [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z].map(|d| c + d) {
            if slice.len() < 64 && !slice.contains(&next) && is_wood(next) {
                slice.push(next);
                queue.push_back(next);
            }
        }
    }
    slice
}

/// The 90° rotation nearest to `rotation`, as where it takes the X, Y and Z axes.
fn snap(rotation: Quat) -> [IVec3; 3] {
    let m = Mat3::from_quat(rotation);
    let columns = [m.x_axis, m.y_axis, m.z_axis];
    let mut axes = [IVec3::ZERO; 3];
    let (mut free_columns, mut free_rows) = (vec![0, 1, 2], vec![0, 1, 2]);
    // Greedily pair each local axis with the world axis it is most aligned with.
    while !free_columns.is_empty() {
        let (c, r) = free_columns
            .iter()
            .flat_map(|c| free_rows.iter().map(move |r| (*c, *r)))
            .max_by(|a, b| columns[a.0][a.1].abs().total_cmp(&columns[b.0][b.1].abs()))
            .expect("pairs left");
        let mut axis = IVec3::ZERO;
        axis[r] = columns[c][r].signum() as i32;
        axes[c] = axis;
        free_columns.retain(|x| *x != c);
        free_rows.retain(|x| *x != r);
    }
    // Never a mirror image.
    axes[2] = axes[0].cross(axes[1]);
    axes
}

/// Whether a cell overlaps a body standing at `feet` (approximated by its box).
fn inside(cell: IVec3, body: &Body, feet: Vec3) -> bool {
    let lo = cell.as_vec3() * CELL;
    let hi = lo + CELL;
    let r = body.radius;
    lo.x < feet.x + r
        && hi.x > feet.x - r
        && lo.z < feet.z + r
        && hi.z > feet.z - r
        && lo.y < feet.y + body.height
        && hi.y > feet.y
}

/// Bodies at rest become blocks again, lined up with the world's grid, wherever they ended up.
/// Blocks that would land inside something are lost.
fn settle(
    mut commands: Commands,
    time: Res<Time>,
    mut terrain: ResMut<Terrain>,
    mut pieces: Query<(Entity, &mut Piece, &GlobalTransform, &LinearVelocity, &AngularVelocity)>,
    bodies: Query<(&Body, &Transform)>,
    mut debris: MessageWriter<Debris>,
    mut sounds: MessageWriter<PlaySound>,
) {
    let dt = time.delta_secs();
    for (entity, mut piece, transform, linear, angular) in &mut pieces {
        if piece.launch.is_some() {
            continue;
        }
        piece.age += dt;
        let still = linear.0.length() < REST_SPEED && angular.0.length() < REST_SPEED;
        piece.resting = if still { piece.resting + dt } else { 0.0 };
        if piece.resting < SETTLE_SECS && piece.age < MAX_FALL_SECS {
            continue;
        }
        commands.entity(entity).despawn();
        let blocks = &piece.cells;
        if blocks.is_empty() {
            continue;
        }
        // Turn about the cell nearest the middle, so the shape stays whole.
        let middle = blocks.iter().map(|(c, _)| c.as_vec3()).sum::<Vec3>() / blocks.len() as f32;
        let pivot = middle.round().as_ivec3();
        let target = (piece.world(transform, pivot) / CELL - 0.5).round().as_ivec3();
        let [x, y, z] = snap(transform.rotation());
        let mut changes = Vec::new();
        let mut lost = Vec::new();
        for (cell, block) in blocks {
            let d = *cell - pivot;
            let at = target + x * d.x + y * d.y + z * d.z;
            let occupied = bodies.iter().any(|(body, feet)| inside(at, body, feet.translation));
            if terrain.block(at).is_air() && !occupied {
                changes.push((at, *block));
            } else if is_leaves(*block) {
                lost.push((at.as_vec3() + 0.5) * CELL);
            }
        }
        shed(&lost, &mut debris, &mut sounds);
        if let Some((first, _)) = changes.first() {
            sounds.write(PlaySound::at(Sfx::Place, first.as_vec3() * CELL));
        }
        terrain.set_blocks(changes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapped_rotations_are_whole_quarter_turns() {
        assert_eq!(snap(Quat::IDENTITY), [IVec3::X, IVec3::Y, IVec3::Z]);
        // A trunk that fell over most of the way lies along the ground.
        let fallen = snap(Quat::from_rotation_z(1.3));
        assert_eq!(fallen[1], IVec3::NEG_X);
        let leaning = snap(Quat::from_rotation_x(0.7) * Quat::from_rotation_y(0.78));
        assert_eq!(leaning[0].cross(leaning[1]), leaning[2]);
        assert!(leaning.iter().all(|a| a.abs().element_sum() == 1));
    }
}
