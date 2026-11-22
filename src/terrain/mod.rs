mod blocks;
mod generator;
mod material;
mod mesher;
mod shards;
mod store;
mod structures;
mod textures;
mod water;

use std::collections::{HashMap, HashSet};

use avian3d::prelude::*;
use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    light::NotShadowCaster,
    mesh::{Indices, PrimitiveTopology},
    pbr::MaterialPlugin,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};

pub use blocks::{Block, CELL, Material, Shape};
use generator::TREE_HEIGHT;
pub use generator::{Biome, TREE_REACH};
pub use material::TerrainMaterial;
pub use material::albedo;
pub use shards::{ALTAR, SHARDS_ABOVE, SHARDS_BELOW, SHARDS_Y};
pub use store::EditsSnapshot;
use store::{ChunkBlocks, ChunkKey, MeshJob, VoxelStore, block_in, chunk_of};
pub use water::SEA_LEVEL;

use crate::AppState;

/// Coarsest level of detail; root chunks are this many doublings of the finest.
const ROOT_LEVEL: u8 = 4;
/// Chunks closer than this many of their own widths split into finer ones. The finest level
/// (full-detail blocks with collision) only reaches a couple of dozen metres.
const SPLIT_DISTANCE: f32 = 1.1;
const SPLIT_TO_FINEST: f32 = 0.75;
/// Root chunk rows: the world spans -256 m to 256 m, and the Shards float in row 8.
const ROOT_ROWS: [i32; 3] = [-1, 0, 8];
/// Chunks sealed under the surface only matter to someone in a cave nearby.
const BURIED_VIEW: f32 = 20.0;
/// From inside a cave, until its fog is all but opaque.
const CAVE_VIEW: f32 = 40.0;
const MAX_TASKS_IN_FLIGHT: usize = 10;
/// Chunks this coarse are beyond the shadow distance.
const SHADOWLESS_LEVEL: u8 = 2;
/// How far (in cells) distant meshes may stray from the blocks when simplified.
const SIMPLIFY_ERROR: f32 = 1.0;
const SIMPLIFY_ERROR_FAR: f32 = 1.5;
/// Re-plan which chunks to show after moving this far.
const RESELECT_DISTANCE: f32 = 2.0;

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "terrain.wesl");
        embedded_asset!(app, "water.wesl");
        app.add_plugins((
            MaterialPlugin::<TerrainMaterial>::default(),
            MaterialPlugin::<water::WaterMaterial>::default(),
        ))
        .init_resource::<ViewDistance>()
        .add_systems(Startup, setup_assets)
        .add_systems(
            Update,
            (select_chunks, start_builds, finish_builds, retire_chunks, stream_water)
                .chain()
                .run_if(resource_exists::<Terrain>),
        );
    }
}

/// How far terrain is drawn, in metres.
#[derive(Resource)]
pub struct ViewDistance(pub f32);

impl Default for ViewDistance {
    fn default() -> Self {
        Self(300.0)
    }
}

/// Marks chunk entities, whose colliders stand in for blocks.
#[derive(Component)]
pub struct TerrainChunk;

/// Terrain is loaded around entities with this component.
#[derive(Component)]
pub struct TerrainAnchor;

#[derive(Resource)]
pub struct TerrainAssets {
    pub material: Handle<TerrainMaterial>,
    water: Handle<water::WaterMaterial>,
}

fn setup_assets(
    mut commands: Commands,
    mut terrain_materials: ResMut<Assets<TerrainMaterial>>,
    mut materials: ResMut<Assets<water::WaterMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    commands.insert_resource(TerrainAssets {
        material: terrain_materials.add(material::new_material(textures::load(&mut images))),
        water: materials.add(water::new_material()),
    });
}

#[derive(Resource)]
pub struct Terrain {
    store: VoxelStore,
    seed: u32,
    chunks: HashMap<ChunkKey, Chunk>,
    /// Chunks that should be on screen, and every ancestor of one.
    wanted: HashSet<ChunkKey>,
    wanted_ancestors: HashSet<ChunkKey>,
    selected_at: Option<Vec3>,
    /// Blocks of finest chunks near the player, for quick lookups.
    loaded: HashMap<IVec3, ChunkBlocks>,
    bounds: HashMap<(u8, IVec2), (f32, f32)>,
    water: HashMap<IVec2, Option<Entity>>,
    /// Local edits not yet passed on to other players.
    journal: Vec<(IVec3, Block)>,
}

#[derive(Default)]
struct Chunk {
    entity: Option<Entity>,
    task: Option<Task<Built>>,
    /// Edited while a task was in flight; rebuild once it finishes.
    stale: bool,
    /// Has a finished mesh (or is known to be empty).
    ready: bool,
    triangles: usize,
}

struct Built {
    mesh: Option<Mesh>,
    triangles: usize,
    collider: Option<Collider>,
    /// Finest chunks keep their blocks for lookups.
    blocks: Option<Vec<Block>>,
}

/// Where a ray meets a block.
pub struct BlockHit {
    pub cell: IVec3,
    /// Face of the cell the ray entered through.
    pub normal: IVec3,
    pub distance: f32,
}

impl Terrain {
    pub fn new(seed: u32) -> Self {
        Self {
            store: VoxelStore::new(seed),
            seed,
            chunks: HashMap::new(),
            wanted: HashSet::new(),
            wanted_ancestors: HashSet::new(),
            selected_at: None,
            loaded: HashMap::new(),
            bounds: HashMap::new(),
            water: HashMap::new(),
            journal: Vec::new(),
        }
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    pub fn surface_height(&self, x: f32, z: f32) -> f32 {
        self.store.generator().height(x, z)
    }

    pub fn biome(&self, x: f32, z: f32) -> Biome {
        let generator = self.store.generator();
        generator.biome(x, z, generator.height(x, z))
    }

    pub fn is_water(&self, p: Vec3) -> bool {
        // Ground dug out below the sea bed stays dry.
        p.y < SEA_LEVEL && p.y > self.surface_height(p.x, p.z) - 1.0
    }

    /// Where a rift sets someone down in the Shards.
    pub fn shards_arrival(&self) -> Vec3 {
        shards::arrival(&self.store.generator().shards)
    }

    /// Where trees were generated within `radius` of `center` (some may be felled since).
    pub fn trees_near(&self, center: Vec3, radius: f32) -> Vec<Vec3> {
        self.store.generator().trees_near(center, radius)
    }

    /// The closest ruin to a point: what it is and the middle of its floor.
    pub fn nearest_ruin(&self, p: Vec3) -> Option<(&'static str, Vec3)> {
        self.store.generator().nearest_structure(p)
    }

    /// A dry spot near the world origin, standing on the ground.
    pub fn find_spawn(&self) -> Vec3 {
        let mut p = IVec2::ZERO;
        let mut step = IVec2::X;
        for i in 0..10_000 {
            // Off the chunk grid's main lines, where seams between levels of detail line up.
            let (x, z) = (p.x as f32 + 5.5, p.y as f32 + 5.5);
            let h = self.surface_height(x, z);
            if h > SEA_LEVEL + 2.0 && h < 60.0 {
                return Vec3::new(x, self.ground_below(Vec3::new(x, h + TREE_HEIGHT, z)), z);
            }
            // Square spiral in 8 m steps.
            p += step * 8;
            if i % 2 == 1 {
                step = step.perp();
            }
        }
        Vec3::new(0.5, self.surface_height(0.5, 0.5) + 1.0, 0.5)
    }

    /// Top of the highest block at or below `p` (trees included).
    pub fn ground_below(&self, p: Vec3) -> f32 {
        let mut cell = (p / CELL).floor().as_ivec3();
        for _ in 0..4096 {
            if !self.block(cell).is_air() {
                return (cell.y + 1) as f32 * CELL;
            }
            cell.y -= 1;
        }
        p.y
    }

    pub fn block(&self, cell: IVec3) -> Block {
        let chunk = chunk_of(cell);
        if let Some(blocks) = self.store.edited(chunk).or_else(|| self.loaded.get(&chunk)) {
            return block_in(blocks, cell);
        }
        self.store.generator().sample(cell.as_vec3() * CELL, CELL, 1, true)[0]
    }

    /// Changes blocks and schedules the affected meshes for rebuilding; the changes are also
    /// kept for the network to pass on (see [`Terrain::take_journal`]).
    pub fn set_blocks(&mut self, changes: impl IntoIterator<Item = (IVec3, Block)>) {
        let changes: Vec<_> = changes.into_iter().collect();
        self.journal.extend(&changes);
        self.apply(changes);
    }

    /// Changes that came over the network: applied, but not passed on again.
    pub fn apply(&mut self, changes: impl IntoIterator<Item = (IVec3, Block)>) {
        let loaded = &self.loaded;
        let dirty = self.store.set_blocks(changes, |chunk| loaded.get(&chunk).cloned());
        for chunk in dirty {
            let key = ChunkKey { level: 0, pos: chunk };
            if let Some(slot) = self.chunks.get_mut(&key) {
                if slot.task.is_some() {
                    slot.stale = true;
                } else {
                    slot.task = Some(spawn_build(self.store.mesh_job(key)));
                }
            }
        }
    }

    /// Whether every mesh and collider between `lo` and `hi` shows the blocks as they are now.
    pub fn is_current(&self, lo: Vec3, hi: Vec3) -> bool {
        let lo = (lo / blocks::CHUNK_SIZE).floor().as_ivec3();
        let hi = (hi / blocks::CHUNK_SIZE).floor().as_ivec3();
        self.chunks.iter().all(|(key, chunk)| {
            key.level != 0 || chunk.task.is_none() || key.pos.cmplt(lo).any() || key.pos.cmpgt(hi).any()
        })
    }

    /// Steps through cells along a ray until one isn't empty.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_distance: f32) -> Option<BlockHit> {
        let dir = dir.normalize();
        let start = origin / CELL;
        let mut cell = start.floor().as_ivec3();
        let step = IVec3::new(dir.x.signum() as i32, dir.y.signum() as i32, dir.z.signum() as i32);
        // Ray length (in cells) between crossings of each axis, and to the first crossing.
        let axis = |s: f32, c: i32, d: f32| {
            if d == 0.0 {
                (f32::INFINITY, f32::INFINITY)
            } else {
                let to_boundary = if d > 0.0 { (c + 1) as f32 - s } else { s - c as f32 };
                (1.0 / d.abs(), to_boundary / d.abs())
            }
        };
        let (dx, tx) = axis(start.x, cell.x, dir.x);
        let (dy, ty) = axis(start.y, cell.y, dir.y);
        let (dz, tz) = axis(start.z, cell.z, dir.z);
        let delta = Vec3::new(dx, dy, dz);
        let mut t_max = Vec3::new(tx, ty, tz);
        let mut normal = IVec3::ZERO;
        let mut t = 0.0;
        while t * CELL <= max_distance {
            if !self.block(cell).is_air() {
                return Some(BlockHit { cell, normal, distance: t * CELL });
            }
            let axis = if t_max.x < t_max.y && t_max.x < t_max.z {
                0
            } else if t_max.y < t_max.z {
                1
            } else {
                2
            };
            t = t_max[axis];
            t_max[axis] += delta[axis];
            cell[axis] += step[axis];
            normal = IVec3::ZERO;
            normal[axis] = -step[axis];
        }
        None
    }

    /// True once the ground around `position` exists, with collision.
    pub fn is_ready_around(&self, position: Vec3) -> bool {
        self.selected_at.is_some() && [0.0, -2.0].iter().all(|dy| self.has_collision_at(position + Vec3::Y * *dy))
    }

    /// Whether solid ground at `p` (if any) is already collidable.
    pub fn has_collision_at(&self, p: Vec3) -> bool {
        let key = ChunkKey { level: 0, pos: (p / blocks::CHUNK_SIZE).floor().as_ivec3() };
        if let Some(chunk) = self.chunks.get(&key) {
            return chunk.ready;
        }
        // Not built: fine only if it's empty air that was never wanted, close to the player.
        self.selected_at.is_some_and(|eye| eye.distance(p) < 2.0 * blocks::CHUNK_SIZE)
            && !self.wanted.contains(&key)
            && !self.wanted.contains(&key.parent())
    }

    /// Chunks on screen, chunks being built, and triangles on screen.
    pub fn chunk_counts(&self) -> (usize, usize, usize) {
        let shown = self.chunks.values().filter(|c| c.entity.is_some()).count();
        let pending = self.chunks.values().filter(|c| c.task.is_some()).count();
        let triangles = self.chunks.values().filter(|c| c.entity.is_some()).map(|c| c.triangles).sum();
        (shown, pending, triangles)
    }

    /// Blocks changed here since the last call.
    pub fn take_journal(&mut self) -> Vec<(IVec3, Block)> {
        std::mem::take(&mut self.journal)
    }

    pub fn edits_snapshot(&self) -> EditsSnapshot {
        self.store.snapshot()
    }

    /// Restores saved edits. Only valid before any chunk is built.
    pub fn restore(&mut self, edits: &[u8]) -> Result<(), String> {
        self.store.decode_edits(edits)
    }

    fn bounds(&mut self, key: ChunkKey) -> (f32, f32) {
        let generator = self.store.generator();
        *self.bounds.entry((key.level, key.pos.xz())).or_insert_with(|| {
            let o = key.origin();
            generator.height_bounds(o.x, o.z, key.size())
        })
    }

    fn has_edits_inside(&self, key: ChunkKey) -> bool {
        if key.level == 0 {
            return self.store.is_edited(key.pos);
        }
        let scale = 1 << key.level;
        let min = key.pos * scale;
        let max = min + IVec3::splat(scale);
        (min.x..max.x)
            .any(|x| (min.y..max.y).any(|y| (min.z..max.z).any(|z| self.store.is_edited(IVec3::new(x, y, z)))))
    }

    /// Decides which chunks should be shown, from coarse far away to fine nearby.
    fn select(&mut self, eye: Vec3, view: f32) {
        let mut wanted = HashSet::new();
        let buried_view = if eye.y < self.surface_height(eye.x, eye.z) - 3.0 { CAVE_VIEW } else { BURIED_VIEW };
        let root = blocks::CHUNK_SIZE * (1 << ROOT_LEVEL) as f32;
        let lo = ((eye.xz() - view) / root).floor().as_ivec2();
        let hi = ((eye.xz() + view) / root).floor().as_ivec2();
        for z in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                for y in ROOT_ROWS {
                    let key = ChunkKey { level: ROOT_LEVEL, pos: IVec3::new(x, y, z) };
                    self.visit(key, eye, (view, buried_view), &mut wanted);
                }
            }
        }
        let mut ancestors = HashSet::new();
        for key in &wanted {
            let mut k = *key;
            while k.level < ROOT_LEVEL {
                k = k.parent();
                if !ancestors.insert(k) {
                    break;
                }
            }
        }
        self.wanted_ancestors = ancestors;
        self.wanted = wanted;
        self.selected_at = Some(eye);
    }

    /// `view` is how far terrain is drawn: anywhere, and sealed underground.
    fn visit(&mut self, key: ChunkKey, eye: Vec3, view: (f32, f32), wanted: &mut HashSet<ChunkKey>) {
        let origin = key.origin();
        let size = key.size();
        let distance = eye.distance(eye.clamp(origin, origin + size));
        if distance > view.0 {
            return;
        }
        let (ground_lo, ground_hi) = self.bounds(key);
        let above = origin.y > ground_hi + TREE_HEIGHT && !shards::contains(origin.y, origin.y + size);
        let buried = origin.y + size < ground_lo - 2.0;
        if (above || (buried && distance > view.1)) && !self.has_edits_inside(key) {
            return;
        }
        let split = if key.level == 1 { SPLIT_TO_FINEST } else { SPLIT_DISTANCE };
        if key.level > 0 && distance < split * size {
            for child in key.children() {
                self.visit(child, eye, view, wanted);
            }
        } else {
            wanted.insert(key);
        }
    }

    /// Whether everything wanted that overlaps `key` is ready, so `key` can go.
    fn replaced(&self, key: ChunkKey) -> bool {
        let mut up = key;
        while up.level < ROOT_LEVEL {
            up = up.parent();
            if self.wanted.contains(&up) {
                return self.chunks.get(&up).is_some_and(|c| c.ready);
            }
        }
        self.ready_below(key)
    }

    fn ready_below(&self, key: ChunkKey) -> bool {
        if key.level == 0 {
            return true;
        }
        key.children().all(|child| {
            if self.wanted.contains(&child) {
                self.chunks.get(&child).is_some_and(|c| c.ready)
            } else if self.wanted_ancestors.contains(&child) {
                self.ready_below(child)
            } else {
                true
            }
        })
    }
}

/// A mesh of loose blocks (cells relative to the mesh origin, at most a chunk across),
/// and its vertex positions for building a collider. `None` if nothing is visible.
pub fn mesh_blocks(cells: &[(IVec3, Block)]) -> Option<(Mesh, Vec<Vec3>)> {
    let n = mesher::SAMPLES;
    let mut grid = vec![Block::AIR; (n * n * n) as usize];
    for (cell, block) in cells {
        let s = *cell + IVec3::ONE;
        if s.cmplt(IVec3::ONE).any() || s.cmpge(IVec3::splat(n - 1)).any() {
            return None;
        }
        grid[(s.x + n * (s.y + n * s.z)) as usize] = *block;
    }
    let data = mesher::mesh(&grid, CELL);
    if data.is_empty() {
        return None;
    }
    let points = data.positions.iter().map(|p| Vec3::from(*p)).collect();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, data.positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, data.smooth);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, data.colors);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, data.ore);
    mesh.insert_indices(Indices::U32(data.indices));
    Some((mesh, points))
}

/// Largest loose piece that can come apart, in cells per axis.
pub const MAX_PIECE: i32 = blocks::CHUNK_CELLS - 2;

fn spawn_build(job: MeshJob) -> Task<Built> {
    AsyncComputeTaskPool::get().spawn(async move { build(&job) })
}

fn build(job: &MeshJob) -> Built {
    let samples = job.samples();
    let mut data = mesher::mesh(&samples, job.key.cell());
    if job.key.level > 0 && !data.is_empty() {
        let error = if job.key.level == 1 { SIMPLIFY_ERROR } else { SIMPLIFY_ERROR_FAR };
        data = data.simplified(job.key.size(), job.key.cell() * error);
    }
    let blocks = (job.key.level == 0).then(|| MeshJob::interior(&samples));
    tint_foliage(&mut data, job);
    if data.is_empty() {
        return Built { mesh: None, triangles: 0, collider: None, blocks };
    }
    let collider = (job.key.level == 0)
        .then(|| {
            let vertices = data.positions.iter().map(|p| Vec3::from(*p)).collect();
            Collider::try_trimesh(vertices, data.triangles()).ok()
        })
        .flatten();

    let triangles = data.indices.len() / 3;
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, data.positions);
    // The smoothed normal; faces get their own flat normal from screen-space derivatives.
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, data.smooth);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, data.colors);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, data.ore);
    mesh.insert_indices(Indices::U32(data.indices));
    Built { mesh: Some(mesh), triangles, collider, blocks }
}

/// Grass and leaves take their biome's colour, sampled on a coarse grid and interpolated.
fn tint_foliage(data: &mut mesher::ChunkMesh, job: &MeshJob) {
    const GRID: usize = 8;
    let size = job.key.size();
    let origin = job.key.origin();
    let step = size / GRID as f32;
    let tints: Vec<Vec3> = (0..(GRID + 1) * (GRID + 1))
        .map(|i| {
            let (x, z) = ((i % (GRID + 1)) as f32, (i / (GRID + 1)) as f32);
            Vec3::from(job.generator().foliage_tint(origin.x + x * step, origin.z + z * step))
        })
        .collect();
    let at = |x: usize, z: usize| tints[x.min(GRID) + z.min(GRID) * (GRID + 1)];
    for (i, color) in data.colors.iter_mut().enumerate() {
        let layer = data.ore[i][0] as u32;
        if layer != textures::GRASS_LAYER && layer != textures::LEAVES_LAYER {
            continue;
        }
        let p = data.positions[i];
        let g = Vec2::new(p[0], p[2]).clamp(Vec2::ZERO, Vec2::splat(size)) / step;
        let (x, z) = (g.x.floor() as usize, g.y.floor() as usize);
        let f = g - g.floor();
        let top = at(x, z).lerp(at(x + 1, z), f.x);
        let bottom = at(x, z + 1).lerp(at(x + 1, z + 1), f.x);
        let tint = top.lerp(bottom, f.y);
        color[..3].copy_from_slice(&tint.to_array());
    }
}

fn select_chunks(
    mut terrain: ResMut<Terrain>,
    view: Res<ViewDistance>,
    anchor: Single<&Transform, With<TerrainAnchor>>,
) {
    let eye = anchor.translation;
    if terrain.selected_at.is_some_and(|at| at.distance(eye) < RESELECT_DISTANCE) && !view.is_changed() {
        return;
    }
    terrain.select(eye, view.0);
}

fn start_builds(mut terrain: ResMut<Terrain>, anchor: Single<&Transform, With<TerrainAnchor>>) {
    let terrain = &mut *terrain;
    let in_flight = terrain.chunks.values().filter(|c| c.task.is_some()).count();
    if in_flight >= MAX_TASKS_IN_FLIGHT {
        return;
    }
    let eye = anchor.translation;
    let distance = |k: &ChunkKey| eye.distance(eye.clamp(k.origin(), k.origin() + k.size()));
    let mut missing: Vec<ChunkKey> =
        terrain.wanted.iter().filter(|k| !terrain.chunks.contains_key(k)).copied().collect();
    // Nearest first, measured to the chunk's box so big far chunks don't jump the queue.
    missing.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
    for key in missing.into_iter().take(MAX_TASKS_IN_FLIGHT - in_flight) {
        let task = spawn_build(terrain.store.mesh_job(key));
        terrain.chunks.insert(key, Chunk { task: Some(task), ..default() });
    }
}

fn finish_builds(
    mut commands: Commands,
    state: Res<State<AppState>>,
    mut terrain: ResMut<Terrain>,
    assets: Res<TerrainAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let terrain = &mut *terrain;
    for (&key, chunk) in &mut terrain.chunks {
        let Some(task) = &mut chunk.task else { continue };
        let Some(built) = block_on(poll_once(task)) else { continue };

        chunk.task = None;
        chunk.ready = true;
        chunk.triangles = built.triangles;
        if chunk.stale {
            chunk.stale = false;
            chunk.task = Some(spawn_build(terrain.store.mesh_job(key)));
        }
        if let Some(blocks) = built.blocks {
            terrain.loaded.insert(key.pos, std::sync::Arc::new(blocks));
        }

        match (built.mesh, chunk.entity) {
            (None, None) => {}
            (None, Some(entity)) => {
                commands.entity(entity).despawn();
                chunk.entity = None;
            }
            (Some(mesh), entity) => {
                let entity = entity.unwrap_or_else(|| {
                    commands
                        .spawn((
                            TerrainChunk,
                            DespawnOnExit(state.get().clone()),
                            MeshMaterial3d(assets.material.clone()),
                            Transform::from_translation(key.origin()),
                        ))
                        .id()
                });
                let mut e = commands.entity(entity);
                e.insert(Mesh3d(meshes.add(mesh)));
                // Shadows only reach a few dozen metres; distant chunks needn't cast them.
                if key.level >= SHADOWLESS_LEVEL {
                    e.insert(NotShadowCaster);
                }
                match built.collider {
                    Some(collider) => e.insert((RigidBody::Static, collider)),
                    None => e.remove::<Collider>(),
                };
                chunk.entity = Some(entity);
            }
        }
    }
}

/// Drops chunks nobody wants any more, once whatever replaces them is on screen.
fn retire_chunks(mut commands: Commands, mut terrain: ResMut<Terrain>) {
    let unwanted: Vec<ChunkKey> = terrain.chunks.keys().filter(|k| !terrain.wanted.contains(k)).copied().collect();
    for key in unwanted {
        // Unfinished builds of unwanted chunks are simply dropped.
        if terrain.chunks[&key].entity.is_some() && !terrain.replaced(key) {
            continue;
        }
        if let Some(entity) = terrain.chunks.remove(&key).and_then(|c| c.entity) {
            commands.entity(entity).despawn();
        }
        if key.level == 0 {
            terrain.loaded.remove(&key.pos);
        }
    }
}

fn stream_water(
    mut commands: Commands,
    state: Res<State<AppState>>,
    mut terrain: ResMut<Terrain>,
    view: Res<ViewDistance>,
    assets: Res<TerrainAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    anchor: Single<&Transform, With<TerrainAnchor>>,
) {
    let terrain = &mut *terrain;
    let center = anchor.translation.xz();
    let radius = view.0;
    let tile_center = |tile: IVec2| tile.as_vec2() * water::TILE + water::TILE / 2.0;
    terrain.water.retain(|tile, entity| {
        let keep = tile_center(*tile).distance(center) < radius + water::TILE;
        if !keep && let Some(e) = entity {
            commands.entity(*e).despawn();
        }
        keep
    });

    let lo = ((center - radius) / water::TILE).floor().as_ivec2();
    let hi = ((center + radius) / water::TILE).floor().as_ivec2();
    let mut built = 0;
    for z in lo.y..=hi.y {
        for x in lo.x..=hi.x {
            let tile = IVec2::new(x, z);
            if built >= 4 || terrain.water.contains_key(&tile) || tile_center(tile).distance(center) > radius {
                continue;
            }
            built += 1;
            let entity = water::mesh(terrain.store.generator(), tile).map(|mesh| {
                commands
                    .spawn((
                        DespawnOnExit(state.get().clone()),
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(assets.water.clone()),
                        Transform::from_translation(Vec3::new(tile.x as f32, 0.0, tile.y as f32) * water::TILE),
                        NotShadowCaster,
                    ))
                    .id()
            });
            terrain.water.insert(tile, entity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raycast_finds_the_ground_and_its_face() {
        let terrain = Terrain::new(1337);
        let (x, z) = (0.6, 0.6);
        let top = terrain.ground_below(Vec3::new(x, 200.0, z));
        let hit = terrain.raycast(Vec3::new(x, top + 3.0, z), Vec3::NEG_Y, 10.0).expect("ground below");
        assert_eq!(hit.normal, IVec3::Y);
        assert!((hit.distance - 3.0).abs() <= CELL + 1e-3, "distance {}", hit.distance);
    }

    #[test]
    fn selection_is_finer_near_the_eye_and_never_overlaps() {
        let mut terrain = Terrain::new(1337);
        let eye = Vec3::new(0.0, terrain.surface_height(0.0, 0.0) + 2.0, 0.0);
        terrain.select(eye, 300.0);
        assert!(terrain.wanted.iter().any(|k| k.level == 0), "no finest chunks near the eye");
        assert!(terrain.wanted.iter().any(|k| k.level >= 3), "no coarse chunks far away");
        for a in &terrain.wanted {
            for b in &terrain.wanted {
                assert!(a == b || (!a.contains(*b) && !b.contains(*a)), "{a:?} overlaps {b:?}");
            }
        }
        assert!(terrain.wanted.len() < 1500, "{} chunks", terrain.wanted.len());
    }
}
