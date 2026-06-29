# Improvement PR Plan — Reviewed Counter-Based Shortener

Improvement roadmap for the **externally reviewed** design (PostgreSQL +
`BIGSERIAL` → `encode_base62(id)`, `301` redirects, `POST /shorten`,
`GET /{short_code}`). The architecture is sound; these PRs fix correctness,
security, and scale gaps found in review. Ordered by priority.

Format matches the project's own plans: each PR is small, independently
reviewable, and green on its own (`build + test + lint`).

Legend: **P0** = block release · **P1** = required to hold 100M+ users · **P2** = quality.

---

## P0 — correctness & security

### PR 1 — Non-sequential, fixed-width short codes  **(P0, highest leverage)**
**Goal:** keep the auto-increment id as the source of uniqueness, but make the
public code non-guessable and a true fixed 7 characters.
**Why:** `encode_base62(id)` is directly enumerable — anyone can walk `/1`,
`/2`, `/B`, … to harvest every URL and infer total volume/growth. It also
produces variable-length codes (1–5 chars at 100M), so the "7-character" spec is
unmet.
**Changes:**
- Insert an order-preserving-free bijection between id and code: a keyed
  **Feistel/Optimus** permutation over the id space, or **Sqids/Hashids**, applied
  *before* base62.
- Offset the sequence (start at `62^6`) or left-pad so every code is exactly 7
  chars; widen the column to `CHAR(8)`+ headroom or document the `62^7` ceiling.
- Encapsulate behind a `CodeCodec { encode(id)->code }` so it's swappable/tested.
**Done when:** codes are fixed-width, non-sequential, 1:1 with id, and a property
test confirms no collisions across a large id range.

### PR 2 — Input validation & safe redirects  **(P0)**
**Goal:** only store and redirect to safe, absolute URLs.
**Why:** `original_url TEXT` with no checks allows `javascript:`/`data:` URIs
(stored-XSS when followed in some contexts), open-redirect abuse, and
megabyte-URL storage abuse.
**Changes:**
- Validate scheme is `http`/`https`, require a host, cap length (e.g. 2048).
- Return `400` with a clear error on invalid input.
- (Optional) integrate a phishing/malware domain blocklist hook.
**Done when:** non-`http(s)` schemes, hostless URLs, and oversized bodies are
rejected with `400`; tests cover each case.

### PR 3 — Schema cleanup  **(P0, small)**
**Goal:** remove waste and latent traps in the DDL.
**Why:** `short_code … UNIQUE` already creates an index; the extra
`CREATE INDEX idx_short_code` is a redundant second index (double write cost,
extra storage). `VARCHAR(7)` silently caps the id space at `62^7`.
**Changes:**
- Drop `idx_short_code`; rely on the unique constraint's index.
- Right-size the code column for the chosen width; add `NOT NULL`/length checks.
- Confirm `original_url` length constraint matches PR 2.
**Done when:** one index on `short_code`, migration is reversible, and the
capacity ceiling is documented or removed.

---

## P1 — scale to 100M+ users

### PR 4 — Redis read cache for redirects  **(P1, biggest scale win)**
**Goal:** serve hot redirects without touching Postgres.
**Why:** redirect traffic is highly skewed; the mapping is immutable for a
link's life, so it caches safely. This removes the dominant read load.
**Changes:**
- Look up `code → url` in Redis first; on miss, read Postgres and populate.
- Invalidate (or skip caching) on delete; set a sane TTL.
- Keep the cache external so app instances stay stateless.
**Done when:** a warm code resolves with zero DB queries; delete invalidates;
load test shows DB read QPS drop sharply for repeated codes.

### PR 5 — Observability, readiness & rate limiting  **(P1)**
**Goal:** make the service operable and abuse-resistant at scale.
**Why:** `POST /shorten` is unauthenticated and unbounded (bulk-creation/spam
vector); there's no readiness signal or metrics to run behind a load balancer.
**Changes:**
- DB-checked readiness endpoint + liveness; structured request logging.
- Metrics: RPS, p50/p99 latency, 4xx/5xx, cache hit ratio.
- Per-client rate limit on create (and ideally API keys/quotas).
**Done when:** a load balancer can distinguish healthy/unhealthy instances,
dashboards show the core metrics, and create is rate-limited.

### PR 6 — Statelessness & connection pooling  **(P1)**
**Goal:** scale out horizontally.
**Why:** 100M users will exhaust raw Postgres connections; instances must be
interchangeable.
**Changes:**
- Confirm no per-instance mutable state (share-nothing); run N instances behind
  a load balancer.
- Front Postgres with **PgBouncer**; size the app pool to the DB.
- Tune the `BIGSERIAL` sequence cache to reduce allocation chatter under high
  write rates.
**Done when:** multiple instances run against one DB/cache with bounded
connections under load.

---

## P2 — product & quality

### PR 7 — Redirect semantics & decoupled analytics  **(P2)**
**Goal:** make the 301/302 choice deliberate and add click tracking without
hurting the hot path.
**Why:** `301` is cached hard by browsers — it blocks click analytics and makes
targets effectively permanent. There's currently no hit tracking at all.
**Changes:**
- Decide policy: `301` for max performance/CDN offload, or `302/307` if
  analytics matter.
- If tracking: count hits **off** the request path (async queue / batched writes
  / Redis `INCR`), never an inline synchronous `UPDATE`.
**Done when:** redirect status is a documented decision; if enabled, hit counts
are recorded without adding latency to redirects.

### PR 8 — Test coverage  **(P2)**
**Goal:** lock in correctness.
**Changes:**
- Unit: code codec (round-trip/bijection), URL validation.
- Integration: create → redirect → 404-on-unknown → delete.
- Concurrency: parallel inserts never yield a duplicate code.
**Done when:** CI runs unit + integration + a concurrency test, all green.

---

## Cross-cutting acceptance criteria (every PR)
- No secrets or unvalidated input on the request path; clear error codes.
- New behaviour has tests; existing tests stay green.
- Migrations are reversible; indexes justified (no redundant indexes).
- Lint/format clean; no inline blocking work on the redirect hot path.

## If only three ship
**PR 1** (non-sequential fixed-width codes), **PR 2** (validation/safe
redirects), **PR 4** (Redis cache) — they close the privacy leak, the abuse
surface, and the scale bottleneck respectively.
