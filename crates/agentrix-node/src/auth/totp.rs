//! TOTP (RFC 6238): HMAC-SHA-1, six digits, 30-second steps, which every authenticator app
//! speaks. A code is accepted within one step of now, and only for a step later than the last
//! one accepted, so each code works once.

use hmac::{Hmac, Mac};
use rand::RngCore;
use sha1::Sha1;
use subtle::ConstantTimeEq;

/// Digits in a code.
pub const DIGITS: usize = 6;
/// Seconds per step.
pub const STEP_SECONDS: u64 = 30;
/// Steps either side of now that are accepted, for clock drift.
pub const WINDOW: u64 = 1;
/// Seed length: 160 bits, as RFC 4226 recommends.
pub const SEED_BYTES: usize = 20;

/// A fresh random seed.
pub fn new_seed() -> [u8; SEED_BYTES] {
    let mut seed = [0u8; SEED_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    seed
}

/// The code for one time step.
pub fn code_at(seed: &[u8], step: u64) -> u32 {
    let mut mac = Hmac::<Sha1>::new_from_slice(seed).expect("HMAC accepts any key length");
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let binary = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    binary % 10u32.pow(DIGITS as u32)
}

/// Checks `code` against the steps around `now_unix`. Refuses any step at or before
/// `last_step`, so a code that was used once is refused. Returns the step that matched; the
/// caller must then store it as the new `last_step` (atomically, see `Store::advance_totp`).
pub fn verify(seed: &[u8], code: &str, now_unix: u64, last_step: u64) -> Option<u64> {
    let typed: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if typed.len() != DIGITS || !typed.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let now = now_unix / STEP_SECONDS;
    let mut matched = None;
    // Always compare all three steps, so timing does not say which one matched.
    for step in now.saturating_sub(WINDOW)..=now + WINDOW {
        let expected = format!("{:0width$}", code_at(seed, step), width = DIGITS);
        let equal: bool = expected.as_bytes().ct_eq(typed.as_bytes()).into();
        if equal && step > last_step && matched.is_none() {
            matched = Some(step);
        }
    }
    matched
}

/// RFC 4648 base32 without padding: the key people type into an authenticator app.
pub fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ALPHABET[((buffer >> bits) & 31) as usize]));
        }
        buffer &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(char::from(ALPHABET[((buffer << (5 - bits)) & 31) as usize]));
    }
    out
}

/// The key in groups of four, as the sign-up screen shows it: `KILE LEB4 KRY7 Q2XM …`.
pub fn grouped(key: &str) -> String {
    key.as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `otpauth://` URI an authenticator app scans.
pub fn otpauth_uri(issuer: &str, account_label: &str, seed: &[u8]) -> String {
    let mut uri = url::Url::parse("otpauth://totp/").expect("static URL parses");
    uri.set_path(&format!("{issuer}:{account_label}"));
    uri.query_pairs_mut()
        .append_pair("secret", &base32(seed))
        .append_pair("issuer", issuer)
        .append_pair("algorithm", "SHA1")
        .append_pair("digits", &DIGITS.to_string())
        .append_pair("period", &STEP_SECONDS.to_string());
    uri.to_string()
}

/// The URI as an inline SVG QR code, for the sign-up screen. The page inserts it as HTML, so
/// it starts at `<svg` (no XML declaration) and holds only the code's modules, never text.
pub fn qr_svg(uri: &str) -> Option<String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(uri.as_bytes()).ok()?;
    let svg = code
        .render::<svg::Color<'_>>()
        .min_dimensions(200, 200)
        .quiet_zone(true)
        .build();
    svg.find("<svg").map(|start| svg[start..].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 appendix B, SHA-1 seed, truncated to six digits.
    const RFC_SEED: &[u8] = b"12345678901234567890";

    #[test]
    fn rfc_6238_vectors() {
        for (time, eight) in [
            (59u64, 94_287_082u32),
            (1_111_111_109, 7_081_804),
            (1_111_111_111, 14_050_471),
            (1_234_567_890, 89_005_924),
            (2_000_000_000, 69_279_037),
        ] {
            assert_eq!(
                code_at(RFC_SEED, time / STEP_SECONDS),
                eight % 1_000_000,
                "t={time}"
            );
        }
    }

    #[test]
    fn accepts_once_and_within_the_window() {
        let now = 1_234_567_890;
        let code = format!("{:06}", code_at(RFC_SEED, now / STEP_SECONDS));
        let step = verify(RFC_SEED, &code, now, 0).expect("current code");
        assert_eq!(
            verify(RFC_SEED, &code, now, step),
            None,
            "a code works once"
        );
        let earlier = format!("{:06}", code_at(RFC_SEED, now / STEP_SECONDS - 1));
        assert!(
            verify(RFC_SEED, &earlier, now, 0).is_some(),
            "one step of drift"
        );
        let old = format!("{:06}", code_at(RFC_SEED, now / STEP_SECONDS - 2));
        assert_eq!(verify(RFC_SEED, &old, now, 0), None, "two steps is too old");
        assert!(verify(RFC_SEED, &format!("{} {}", &code[..3], &code[3..]), now, 0).is_some());
        assert_eq!(verify(RFC_SEED, "12345", now, 0), None);
        assert_eq!(verify(RFC_SEED, "abcdef", now, 0), None);
    }

    #[test]
    fn base32_matches_rfc_4648() {
        assert_eq!(base32(RFC_SEED), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert_eq!(base32(b"f"), "MY");
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
        assert_eq!(grouped("GEZDGNBVGY"), "GEZD GNBV GY");
    }

    #[test]
    fn otpauth_uri_is_scannable() {
        let uri = otpauth_uri("Agentrix", "wanjiru@kilele.example", RFC_SEED);
        assert!(uri.starts_with("otpauth://totp/Agentrix:wanjiru@kilele.example?secret=GEZD"));
        assert!(uri.contains("issuer=Agentrix") && uri.contains("period=30"));
        let svg = qr_svg(&uri).expect("QR renders");
        assert!(
            svg.starts_with("<svg") && !svg.contains("wanjiru"),
            "inline, and no text from the label"
        );
    }
}
