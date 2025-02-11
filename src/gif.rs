use std::{borrow::Cow, convert::Infallible, sync::Arc, time::Duration};

use bytes::{BufMut, Bytes, BytesMut};

use crate::{
    shared_bitmap::{SharedBitmap, CHUNK_BYTES, NUM_CHUNKS},
    NUM_SLIDERS,
};

type RawFrame = [u8; NUM_CHUNKS * CHUNK_BYTES];
type Sender = tokio::sync::watch::Sender<(Box<RawFrame>, gif::Frame<'static>)>;

static GLOBAL_PALLETE: [u8; 256 * 3] = {
    let mut pallete = [0; 256 * 3];
    let mut i = 0;
    while i < 256 {
        let idx = i * 3;
        let v = i as u8;
        pallete[idx] = v;
        pallete[idx + 1] = v;
        pallete[idx + 2] = v;
        i += 1;
    }
    pallete
};

fn populate_frame(
    frame: &mut gif::Frame<'static>,
    buf: &mut RawFrame,
    bitmap: &SharedBitmap,
    force: bool,
) -> bool {
    // Fill must come first
    if !bitmap.fill_bytes_mut(buf) && !force {
        return false;
    }
    let mut tmp_frame = gif::Frame {
        width: 1000,
        height: 1000,
        buffer: Cow::Borrowed(&buf[..NUM_SLIDERS]),
        ..Default::default()
    };
    tmp_frame.make_lzw_pre_encoded();
    frame.buffer = tmp_frame.buffer.into_owned().into();
    true
}

struct Inner {
    sender: Sender,
    notify_recievers: tokio::sync::Notify,
    bitmap: Arc<SharedBitmap>,
}

#[derive(Clone)]
pub struct GifFrames {
    inner: Arc<Inner>,
}

impl GifFrames {
    pub fn new(bitmap: Arc<SharedBitmap>) -> Self {
        let mut buf: Box<RawFrame> = vec![0; NUM_CHUNKS * CHUNK_BYTES].try_into().unwrap();
        let mut frame = gif::Frame {
            width: 1000,
            height: 1000,
            buffer: Cow::Borrowed(&[]),
            ..Default::default()
        };
        populate_frame(&mut frame, &mut buf, &bitmap, true);
        let sender = Sender::new((buf, frame));
        Self {
            inner: Arc::new(Inner {
                sender,
                notify_recievers: tokio::sync::Notify::new(),
                bitmap,
            }),
        }
    }

    pub fn byte_stream(self) -> impl futures::Stream<Item = Bytes> {
        struct State {
            receiver: tokio::sync::watch::Receiver<(Box<RawFrame>, gif::Frame<'static>)>,
            gif: gif::Encoder<bytes::buf::Writer<bytes::BytesMut>>,
        }

        let mut receiver = self.inner.sender.subscribe();
        self.inner.notify_recievers.notify_one();
        receiver.mark_changed();

        futures::stream::unfold(
            State {
                receiver,
                gif: gif::Encoder::new(
                    BytesMut::with_capacity(4096).writer(),
                    1000,
                    1000,
                    &GLOBAL_PALLETE,
                )
                .unwrap(),
            },
            |mut state| async move {
                let Ok(()) = state.receiver.changed().await else {
                    return None;
                };
                {
                    let borrowed = state.receiver.borrow_and_update();
                    let frame = &borrowed.1;

                    state.gif.write_lzw_pre_encoded_frame(frame).unwrap();
                }
                Some((state.gif.get_mut().get_mut().split().freeze(), state))
            },
        )
    }

    pub async fn produce_frames(self) -> Infallible {
        let sender = &self.inner.sender;

        let mut interval = tokio::time::interval(Duration::from_millis(100));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            while sender.is_closed() {
                self.inner.notify_recievers.notified().await;
            }
            tokio::select! {
                _ = interval.tick() => {},
                _ = sender.closed() => continue,
            }
            let bitmap = self.inner.bitmap.clone();
            let sender = sender.clone();
            tokio::task::spawn_blocking(move || {
                sender.send_if_modified(move |(buf, frame)| {
                    populate_frame(frame, buf, &bitmap, false)
                });
            })
            .await
            .unwrap();
        }
    }
}
