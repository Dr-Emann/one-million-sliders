use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use memmap2::{MmapOptions, MmapRaw};
use tokio::sync::{watch, Notify};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use crate::NUM_SLIDERS;

pub const CHUNK_BYTES: usize = 128;
pub const CHUNK_BITS: usize = CHUNK_BYTES * 8;

const TOTAL_BITS: usize = crate::NUM_CHECKBOXES;
const NUM_CHUNKS: usize = (TOTAL_BITS + CHUNK_BITS - 1) / CHUNK_BITS;

#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
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

    // Returns if the byte was added, otherwise, it was removed
    pub fn toggle(&mut self, index: u16) -> bool {
        let (byte_index, mask) = Self::index_mask(index);
        let byte = &mut self.0[byte_index];
        *byte ^= mask;
        (*byte & mask) != 0
    }

    #[inline]
    const fn index_mask(index: u16) -> (usize, u8) {
        debug_assert!(index < CHUNK_BITS as u16);
        let index = index % CHUNK_BITS as u16;
        let byte_index = index / 8;
        let bit_index = index % 8;
        (byte_index as usize, 1 << bit_index)
    }
}

#[repr(align(128))]
struct Segment {
    lock: Mutex<()>,
    notify_changed: Notify,
    watch: watch::Sender<Chunk>,
}

impl Default for Segment {
    fn default() -> Self {
        Self {
            lock: Mutex::new(()),
            notify_changed: Notify::new(),
            watch: watch::Sender::new(Chunk::new()),
        }
    }
}
impl Segment {
    fn from_bytes(current_slice: &[u8; CHUNK_BYTES]) -> Self {
        Self {
            lock: Mutex::new(()),
            notify_changed: Notify::new(),
            watch: watch::Sender::new(Chunk(*current_slice)),
        }
    }
}

pub struct SharedBitmap {
    segments: Box<[Segment; NUM_CHUNKS]>,
    map: MmapRaw,
    bits_set: AtomicU64,
    bytes_sum: AtomicU64,
}

impl SharedBitmap {
    pub fn load_or_create<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::_load_or_create(path.as_ref())
    }

    fn _load_or_create(path: &Path) -> io::Result<Self> {
        let file = File::options()
            .write(true)
            .read(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        file.set_len(NUM_CHUNKS as u64 * CHUNK_BYTES as u64)?;

        let map = unsafe { MmapOptions::new().map_mut(&file)? };
        let count = map.iter().map(|&byte| byte.count_ones() as u64).sum();
        let bytes_sum = map.iter().copied().map(u64::from).sum();

        let segment = |i| {
            let slice = &map[i * CHUNK_BYTES..][..CHUNK_BYTES];
            let slice: &[u8; CHUNK_BYTES] = slice.try_into().unwrap();
            Segment::from_bytes(slice)
        };
        let segments: Box<[Segment]> = (0..NUM_CHUNKS).map(segment).collect();
        let segments = segments.try_into().map_err(|_| ()).unwrap();
        Ok(Self {
            segments,
            map: MmapRaw::from(map),
            bits_set: AtomicU64::new(count),
            bytes_sum: AtomicU64::new(bytes_sum),
        })
    }

    pub fn run_tasks(self: &Arc<Self>) -> SharedBitmapRunningTasks {
        let tasks: Vec<_> = (0..self.segments.len())
            .map(|i| {
                let shared = Arc::clone(self);
                tokio::spawn(async move {
                    let segment = &shared.segments[i];
                    let mut next_possible_update = Instant::now();
                    loop {
                        segment.notify_changed.notified().await;
                        tokio::time::sleep_until(next_possible_update).await;
                        next_possible_update =
                            Instant::now() + std::time::Duration::from_millis(100);

                        let chunk = shared.with_chunk(i, |chunk, _| *chunk);
                        segment.watch.send_modify(|c| *c = chunk);
                    }
                })
            })
            .collect();
        SharedBitmapRunningTasks { tasks }
    }

    fn with_chunk<F, O>(&self, index: usize, f: F) -> O
    where
        F: FnOnce(&mut Chunk, &Notify) -> O,
    {
        let segment = &self.segments[index];
        let _guard = segment.lock.lock().unwrap();
        // SAFETY: The above guard ensures that only one segment has this chunk at a time
        let chunk = unsafe {
            &mut *self
                .map
                .as_mut_ptr()
                .add(index * CHUNK_BYTES)
                .cast::<Chunk>()
        };
        f(chunk, &segment.notify_changed)
    }

    pub fn set_byte(&self, index: usize, byte: u8) {
        let mut prev = 0;
        self.with_chunk(index / CHUNK_BYTES, |chunk, notify| {
            let inner_idx = index % CHUNK_BYTES;
            prev = chunk.0[inner_idx];
            chunk.0[inner_idx] = byte;
            notify.notify_one();
        });
        let bit_diff = byte.count_ones() as i32 - prev.count_ones() as i32;
        let diff = byte as i32 - prev as i32;
        // use `as u64` which will sign extend, adding a sign extended negative value will act the
        // same as subtracting
        self.bits_set
            .fetch_add(bit_diff as u64, std::sync::atomic::Ordering::Relaxed);
        self.bytes_sum
            .fetch_add(diff as u64, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn toggle(&self, bit_index: usize) {
        let mut added = false;
        self.with_chunk(bit_index / CHUNK_BITS, |chunk, notify| {
            added = chunk.toggle((bit_index % CHUNK_BITS) as u16);
            notify.notify_one();
        });
        let diff = if added { 1 } else { -1 };
        self.bits_set
            .fetch_add(diff as u64, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn watch(&self, segment_index: usize) -> watch::Receiver<Chunk> {
        self.segments[segment_index].watch.subscribe()
    }

    pub fn count(&self) -> u64 {
        self.bits_set.load(std::sync::atomic::Ordering::Relaxed)
    }
    
    pub fn average(&self) -> f64 {
        let sum = self.bytes_sum.load(std::sync::atomic::Ordering::Relaxed);
        sum as f64 / NUM_SLIDERS as f64 / 255.0
    }
}

pub struct SharedBitmapRunningTasks {
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for SharedBitmapRunningTasks {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
