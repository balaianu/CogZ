//! System resource checks for model loading decisions.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Check if there is enough free memory to load a model.
/// Returns true if the check is disabled (min_free_mb == 0) or if
/// available memory exceeds the floor. On non-Linux platforms, always
/// returns true (no /proc/meminfo).
pub fn has_enough_memory(min_free_mb: u64) -> bool {
    if min_free_mb == 0 {
        return true;
    }
    let available_kb = read_mem_available().unwrap_or(u64::MAX);
    available_kb / 1024 >= min_free_mb
}

/// Read MemAvailable from /proc/meminfo (Linux only).
fn read_mem_available() -> Option<u64> {
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemAvailable:") {
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb);
        }
    }
    None
}

/// Track last-use time for idle TTL unloading.
#[derive(Debug)]
pub struct IdleTracker {
    last_used: Mutex<Option<Instant>>,
    ttl: Duration,
}

impl IdleTracker {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            last_used: Mutex::new(None),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Mark the model as used right now.
    pub fn touch(&self) {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }

    /// Returns true if the model has been idle longer than the TTL.
    /// Always returns false when TTL is zero (never unload).
    pub fn is_idle(&self) -> bool {
        if self.ttl.is_zero() {
            return false;
        }
        match *self.last_used.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(last) => last.elapsed() >= self.ttl,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_tracker_zero_ttl_never_idle() {
        let tracker = IdleTracker::new(0);
        tracker.touch();
        assert!(!tracker.is_idle());
    }

    #[test]
    fn idle_tracker_with_ttl_is_idle_after_wait() {
        let tracker = IdleTracker::new(1);
        tracker.touch();
        assert!(!tracker.is_idle());
        std::thread::sleep(Duration::from_millis(1100));
        assert!(tracker.is_idle());
    }

    #[test]
    fn idle_tracker_without_touch_not_idle() {
        let tracker = IdleTracker::new(1);
        assert!(!tracker.is_idle());
    }

    #[test]
    fn has_enough_memory_disabled_when_zero() {
        assert!(has_enough_memory(0));
    }
}
