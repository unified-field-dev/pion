//! In-process token-bucket rate limiter for the Parton agent HTTP surface.
//!
//! Deliberately minimal: a `std::sync::Mutex<HashMap<..>>` guarding one bucket per rate-limit key
//! (peer IP, optionally combined with `node_id`). No extra crates — this only needs to survive one
//! process's lifetime and does not need to be shared across replicas.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Default `PION_AGENT_RATE_LIMIT_PER_MIN` when the env var is unset or invalid.
const DEFAULT_LIMIT_PER_MIN: u32 = 120;

/// Above this many distinct keys, opportunistically evict buckets idle for over an hour so a
/// long-lived process serving many distinct peers doesn't grow this map unbounded.
const CLEANUP_THRESHOLD: usize = 10_000;
const STALE_BUCKET_TTL: Duration = Duration::from_hours(1);

struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

#[derive(Default)]
struct Limiter {
    buckets: Mutex<HashMap<String, Bucket>>,
}

fn limiter() -> &'static Limiter {
    static INSTANCE: OnceLock<Limiter> = OnceLock::new();
    INSTANCE.get_or_init(Limiter::default)
}

/// Reads `PION_AGENT_RATE_LIMIT_PER_MIN` (positive integer per-key-per-minute budget).
///
/// Falls back to [`DEFAULT_LIMIT_PER_MIN`] when unset or unparseable. `0` disables rate limiting
/// entirely (local experiments only — never set this on a reachable deployment).
pub(super) fn limit_per_min() -> u32 {
    std::env::var("PION_AGENT_RATE_LIMIT_PER_MIN")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(DEFAULT_LIMIT_PER_MIN)
}

/// True when a request identified by `key` is allowed under a `capacity`-per-minute token bucket.
///
/// Tokens refill continuously (`capacity / 60` per second) rather than in fixed windows, so a
/// burst right at a window boundary can't double the effective rate. A `capacity` of `0` disables
/// rate limiting unconditionally.
///
/// # Critical section
///
/// Holds `Limiter::buckets`'s [`Mutex`] for the whole read-refill-decrement sequence on `key`'s
/// bucket (plus the opportunistic stale-bucket sweep above [`CLEANUP_THRESHOLD`]), so concurrent
/// callers never observe or apply a partial refill.
pub(super) fn allow(key: &str, capacity: u32) -> bool {
    if capacity == 0 {
        return true;
    }
    let capacity = f64::from(capacity);
    let refill_per_sec = capacity / 60.0;
    let now = Instant::now();

    let limiter = limiter();
    let mut buckets = limiter
        .buckets
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if buckets.len() > CLEANUP_THRESHOLD {
        buckets.retain(|_, b| now.duration_since(b.last_refill) < STALE_BUCKET_TTL);
    }

    let bucket = buckets.entry(key.to_string()).or_insert_with(|| Bucket {
        tokens: capacity,
        last_refill: now,
    });
    let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
    bucket.tokens = (bucket.tokens + elapsed * refill_per_sec).min(capacity);
    bucket.last_refill = now;

    if bucket.tokens >= 1.0 {
        bucket.tokens -= 1.0;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn allows_up_to_capacity_then_blocks() {
        let key = "test-key-allows-up-to-capacity-then-blocks";
        for i in 0..5 {
            assert!(
                allow(key, 5),
                "request {i} should be allowed within capacity"
            );
        }
        assert!(
            !allow(key, 5),
            "6th request within the same instant should be blocked"
        );
    }

    #[test]
    fn zero_capacity_disables_rate_limiting() {
        let key = "test-key-zero-capacity-disables";
        for _ in 0..1000 {
            assert!(allow(key, 0), "capacity 0 must always allow");
        }
    }

    #[test]
    fn distinct_keys_do_not_share_a_bucket() {
        let key_a = "test-key-distinct-a";
        let key_b = "test-key-distinct-b";
        for _ in 0..3 {
            assert!(allow(key_a, 3));
        }
        assert!(!allow(key_a, 3), "key_a should be exhausted");
        assert!(
            allow(key_b, 3),
            "key_b must have its own independent budget"
        );
    }

    #[test]
    fn refills_over_time() {
        let key = "test-key-refills-over-time";
        assert!(allow(key, 60), "first request consumes a token");
        // Manually drain remaining tokens for a deterministic starting point.
        for _ in 0..59 {
            allow(key, 60);
        }
        assert!(
            !allow(key, 60),
            "bucket should be empty immediately after draining"
        );
        std::thread::sleep(Duration::from_millis(50));
        // 60 tokens/min == 1/sec; 50ms isn't quite enough for a full token, but the bucket
        // should have partially refilled rather than staying pinned at (near) zero forever.
        std::thread::sleep(Duration::from_millis(1050));
        assert!(
            allow(key, 60),
            "bucket should have refilled at least one token after >1s"
        );
    }

    #[test]
    #[serial]
    fn limit_per_min_falls_back_to_default_when_unset_or_invalid() {
        std::env::remove_var("PION_AGENT_RATE_LIMIT_PER_MIN");
        assert_eq!(limit_per_min(), DEFAULT_LIMIT_PER_MIN);
        std::env::set_var("PION_AGENT_RATE_LIMIT_PER_MIN", "not-a-number");
        assert_eq!(limit_per_min(), DEFAULT_LIMIT_PER_MIN);
        std::env::remove_var("PION_AGENT_RATE_LIMIT_PER_MIN");
    }

    #[test]
    #[serial]
    fn limit_per_min_reads_configured_value() {
        std::env::set_var("PION_AGENT_RATE_LIMIT_PER_MIN", "42");
        assert_eq!(limit_per_min(), 42);
        std::env::remove_var("PION_AGENT_RATE_LIMIT_PER_MIN");
    }
}
