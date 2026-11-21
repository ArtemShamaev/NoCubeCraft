use std::{collections::HashMap, sync::Arc};

use bevy::math::{IVec3, Vec3};

use super::{
    blocks::{Block, CELL, CHUNK_CELLS, CHUNK_SIZE},
    generator::Generator,
    mesher::SAMPLES,
};

/// Blocks of one finest-level chunk, `CHUNK_CELLS`³, x fastest.
pub type ChunkBlocks = Arc<Vec<Block>>;

/// A chunk at some level of detail. Level 0 is `CHUNK_SIZE` metres with 25 cm cells;
/// each level up doubles both.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChunkKey {
    pub level: u8,
    pub pos: IVec3,
}

impl ChunkKey {
    pub fn size(self) -> f32 {
        CHUNK_SIZE * (1 << self.level) as f32
    }

    pub fn cell(self) -> f32 {
        CELL * (1 << self.level) as f32
    }

    pub fn origin(self) -> Vec3 {
        self.pos.as_vec3() * self.size()
    }

    pub fn parent(self) -> ChunkKey {
        ChunkKey { level: self.level + 1, pos: self.pos.div_euclid(IVec3::splat(2)) }
    }

    pub fn children(self) -> impl Iterator<Item = ChunkKey> {
        (0..8).map(move |i| ChunkKey {
            level: self.level - 1,
            pos: self.pos * 2 + IVec3::new(i & 1, (i >> 1) & 1, i >> 2),
        })
    }

    /// Whether `other` lies inside this chunk (or is it).
    #[cfg(test)]
    pub fn contains(self, other: ChunkKey) -> bool {
        other.level <= self.level && other.pos.div_euclid(IVec3::splat(1 << (self.level - other.level))) == self.pos
    }
}

/// The finest-level chunk holding a cell.
pub fn chunk_of(cell: IVec3) -> IVec3 {
    cell.div_euclid(IVec3::splat(CHUNK_CELLS))
}

fn local_index(local: IVec3) -> usize {
    (local.x + CHUNK_CELLS * (local.y + CHUNK_CELLS * local.z)) as usize
}

/// The world's blocks: generated on demand, with edited chunks kept in memory.
pub struct VoxelStore {
    generator: Arc<Generator>,
    edited: HashMap<IVec3, ChunkBlocks>,
}

/// Everything a background task needs to build one chunk's mesh.
pub struct MeshJob {
    pub key: ChunkKey,
    generator: Arc<Generator>,
    edited: Vec<(IVec3, ChunkBlocks)>,
}

impl MeshJob {
    pub fn generator(&self) -> &Generator {
        &self.generator
    }
}

impl VoxelStore {
    pub fn new(seed: u32) -> Self {
        Self { generator: Arc::new(Generator::new(seed)), edited: HashMap::new() }
    }

    pub fn generator(&self) -> &Generator {
        &self.generator
    }

    pub fn is_edited(&self, chunk: IVec3) -> bool {
        self.edited.contains_key(&chunk)
    }

    pub fn edited(&self, chunk: IVec3) -> Option<&ChunkBlocks> {
        self.edited.get(&chunk)
    }

    /// Generated blocks of a finest-level chunk.
    pub fn generate(&self, chunk: IVec3) -> Vec<Block> {
        self.generator.sample(chunk.as_vec3() * CHUNK_SIZE, CELL, CHUNK_CELLS as usize, true)
    }

    pub fn mesh_job(&self, key: ChunkKey) -> MeshJob {
        // Finest chunks touched by this chunk and its one-cell apron.
        let min = ((key.origin() - key.cell()) / CHUNK_SIZE).floor().as_ivec3();
        let max = ((key.origin() + key.size() + key.cell()) / CHUNK_SIZE).floor().as_ivec3();
        let edited = if (max - min + 1).element_product() as usize > self.edited.len() {
            self.edited
                .iter()
                .filter(|(c, _)| c.cmpge(min).all() && c.cmple(max).all())
                .map(|(c, b)| (*c, b.clone()))
                .collect()
        } else {
            chunk_range(min, max).filter_map(|c| self.edited.get(&c).map(|b| (c, b.clone()))).collect()
        };
        MeshJob { key, generator: self.generator.clone(), edited }
    }

    /// Changes blocks, starting edited chunks from `base` (blocks already known for a chunk)
    /// or the generator. Returns finest chunks whose meshes are now stale.
    pub fn set_blocks(
        &mut self,
        changes: impl IntoIterator<Item = (IVec3, Block)>,
        base: impl Fn(IVec3) -> Option<ChunkBlocks>,
    ) -> Vec<IVec3> {
        let mut dirty = Vec::new();
        for (cell, block) in changes {
            let chunk = chunk_of(cell);
            if !self.edited.contains_key(&chunk) {
                let blocks = base(chunk).unwrap_or_else(|| Arc::new(self.generate(chunk)));
                self.edited.insert(chunk, blocks);
            }
            let local = cell - chunk * CHUNK_CELLS;
            Arc::make_mut(self.edited.get_mut(&chunk).expect("inserted above"))[local_index(local)] = block;

            // Neighbours see this cell through their apron (corners included, for occlusion).
            let reach = |l: i32| -> &[i32] {
                match l {
                    0 => &[-1, 0],
                    l if l == CHUNK_CELLS - 1 => &[0, 1],
                    _ => &[0],
                }
            };
            for &dz in reach(local.z) {
                for &dy in reach(local.y) {
                    for &dx in reach(local.x) {
                        let c = chunk + IVec3::new(dx, dy, dz);
                        if !dirty.contains(&c) {
                            dirty.push(c);
                        }
                    }
                }
            }
        }
        dirty
    }

    /// A cheap copy of all edits that can be encoded on another thread.
    pub fn snapshot(&self) -> EditsSnapshot {
        let chunks = self.edited.iter().map(|(c, v)| (*c, v.clone())).collect();
        EditsSnapshot { generator: self.generator.clone(), chunks }
    }

    /// Restores edits written by [`EditsSnapshot::encode`].
    pub fn decode_edits(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut r = Reader(bytes);
        let mut edited = HashMap::new();
        if r.take(4)? != MAGIC {
            return Err("not a NoCubeCraft world file".into());
        }
        for _ in 0..r.u32()? {
            let chunk = IVec3::new(r.i32()?, r.i32()?, r.i32()?);
            let mut blocks = self.generate(chunk);
            for _ in 0..r.u32()? {
                let index = r.u32()? as usize;
                let block = Block::from_bits(r.u16()?).ok_or("unknown block")?;
                *blocks.get_mut(index).ok_or("cell index out of range")? = block;
            }
            edited.insert(chunk, Arc::new(blocks));
        }
        self.edited = edited;
        Ok(())
    }
}

impl MeshJob {
    /// The chunk plus its apron, as expected by [`super::mesher::mesh`].
    pub fn samples(&self) -> Vec<Block> {
        let cell = self.key.cell();
        let min = self.key.origin() - Vec3::splat(cell);
        let n = SAMPLES as usize;
        let mut samples = self.generator.sample(min, cell, n, self.key.level <= 1);
        let coarse = self.key.level > 0;

        for (chunk, blocks) in &self.edited {
            // Sample cells whose centre falls inside the edited chunk take the finest block there.
            let lo_m = chunk.as_vec3() * CHUNK_SIZE;
            let lo = ((lo_m - min) / cell - 0.5).ceil().as_ivec3().max(IVec3::ZERO);
            let hi = ((lo_m + CHUNK_SIZE - min) / cell - 0.5).ceil().as_ivec3().min(IVec3::splat(SAMPLES));
            for z in lo.z..hi.z {
                for y in lo.y..hi.y {
                    for x in lo.x..hi.x {
                        let s = IVec3::new(x, y, z);
                        let center = min + (s.as_vec3() + 0.5) * cell;
                        let fine = (center / CELL).floor().as_ivec3() - chunk * CHUNK_CELLS;
                        let block = blocks[local_index(fine)];
                        samples[(x + SAMPLES * (y + SAMPLES * z)) as usize] =
                            if coarse && !block.is_air() { Block::solid(block.material) } else { block };
                    }
                }
            }
        }
        samples
    }

    /// A finest chunk's own blocks (no apron), kept for block lookups near the player.
    pub fn interior(samples: &[Block]) -> Vec<Block> {
        let mut out = Vec::with_capacity((CHUNK_CELLS * CHUNK_CELLS * CHUNK_CELLS) as usize);
        for z in 1..=CHUNK_CELLS {
            for y in 1..=CHUNK_CELLS {
                for x in 1..=CHUNK_CELLS {
                    out.push(samples[(x + SAMPLES * (y + SAMPLES * z)) as usize]);
                }
            }
        }
        out
    }
}

/// A cell's block within the finest chunk that holds it.
pub fn block_in(blocks: &[Block], cell: IVec3) -> Block {
    blocks[local_index(cell - chunk_of(cell) * CHUNK_CELLS)]
}

fn chunk_range(min: IVec3, max: IVec3) -> impl Iterator<Item = IVec3> {
    (min.z..=max.z)
        .flat_map(move |z| (min.y..=max.y).flat_map(move |y| (min.x..=max.x).map(move |x| IVec3::new(x, y, z))))
}

const MAGIC: &[u8] = b"BCW2";

pub struct EditsSnapshot {
    generator: Arc<Generator>,
    chunks: Vec<(IVec3, ChunkBlocks)>,
}

impl EditsSnapshot {
    /// Only cells that differ from the generated world are stored.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend((self.chunks.len() as u32).to_le_bytes());
        for (chunk, blocks) in &self.chunks {
            let generated = self.generator.sample(chunk.as_vec3() * CHUNK_SIZE, CELL, CHUNK_CELLS as usize, true);
            let changed: Vec<usize> = (0..generated.len()).filter(|&i| blocks[i] != generated[i]).collect();
            for v in chunk.to_array() {
                out.extend(v.to_le_bytes());
            }
            out.extend((changed.len() as u32).to_le_bytes());
            for i in changed {
                out.extend((i as u32).to_le_bytes());
                out.extend(blocks[i].to_bits().to_le_bytes());
            }
        }
        out
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        if self.0.len() < n {
            return Err("world file is cut short".into());
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }

    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("2 bytes")))
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")))
    }

    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::blocks::{Material, Shape};

    #[test]
    fn edit_on_chunk_corner_dirties_all_touching_chunks() {
        let mut store = VoxelStore::new(3);
        let dirty = store.set_blocks([(IVec3::ZERO, Block::AIR)], |_| None);
        assert_eq!(dirty.len(), 8);
        let dirty = store.set_blocks([(IVec3::splat(10), Block::AIR)], |_| None);
        assert_eq!(dirty, vec![IVec3::ZERO]);
    }

    #[test]
    fn neighbour_sees_edit_through_apron() {
        let mut store = VoxelStore::new(3);
        let key = ChunkKey { level: 0, pos: IVec3::new(-1, 1, 0) };
        // Cell (0, 70, 5) is in chunk (0, 1, 0) and in the +X apron of chunk (-1, 1, 0).
        let cell = IVec3::new(0, 70, 5);
        let block = Block::new(Material::Bricks, Shape::slope(1));
        store.set_blocks([(cell, block)], |_| None);
        let samples = store.mesh_job(key).samples();
        let s = cell - key.pos * CHUNK_CELLS + IVec3::ONE;
        assert_eq!(samples[(s.x + SAMPLES * (s.y + SAMPLES * s.z)) as usize], block);
    }

    #[test]
    fn coarse_chunks_see_edits() {
        let mut store = VoxelStore::new(3);
        // A 1 m cube of bricks high in the air.
        let changes: Vec<_> =
            (0..64).map(|i| (IVec3::new(i % 4, 400 + i / 4 % 4, i / 16), Block::solid(Material::Bricks))).collect();
        store.set_blocks(changes, |_| None);
        let key = ChunkKey { level: 2, pos: IVec3::new(0, 1, 0) };
        let samples = store.mesh_job(key).samples();
        assert!(samples.iter().any(|b| b.material == Material::Bricks));
    }

    #[test]
    fn edits_survive_encoding() {
        let mut store = VoxelStore::new(5);
        let changes = [
            (IVec3::new(3, 80, -40), Block::AIR),
            (IVec3::new(3, 81, -40), Block::new(Material::Planks, Shape::SLAB)),
            (IVec3::new(-70, 90, 2), Block::solid(Material::Bricks)),
        ];
        store.set_blocks(changes, |_| None);
        let bytes = store.snapshot().encode();

        let mut loaded = VoxelStore::new(5);
        loaded.decode_edits(&bytes).unwrap();
        assert_eq!(loaded.edited.len(), store.edited.len());
        for (chunk, blocks) in &store.edited {
            assert_eq!(**blocks, *loaded.edited[chunk]);
        }
        assert!(loaded.decode_edits(&bytes[..bytes.len() - 3]).is_err());
    }

    #[test]
    fn keys_nest() {
        let key = ChunkKey { level: 0, pos: IVec3::new(-3, 5, 7) };
        assert!(key.parent().contains(key));
        assert!(key.parent().parent().contains(key));
        assert!(key.parent().children().any(|c| c == key));
        assert!(!ChunkKey { level: 1, pos: IVec3::ZERO }.contains(key));
    }
}
