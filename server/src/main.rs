mod shared_bitmap;

use crate::shared_bitmap::{SharedBitmap, CHUNK_BYTES, CHUNK_SIZE, MAX_BIT_IDX};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{sse, Sse};
use axum::routing::{get, post};
use axum::Router;
use base64::prelude::BASE64_STANDARD_NO_PAD;
use base64::Engine;
use futures::{stream, Stream};
use std::convert::Infallible;
use std::future::IntoFuture;
use std::mem;
use std::net::Ipv6Addr;
use std::pin::pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{watch, Notify};
use tokio::time::{Instant, MissedTickBehavior};
use tokio_stream::StreamExt;

const NUM_BITS: u64 = 1_000_000;
const _: () = assert!(NUM_BITS <= MAX_BIT_IDX);

// Apply backpressure if there are too many pending toggles
const MAX_TOGGLES: usize = 500;

struct SharedState {
    bitmap: SharedBitmap,
    has_toggles: Notify,
    pending_toggles: Mutex<Vec<u64>>,
    has_toggles_space: Notify,
}

impl SharedState {
    fn new() -> Self {
        let bitmap = SharedBitmap::new();

        Self {
            bitmap,
            has_toggles: Default::default(),
            pending_toggles: Default::default(),
            has_toggles_space: Default::default(),
        }
    }

    async fn do_toggles(&self) {
        let mut current_toggles = Vec::new();
        let mut notified = pin!(self.has_toggles.notified());
        loop {
            notified.as_mut().await;
            notified.set(self.has_toggles.notified());
            notified.as_mut().enable();
            mem::swap(
                &mut current_toggles,
                &mut *self.pending_toggles.lock().unwrap(),
            );
            self.has_toggles_space.notify_one();

            current_toggles.sort_unstable();
            tracing::trace!(count = current_toggles.len(), "Toggling bits");
            self.bitmap.toggle_all(&current_toggles);

            current_toggles.clear();
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    tracing::info!("Got here");
    let app = Router::new()
        .route("/updates", get(range_updates))
        .route("/toggle/:idx", post(toggle));
    let state = Arc::new(SharedState::new());
    let app = app.with_state(Arc::clone(&state));

    let listener = TcpListener::bind((Ipv6Addr::UNSPECIFIED, 8000)).await.unwrap();

    let toggles = async move {
        tokio::spawn(async move { state.do_toggles().await })
            .await
            .unwrap();
        Ok(())
    };
    let service = axum::serve(listener, app);
    let service_future = pin!(service.into_future());

    tokio::try_join!(toggles, service_future).unwrap();
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
struct Range {
    start: u64,
    end: u64,
}

#[axum::debug_handler]
#[tracing::instrument(skip_all, fields(start=range.start, end=range.end))]
async fn range_updates(
    State(state): State<Arc<SharedState>>,
    Query(mut range): Query<Range>,
) -> axum::response::Result<Sse<impl Stream<Item = Result<sse::Event, Infallible>>>> {
    if range.start > range.end {
        return Err((StatusCode::BAD_REQUEST, "start must be less than end").into());
    }
    range.start = range.start / CHUNK_SIZE as u64 * CHUNK_SIZE as u64;
    range.end = range.end.next_multiple_of(CHUNK_SIZE as u64);

    if range.end - range.start > 10_000 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot listen to such a large range",
        )
            .into());
    }

    let watches = (range.start / CHUNK_SIZE as u64..range.end / CHUNK_SIZE as u64).map(|i| {
        struct State {
            next_notify: Instant,
            watcher: watch::Receiver<shared_bitmap::Chunk>,
        }
        let i = i as u32;

        let state = State {
            next_notify: Instant::now(),
            watcher: {
                let mut watcher = state.bitmap.watch(i);
                // Report the initial state too
                watcher.mark_changed();
                watcher
            },
        };
        Box::pin(stream::unfold(state, move |mut state| async move {
            state.watcher.changed().await.unwrap();
            tokio::time::sleep_until(state.next_notify.clone()).await;
            let chunk = *state.watcher.borrow_and_update();

            state.next_notify = Instant::now() + Duration::from_millis(100);

            return Some(((i, chunk), state));
        }))
    });
    let mut b64_chunk = [0; CHUNK_BYTES * 4 / 3 + 4];
    let mut i_buffer = itoa::Buffer::new();
    let stream = stream::select_all(watches).map(move |(i, chunk)| {
        let len = BASE64_STANDARD_NO_PAD
            .encode_slice(&chunk.0, &mut b64_chunk)
            .expect("a chunk is guaranteed to fit in the available space");
        // SAFETY: base64 encoding is guaranteed to be valid UTF-8
        let b64_chunk: &str = unsafe { std::str::from_utf8_unchecked(&b64_chunk[..len]) };
        let i_str = i_buffer.format(u64::from(i) * CHUNK_SIZE as u64);
        sse::Event::default()
            .data(b64_chunk)
            .id(i_str)
            .event("update")
    });

    let mut interval = tokio::time::interval(Duration::from_millis(250));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    interval.reset_immediately();
    // This will never be the actual count, so we'll always send the first update
    let mut last_count = u64::MAX;
    let mut int_buffer = itoa::Buffer::new();
    let count_stream =
        tokio_stream::wrappers::IntervalStream::new(interval).filter_map(move |_tick| {
            let count = state.bitmap.count();
            if count != last_count {
                last_count = count;
                let count_str = int_buffer.format(count);
                Some(sse::Event::default().data(count_str).event("count"))
            } else {
                None
            }
        });

    let stream = stream::select(count_stream, stream);
    let stream = stream.map(Ok);

    Ok(Sse::new(stream).keep_alive(sse::KeepAlive::new()))
}

#[axum::debug_handler]
#[tracing::instrument(skip_all, fields(idx))]
async fn toggle(
    State(state): State<Arc<SharedState>>,
    Path(idx): Path<u64>,
) -> axum::response::Result<()> {
    if idx >= NUM_BITS {
        return Err((StatusCode::BAD_REQUEST, "Index too large").into());
    }
    tokio::time::timeout(Duration::from_secs(1), toggle_loop(&state, idx))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(())
}

async fn toggle_loop(state: &SharedState, idx: u64) {
    let try_push = || {
        let mut pending_toggles = state.pending_toggles.lock().unwrap();
        if pending_toggles.len() >= MAX_TOGGLES {
            false
        } else {
            pending_toggles.push(idx);
            state.has_toggles.notify_one();
            if pending_toggles.len() < MAX_TOGGLES {
                state.has_toggles_space.notify_one();
            }
            true
        }
    };
    let mut notified = pin!(state.has_toggles_space.notified());
    loop {
        notified.as_mut().enable();
        if try_push() {
            return;
        }
        tracing::debug!("Backing off till there's space");
        notified.as_mut().await;
        notified.set(state.has_toggles_space.notified());
    }
}
