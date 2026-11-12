//! Short-lived server-side state between the two halves of a ceremony: a passkey challenge, a
//! password that still needs its TOTP code, or a sign-up waiting for its first code. Kept in
//! memory (single node), keyed by an unguessable id, used once, gone after `ttl`.

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
        self.put_as(id.clone(), value);
        id
    }

    /// Stores a value under a given key (for example a challenge), replacing any earlier one.
    pub fn put_as(&self, key: String, value: T) {
        let mut items = self.items.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        items.retain(|_, (at, _)| now.duration_since(*at) < self.ttl);
        if items.len() >= self.cap {
            // Full of live entries: drop the oldest rather than refuse a new sign-in.
            if let Some(oldest) = items
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| k.clone())
            {
                items.remove(&oldest);
            }
        }
        items.insert(key, (now, value));
    }

    /// Takes a value out, once. `None` if unknown or expired.
    pub fn take(&self, key: &str) -> Option<T> {
        let mut items = self.items.lock().unwrap_or_else(|p| p.into_inner());
        let (at, value) = items.remove(key)?;
        (at.elapsed() < self.ttl).then_some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn once_and_expiring() {
        let pending = Pending::new(Duration::from_secs(60), 2);
        let a = pending.put(1);
        assert_eq!(pending.take(&a), Some(1));
        assert_eq!(pending.take(&a), None, "used once");
        let b = pending.put(2);
        let c = pending.put(3);
        let d = pending.put(4);
        assert_eq!(pending.take(&b), None, "the oldest was dropped at the cap");
        assert_eq!((pending.take(&c), pending.take(&d)), (Some(3), Some(4)));
        let gone = Pending::new(Duration::ZERO, 2);
        let e = gone.put(5);
        assert_eq!(gone.take(&e), None, "expired");
    }
}
