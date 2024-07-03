mod shared_bitmap;

use axum::routing::get;
use axum::Router;
use std::net::Ipv6Addr;
use std::sync::Arc;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use croaring::Bitmap64;
use crate::shared_bitmap::ToggleOp;

#[tokio::main]
async fn main() {
    let app = Router::new().route("/updates", get(range_updates));

    let listener = tokio::net::TcpListener::bind((Ipv6Addr::UNSPECIFIED, 8000))
        .await
        .unwrap();

    let (write, read) = left_right::new::<Bitmap64, ToggleOp>();
    axum::serve(listener, app).await.unwrap()
}

#[derive(serde::Deserialize, serde::Serialize)]
struct Range {
    start: u64,
    end: u64,
}

#[axum::debug_handler]
async fn range_updates(State(state): State<Arc<BitmapState>>, Query(range): Query<Range>) -> axum::response::Result<()> {
    if range.start > range.end {
        return Err((StatusCode::BAD_REQUEST, "start must be less than end").into());
    }
    println!("start: {}, end: {}", range.start, range.end);
    Ok(())
}

pub struct BitmapState {
    writes: tokio::sync::mpsc::Sender<u64>,
    bitmap: left_right::ReadHandle<Bitmap64>,
}