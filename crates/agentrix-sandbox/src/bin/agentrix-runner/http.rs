//! The runner's HTTP API on the VPC (design §4.2).
//!
//! | Route | What it does |
//! |---|---|
//! | `PUT /blobs/{digest}` | Control uploads an input; it lands only if its sha256 matches |
//! | `POST /jobs` | A control-signed `SignedJobSpec`; the response streams `JobEvent`s as NDJSON and ends with `result` |
//! | `GET /artifacts/{digest}` | Control fetches a result file |
//! | `GET /healthz` | The runtime rooms use, and free room slots |

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use futures::StreamExt as _;
use tokio::io::AsyncReadExt as _;
use tokio::sync::mpsc;

use crate::cas::PutError;
use crate::job::{Refused, Runner};
use crate::room::RoomRuntime;
use crate::spec::{Digest, SignedJobSpec};

/// Largest input blob, in bytes.
pub const MAX_BLOB: u64 = 256 * 1024 * 1024;

/// The runner's routes.
pub fn router<R: RoomRuntime>(runner: Arc<Runner<R>>) -> Router {
    Router::new()
        .route("/blobs/{digest}", put(put_blob::<R>))
        .route("/jobs", post(post_job::<R>))
        .route("/artifacts/{digest}", get(get_artifact::<R>))
        .route("/healthz", get(healthz::<R>))
        .with_state(runner)
}

fn error(status: StatusCode, message: impl ToString) -> Response {
    (
        status,
        Json(serde_json::json!({ "error": message.to_string() })),
    )
        .into_response()
}

async fn put_blob<R: RoomRuntime>(
    State(r): State<Arc<Runner<R>>>,
    Path(digest): Path<String>,
    body: Body,
) -> Response {
    let Ok(expected) = digest.parse::<Digest>() else {
        return error(StatusCode::BAD_REQUEST, "the path must be a sha256: digest");
    };
    if r.cas.has(&expected).await {
        return (
            StatusCode::OK,
            Json(serde_json::json!({ "digest": expected })),
        )
            .into_response();
    }
    let mut w = match r.cas.writer(expected, MAX_BLOB).await {
        Ok(w) => w,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                w.abort().await;
                return error(StatusCode::BAD_REQUEST, e);
            }
        };
        if let Err(e) = w.write(&chunk).await {
            w.abort().await;
            let status = if matches!(e, PutError::TooLarge(_)) {
                StatusCode::PAYLOAD_TOO_LARGE
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            return error(status, e);
        }
    }
    match w.finish().await {
        Ok(d) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "digest": d })),
        )
            .into_response(),
        Err(e @ PutError::Mismatch { .. }) => error(StatusCode::BAD_REQUEST, e),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn post_job<R: RoomRuntime>(
    State(r): State<Arc<Runner<R>>>,
    Json(signed): Json<SignedJobSpec>,
) -> Response {
    let job = match r.admit(signed).await {
        Ok(job) => job,
        Err(e) => {
            let status = match &e {
                Refused::Spec(crate::spec::SpecError::BadSignature(_)) => StatusCode::FORBIDDEN,
                Refused::Spec(_) => StatusCode::UNPROCESSABLE_ENTITY,
                Refused::Replayed(_) | Refused::MissingInputs(_) => StatusCode::CONFLICT,
                Refused::Busy => StatusCode::TOO_MANY_REQUESTS,
                Refused::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
            };
            return error(status, e);
        }
    };
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(r.clone().run(job, tx));
    let lines = tokio_stream::wrappers::ReceiverStream::new(rx).map(|ev| {
        let mut line = serde_json::to_vec(&ev).unwrap_or_else(|_| {
            b"{\"event\":\"error\",\"message\":\"unserialisable event\"}".to_vec()
        });
        line.push(b'\n');
        Ok::<_, std::convert::Infallible>(Bytes::from(line))
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(lines))
        .unwrap_or_else(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))
}

async fn get_artifact<R: RoomRuntime>(
    State(r): State<Arc<Runner<R>>>,
    Path(digest): Path<String>,
) -> Response {
    let Ok(d) = digest.parse::<Digest>() else {
        return error(StatusCode::BAD_REQUEST, "the path must be a sha256: digest");
    };
    let file = match tokio::fs::File::open(r.cas.path(&d)).await {
        Ok(f) => f,
        Err(_) => return error(StatusCode::NOT_FOUND, "no such artifact"),
    };
    let chunks = futures::stream::unfold(file, |mut f| async move {
        let mut buf = vec![0u8; 64 * 1024];
        match f.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok::<_, std::io::Error>(Bytes::from(buf)), f))
            }
            Err(e) => Some((Err(e), f)),
        }
    });
    Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header("x-content-digest", d.to_string())
        .body(Body::from_stream(chunks))
        .unwrap_or_else(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))
}

async fn healthz<R: RoomRuntime>(State(r): State<Arc<Runner<R>>>) -> Response {
    Json(serde_json::json!({
        "ok": true,
        "runtime": r.oci_runtime,
        "free_rooms": r.slots.free(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
    .into_response()
}
