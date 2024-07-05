mod comm;
mod shared_bitmap;

use crate::shared_bitmap::{SharedBitmap, CHUNK_SIZE};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{sse, Sse};
use axum::routing::{get, post};
use axum::Router;
use futures::stream::StreamExt;
use futures::{stream, Stream};
use std::convert::Infallible;
use std::net::Ipv6Addr;
use tokio_stream::wrappers::WatchStream;

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/1k-updates", get(range_updates))
        .route("/toggle/:idx", post(toggle));
    let app = app.with_state(SharedBitmap::new());

    let listener = tokio::net::TcpListener::bind((Ipv6Addr::UNSPECIFIED, 8000))
        .await
        .unwrap();

    axum::serve(listener, app).await.unwrap()
}

#[derive(serde::Deserialize, serde::Serialize)]
struct Range {
    start: u64,
    end: u64,
}

#[axum::debug_handler]
async fn range_updates(
    State(bitmap): State<SharedBitmap>,
    Query(range): Query<Range>,
) -> axum::response::Result<Sse<impl Stream<Item = Result<sse::Event, Infallible>>>> {
    if range.start > range.end {
        return Err((StatusCode::BAD_REQUEST, "start must be less than end").into());
    }
    if range.start % shared_bitmap::CHUNK_SIZE as u64 != 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "start must be a multiple of CHUNK_SIZE",
        )
            .into());
    }
    if range.end % shared_bitmap::CHUNK_SIZE as u64 != 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "end must be a multiple of CHUNK_SIZE",
        )
            .into());
    }
    if range.end - range.start > 5000 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot listen to such a large range",
        )
            .into());
    }

    let watches = (range.start / CHUNK_SIZE as u64..range.end / CHUNK_SIZE as u64).map(|i| {
        let i = i as u32;
        WatchStream::new(bitmap.watch(i)).map(move |chunk| (i, chunk))
    });
    let stream = stream::select_all(watches)
        .map(|(i, _chunk)| Ok(sse::Event::default().data("HI").id(i.to_string())));

    Ok(Sse::new(stream))
}

#[axum::debug_handler]
async fn toggle(State(bitmap): State<SharedBitmap>, Path(idx): Path<u64>) {
    bitmap.toggle(idx);
}
