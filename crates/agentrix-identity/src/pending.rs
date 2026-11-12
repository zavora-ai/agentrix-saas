//! Short-lived state between the two halves of a ceremony (a passkey challenge, a code waiting
//! to be typed, recovery codes waiting to be shown once). In memory on the single node, keyed
//! by an unguessable id, gone after `ttl`. The first release's `auth/pending.rs`, with `get_mut` for
//! counting tries.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;

/// A random 128-bit id, base64url.
pub fn new_id() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Values waiting for their second half.
#[derive(Debug)]
pub struct Pending<T> {
    items: Mutex<HashMap<String, (Instant, T)>>,
    ttl: Duration,
    cap: usize,
}

impl<T> Pending<T> {
    /// At most `cap` live entries, each kept for `ttl`.
    pub fn new(ttl: Duration, cap: usize) -> Self {
        Self {
            items: Mutex::new(HashMap::new()),
            ttl,
            cap,
        }
    }

    /// Stores a value under a fresh id.
    pub fn put(&self, value: T) -> String {
        let id = new_id();
        let mut items = self.items.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        items.retain(|_, (at, _)| now.duration_since(*at) < self.ttl);
        if items.len() >= self.cap {
            if let Some(oldest) = items
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| k.clone())
            {
                items.remove(&oldest);
            }
        }
        items.insert(id.clone(), (now, value));
        id
    }

    /// Takes a value out, once. `None` if unknown or expired.
    pub fn take(&self, key: &str) -> Option<T> {
        let mut items = self.items.lock().unwrap_or_else(|p| p.into_inner());
        let (at, value) = items.remove(key)?;
        (at.elapsed() < self.ttl).then_some(value)
    }

    /// Runs `f` on a live value in place (to count a wrong try), without taking it.
    pub fn with<R>(&self, key: &str, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        let mut items = self.items.lock().unwrap_or_else(|p| p.into_inner());
        let (at, value) = items.get_mut(key)?;
        if at.elapsed() >= self.ttl {
            items.remove(key);
            return None;
        }
        Some(f(value))
    }

    /// Removes a value.
    pub fn remove(&self, key: &str) {
        self.items
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn once_expiring_and_in_place() {
        let pending = Pending::new(Duration::from_secs(60), 2);
        let a = pending.put(1);
        assert_eq!(
            pending.with(&a, |v| {
                *v += 1;
                *v
            }),
            Some(2)
        );
        assert_eq!(pending.take(&a), Some(2));
        assert_eq!(pending.take(&a), None, "used once");
        let b = pending.put(2);
        let c = pending.put(3);
        let d = pending.put(4);
        assert_eq!(pending.take(&b), None, "the oldest was dropped at the cap");
        assert_eq!((pending.take(&c), pending.take(&d)), (Some(3), Some(4)));
        let gone = Pending::new(Duration::ZERO, 2);
        let e = gone.put(5);
        assert_eq!(gone.with(&e, |v| *v), None, "expired");
    }
}
