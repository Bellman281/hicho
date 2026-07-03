//! A small read-cache abstraction for the hot fetch path.
//!
//! The `Cache` port stores a serialized paste keyed by id. All operations are
//! **best-effort**: a cache error degrades to a miss / no-op, never an error to
//! the client (the DB remains the source of truth). One-shot pastes are never
//! cached (they must burn on first read).
//!
//! Implementations:
//! - [`NoOpCache`] — disabled (default; the app runs fine without Redis).
//! - [`InMemoryCache`] — process-local map; used to unit-test caching logic.
//! - [`RedisCache`] — production, shared across instances.

use std::collections::HashMap;

use tokio::sync::{mpsc, oneshot};

use crate::domain::BoxedError;

/// Read-cache port. Best-effort: errors are swallowed (logged in real impls).
#[async_trait::async_trait]
pub trait Cache: Send + Sync + 'static {
    async fn get(&self, key: &str) -> Option<String>;
    async fn set(&self, key: &str, value: &str, ttl_secs: u64);
    async fn delete(&self, key: &str);
}

/// Caching disabled — every read misses; writes are no-ops.
#[derive(Debug, Default)]
pub struct NoOpCache;

#[async_trait::async_trait]
impl Cache for NoOpCache {
    async fn get(&self, _key: &str) -> Option<String> {
        None
    }
    async fn set(&self, _key: &str, _value: &str, _ttl_secs: u64) {}
    async fn delete(&self, _key: &str) {}
}

/// Process-local cache (ignores TTL). For tests and single-instance local runs.
///
/// Implemented as an actor — one task owns the map, callers message it over a
/// channel — for consistency with the in-memory repository (no lock anywhere).
/// For a test/dev double this is a performance wash; it's here so the codebase
/// uses one sharing model throughout. Production caching is Redis.
#[derive(Debug, Clone)]
pub struct InMemoryCache {
    tx: mpsc::UnboundedSender<CacheCmd>,
}

/// Commands the owning cache task understands.
enum CacheCmd {
    Get {
        key: String,
        reply: oneshot::Sender<Option<String>>,
    },
    Set {
        key: String,
        value: String,
    },
    Delete {
        key: String,
    },
}

impl Default for InMemoryCache {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryCache {
    /// Spawn the owning task. Must be called within a Tokio runtime (true in
    /// every `#[tokio::test]`; production uses Redis, not this).
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run(rx));
        Self { tx }
    }
}

/// The cache actor loop: sole owner of the entries map.
async fn run(mut rx: mpsc::UnboundedReceiver<CacheCmd>) {
    let mut entries: HashMap<String, String> = HashMap::new();
    while let Some(cmd) = rx.recv().await {
        match cmd {
            CacheCmd::Get { key, reply } => {
                let _ = reply.send(entries.get(&key).cloned());
            }
            CacheCmd::Set { key, value } => {
                entries.insert(key, value);
            }
            CacheCmd::Delete { key } => {
                entries.remove(&key);
            }
        }
    }
}

#[async_trait::async_trait]
impl Cache for InMemoryCache {
    async fn get(&self, key: &str) -> Option<String> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(CacheCmd::Get {
                key: key.to_owned(),
                reply,
            })
            .is_err()
        {
            return None; // actor gone -> treat as a miss (best-effort)
        }
        rx.await.unwrap_or(None)
    }
    async fn set(&self, key: &str, value: &str, _ttl_secs: u64) {
        // Fire-and-forget; the channel is FIFO, so a set is processed before any
        // later get on the same handle (best-effort, TTL ignored as before).
        let _ = self.tx.send(CacheCmd::Set {
            key: key.to_owned(),
            value: value.to_owned(),
        });
    }
    async fn delete(&self, key: &str) {
        let _ = self.tx.send(CacheCmd::Delete {
            key: key.to_owned(),
        });
    }
}

/// Redis-backed cache shared across instances. All ops are best-effort: any
/// Redis error is logged and treated as a miss / no-op, so the service falls
/// back to the database.
pub struct RedisCache {
    manager: redis::aio::ConnectionManager,
}

impl RedisCache {
    /// Connect and build a multiplexed, auto-reconnecting connection manager.
    pub async fn connect(url: &str) -> Result<Self, BoxedError> {
        let client = redis::Client::open(url)?;
        let manager = redis::aio::ConnectionManager::new(client).await?;
        Ok(Self { manager })
    }
}

#[async_trait::async_trait]
impl Cache for RedisCache {
    async fn get(&self, key: &str) -> Option<String> {
        use redis::AsyncCommands;
        let mut conn = self.manager.clone();
        match conn.get::<_, Option<String>>(key).await {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(error = %err, "redis get failed; treating as miss");
                None
            }
        }
    }

    async fn set(&self, key: &str, value: &str, ttl_secs: u64) {
        use redis::AsyncCommands;
        let mut conn = self.manager.clone();
        let result: redis::RedisResult<()> = if ttl_secs > 0 {
            conn.set_ex(key, value, ttl_secs).await
        } else {
            conn.set(key, value).await
        };
        if let Err(err) = result {
            tracing::warn!(error = %err, "redis set failed");
        }
    }

    async fn delete(&self, key: &str) {
        use redis::AsyncCommands;
        let mut conn = self.manager.clone();
        let result: redis::RedisResult<()> = conn.del(key).await;
        if let Err(err) = result {
            tracing::warn!(error = %err, "redis del failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_cache_roundtrips_and_deletes() {
        let cache = InMemoryCache::default();
        assert_eq!(cache.get("k").await, None);
        cache.set("k", "v", 60).await;
        assert_eq!(cache.get("k").await.as_deref(), Some("v"));
        cache.delete("k").await;
        assert_eq!(cache.get("k").await, None);
    }

    #[tokio::test]
    async fn noop_cache_never_stores() {
        let cache = NoOpCache;
        cache.set("k", "v", 60).await;
        assert_eq!(cache.get("k").await, None);
    }
}
