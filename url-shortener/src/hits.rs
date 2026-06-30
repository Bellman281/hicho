//! Hit counting via a background batcher — the channel / actor pattern.
//!
//! Redirects are the hot path. Counting a hit with a synchronous
//! `UPDATE hits = hits + 1` per redirect puts *every* redirect in contention for
//! a database write. Instead, a redirect **sends** the code down an in-process
//! channel (a non-blocking enqueue) and returns immediately; a single background
//! task owns the aggregation, coalescing many hits for the same code into one
//! batched `hits = hits + n` write that it flushes on an interval.
//!
//! This is the canonical place for channels in this codebase: there is genuine
//! write contention, the work is naturally *fire-and-forget* (hit counts were
//! already best-effort), and a single owner can batch. Contrast the in-memory
//! map and the rate limiter, whose critical sections are sub-microsecond and
//! synchronous — there a `Mutex` is both simpler and faster than routing every
//! call through an actor task (which would serialize them onto one core).
//!
//! Two implementations sit behind the [`HitRecorder`] port:
//! - [`ImmediateHitRecorder`] — one write per hit. Exact and synchronous; the
//!   default, and convenient for deterministic tests.
//! - [`BatchingHitRecorder`] — the channel batcher above, wired in `main`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::domain::{LinkRepository, ShortCode};

/// Default cap on how stale a buffered count can get before it is flushed.
pub const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_secs(2);
/// Default number of *distinct* codes buffered before an early flush is forced.
pub const DEFAULT_MAX_PENDING: usize = 1024;

/// Records a redirect hit for a code. Best-effort and non-blocking: a failed or
/// dropped count must never fail (or slow) a redirect.
#[async_trait::async_trait]
pub trait HitRecorder: Send + Sync + 'static {
    /// Record one hit. Implementations must not block the caller on I/O.
    async fn record(&self, code: ShortCode);

    /// Flush any buffered counts to the store. Default: nothing is buffered, so
    /// this is a no-op. The batcher overrides it for graceful shutdown.
    async fn flush(&self) {}
}

/// Increments immediately — one DB write per hit. Exact and simple; used by
/// default (see `LinkService::with_cache`) and in unit tests where immediate,
/// deterministic counts are convenient.
pub struct ImmediateHitRecorder {
    repo: Arc<dyn LinkRepository>,
}

impl ImmediateHitRecorder {
    pub fn new(repo: Arc<dyn LinkRepository>) -> Self {
        Self { repo }
    }
}

#[async_trait::async_trait]
impl HitRecorder for ImmediateHitRecorder {
    async fn record(&self, code: ShortCode) {
        // Best-effort: a failed counter write must never fail a redirect.
        let _ = self.repo.increment_hits(&code).await;
    }
}

/// Messages the background task understands.
enum Msg {
    /// One observed hit for a code.
    Hit(ShortCode),
    /// Flush now and acknowledge on the oneshot (used by `flush`/shutdown).
    Flush(oneshot::Sender<()>),
}

/// Batches hits in a single background task and flushes them periodically. The
/// hot path only does a non-blocking channel send; all map mutation and DB
/// writes happen on the owning task, so there is no shared lock at all.
pub struct BatchingHitRecorder {
    tx: mpsc::UnboundedSender<Msg>,
}

impl BatchingHitRecorder {
    /// Spawn the background batcher.
    ///
    /// - `interval` bounds how stale a count can be (it is flushed at least this
    ///   often).
    /// - `max_pending` forces an early flush once that many *distinct* codes are
    ///   buffered, bounding memory under a burst.
    ///
    /// Returns the recorder plus the task's [`JoinHandle`] so the composition
    /// root can await/abort it on shutdown.
    pub fn spawn(
        repo: Arc<dyn LinkRepository>,
        interval: Duration,
        max_pending: usize,
    ) -> (Arc<Self>, JoinHandle<()>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(run(repo, rx, interval, max_pending.max(1)));
        (Arc::new(Self { tx }), handle)
    }

    /// Spawn with the default interval and pending cap.
    pub fn spawn_default(repo: Arc<dyn LinkRepository>) -> (Arc<Self>, JoinHandle<()>) {
        Self::spawn(repo, DEFAULT_FLUSH_INTERVAL, DEFAULT_MAX_PENDING)
    }
}

#[async_trait::async_trait]
impl HitRecorder for BatchingHitRecorder {
    async fn record(&self, code: ShortCode) {
        // If the receiver is gone (shutting down), silently drop — best-effort.
        let _ = self.tx.send(Msg::Hit(code));
    }

    async fn flush(&self) {
        let (ack, done) = oneshot::channel();
        // If the task is already gone, there is nothing left to flush.
        if self.tx.send(Msg::Flush(ack)).is_ok() {
            let _ = done.await;
        }
    }
}

/// The actor loop: own the pending map, drain the channel, flush on a timer or
/// on demand, and do a final flush when all senders drop (no lost counts).
async fn run(
    repo: Arc<dyn LinkRepository>,
    mut rx: mpsc::UnboundedReceiver<Msg>,
    interval: Duration,
    max_pending: usize,
) {
    let mut pending: HashMap<String, i64> = HashMap::new();
    let mut ticker = tokio::time::interval(interval);
    // Don't try to "catch up" missed ticks after a slow flush.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            maybe = rx.recv() => match maybe {
                Some(Msg::Hit(code)) => {
                    *pending.entry(code.as_str().to_owned()).or_insert(0) += 1;
                    if pending.len() >= max_pending {
                        flush(&repo, &mut pending).await;
                    }
                }
                Some(Msg::Flush(ack)) => {
                    flush(&repo, &mut pending).await;
                    let _ = ack.send(());
                }
                None => {
                    // All senders dropped: final flush, then exit.
                    flush(&repo, &mut pending).await;
                    return;
                }
            },
            _ = ticker.tick() => {
                flush(&repo, &mut pending).await;
            }
        }
    }
}

/// Write every buffered count as a single `+= n` per code, then clear. Draining
/// (rather than retaining) keeps a failed write from being retried forever —
/// consistent with best-effort hit counting.
async fn flush(repo: &Arc<dyn LinkRepository>, pending: &mut HashMap<String, i64>) {
    if pending.is_empty() {
        return;
    }
    for (code, n) in pending.drain() {
        let code = ShortCode::from_trusted(code);
        let _ = repo.increment_hits_by(&code, n).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Link, TargetUrl};
    use crate::infrastructure::InMemoryLinkRepository;

    async fn repo_with(code: &str) -> Arc<InMemoryLinkRepository> {
        let repo = Arc::new(InMemoryLinkRepository::default());
        let link = Link::new(
            ShortCode::parse(code).unwrap(),
            TargetUrl::parse("https://example.com").unwrap(),
            1_700_000_000,
        );
        repo.insert(&link).await.unwrap();
        repo
    }

    async fn hits_of(repo: &Arc<InMemoryLinkRepository>, code: &str) -> i64 {
        repo.get(&ShortCode::parse(code).unwrap())
            .await
            .unwrap()
            .unwrap()
            .hits
    }

    #[tokio::test]
    async fn immediate_recorder_counts_each_hit() {
        let repo = repo_with("abc").await;
        let rec = ImmediateHitRecorder::new(repo.clone());
        rec.record(ShortCode::parse("abc").unwrap()).await;
        rec.record(ShortCode::parse("abc").unwrap()).await;
        assert_eq!(hits_of(&repo, "abc").await, 2);
    }

    #[tokio::test]
    async fn batcher_coalesces_hits_and_flush_persists_them() {
        let repo = repo_with("abc").await;
        // Long interval + high cap so nothing flushes until we ask it to.
        let (rec, _h) =
            BatchingHitRecorder::spawn(repo.clone(), Duration::from_secs(3600), 1_000_000);

        for _ in 0..5 {
            rec.record(ShortCode::parse("abc").unwrap()).await;
        }
        // Not necessarily written yet (it's asynchronous)...
        rec.flush().await;
        // ...but after an explicit flush the batched count is persisted.
        assert_eq!(hits_of(&repo, "abc").await, 5);
    }

    #[tokio::test]
    async fn dropping_the_recorder_flushes_remaining_counts() {
        let repo = repo_with("xy").await;
        let (rec, handle) =
            BatchingHitRecorder::spawn(repo.clone(), Duration::from_secs(3600), 1_000_000);
        rec.record(ShortCode::parse("xy").unwrap()).await;
        rec.record(ShortCode::parse("xy").unwrap()).await;

        drop(rec); // last sender gone -> task does a final flush and exits
        handle.await.unwrap();

        assert_eq!(hits_of(&repo, "xy").await, 2);
    }

    #[tokio::test]
    async fn max_pending_forces_an_early_flush_without_an_explicit_call() {
        let repo = repo_with("abc").await;
        // Cap of 1 distinct code => flushes as soon as one code is buffered.
        let (rec, _h) = BatchingHitRecorder::spawn(repo.clone(), Duration::from_secs(3600), 1);
        rec.record(ShortCode::parse("abc").unwrap()).await;
        // Give the task a moment to process the send and auto-flush.
        rec.flush().await;
        assert_eq!(hits_of(&repo, "abc").await, 1);
    }
}
