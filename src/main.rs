use std::convert::Infallible;
use std::future::IntoFuture;
use std::io;
use std::net::Ipv6Addr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{sse, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::prelude::BASE64_STANDARD_NO_PAD;
use base64::Engine;
use futures::{stream, Stream};
use listenfd::ListenFd;
use std::path::Path as FsPath;
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

mod log;
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
    fn new(bitmap_path: impl AsRef<FsPath>, log_path: impl AsRef<FsPath>) -> io::Result<Self> {
        Self::_new(bitmap_path.as_ref(), log_path.as_ref())
    }

    fn _new(bitmap_path: &FsPath, log_path: &FsPath) -> io::Result<Self> {
        let bitmap = Arc::new(SharedBitmap::load_or_create(bitmap_path, log_path)?);
        let tasks = Arc::new(bitmap.spawn_tasks());

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
        .route("/snapshot", get(range_snapshot))
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
    let state = SharedState::new("bitmap.bin", "log-with-times.bin").unwrap();
    let app = app.with_state(state.clone());

    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|port_str| port_str.parse().ok())
        .unwrap_or(8000);
    let listener = listener_socket(port).await.unwrap();

    tokio::select! {
        res = axum::serve(listener, app).into_future() => {
            res.unwrap();
        },
        _ = shutdown_fut() => {
        }
    }

    tracing::info!("server shut down, flushing log");
    Arc::into_inner(state.bitmap).unwrap().finish();
    tracing::info!("exiting");
}

async fn shutdown_fut() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut sigterm =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();

    tokio::select! {
        _ = ctrl_c => {},
        _ = sigterm.recv() => {},
    }
}

async fn listener_socket(port: u16) -> io::Result<TcpListener> {
    let mut listenfd = ListenFd::from_env();
    match listenfd
        .take_tcp_listener(0)
        .expect("passed listener is not a TCP listener")
    {
        Some(std_listener) => {
            tracing::info!("using passed tcp listener");
            TcpListener::from_std(std_listener)
        }
        None => {
            tracing::info!("binding to port={port} directly");
            TcpListener::bind((Ipv6Addr::UNSPECIFIED, port)).await
        }
    }
}

const MAX_RANGE_BITS: usize = NUM_CHECKBOXES.next_multiple_of(CHUNK_BITS);

#[derive(serde::Deserialize, serde::Serialize, Debug)]
struct Range {
    start: u64,
    end: u64,
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
struct Snapshot {
    start: u64,
    bits: String,
}

fn range_validate(range: &Range) -> Result<(usize, usize), axum::response::ErrorResponse> {
    if range.start > range.end {
        return Err((StatusCode::BAD_REQUEST, "start must be less than end").into());
    }
    if range.end > NUM_CHECKBOXES as u64 {
        return Err((StatusCode::BAD_REQUEST, "end too large").into());
    }
    let start_chunk = (range.start / CHUNK_BITS as u64) as usize;
    let end_chunk = range.end.div_ceil(CHUNK_BITS as u64) as usize;
    if (end_chunk - start_chunk) * CHUNK_BITS > MAX_RANGE_BITS {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot listen to such a large range",
        )
            .into());
    }
    Ok((start_chunk, end_chunk))
}

#[tracing::instrument(skip(state, range), fields(start=range.start, end=range.end))]
async fn range_snapshot(
    State(state): State<SharedState>,
    Query(range): Query<Range>,
) -> axum::response::Result<Json<Snapshot>> {
    use std::io::Write;

    let (start_chunk, end_chunk) = range_validate(&range)?;
    let num_bytes = (end_chunk - start_chunk) * CHUNK_BYTES;
    let buf = Vec::with_capacity(num_bytes * 4 / 3 + 4);
    let mut writer = base64::write::EncoderWriter::new(buf, &BASE64_STANDARD_NO_PAD);

    let chunks = &state.bitmap.raw_chunks()[start_chunk..end_chunk];
    let mut chunk_buf = [0; CHUNK_BYTES];
    for chunk in chunks {
        chunk.load(&mut chunk_buf);
        writer.write_all(&chunk_buf).unwrap();
    }
    let b64_output = writer
        .finish()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    // SAFETY: base64 encoding is guaranteed to be valid UTF-8
    let b64_output = unsafe { String::from_utf8_unchecked(b64_output) };

    Ok(Json(Snapshot {
        start: range.start,
        bits: b64_output,
    }))
}

#[tracing::instrument(skip(state, range), fields(start=range.start, end=range.end))]
async fn range_updates(
    State(state): State<SharedState>,
    Query(range): Query<Range>,
) -> axum::response::Result<Sse<impl Stream<Item = Result<sse::Event, Infallible>>>> {
    let (start_chunk, end_chunk) = range_validate(&range)?;

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
            .encode_slice(chunk, &mut b64_chunk)
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
    // This will never be the actual sum, so we'll always send the first update
    let mut last_sum = u64::MAX;
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
            let sum = state.bitmap.sum();
            if sum != last_sum {
                debug!(parent: &span, sum, last_sum, "going to send a sum update");
                last_sum = sum;
                let sum_str = int_buffer.format(sum);
                Some(sse::Event::default().data(sum_str).event("sum"))
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
