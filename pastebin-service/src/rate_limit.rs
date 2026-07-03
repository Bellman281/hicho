//! A small in-process, per-IP token-bucket rate limiter.
//!
//! Each client IP gets a bucket of `burst` tokens that refills at `rps` tokens
//! per second; a request consumes one token, and is rejected when the bucket is
//! empty. `rps == 0` disables limiting entirely.
//!
//! Scope: this is a **per-instance** limiter — each replica counts
//! independently. A globally-consistent limit across replicas needs a shared
//! store (e.g. Redis). A background sweeper evicts idle buckets so the map
//! cannot grow without bound (e.g. under a flood of distinct source IPs).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// Buckets untouched for at least this long are evicted. A bucket idle this long
/// has fully refilled, so dropping it is behavior-preserving — a later request
/// simply recreates a full one.
const IDLE_EVICT: Duration = Duration::from_secs(600);

type Buckets = Mutex<HashMap<IpAddr, Bucket>>;

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

/// Thread-safe per-IP token-bucket limiter.
#[derive(Debug)]
pub struct RateLimiter {
    capacity: f64,
    refill_per_sec: f64,
    buckets: Arc<Buckets>,
}

impl RateLimiter {
    /// Build a limiter. `rps == 0` disables limiting. When `burst == 0` but
    /// `rps > 0`, the burst defaults to `rps`.
    pub fn new(rps: u32, burst: u32) -> Self {
        let capacity = if burst == 0 { rps } else { burst };
        let buckets: Arc<Buckets> = Arc::new(Mutex::new(HashMap::new()));
        // When limiting is enabled AND we're inside a Tokio runtime (the running
        // server, not a synchronous unit test), spawn a sweeper that evicts idle
        // buckets. It holds only a Weak ref, so it exits when the limiter drops.
        if rps > 0 {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(sweep_idle(Arc::downgrade(&buckets)));
            }
        }
        Self {
            capacity: capacity as f64,
            refill_per_sec: rps as f64,
            buckets,
        }
    }

    /// Returns true if the request is allowed (and consumes a token).
    pub fn check(&self, ip: IpAddr) -> bool {
        if self.refill_per_sec <= 0.0 {
            return true; // disabled
        }
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let bucket = buckets.entry(ip).or_insert(Bucket {
            tokens: self.capacity,
            last: now,
        });
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Periodically drop buckets untouched for `IDLE_EVICT`, bounding memory. Exits
/// once the `RateLimiter` (the last strong `Arc`) is dropped.
async fn sweep_idle(buckets: Weak<Buckets>) {
    let mut ticker = tokio::time::interval(IDLE_EVICT);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let Some(buckets) = buckets.upgrade() else {
            return;
        };
        let now = Instant::now();
        let mut map = buckets.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, b| now.duration_since(b.last) < IDLE_EVICT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn rps_zero_disables_limiting() {
        let rl = RateLimiter::new(0, 0);
        let a = ip("1.2.3.4");
        for _ in 0..1000 {
            assert!(rl.check(a));
        }
    }

    #[test]
    fn burst_is_consumed_then_blocked() {
        let rl = RateLimiter::new(1, 1);
        let a = ip("1.2.3.4");
        assert!(rl.check(a));
        assert!(!rl.check(a));
    }

    #[test]
    fn buckets_are_independent_per_ip() {
        let rl = RateLimiter::new(1, 1);
        let a = ip("1.1.1.1");
        let b = ip("2.2.2.2");
        assert!(rl.check(a));
        assert!(!rl.check(a));
        assert!(rl.check(b));
    }
}
