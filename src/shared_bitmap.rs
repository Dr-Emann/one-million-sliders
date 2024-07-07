use dashmap::DashMap;
use std::sync::atomic::AtomicU64;
use tokio::sync::watch;

pub const CHUNK_SIZE: usize = 512;
pub const CHUNK_BYTES: usize = (CHUNK_SIZE + 7) / 8;
pub const MAX_BIT_IDX: u64 = (u32::MAX as u64) * CHUNK_SIZE as u64;

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

    // Returns if the byte was added
    pub fn toggle(&mut self, index: u16) -> bool {
        let (byte_index, mask) = Self::index_mask(index);
        let byte = &mut self.0[byte_index];
        *byte ^= mask;
        (*byte & mask) != 0
    }

    // Returns the difference in the number of set bits
    pub fn toggle_all(&mut self, rhs: Self) -> i16 {
        const _: () = assert!((CHUNK_SIZE as u64) < u16::MAX as u64);
        let mut diff = 0;
        for (lhs, &rhs) in self.0.iter_mut().zip(rhs.0.iter()) {
            let new_val = *lhs ^ rhs;
            diff += new_val.count_ones() as i16 - lhs.count_ones() as i16;
            *lhs = new_val;
        }
        diff
    }

    #[inline]
    const fn index_mask(index: u16) -> (usize, u8) {
        debug_assert!(index < CHUNK_SIZE as u16);
        let index = index % CHUNK_SIZE as u16;
        let byte_index = index / 8;
        let bit_index = index % 8;
        (byte_index as usize, 1 << bit_index)
    }
}

struct Segment {
    watch: watch::Sender<Chunk>,
}

impl Default for Segment {
    fn default() -> Self {
        Self {
            watch: watch::Sender::new(Chunk::new()),
        }
    }
}

#[derive(Default)]
pub struct SharedBitmap {
    segments: DashMap<u32, Segment>,
    count: AtomicU64,
}

impl SharedBitmap {
    pub fn new() -> Self {
        Self::default()
    }

    // This is so much better if indices are ordered
    pub fn toggle_all(&self, indices: &[u64]) {
        let Some(&first_index) = indices.first() else {
            return;
        };
        let (mut last_segment_index, _) = split_idx(first_index);
        let mut segment = self
            .segments
            .entry(last_segment_index)
            .or_insert_with(|| Segment {
                watch: watch::Sender::new(Chunk::new()),
            })
            .downgrade();
        let mut current_chunk = Chunk::new();
        for &index in indices {
            let (segment_index, inner_index) = split_idx(index);
            if segment_index != last_segment_index {
                segment.watch.send_if_modified(|chunk| {
                    let diff = chunk.toggle_all(current_chunk);
                    if diff != 0 {
                        // Intentionally use i16 as u64, which will sign extend negative values, which
                        // will work correctly with the atomic add (which overflows)
                        self.count
                            .fetch_add(diff as u64, std::sync::atomic::Ordering::Relaxed);
                        true
                    } else {
                        false
                    }
                });
                // Ensure we're not holding the dashmap lock
                drop(segment);
                segment = self
                    .segments
                    .entry(segment_index)
                    .or_insert_with(|| Segment {
                        watch: watch::Sender::new(Chunk::new()),
                    })
                    .downgrade();
                current_chunk = const { Chunk::new() };
                last_segment_index = segment_index;
            }
            current_chunk.toggle(inner_index);
        }
        segment.watch.send_if_modified(|chunk| {
            let diff = chunk.toggle_all(current_chunk);
            if diff != 0 {
                // Intentionally use i16 as u64, which will sign extend negative values, which
                // will work correctly with the atomic add (which overflows)
                self.count
                    .fetch_add(diff as u64, std::sync::atomic::Ordering::Relaxed);
                true
            } else {
                false
            }
        });
    }

    pub fn watch(&self, segment_index: u32) -> watch::Receiver<Chunk> {
        if let Some(segment) = self.segments.get(&segment_index) {
            return segment.watch.subscribe();
        }
        
        let segment = self
            .segments
            .entry(segment_index)
            .or_insert_with(|| Segment {
                watch: watch::Sender::new(Chunk::new()),
            })
            .downgrade();
        segment.watch.subscribe()
    }

    pub fn count(&self) -> u64 {
        self.count.load(std::sync::atomic::Ordering::Relaxed)
    }
}

const fn split_idx(index: u64) -> (u32, u16) {
    assert!(index < MAX_BIT_IDX);
    let segment_index = (index / CHUNK_SIZE as u64) as u32;
    let inner_index = (index % CHUNK_SIZE as u64) as u16;
    (segment_index, inner_index)
}
