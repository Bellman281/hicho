# URL Shortener

A clean, layered URL shortener REST API built with **Axum** and **SQLite/sqlx**.

> Status: **PR #1 (scaffold)** — config, error type, layered module skeleton,
> graceful shutdown, and a `GET /health` endpoint with an integration test.
> See [`../docs/PR_PLAN_url_shortener.md`](../docs/PR_PLAN_url_shortener.md) for
> the roadmap.

## Layout

Present in PR #1:

```
src/
  main.rs          Composition root (config -> app -> serve + graceful shutdown)
  lib.rs           AppState + build_app(); wires the router
  config.rs        Env-driven Config
  error.rs         AppError -> IntoResponse (+ unit tests)
  api/             Axum router + handlers
tests/
  health.rs        Integration test for /health
```

Added by later PRs (created when they have content): `domain/` (#2),
`application/` (#3), `infrastructure/` (#4).

Architecture rationale: [`../docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md).

## Run

```bash
cp .env.example .env        # optional; defaults work out of the box
cargo run
curl localhost:8080/health  # {"status":"ok"}
```

## Quality gates

```bash
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

The crate sets `#![forbid(unsafe_code)]`. Configuration is injected (no
globals), shared state is held behind a single `Arc`, request bodies are size
limited, and shutdown is graceful so resources drain without leaks.
