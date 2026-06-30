//! Integration tests for the middleware stack and routing edges: oversized
//! bodies are rejected (413), and unsupported methods on a known path are 405.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use tower::ServiceExt; // for `oneshot`

use pastebin_service::domain::PasteRepository;
use pastebin_service::infrastructure::InMemoryPasteRepository;
use pastebin_service::{build_app, Config};

fn config() -> Config {
    Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        // Small limit so the test can exceed it cheaply.
        max_body_bytes: 1024,
        database_url: "sqlite::memory:".to_owned(),
        database_max_connections: 1,
        public_base_url: "http://localhost".to_owned(),
        request_timeout_secs: 10,
        max_concurrent_requests: 1024,
    }
}

fn app() -> Router {
    let repo: Arc<dyn PasteRepository> = Arc::new(InMemoryPasteRepository::default());
    build_app(config(), repo)
}

#[tokio::test]
async fn oversized_body_is_rejected_with_413() {
    let big = "x".repeat(4096); // ~4 KiB against a 1 KiB limit
    let body = format!(r#"{{"content":"{big}"}}"#);

    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/pastes")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn unsupported_method_on_known_path_is_405() {
    let response = app()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/pastes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}
