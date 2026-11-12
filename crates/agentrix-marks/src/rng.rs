//! The seeded generator: the same numbers from the same key, in Rust and in JavaScript.
//!
//! `seed_from` is cyrb-style string hashing over UTF-16 code units (what JavaScript's
//! `charCodeAt` reads); `Rng` is mulberry32. Both use only 32-bit integer operations, which
//! the two languages define identically.

/// Hashes a key to a 32-bit seed.
pub fn seed_from(key: &str) -> u32 {
    let units: Vec<u16> = key.encode_utf16().collect();
    let mut h: u32 = 1_779_033_703 ^ (units.len() as u32);
    for u in units {
        h = (h ^ u32::from(u)).wrapping_mul(3_432_918_353);
        h = h.rotate_left(13);
    }
    h = (h ^ (h >> 16)).wrapping_mul(2_246_822_507);
    h = (h ^ (h >> 13)).wrapping_mul(3_266_489_909);
    h ^ (h >> 16)
}

/// mulberry32, seeded from a key.
pub struct Rng(u32);

impl Rng {
    /// A generator for this key.
    pub fn for_key(key: &str) -> Self {
        Rng(seed_from(key))
    }

    /// The next number in [0, 1).
    pub fn float(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6D2B_79F5);
        let a = self.0;
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_prototype() {
        // Values printed by concept/identity/marks.html's seedFrom and rngFor under Node.
        assert_eq!(seed_from(""), 167_010_153);
        assert_eq!(seed_from("kilele:cakes:7"), 1_979_183_986);
        assert_eq!(seed_from("tana:newsroom:21"), 3_382_400_146);
        assert_eq!(seed_from("café:ü:1"), 3_204_835_716);
        let mut r = Rng::for_key("kilele:cakes:7");
        assert_eq!(r.float(), 0.374_054_935_760_796_07);
        assert_eq!(r.float(), 0.078_903_959_598_392_25);
        assert_eq!(r.float(), 0.570_832_940_284_162_8);

        let mut a = Rng::for_key("kilele:cakes:7");
        let mut b = Rng::for_key("kilele:cakes:7");
        for _ in 0..100 {
            let x = a.float();
            assert!((0.0..1.0).contains(&x));
            assert_eq!(x.to_bits(), b.float().to_bits());
        }
        assert_ne!(seed_from("kilele:cakes:7"), seed_from("kilele:cakes:8"));
    }
}
