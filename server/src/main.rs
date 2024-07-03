mod comm;
mod shared_bitmap;

use std::convert::Infallible;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{sse, Sse};
use axum::routing::get;
use axum::{BoxError, Router};
use futures::{Stream, stream};
use std::net::Ipv6Addr;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let app = Router::new().route("/1k-updates/:i", get(range_updates));

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
    Path(i): Path<u32>,
) -> axum::response::Result<Sse<impl Stream<Item = Result<sse::Event, Infallible>>>> {
    if i > 1000 {
        return Err(StatusCode::NOT_FOUND.into());
    }

    let stream = stream::try_unfold(0, |mut last| async move {
        last += 1;

        if last > 100 {
            return Ok(None);
        }

        Ok(Some((sse::Event::default().json_data(last).unwrap(), last)))
    });

    Ok(Sse::new(stream))
}
