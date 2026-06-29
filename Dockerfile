# syntax=docker/dockerfile:1

# ---- builder ----------------------------------------------------------------
# rust:1.82 is Debian bookworm-based and ships a C toolchain, which the bundled
# SQLite (libsqlite3-sys) needs to compile.
FROM rust:1.82 AS builder
WORKDIR /app

# Copy the workspace and build the release binary. (Migrations live under
# url-shortener/migrations and are embedded at compile time by sqlx::migrate!.)
COPY Cargo.toml Cargo.lock ./
COPY url-shortener ./url-shortener
RUN cargo build --release --bin url-shortener

# ---- runtime ----------------------------------------------------------------
# bookworm-slim matches the builder's glibc, so the dynamically linked binary
# runs as-is. curl is included only for the container HEALTHCHECK.
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --uid 10001 app \
 && mkdir -p /data && chown app:app /data

COPY --from=builder /app/target/release/url-shortener /usr/local/bin/url-shortener

USER app
# Bind to all interfaces inside the container; persist the DB on the /data volume.
ENV APP_BIND_ADDR=0.0.0.0:8080 \
    DATABASE_URL=sqlite:///data/links.db \
    DATABASE_MAX_CONNECTIONS=5 \
    PUBLIC_BASE_URL=http://localhost:8080 \
    REQUEST_TIMEOUT_SECS=10 \
    MAX_CONCURRENT_REQUESTS=1024 \
    RUST_LOG=info

EXPOSE 8080
VOLUME ["/data"]

HEALTHCHECK --interval=10s --timeout=3s --retries=5 \
  CMD curl -fsS http://localhost:8080/health/ready || exit 1

CMD ["url-shortener"]
