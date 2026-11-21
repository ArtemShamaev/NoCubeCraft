//! Structural physics: pieces of the world that lose their support fall as rigid bodies —
//! a tree cut at the trunk topples, sand slides down — and turn back into blocks where they land.

use std::collections::{HashSet, VecDeque};

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    AppState,
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
        app.add_message::<Removed>()
            .add_systems(Update, (check_support, settle).chain().run_if(in_state(AppState::Game)));
    }
}

/// Cells just emptied, and where the player stood: pieces tip away from them.
#[derive(Message, Clone)]
pub struct Removed {
    pub cells: Vec<IVec3>,
    pub from: Vec3,
}

#[derive(Component)]
struct Piece {
    /// Blocks relative to the body's origin.
    cells: Vec<(IVec3, Block)>,
    age: f32,
    resting: f32,
}

fn neighbours(cell: IVec3) -> [IVec3; 6] {
    [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z].map(|d| cell + d)
}

/// Leaves hold nothing up: a cut trunk falls even if its crown touches a neighbour's.
fn supports(block: Block) -> bool {
    !block.is_air() && block.material != Material::Leaves
}

/// The piece connected to `start` (through anything but leaves), plus its own crown,
/// unless it is too big to be loose and so counts as part of the ground.
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
            if local.insert(next) && supports(terrain.block(next)) {
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

fn check_support(
    mut commands: Commands,
    mut removed: MessageReader<Removed>,
    mut terrain: ResMut<Terrain>,
    assets: Res<TerrainAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut sounds: MessageWriter<PlaySound>,
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
                if seen.contains(&next) || !supports(terrain.block(next)) {
                    continue;
                }
                if let Some(cells) = piece(&terrain, next, &mut seen) {
                    pieces.push((cells, true));
                }
            }
        }
        for (cells, tip) in pieces {
            let lo = cells.iter().fold(IVec3::MAX, |m, (c, _)| m.min(*c));
            let local: Vec<(IVec3, Block)> = cells
                .iter()
                .map(|(c, b)| (*c - lo, if b.shape == Shape::CUBE { *b } else { Block::solid(b.material) }))
                .collect();
            let Some((mesh, points)) = mesh_blocks(&local) else { continue };
            let Some(collider) = Collider::convex_hull(&points) else { continue };
            terrain.set_blocks(cells.iter().map(|(c, _)| (*c, Block::AIR)));
            let origin = lo.as_vec3() * CELL;
            // Tall pieces (trees) tip over, away from whoever cut them.
            let size = cells.iter().fold(IVec3::MIN, |m, (c, _)| m.max(*c)) - lo;
            let center = origin + (size.as_vec3() + 1.0) * CELL / 2.0;
            let away = (center - event.from).with_y(0.0).normalize_or(Vec3::X);
            let spin = if tip && size.y > size.x.max(size.z) { away.cross(Vec3::Y) * -0.9 } else { Vec3::ZERO };
            commands.spawn((
                DespawnOnExit(AppState::Game),
                Piece { cells: local, age: 0.0, resting: 0.0 },
                RigidBody::Dynamic,
                collider,
                AngularVelocity(spin),
                LinearVelocity(away * if tip { 0.6 } else { 0.0 }),
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(assets.material.clone()),
                Transform::from_translation(origin),
            ));
            sounds.write(PlaySound::at(if tip { Sfx::DigWood } else { Sfx::DigSoft }, center));
        }
    }
}

/// Bodies at rest become blocks again, wherever they ended up.
fn settle(
    mut commands: Commands,
    time: Res<Time>,
    mut terrain: ResMut<Terrain>,
    mut pieces: Query<(Entity, &mut Piece, &GlobalTransform, &LinearVelocity, &AngularVelocity)>,
    mut sounds: MessageWriter<PlaySound>,
) {
    let dt = time.delta_secs();
    for (entity, mut piece, transform, linear, angular) in &mut pieces {
        piece.age += dt;
        let still = linear.0.length() < REST_SPEED && angular.0.length() < REST_SPEED;
        piece.resting = if still { piece.resting + dt } else { 0.0 };
        if piece.resting < SETTLE_SECS && piece.age < MAX_FALL_SECS {
            continue;
        }
        let mut changes = Vec::new();
        for (cell, block) in &piece.cells {
            let world = transform.transform_point((cell.as_vec3() + 0.5) * CELL);
            let target = (world / CELL).floor().as_ivec3();
            if terrain.block(target).is_air() {
                changes.push((target, *block));
            }
        }
        if let Some((first, _)) = changes.first() {
            sounds.write(PlaySound::at(Sfx::Place, first.as_vec3() * CELL));
        }
        terrain.set_blocks(changes);
        commands.entity(entity).despawn();
    }
}
