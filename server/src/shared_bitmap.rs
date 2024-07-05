use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{watch, Notify};

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

struct Segment {
    chunk: Chunk,
    changed: Arc<Notify>,
    watch: watch::Receiver<Chunk>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Clone, Default)]
pub struct SharedBitmap {
    segments: Arc<DashMap<u32, Segment>>,
}

impl SharedBitmap {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn toggle(&self, index: u64) {
        let (segment_index, inner_index) = split_idx(index);
        let mut segment = self.segments.entry(segment_index).or_insert_with(|| {
            Self::new_segment(Arc::clone(&self.segments), segment_index)
        });

        segment.chunk.toggle(inner_index);
        segment.changed.notify_one()
    }

    pub fn watch(&self, segment_index: u32) -> watch::Receiver<Chunk> {
        let segment = self.segments.entry(segment_index).or_insert_with(|| {
            Self::new_segment(Arc::clone(&self.segments), segment_index)
        });
        segment.watch.clone()
    }

    fn new_segment(segments: Arc<DashMap<u32, Segment>>, segment_index: u32) -> Segment {
        let chunk = Chunk::new();
        let (tx, rx) = watch::channel(chunk);
        let notify = Arc::new(Notify::new());
        let task = {
            let notify = Arc::clone(&notify);
            tokio::spawn(async move {
                loop {
                    notify.notified().await;
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                    let Some(segment) = segments.get(&segment_index) else {
                        break;
                    };
                    let segment = segment.value();
                    tx.send_if_modified(
                        |chunk| {
                            if *chunk == segment.chunk {
                                false
                            } else {
                                *chunk = segment.chunk;
                                true
                            }
                        },
                    );
                }
            })
        };
        Segment {
            chunk,
            changed: notify,
            watch: rx,
            task,
        }

    }
}

const fn split_idx(index: u64) -> (u32, u16) {
    assert!(index < MAX_BIT_IDX);
    let segment_index = (index / CHUNK_SIZE as u64) as u32;
    let inner_index = (index % CHUNK_SIZE as u64) as u16;
    (segment_index, inner_index)
}
