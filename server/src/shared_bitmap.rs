use dashmap::DashMap;
use std::mem;

const SEGMENT_LENGTH: usize = u16::MAX as usize + 1;
const BITMAP_BYTES: usize = SEGMENT_LENGTH / 8;

const LIST_BITMAP_THRESHOLD: usize = BITMAP_BYTES / mem::size_of::<u16>();

const _: () = assert!(LIST_BITMAP_THRESHOLD <= 4096);

enum Segment {
    List(Vec<u16>),
    Bitmap(Box<[u8; BITMAP_BYTES]>),
}

impl Segment {
    fn get(&self, index: u16) -> bool {
        match self {
            Segment::List(list) => list.contains(&index),
            Segment::Bitmap(bitmap) => {
                let byte_index = index as usize / 8;
                let bit_index = index as usize % 8;
                bitmap[byte_index] & (1 << bit_index) != 0
            }
        }
    }
    
    fn toggle(&mut self, index: u16) {
        match self {
            Segment::List(list) => {
                match list.binary_search(&index) {
                    Ok(i) => {
                        list.remove(i);
                    }
                    Err(i) => {
                        if list.len() < LIST_BITMAP_THRESHOLD {
                            list.insert(i, index);
                        } else {
                            let mut bitmap = Box::new([0; BITMAP_BYTES]);
                            for &mut i in list {
                                let byte_index = i as usize / 8;
                                let bit_index = i as usize % 8;
                                bitmap[byte_index] |= 1 << bit_index;
                            }
                            let byte_index = index as usize / 8;
                            let bit_index = index as usize % 8;
                            bitmap[byte_index] |= 1 << bit_index;
                            *self = Segment::Bitmap(bitmap);
                        }
                    }
                }
            }
            Segment::Bitmap(bitmap) => {
                let byte_index = index as usize / 8;
                let bit_index = index as usize % 8;
                bitmap[byte_index] ^= 1 << bit_index;
            }
        }
    }
}

pub struct SharedBitmap {
    segments: DashMap<u32, Segment>,
}

impl SharedBitmap {
    pub fn get(&self, index: u64) -> bool {
        let (segment_index, inner_index) = split_idx(index);
        let Some(segment) = self.segments.get(&segment_index) else {
            return false;
        };
        segment.value().get(inner_index)
    }

    pub fn toggle(&self, index: u64) {
        let (segment_index, inner_index) = split_idx(index);
        let mut segment = self.segments.entry(segment_index).or_insert_with(|| Segment::List(Vec::new()));
        segment.value_mut().toggle(inner_index)
    }
}

const fn split_idx(index: u64) -> (u32, u16) {
    assert!(index < 0x1_0000_0000_0000, "only 48 bit indexes");
    let segment_index = (index / SEGMENT_LENGTH as u64) as u32;
    let inner_index = (index % SEGMENT_LENGTH as u64) as u16;
    (segment_index, inner_index)
}