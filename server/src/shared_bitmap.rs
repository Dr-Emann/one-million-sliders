use dashmap::DashMap;
use tokio::sync::watch;

pub const MAX_BIT_IDX: u64 = (u32::MAX as u64) * CHUNK_SIZE as u64;

pub const CHUNK_SIZE: usize = 512;
pub const CHUNK_BYTES: usize = (CHUNK_SIZE + 7) / 8;

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct Chunk(pub [u8; CHUNK_BYTES]);

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}

impl Chunk {
    pub const fn new() -> Self {
        Self([0; CHUNK_BYTES])
    }

    pub fn get(&self, index: u16) -> bool {
        let (byte_index, mask) = Self::index_mask(index);
        let byte = self.0[byte_index];
        (byte & mask) != 0
    }

    pub fn toggle(&mut self, index: u16) {
        let (byte_index, mask) = Self::index_mask(index);
        let byte = &mut self.0[byte_index];
        *byte ^= mask;
    }

    #[inline]
    const fn index_mask(index: u16) -> (usize, u8) {
        let byte_index = index / 8;
        let bit_index = index % 8;
        (byte_index as usize, 1 << bit_index)
    }
}

pub struct SharedBitmap {
    segments: DashMap<u32, watch::Sender<Chunk>>,
}

impl SharedBitmap {
    pub fn get(&self, index: u64) -> bool {
        let (segment_index, inner_index) = split_idx(index);
        let Some(segment) = self.segments.get(&segment_index) else {
            return false;
        };
        let chunk = segment.value().borrow();
        chunk.get(inner_index)
    }

    pub fn toggle(&self, index: u64) {
        let (segment_index, inner_index) = split_idx(index);
        let mut segment = self.segments.entry(segment_index).or_insert_with(|| watch::Sender::new(Chunk::new()));
        segment.value_mut().send_modify(|chunk| {
            chunk.toggle(inner_index)
        })
    }
}

const fn split_idx(index: u64) -> (u32, u16) {
    assert!(index < MAX_BIT_IDX);
    let segment_index = (index / CHUNK_SIZE as u64) as u32;
    let inner_index = (index % CHUNK_SIZE as u64) as u16;
    (segment_index, inner_index)
}