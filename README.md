# Three Independent Rust Services

This repository holds **three completely separate projects**. Each is fully
self-contained in its own folder — its own `Cargo.toml`, build, tests, Docker
setup, docs, and `LICENSE`. They share **no code and no database**, and there is
**no Cargo workspace** tying them together: `cd` into any folder and it builds
and runs on its own. **Delete any folder and the others are unaffected.**

| Folder | Project | Port | README |
|---|---|---|---|
| [`url-shortener/`](./url-shortener) | URL shortener REST API (Axum + SQLite) — short codes, redirects, TTL, host blocklist, rate limiting, optional Redis cache. [Architecture ›](./url-shortener/README.md#architecture) | 8080 | [README](./url-shortener/README.md) |
| [`pastebin-service/`](./pastebin-service) | Zero-knowledge pastebin (Axum + SQLite) — browser AES-256-GCM, key in URL fragment, optional password, TTL, burn-after-read. | 8090 | [README](./pastebin-service/README.md) |
| [`tcp-actor-server/`](./tcp-actor-server) | Non-blocking TCP/HTTP server (Tokio, actor model) — per-connection tasks, semaphore-bounded concurrency, registry actor, lock-free metrics, graceful drain. [Architecture ›](./tcp-actor-server/README.md#architecture) | 8100 | [README](./tcp-actor-server/README.md) |

Concurrency across the services follows one philosophy — the tool matched to the
access pattern (locks, atomics, channels/actors); see
[`url-shortener/docs/CONCURRENCY.md`](./url-shortener/docs/CONCURRENCY.md). A
beginner's book that teaches Rust using only code from these services lives at
[`Learn-Rust-by-Building.pdf`](./Learn-Rust-by-Building.pdf).

Each builds and tests standalone:

```bash
cd url-shortener    && cargo test && cargo run     # serves on :8080
cd pastebin-service && cargo test && cargo run     # serves on :8090
cd tcp-actor-server && cargo test && cargo run     # serves on :8100
```

Each folder also ships its own `Dockerfile` + `docker-compose.yml` and a `docs/`
directory (architecture and, for the shortener, design/scaling notes).

The only necessarily-shared file is CI:
[`.github/workflows/ci.yml`](./.github/workflows/ci.yml) — GitHub only reads
workflows from the repo root, so it builds each service independently via a
matrix (one job per folder). To remove a service, delete its folder and its
matrix entry.
