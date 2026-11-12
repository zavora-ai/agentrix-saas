//! Counts per key in a sliding window: codes sent per email, failed sign-ins per email. In
//! process memory on the single node (Valkey in the fleet, 02 §8 "Rate limits").

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// At most `limit` events per key in `window`.
#[derive(Debug)]
pub struct Throttle {
    events: Mutex<HashMap<String, Vec<Instant>>>,
    limit: usize,
    window: Duration,
}

impl Throttle {
    /// A throttle allowing `limit` events per key in `window`.
    pub fn new(limit: usize, window: Duration) -> Self {
        Self {
            events: Mutex::new(HashMap::new()),
            limit,
            window,
        }
    }

    /// Whether another event is allowed for `key` now. Does not record one.
    pub fn allowed(&self, key: &str) -> bool {
        let mut events = self.events.lock().unwrap_or_else(|p| p.into_inner());
        let window = self.window;
        match events.get_mut(key) {
            Some(list) => {
                list.retain(|at| at.elapsed() < window);
                list.len() < self.limit
            }
            None => true,
        }
    }

    /// Records an event for `key`.
    pub fn record(&self, key: &str) {
        let mut events = self.events.lock().unwrap_or_else(|p| p.into_inner());
        if events.len() > 50_000 {
            let window = self.window;
            events.retain(|_, list| list.iter().any(|at| at.elapsed() < window));
        }
        events
            .entry(key.to_owned())
            .or_default()
            .push(Instant::now());
    }

    /// Forgets `key` (after a success).
    pub fn clear(&self, key: &str) {
        self.events
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_per_key() {
        let t = Throttle::new(2, Duration::from_secs(60));
        assert!(t.allowed("a"));
        t.record("a");
        t.record("a");
        assert!(!t.allowed("a"));
        assert!(t.allowed("b"));
        t.clear("a");
        assert!(t.allowed("a"));
    }
}
