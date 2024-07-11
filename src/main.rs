use std::convert::Infallible;
use std::io;
use std::net::Ipv6Addr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{sse, Sse};
use axum::routing::{get, post};
use axum::Router;
use base64::prelude::BASE64_STANDARD_NO_PAD;
use base64::Engine;
use futures::{stream, Stream};
use tokio::net::TcpListener;
use tokio::time::MissedTickBehavior;
use tokio_stream::StreamExt;
use tower::ServiceBuilder;
use tower_http::services::ServeDir;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tower_http::LatencyUnit;
use tracing::{debug, Span};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::shared_bitmap::{SharedBitmap, SharedBitmapRunningTasks, CHUNK_BITS, CHUNK_BYTES};

mod shared_bitmap;

// One byte per slider
const NUM_SLIDERS: usize = 1_000_000;
const NUM_CHECKBOXES: usize = NUM_SLIDERS * 8;

#[derive(Clone)]
struct SharedState {
    bitmap: Arc<SharedBitmap>,
    _tasks: Arc<SharedBitmapRunningTasks>,
}

impl SharedState {
    fn new() -> io::Result<Self> {
        let bitmap = Arc::new(SharedBitmap::load_or_create("bitmap.bin")?);
        let tasks = Arc::new(bitmap.run_tasks());

        Ok(Self {
            bitmap,
            _tasks: tasks,
        })
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let app = Router::new()
        .route("/updates", get(range_updates))
        .route("/toggle/:idx", post(toggle))
        .route("/set_byte/:idx/:value", post(set_byte))
        .nest_service("/", ServeDir::new("www"))
        .layer(
            ServiceBuilder::new()
                .layer(
                    TraceLayer::new_for_http()
                        .on_response(DefaultOnResponse::new().latency_unit(LatencyUnit::Micros)),
                )
                .layer(tower_http::cors::CorsLayer::new().allow_origin(tower_http::cors::Any))
                .layer(
                    tower_http::compression::CompressionLayer::new()
                        .gzip(true)
                        .br(true),
                ),
        );
    let app = app.with_state(SharedState::new().unwrap());

    let listener = TcpListener::bind((Ipv6Addr::UNSPECIFIED, 8000))
        .await
        .unwrap();

    axum::serve(listener, app).await.unwrap();
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
struct Range {
    start: u64,
    end: u64,
}

#[tracing::instrument(skip(state, range), fields(start=range.start, end=range.end))]
async fn range_updates(
    State(state): State<SharedState>,
    Query(range): Query<Range>,
) -> axum::response::Result<Sse<impl Stream<Item = Result<sse::Event, Infallible>>>> {
    if range.start > range.end {
        return Err((StatusCode::BAD_REQUEST, "start must be less than end").into());
    }
    if range.end > NUM_CHECKBOXES as u64 {
        return Err((StatusCode::BAD_REQUEST, "end too large").into());
    }
    let start_chunk = (range.start / CHUNK_BITS as u64) as usize;
    let end_chunk = ((range.end + CHUNK_BITS as u64 - 1) / CHUNK_BITS as u64) as usize;
    if (end_chunk - start_chunk) * CHUNK_BITS > 10_000 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot listen to such a large range",
        )
            .into());
    }

    let span = Span::current();
    let watches = (start_chunk..end_chunk).map(|i| {
        let span = span.clone();
        tokio_stream::wrappers::WatchStream::new(state.bitmap.watch(i)).map(move |chunk| {
            debug!(parent: &span, i, "going to send a chunk update");
            (i, chunk)
        })
    });
    let mut b64_chunk = [0; CHUNK_BYTES * 4 / 3 + 4];
    let mut i_buffer = itoa::Buffer::new();
    let stream = stream::select_all(watches).map(move |(i, chunk)| {
        let len = BASE64_STANDARD_NO_PAD
            .encode_slice(chunk.0, &mut b64_chunk)
            .expect("a chunk is guaranteed to fit in the available space");
        // SAFETY: base64 encoding is guaranteed to be valid UTF-8
        let b64_chunk: &str = unsafe { std::str::from_utf8_unchecked(&b64_chunk[..len]) };
        let i_str = i_buffer.format(i as u64 * CHUNK_BITS as u64);
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
    struct LogOnDisconnect(Span);
    impl Drop for LogOnDisconnect {
        fn drop(&mut self) {
            debug!(parent: &self.0, "client disconnected");
        }
    }
    let log_on_disconnect = LogOnDisconnect(span.clone());
    let count_stream =
        tokio_stream::wrappers::IntervalStream::new(interval).filter_map(move |_tick| {
            // Move the logger into the closure to ensure it's dropped when the stream ends
            let _log_on_disconnect = &log_on_disconnect;
            let count = state.bitmap.count();
            if count != last_count {
                debug!(parent: &span, count, last_count, "going to send a count update");
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

#[tracing::instrument(skip(state))]
async fn toggle(
    State(state): State<SharedState>,
    Path(idx): Path<u64>,
) -> axum::response::Result<()> {
    if idx >= NUM_CHECKBOXES as u64 {
        return Err((StatusCode::BAD_REQUEST, "Index too large").into());
    }
    state.bitmap.toggle(idx as usize);
    Ok(())
}

#[tracing::instrument(skip(state))]
async fn set_byte(
    State(state): State<SharedState>,
    Path((idx, value)): Path<(u64, u8)>,
) -> axum::response::Result<()> {
    if idx >= NUM_SLIDERS as u64 {
        return Err((StatusCode::BAD_REQUEST, "Index too large").into());
    }
    state.bitmap.set_byte(idx as usize, value);
    Ok(())
}
