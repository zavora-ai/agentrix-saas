//! S2-09: the independent security review of `agentrix-egress` (R12.3, R12.6).
//!
//! One test per case. A case that fails blocks the deploy. Each test names the attack, sends it
//! through the real proxy over real sockets, and asserts the room learns nothing and reaches
//! nothing it should not.
//!
//! | Case | Tests |
//! |---|---|
//! | Instance metadata and private ranges, in every IPv4 encoding | `metadata_in_*` |
//! | IPv4 inside IPv6 (mapped, compatible, NAT64, translated, 6to4, Teredo) | `ipv4_inside_ipv6_*` |
//! | IPv6 non-public ranges | `ipv6_non_public_ranges_are_refused` |
//! | DNS answers checked after resolution; the checked address is the one connected | `a_name_*`, `the_connection_goes_to_the_checked_address` |
//! | Redirects to refused hosts | `a_redirect_*` |
//! | CONNECT abuse | `connect_*` |
//! | Request smuggling and header tricks | `smuggling_*`, `header_*` |
//! | Absolute-form URI tricks | `absolute_form_*` |
//! | Host mismatch (virtual-host and domain fronting) | `host_header_*`, `connect_sni_*` |
//!
//! The upstream these tests use lives on loopback, so its policy sets `permit_non_public`; every
//! case that tests the address rule uses a policy without it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use agentrix_egress::policy::{Origin, Policy};
use agentrix_egress::proxy::{self, Decision, Event, Sink};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// --- harness -------------------------------------------------------------------------------

/// An upstream on loopback that records every byte it receives and answers each connection
/// with `response`.
struct Upstream {
    port: u16,
    received: Arc<Mutex<Vec<u8>>>,
}

impl Upstream {
    async fn start(response: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let received = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&received);
        tokio::spawn(async move {
            loop {
                let (mut s, _) = listener.accept().await.unwrap();
                let log = Arc::clone(&log);
                tokio::spawn(async move {
                    let mut buf = [0u8; 8192];
                    // Read what arrives within a short window, so pipelined bytes are seen too.
                    let deadline = tokio::time::Instant::now() + Duration::from_millis(400);
                    let mut answered = false;
                    while let Ok(Ok(n)) = tokio::time::timeout_at(deadline, s.read(&mut buf)).await
                    {
                        if n == 0 {
                            break;
                        }
                        log.lock().unwrap().extend_from_slice(&buf[..n]);
                        if !answered {
                            answered = true;
                            let _ = s.write_all(response.as_bytes()).await;
                        }
                    }
                    if !answered {
                        let _ = s.write_all(response.as_bytes()).await;
                    }
                    let _ = s.shutdown().await;
                });
            }
        });
        Self { port, received }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.received.lock().unwrap()).into_owned()
    }
}

const OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";

async fn proxy_with(policy: Policy) -> (u16, Arc<Mutex<Vec<Event>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink: Sink = {
        let events = Arc::clone(&events);
        Arc::new(move |e| events.lock().unwrap().push(e))
    };
    tokio::spawn(proxy::serve(listener, policy, "job-review", 8, sink));
    (port, events)
}

/// Sends `request` raw and returns everything the proxy answers.
async fn send(proxy_port: u16, request: &[u8]) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    s.write_all(request).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(8), s.read_to_end(&mut out)).await;
    String::from_utf8_lossy(&out).into_owned()
}

async fn last_event(events: &Arc<Mutex<Vec<Event>>>) -> Event {
    for _ in 0..500 {
        if let Some(e) = events.lock().unwrap().last().cloned() {
            return e;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no event recorded");
}

/// A policy allowing exactly `host` (on 80 and 443), public addresses only.
fn only(host: &str) -> Policy {
    Policy::only([Origin::parse(host).unwrap()])
}

/// The loopback upstream on `port`, allowed under the name `127.0.0.1`.
fn loopback(port: u16) -> Policy {
    let mut policy = Policy::only([Origin::parse(&format!("127.0.0.1:{port}")).unwrap()]);
    policy.permit_non_public = true;
    policy
}

/// Asserts the request for `host` (on the allowlist, so only the address rule can stop it) was
/// refused before anything was sent: a 403, a `deny` event and no bytes. An IP literal must be
/// refused as non-public. A name may instead fail to resolve, which also reaches nothing, but
/// it must never connect.
async fn assert_refused_as_non_public(host: &str, request: &str) {
    let (port, events) = proxy_with(only(host)).await;
    let reply = send(port, request.as_bytes()).await;
    assert!(reply.starts_with("HTTP/1.1 403"), "{host}: {reply}");
    let e = last_event(&events).await;
    assert_eq!(e.decision, Decision::Deny, "{host}");
    assert_eq!(
        (e.bytes_up, e.bytes_down, e.ip.as_deref()),
        (0, 0, None),
        "{host}"
    );
    let reason = e.reason.unwrap_or_default();
    let literal = host.starts_with('[') || host.parse::<std::net::IpAddr>().is_ok();
    let refused = reason.contains("non-public") || (!literal && reason.contains("DNS"));
    assert!(refused, "{host}: not refused by the address rule: {reason}");
}

// --- R12.3: metadata, loopback, private and link-local, in every IPv4 encoding --------------

#[tokio::test]
async fn metadata_in_dotted_decimal_is_refused() {
    assert_refused_as_non_public(
        "169.254.169.254",
        "GET http://169.254.169.254/latest/meta-data/ HTTP/1.1\r\n\r\n",
    )
    .await;
}

/// `2852039166` is 169.254.169.254 as one decimal number. The system resolver accepts the
/// `inet_aton` forms, so the proxy must judge the resolved address, not the spelling.
#[tokio::test]
async fn metadata_in_decimal_is_refused() {
    assert_refused_as_non_public("2852039166", "GET http://2852039166/ HTTP/1.1\r\n\r\n").await;
}

#[tokio::test]
async fn metadata_in_octal_is_refused() {
    assert_refused_as_non_public(
        "0251.0376.0251.0376",
        "GET http://0251.0376.0251.0376/ HTTP/1.1\r\n\r\n",
    )
    .await;
}

#[tokio::test]
async fn metadata_in_hex_is_refused() {
    assert_refused_as_non_public("0xa9fea9fe", "GET http://0xa9fea9fe/ HTTP/1.1\r\n\r\n").await;
    assert_refused_as_non_public(
        "0xa9.0xfe.0xa9.0xfe",
        "GET http://0xa9.0xfe.0xa9.0xfe/ HTTP/1.1\r\n\r\n",
    )
    .await;
}

#[tokio::test]
async fn metadata_in_short_forms_is_refused() {
    // a.b.c with c as 16 bits, and a.b with b as 24 bits.
    assert_refused_as_non_public(
        "169.254.43518",
        "GET http://169.254.43518/ HTTP/1.1\r\n\r\n",
    )
    .await;
    assert_refused_as_non_public("169.16689662", "GET http://169.16689662/ HTTP/1.1\r\n\r\n").await;
}

#[tokio::test]
async fn loopback_private_and_link_local_literals_are_refused() {
    for host in [
        "127.0.0.1",
        "127.1",
        "0",
        "10.0.0.1",
        "172.16.0.1",
        "192.168.0.1",
        "100.64.0.1",
    ] {
        assert_refused_as_non_public(host, &format!("GET http://{host}/ HTTP/1.1\r\n\r\n")).await;
    }
}

// --- R12.3: IPv4 smuggled inside IPv6 -------------------------------------------------------

#[tokio::test]
async fn ipv4_inside_ipv6_mapped_compatible_and_nat64_is_refused() {
    for host in [
        "[::ffff:169.254.169.254]",
        "[::ffff:a9fe:a9fe]",
        "[::169.254.169.254]",
        "[64:ff9b::a9fe:a9fe]",
        "[::ffff:127.0.0.1]",
    ] {
        assert_refused_as_non_public(host, &format!("CONNECT {host}:443 HTTP/1.1\r\n\r\n")).await;
    }
}

/// `::ffff:0:a.b.c.d` (IPv4-translated, RFC 2765) and the NAT64 local-use prefix
/// `64:ff9b:1::/48` (RFC 8215) carry an IPv4 address that a translator on the path delivers.
#[tokio::test]
async fn ipv4_inside_ipv6_translated_and_local_nat64_is_refused() {
    for host in ["[::ffff:0:a9fe:a9fe]", "[64:ff9b:1::a9fe:a9fe]"] {
        assert_refused_as_non_public(host, &format!("CONNECT {host}:443 HTTP/1.1\r\n\r\n")).await;
    }
}

/// 6to4 (`2002::/16`) embeds an IPv4 address in bits 16–48; Teredo (`2001::/32`) embeds one,
/// inverted, in the last 32 bits. Both are transition ranges no sealed room needs.
#[tokio::test]
async fn ipv4_inside_ipv6_6to4_and_teredo_is_refused() {
    for host in [
        "[2002:a9fe:a9fe::1]",
        "[2001:0:4136:e378:8000:63bf:5601:5601]",
    ] {
        assert_refused_as_non_public(host, &format!("CONNECT {host}:443 HTTP/1.1\r\n\r\n")).await;
    }
}

#[tokio::test]
async fn ipv6_non_public_ranges_are_refused() {
    for host in [
        "[::1]",
        "[::]",
        "[fe80::1]",
        "[fd00:ec2::254]", // AWS's IPv6 metadata address, unique local
        "[fc00::1]",
        "[ff02::1]",
        "[2001:db8::1]",
        "[3fff::1]",    // documentation, RFC 9637
        "[2001:2::1]",  // benchmarking, RFC 5180
        "[2001:10::1]", // ORCHID
        "[100::1]",
    ] {
        assert_refused_as_non_public(host, &format!("CONNECT {host}:443 HTTP/1.1\r\n\r\n")).await;
    }
}

// --- R12.3: DNS is checked after resolution, and the checked address is the one used --------

#[tokio::test]
async fn a_name_resolving_to_loopback_is_refused_after_resolution() {
    assert_refused_as_non_public("localhost", "GET http://localhost/ HTTP/1.1\r\n\r\n").await;
}

/// Rebinding: a public DNS name whose answer is the metadata address. Needs network DNS, so it
/// is ignored by default; run with `--ignored` on the sandbox VM.
#[tokio::test]
#[ignore = "needs public DNS (nip.io)"]
async fn a_name_resolving_to_metadata_is_refused_after_resolution() {
    assert_refused_as_non_public(
        "169.254.169.254.nip.io",
        "GET http://169.254.169.254.nip.io/ HTTP/1.1\r\n\r\n",
    )
    .await;
}

/// The proxy resolves once and connects to an address it checked, so a second, different
/// answer cannot slip in between check and connect. The recorded address is the checked one.
#[tokio::test]
async fn the_connection_goes_to_the_checked_address() {
    let upstream = Upstream::start(OK).await;
    let (port, events) = proxy_with(loopback(upstream.port)).await;
    let reply = send(
        port,
        format!("GET http://127.0.0.1:{}/ HTTP/1.1\r\n\r\n", upstream.port).as_bytes(),
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    let e = last_event(&events).await;
    assert_eq!(
        (e.decision, e.ip.as_deref()),
        (Decision::Allow, Some("127.0.0.1"))
    );
}

// --- R12.3: redirects -------------------------------------------------------------------------

/// The allowed site answers with a redirect to the metadata service. The proxy passes the
/// redirect back (it does not follow redirects itself), and the room's follow-up request is
/// judged on its own and refused.
#[tokio::test]
async fn a_redirect_to_a_refused_host_is_refused_when_followed() {
    let upstream = Upstream::start(
        "HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/latest/meta-data/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    )
    .await;
    let (port, events) = proxy_with(loopback(upstream.port)).await;
    let first = send(
        port,
        format!("GET http://127.0.0.1:{}/ HTTP/1.1\r\n\r\n", upstream.port).as_bytes(),
    )
    .await;
    assert!(first.starts_with("HTTP/1.1 302"), "{first}");
    let follow = send(
        port,
        b"GET http://169.254.169.254/latest/meta-data/ HTTP/1.1\r\n\r\n",
    )
    .await;
    assert!(follow.starts_with("HTTP/1.1 403"), "{follow}");
    assert_eq!(last_event(&events).await.decision, Decision::Deny);
}

#[tokio::test]
async fn a_redirect_to_another_site_is_refused_when_followed() {
    let (port, _) = proxy_with(only("kilele.example")).await;
    let follow = send(port, b"GET http://evil.example/ HTTP/1.1\r\n\r\n").await;
    assert!(follow.starts_with("HTTP/1.1 403"), "{follow}");
}

// --- R12.3: CONNECT abuse ---------------------------------------------------------------------

#[tokio::test]
async fn connect_to_a_port_off_the_allowlist_is_refused() {
    let (port, _) = proxy_with(only("kilele.example")).await;
    for target in [
        "kilele.example:22",
        "kilele.example:25",
        "kilele.example:8443",
    ] {
        let reply = send(
            port,
            format!("CONNECT {target} HTTP/1.1\r\n\r\n").as_bytes(),
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 403"), "{target}: {reply}");
    }
}

#[tokio::test]
async fn connect_without_a_port_or_with_a_url_is_refused() {
    let (port, _) = proxy_with(only("kilele.example")).await;
    for target in [
        "kilele.example",
        "https://kilele.example:443",
        "kilele.example:443/path",
    ] {
        let reply = send(
            port,
            format!("CONNECT {target} HTTP/1.1\r\n\r\n").as_bytes(),
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 403"), "{target}: {reply}");
    }
}

/// A minimal TLS 1.2 ClientHello whose SNI is `name`.
fn client_hello(name: &str) -> Vec<u8> {
    let name = name.as_bytes();
    let mut sni = Vec::new();
    sni.extend_from_slice(&((name.len() + 3) as u16).to_be_bytes()); // server name list length
    sni.push(0); // host_name
    sni.extend_from_slice(&(name.len() as u16).to_be_bytes());
    sni.extend_from_slice(name);
    let mut extensions = Vec::new();
    extensions.extend_from_slice(&0u16.to_be_bytes()); // server_name
    extensions.extend_from_slice(&(sni.len() as u16).to_be_bytes());
    extensions.extend_from_slice(&sni);
    let mut body = vec![0x03, 0x03]; // TLS 1.2
    body.extend_from_slice(&[0u8; 32]); // random
    body.push(0); // session id
    body.extend_from_slice(&[0x00, 0x02, 0x00, 0x2f]); // one cipher suite
    body.extend_from_slice(&[0x01, 0x00]); // null compression
    body.extend_from_slice(&(extensions.len() as u16).to_be_bytes());
    body.extend_from_slice(&extensions);
    let mut handshake = vec![0x01]; // ClientHello
    handshake.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    handshake.extend_from_slice(&body);
    let mut record = vec![0x16, 0x03, 0x01];
    record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
    record.extend_from_slice(&handshake);
    record
}

/// Domain fronting: a tunnel to an allowed host whose TLS names another site. On a shared CDN
/// address the TLS server name picks the site, so the proxy must check it matches the CONNECT
/// host, and refuse a tunnel with no TLS or no server name.
#[tokio::test]
async fn connect_sni_must_match_the_connect_host() {
    let upstream = Upstream::start(OK).await;
    let (port, _) = proxy_with(loopback(upstream.port)).await;
    let mut request = format!("CONNECT 127.0.0.1:{} HTTP/1.1\r\n\r\n", upstream.port).into_bytes();
    request.extend_from_slice(&client_hello("evil.example"));
    let _ = send(port, &request).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !upstream.text().contains("evil.example"),
        "a ClientHello naming another site reached the destination"
    );
}

/// A tunnel is for TLS. Plain HTTP inside a tunnel escapes the request checks: its Host header
/// can name any site on the destination's address.
#[tokio::test]
async fn connect_carrying_plain_http_is_refused() {
    let upstream = Upstream::start(OK).await;
    let (port, _) = proxy_with(loopback(upstream.port)).await;
    let request = format!(
        "CONNECT 127.0.0.1:{} HTTP/1.1\r\n\r\nGET / HTTP/1.1\r\nHost: evil.example\r\n\r\n",
        upstream.port
    );
    let _ = send(port, request.as_bytes()).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !upstream.text().contains("evil.example"),
        "plain HTTP naming another site went through a tunnel: {}",
        upstream.text()
    );
}

// --- R12.3: Host mismatch ---------------------------------------------------------------------

/// The request names an allowed site but its Host header names another. On a shared address
/// the Host header picks the site, so the upstream must never see a Host other than the
/// target's.
#[tokio::test]
async fn host_header_must_match_the_target() {
    let upstream = Upstream::start(OK).await;
    let (port, _) = proxy_with(loopback(upstream.port)).await;
    let reply = send(
        port,
        format!(
            "GET http://127.0.0.1:{}/ HTTP/1.1\r\nHost: evil.example\r\n\r\n",
            upstream.port
        )
        .as_bytes(),
    )
    .await;
    assert!(
        !upstream
            .text()
            .to_ascii_lowercase()
            .contains("host: evil.example"),
        "the upstream saw another site's Host (proxy answered {reply:?})"
    );
}

#[tokio::test]
async fn host_header_twice_is_refused() {
    let upstream = Upstream::start(OK).await;
    let (port, _) = proxy_with(loopback(upstream.port)).await;
    let reply = send(
        port,
        format!(
            "GET http://127.0.0.1:{p}/ HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nHost: evil.example\r\n\r\n",
            p = upstream.port
        )
        .as_bytes(),
    )
    .await;
    assert!(
        !upstream.text().contains("evil.example"),
        "a second Host reached the upstream (proxy answered {reply:?})"
    );
}

// --- R12.3: smuggling and header tricks ---------------------------------------------------------

/// Content-Length and Transfer-Encoding together let the proxy and the upstream disagree on
/// where the body ends (RFC 9112 §6.1); such a request must be refused.
#[tokio::test]
async fn smuggling_cl_te_is_refused() {
    let upstream = Upstream::start(OK).await;
    let (port, _) = proxy_with(loopback(upstream.port)).await;
    let request = format!(
        "POST http://127.0.0.1:{p}/ HTTP/1.1\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\nGET http://169.254.169.254/ HTTP/1.1\r\nHost: 169.254.169.254\r\n\r\n",
        p = upstream.port
    );
    let reply = send(port, request.as_bytes()).await;
    assert!(
        reply.starts_with("HTTP/1.1 4"),
        "CL+TE was not refused: {reply}"
    );
    assert!(
        upstream.text().is_empty(),
        "the upstream received: {}",
        upstream.text()
    );
}

/// Requests pipelined after the first on one connection must be judged too, or refused. Here
/// the second asks the same address for another site.
#[tokio::test]
async fn smuggling_a_pipelined_request_for_another_site_is_not_delivered() {
    let upstream = Upstream::start(OK).await;
    let (port, _) = proxy_with(loopback(upstream.port)).await;
    let request = format!(
        "GET http://127.0.0.1:{p}/ HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nContent-Length: 0\r\n\r\nGET /admin HTTP/1.1\r\nHost: evil.example\r\n\r\n",
        p = upstream.port
    );
    let _ = send(port, request.as_bytes()).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !upstream.text().contains("evil.example"),
        "a pipelined request reached the upstream unjudged: {}",
        upstream.text()
    );
}

/// An obsolete line fold (a header line starting with a space) must not become a header of its
/// own: re-emitting it as `Host: …` would smuggle a second Host past the check.
#[tokio::test]
async fn header_obs_fold_does_not_become_a_header() {
    let upstream = Upstream::start(OK).await;
    let (port, _) = proxy_with(loopback(upstream.port)).await;
    let request = format!(
        "GET http://127.0.0.1:{}/ HTTP/1.1\r\nX-Note: a\r\n Host: evil.example\r\n\r\n",
        upstream.port
    );
    let _ = send(port, request.as_bytes()).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !upstream
            .text()
            .to_ascii_lowercase()
            .contains("\nhost: evil.example"),
        "a folded line became a Host header: {}",
        upstream.text()
    );
}

/// Whitespace between a header name and its colon is invalid (RFC 9112 §5.1); a proxy that
/// trims it turns `Host : evil` into a Host header the client never validly sent.
#[tokio::test]
async fn header_space_before_colon_is_refused() {
    let upstream = Upstream::start(OK).await;
    let (port, _) = proxy_with(loopback(upstream.port)).await;
    let request = format!(
        "GET http://127.0.0.1:{}/ HTTP/1.1\r\nHost : evil.example\r\n\r\n",
        upstream.port
    );
    let _ = send(port, request.as_bytes()).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !upstream
            .text()
            .to_ascii_lowercase()
            .contains("host: evil.example"),
        "a malformed header was normalised into a Host header: {}",
        upstream.text()
    );
}

/// A bare LF inside a header value ends the line for lenient upstreams, injecting a header the
/// proxy never saw as one.
#[tokio::test]
async fn header_bare_lf_is_refused() {
    let upstream = Upstream::start(OK).await;
    let (port, _) = proxy_with(loopback(upstream.port)).await;
    let request = format!(
        "GET http://127.0.0.1:{}/ HTTP/1.1\r\nX-Note: a\nHost: evil.example\r\n\r\n",
        upstream.port
    );
    let _ = send(port, request.as_bytes()).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !upstream.text().contains("evil.example"),
        "a bare LF smuggled a header to the upstream: {}",
        upstream.text()
    );
}

// --- R12.3: absolute-form URI tricks ----------------------------------------------------------

#[tokio::test]
async fn absolute_form_tricks_are_refused() {
    let (port, events) = proxy_with(only("kilele.example")).await;
    for target in [
        "http://kilele.example@evil.example/",
        "http://evil.example#@kilele.example/",
        "http://evil.example?@kilele.example/",
        "http://kilele.example:80@evil.example/",
        "http://evil.example\\@kilele.example/",
        "https://kilele.example/",
        "HTTP://evil.example/",
        "ftp://kilele.example/",
        "//evil.example/",
        "/",
    ] {
        let reply = send(
            port,
            format!("GET {target} HTTP/1.1\r\nHost: kilele.example\r\n\r\n").as_bytes(),
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 403"), "{target}: {reply}");
        let e = last_event(&events).await;
        assert_eq!(e.decision, Decision::Deny, "{target}");
    }
}

#[tokio::test]
async fn absolute_form_lookalike_hosts_are_refused() {
    let (port, _) = proxy_with(only("kilele.example")).await;
    for host in [
        "kilele.example.evil.example",
        "evilkilele.example",
        "kilele.example%2eevil.example",
        "kilele%2eexample",
    ] {
        let reply = send(
            port,
            format!("GET http://{host}/ HTTP/1.1\r\n\r\n").as_bytes(),
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 403"), "{host}: {reply}");
    }
}
