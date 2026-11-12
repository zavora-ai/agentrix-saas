//! Which addresses a sealed room may reach.
//!
//! A room may connect only to **public unicast** addresses. Everything a hostile page could use
//! to reach inside a cloud network is refused: loopback, RFC 1918 private ranges, link-local
//! (including `169.254.169.254`, the instance-metadata address on Vultr and most clouds),
//! carrier-grade NAT, benchmarking, documentation, multicast, broadcast and reserved space,
//! plus the IPv6 equivalents and IPv4 addresses smuggled inside IPv6 (mapped, compatible and
//! NAT64 forms).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Whether `ip` is a public unicast address a sealed room may connect to.
///
/// ```
/// use agentrix_egress::addr::is_public;
/// assert!(is_public("93.184.215.14".parse().unwrap()));
/// assert!(!is_public("169.254.169.254".parse().unwrap()));
/// assert!(!is_public("::ffff:10.0.0.1".parse().unwrap()));
/// ```
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || a == 0 // "this network"
        || (a == 100 && (64..=127).contains(&b)) // shared address space (CGNAT), RFC 6598
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments
        || (a == 198 && (b == 18 || b == 19)) // benchmarking, RFC 2544
        || a >= 240) // reserved and limited broadcast
}

/// IPv6 is allowed only in global unicast (`2000::/3`), and not in its special-purpose and
/// transition parts. An allowlist is the safer shape here: everything outside `2000::/3`
/// (loopback, unique local, link-local, multicast, discard, the NAT64 local-use prefix) is
/// refused without listing it.
fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = embedded_v4(ip) {
        return is_public_v4(v4);
    }
    let s = ip.segments();
    let global_unicast = (s[0] & 0xe000) == 0x2000; // 2000::/3
    // 2001::/23 is IANA special-purpose (Teredo, benchmarking, ORCHID and others);
    // 2001:db8::/32 and 3fff::/20 (RFC 9637) are documentation; 2002::/16 is 6to4.
    let special = (s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
        || s[0] == 0x2002
        || (s[0] == 0x3fff && s[1] < 0x1000);
    global_unicast && !special
}

/// An IPv4 address carried inside an IPv6 one, judged as IPv4: mapped (`::ffff:a.b.c.d`),
/// translated (`::ffff:0:a.b.c.d`, RFC 2765), compatible (`::a.b.c.d`) and the NAT64
/// well-known prefix (`64:ff9b::a.b.c.d`). The NAT64 local-use prefix (`64:ff9b:1::/48`) is
/// outside `2000::/3` and refused whatever it carries.
fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = ip.segments();
    let tail = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
    let mapped = s[..5].iter().all(|&x| x == 0) && s[5] == 0xffff;
    let translated = s[..4].iter().all(|&x| x == 0) && s[4] == 0xffff && s[5] == 0;
    let compatible = s[..6].iter().all(|&x| x == 0) && !(s[6] == 0 && s[7] <= 1);
    let nat64 = s[0] == 0x0064 && s[1] == 0xff9b && s[2..6].iter().all(|&x| x == 0);
    (mapped || translated || compatible || nat64).then_some(tail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> bool {
        is_public(s.parse().unwrap())
    }

    #[test]
    fn public_addresses_are_allowed() {
        for ip in [
            "93.184.215.14",
            "1.1.1.1",
            "149.28.0.1",
            "2606:4700:4700::1111",
            "2a03:2880::1",
        ] {
            assert!(p(ip), "{ip} should be public");
        }
    }

    #[test]
    fn metadata_and_internal_ranges_are_refused() {
        for ip in [
            "169.254.169.254", // cloud instance metadata
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "100.64.0.1", // CGNAT
            "0.0.0.0",
            "0.1.2.3",
            "192.0.0.8",
            "198.18.0.1",
            "192.0.2.1", // documentation
            "224.0.0.1",
            "255.255.255.255",
            "240.0.0.1",
            "::1",
            "::",
            "fd00:ec2::254", // IPv6 metadata (unique local)
            "fe80::1",
            "fec0::1",
            "2001:db8::1",
            "ff02::1",
            "100::1",
            "3fff::1",                              // documentation, RFC 9637
            "2001:2::1",                            // benchmarking
            "2001:10::1",                           // ORCHID
            "2001:0:4136:e378:8000:63bf:5601:5601", // Teredo
            "2002:a9fe:a9fe::1",                    // 6to4
            "64:ff9b:1::a9fe:a9fe",                 // NAT64 local use
            "::ffff:0:a9fe:a9fe",                   // IPv4-translated
        ] {
            assert!(!p(ip), "{ip} must be refused");
        }
    }

    #[test]
    fn ipv4_smuggled_in_ipv6_is_judged_as_ipv4() {
        assert!(!p("::ffff:169.254.169.254"));
        assert!(!p("::ffff:127.0.0.1"));
        assert!(!p("64:ff9b::10.0.0.1"));
        assert!(!p("::10.0.0.1"));
        assert!(p("::ffff:93.184.215.14"));
        assert!(!p("::ffff:0:10.0.0.1"));
        assert!(p("64:ff9b::93.184.215.14"));
    }
}
