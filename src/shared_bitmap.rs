use memmap2::{MmapOptions, MmapRaw};
use std::convert::Infallible;
use std::fs::File;
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize};
use std::sync::Arc;
use std::time::SystemTime;
use std::{io, mem, slice};
use tokio::sync::{watch, Notify};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::log::{self, Log};

pub const CHUNK_BYTES: usize = 128;
pub const CHUNK_BITS: usize = CHUNK_BYTES * 8;
pub const NUM_CHUNKS: usize = TOTAL_BITS.div_ceil(CHUNK_BITS);

const TOTAL_BITS: usize = crate::NUM_CHECKBOXES;

#[repr(transparent)]
pub struct Chunk([AtomicU8; CHUNK_BYTES]);

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}

impl Chunk {
    pub const fn new() -> Self {
        Self([const { AtomicU8::new(0) }; CHUNK_BYTES])
    }

    // Returns if the byte was added, otherwise, it was removed
    pub fn toggle(&self, index: u16) -> bool {
        let (byte_index, mask) = Self::index_mask(index);
        let byte = &self.0[byte_index];
        let orig = byte.fetch_xor(mask, std::sync::atomic::Ordering::Relaxed);
        (orig & mask) != 0
    }

    pub fn set_byte(&self, index: usize, byte: u8) -> u8 {
        self.0[index].swap(byte, std::sync::atomic::Ordering::Relaxed)
    }

    pub fn load(&self, dst: &mut [u8; CHUNK_BYTES]) {
        Self::load_chunks(std::array::from_ref(self), dst);
    }

    // Returns if any values were different than previously in dst
    pub fn load_chunks(chunks: &[Chunk], dst: &mut [u8]) -> bool {
        assert_eq!(dst.len(), chunks.len() * CHUNK_BYTES);

        let chunks_bytes = unsafe {
            slice::from_raw_parts(
                chunks.as_ptr().cast::<AtomicU8>(),
                chunks.len() * size_of::<Chunk>(),
            )
        };
        let (prefix, aligned, suffix) = unsafe { chunks_bytes.align_to::<AtomicUsize>() };
        let (prefix_dst, rest) = dst.split_at_mut(prefix.len());
        let (aligned_dst, suffix_dxt) = rest.split_at_mut(aligned.len() * size_of::<usize>());

        let mut changed = false;
        for (d, s) in prefix_dst.iter_mut().zip(prefix.iter()) {
            let s = s.load(std::sync::atomic::Ordering::Relaxed);
            changed |= *d != s;
            *d = s;
        }
        for (d, s) in aligned_dst
            .chunks_exact_mut(size_of::<usize>())
            .zip(aligned)
        {
            let s = s.load(std::sync::atomic::Ordering::Relaxed);
            changed |= !s.to_ne_bytes().iter().eq(d.iter());
            d.copy_from_slice(&s.to_ne_bytes());
        }
        for (d, s) in suffix_dxt.iter_mut().zip(suffix.iter()) {
            let s = s.load(std::sync::atomic::Ordering::Relaxed);
            changed |= *d != s;
            *d = s;
        }
        changed
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

struct Segment {
    notify_changed: Notify,
    watch: watch::Sender<[u8; CHUNK_BYTES]>,
}

impl Default for Segment {
    fn default() -> Self {
        Self {
            notify_changed: Notify::new(),
            watch: watch::Sender::new([0; CHUNK_BYTES]),
        }
    }
}
impl Segment {
    fn from_bytes(current_slice: &[u8; CHUNK_BYTES]) -> Self {
        Self {
            notify_changed: Notify::new(),
            watch: watch::Sender::new(*current_slice),
        }
    }
}

pub struct SharedBitmap {
    segments: Box<[Segment; NUM_CHUNKS]>,
    map: MmapRaw,
    bits_set: AtomicU64,
    bytes_sum: AtomicU64,
    pub log: Log,
}

impl SharedBitmap {
    pub fn load_or_create(
        bitmap_path: impl AsRef<Path>,
        log_path: impl AsRef<Path>,
    ) -> io::Result<Self> {
        Self::_load_or_create(bitmap_path.as_ref(), log_path.as_ref())
    }

    fn _load_or_create(bitmap_path: &Path, log_path: &Path) -> io::Result<Self> {
        let bitmap_file = File::options()
            .write(true)
            .read(true)
            .create(true)
            .truncate(false)
            .open(bitmap_path)?;
        bitmap_file.set_len(NUM_CHUNKS as u64 * CHUNK_BYTES as u64)?;

        let log = Log::new(log_path)?;

        let map = unsafe { MmapOptions::new().map_mut(&bitmap_file)? };
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
            log,
        })
    }

    pub fn run_tasks(
        self: &Arc<Self>,
    ) -> impl Iterator<Item = impl Future<Output = Infallible>> + '_ {
        (0..self.segments.len()).map(|i| {
            let shared = Arc::clone(self);
            async move {
                let segment = &shared.segments[i];
                let mut next_possible_update = Instant::now();
                loop {
                    segment.notify_changed.notified().await;
                    tokio::time::sleep_until(next_possible_update).await;
                    next_possible_update = Instant::now() + std::time::Duration::from_millis(100);

                    let chunk = &shared.raw_chunks()[i];
                    segment.watch.send_modify(|c| chunk.load(c));
                }
            }
        })
    }

    pub fn spawn_tasks(self: &Arc<Self>) -> SharedBitmapRunningTasks {
        let tasks = self.run_tasks().map(tokio::spawn).collect();
        SharedBitmapRunningTasks { tasks }
    }

    pub fn raw_chunks(&self) -> &[Chunk] {
        debug_assert_eq!(self.map.len(), NUM_CHUNKS * mem::size_of::<Chunk>());

        unsafe { std::slice::from_raw_parts(self.map.as_ptr().cast::<Chunk>(), NUM_CHUNKS) }
    }

    pub fn fill_bytes_mut(&self, bytes: &mut [u8; CHUNK_BYTES * NUM_CHUNKS]) -> bool {
        Chunk::load_chunks(self.raw_chunks(), bytes)
    }

    fn chunk_notify(&self, index: usize) -> (&Chunk, &Notify) {
        let chunk = &self.raw_chunks()[index];
        let segment = &self.segments[index];
        (chunk, &segment.notify_changed)
    }

    pub fn set_byte(&self, index: usize, byte: u8) {
        let (chunk, notify) = self.chunk_notify(index / CHUNK_BYTES);
        let inner_idx = index % CHUNK_BYTES;

        let prev = chunk.set_byte(inner_idx, byte);
        notify.notify_one();
        self.log.log_msg(log::Record::SetByte {
            time: SystemTime::now(),
            offset: index as u32,
            value: byte,
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
        let (chunk, notify) = self.chunk_notify(bit_index / CHUNK_BITS);
        let prev_bit = chunk.toggle((bit_index % CHUNK_BITS) as u16);
        notify.notify_one();
        self.log.log_msg(log::Record::Toggle {
            time: SystemTime::now(),
            offset: bit_index as u32,
        });
        let diff = if prev_bit { -1 } else { 1 };
        self.bits_set
            .fetch_add(diff as u64, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn watch(&self, segment_index: usize) -> watch::Receiver<[u8; CHUNK_BYTES]> {
        self.segments[segment_index].watch.subscribe()
    }

    #[allow(dead_code)]
    pub fn count(&self) -> u64 {
        self.bits_set.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn sum(&self) -> u64 {
        self.bytes_sum.load(std::sync::atomic::Ordering::Relaxed)
    }
}

pub struct SharedBitmapRunningTasks {
    tasks: Vec<JoinHandle<Infallible>>,
}

impl SharedBitmapRunningTasks {
    pub fn add(&mut self, task: JoinHandle<Infallible>) {
        self.tasks.push(task);
    }
}

impl Drop for SharedBitmapRunningTasks {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
