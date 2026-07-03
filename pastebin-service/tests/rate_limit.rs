//! Integration test: the per-IP rate-limit middleware returns 429 once the
//! bucket is exhausted. Configured with rps=1, burst=1, so the first request
//! passes and the immediate second is rejected.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use tower::ServiceExt; // for `oneshot`

use pastebin_service::domain::PasteRepository;
use pastebin_service::infrastructure::InMemoryPasteRepository;
use pastebin_service::{build_app, Config};

fn config() -> Config {
    Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        max_body_bytes: 1024 * 1024,
        database_url: "sqlite::memory:".to_owned(),
        database_max_connections: 1,
        public_base_url: "http://localhost".to_owned(),
        request_timeout_secs: 10,
        max_concurrent_requests: 1024,
        rate_limit_rps: 1,
        rate_limit_burst: 1,
        trust_proxy: false,
    }
}

fn app() -> Router {
    let repo: Arc<dyn PasteRepository> = Arc::new(InMemoryPasteRepository::default());
    build_app(config(), repo)
}

async fn status(app: &Router, uri: &str) -> StatusCode {
    app.clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn second_request_is_rate_limited() {
    let app = app();
    assert_eq!(status(&app, "/health").await, StatusCode::OK);
    assert_eq!(status(&app, "/health").await, StatusCode::TOO_MANY_REQUESTS);
}
