//! The proxy: HTTP `CONNECT` tunnels and absolute-form `http://` requests, nothing else.
//!
//! For every request it:
//! 1. parses the head strictly (CRLF only, no control characters, no line folding, token
//!    header names), refusing anything an upstream could read differently;
//! 2. checks the destination against the job's origin allowlist;
//! 3. for `http://`, allows only GET or HEAD with no body, one request per connection, with
//!    any Host header naming the target, and sends its own Host;
//! 4. resolves DNS itself and refuses the request if **any** answer is non-public, so a
//!    hostile name cannot point a room at the metadata service or the host network;
//! 5. connects to the address it resolved (pinning it, so there is no rebinding between check
//!    and connect);
//! 6. for a tunnel, requires the first message to be a TLS ClientHello whose server name is
//!    the tunnel's host (no domain fronting, no plain HTTP, no encrypted ClientHello);
//! 7. caps bytes per direction and connection lifetime;
//! 8. reports one [`Event`] per request, allowed or refused, for the job's receipt.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use crate::addr::is_public;
use crate::policy::Policy;

/// Largest request head the proxy reads before refusing.
const MAX_HEAD: usize = 16 * 1024;
/// How long DNS plus the upstream connect may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// One request, as recorded in the job's receipt.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Event {
    /// Milliseconds since the Unix epoch when the request arrived.
    pub ts_ms: u128,
    /// The job this proxy serves.
    pub job: String,
    /// `CONNECT` or the HTTP method of an absolute-form request.
    pub method: String,
    /// Destination host as requested.
    pub host: String,
    /// Destination port.
    pub port: u16,
    /// `allow` or `deny`.
    pub decision: Decision,
    /// Why a request was refused or ended early.
    pub reason: Option<String>,
    /// The address actually connected to.
    pub ip: Option<String>,
    /// Bytes from the room to the destination.
    pub bytes_up: u64,
    /// Bytes from the destination to the room.
    pub bytes_down: u64,
    /// Duration of the request in milliseconds.
    pub ms: u128,
}

/// The outcome of one request.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    /// Connected.
    Allow,
    /// Refused before any byte reached the destination.
    Deny,
}

/// Where events go: the receipt writer, stdout, or a test collector.
pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

/// A sink that prints one JSON object per line on stdout.
pub fn stdout_sink() -> Sink {
    Arc::new(|event: Event| {
        if let Ok(line) = serde_json::to_string(&event) {
            println!("{line}");
        }
    })
}

/// Serves `listener` under `policy` for `job` until the task is dropped.
///
/// At most `max_connections` requests are handled at once; the rest wait.
pub async fn serve(
    listener: TcpListener,
    policy: Policy,
    job: impl Into<String>,
    max_connections: usize,
    sink: Sink,
) -> std::io::Result<()> {
    let policy = Arc::new(policy);
    let job: Arc<str> = Arc::from(job.into());
    let permits = Arc::new(Semaphore::new(max_connections.max(1)));
    loop {
        let (client, from) = listener.accept().await?;
        if !policy.serves(from.ip()) {
            drop(client);
            sink(Event {
                ts_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0),
                job: job.to_string(),
                method: String::new(),
                host: String::new(),
                port: 0,
                decision: Decision::Deny,
                reason: Some(format!(
                    "connection from {}, which is not this job's room",
                    from.ip()
                )),
                ip: None,
                bytes_up: 0,
                bytes_down: 0,
                ms: 0,
            });
            continue;
        }
        let permit = Arc::clone(&permits)
            .acquire_owned()
            .await
            .expect("semaphore never closes");
        let (policy, job, sink) = (Arc::clone(&policy), Arc::clone(&job), Arc::clone(&sink));
        tokio::spawn(async move {
            handle(client, &policy, &job, &sink).await;
            drop(permit);
        });
    }
}

struct Request {
    method: String,
    target: String,
    /// Kept for diagnostics; every forwarded request is sent as HTTP/1.1.
    #[allow(dead_code)]
    version: String,
    headers: Vec<(String, String)>,
    /// Bytes read past the head (the start of a tunnelled stream, or pipelined bytes).
    rest: Vec<u8>,
}

enum Target {
    Tunnel {
        host: String,
        port: u16,
    },
    Http {
        host: String,
        port: u16,
        path: String,
    },
}

async fn handle(mut client: TcpStream, policy: &Policy, job: &str, sink: &Sink) {
    let started = Instant::now();
    let mut event = Event {
        ts_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
        job: job.to_string(),
        method: String::new(),
        host: String::new(),
        port: 0,
        decision: Decision::Deny,
        reason: None,
        ip: None,
        bytes_up: 0,
        bytes_down: 0,
        ms: 0,
    };
    // Set once the client has had a reply (the tunnel's 200), after which a refusal can only
    // close the connection.
    let mut answered = false;
    let outcome = serve_one(&mut client, policy, &mut event, &mut answered).await;
    if let Err(reason) = outcome {
        if event.decision == Decision::Deny && !answered {
            let body = format!(
                "agentrix-egress refused {}:{}: {reason}\n",
                event.host, event.port
            );
            let response = format!(
                "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = client.write_all(response.as_bytes()).await;
        }
        let _ = client.shutdown().await;
        event.reason = Some(reason);
    }
    event.ms = started.elapsed().as_millis();
    sink(event);
}

async fn serve_one(
    client: &mut TcpStream,
    policy: &Policy,
    event: &mut Event,
    answered: &mut bool,
) -> Result<(), String> {
    let request = read_request(client).await?;
    event.method = request.method.clone();
    let target = parse_target(&request)?;
    let (host, port) = match &target {
        Target::Tunnel { host, port } | Target::Http { host, port, .. } => (host.clone(), *port),
    };
    event.host = host.clone();
    event.port = port;

    if !policy.allows(&host, port) {
        return Err("destination is not on this job's allowlist".into());
    }
    if matches!(target, Target::Http { .. }) {
        check_plain_request(&request, &host, port)?;
    }
    let addrs = resolve(&host, port).await?;
    if !policy.permit_non_public
        && let Some(bad) = addrs.iter().find(|a| !is_public(a.ip()))
    {
        return Err(format!(
            "{host} resolves to a non-public address ({})",
            bad.ip()
        ));
    }
    let mut upstream = connect(&addrs).await?;
    event.ip = upstream.peer_addr().ok().map(|a| a.ip().to_string());

    match target {
        Target::Tunnel { .. } => {
            client
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .map_err(|e| e.to_string())?;
            *answered = true;
            // A tunnel is for TLS to the host it names. Its first message must be a
            // ClientHello whose server name is that host: on a shared address (a CDN) the
            // server name picks the site, so a mismatch is domain fronting.
            let hello = read_client_hello(client, request.rest).await?;
            match hello.sni.as_deref() {
                Some(sni) if same_host(sni, &host) => {}
                Some(sni) => {
                    return Err(format!(
                        "the TLS server name {sni:?} is not the tunnel's host {host}"
                    ));
                }
                None => return Err("the TLS ClientHello names no server".into()),
            }
            event.decision = Decision::Allow;
            upstream
                .write_all(&hello.bytes)
                .await
                .map_err(|e| e.to_string())?;
            event.bytes_up += hello.bytes.len() as u64;
            let (up, down, timed_out) = pipe(
                client,
                upstream,
                policy.max_bytes,
                policy.max_connection_secs,
            )
            .await;
            event.bytes_up += up;
            event.bytes_down += down;
            if timed_out {
                return Err("connection lifetime cap reached".into());
            }
            if up >= policy.max_bytes || down >= policy.max_bytes {
                return Err("byte cap reached".into());
            }
        }
        Target::Http { host, path, .. } => {
            event.decision = Decision::Allow;
            let head = forward_head(&request, &host, port, &path);
            upstream
                .write_all(head.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            event.bytes_up += head.len() as u64;
            // Exactly one request per connection: nothing more from the client reaches the
            // destination, so a pipelined second request is never delivered.
            let (down, timed_out) = pipe_down(
                client,
                upstream,
                policy.max_bytes,
                policy.max_connection_secs,
            )
            .await;
            event.bytes_down += down;
            if timed_out {
                return Err("connection lifetime cap reached".into());
            }
            if down >= policy.max_bytes {
                return Err("byte cap reached".into());
            }
        }
    }
    Ok(())
}

/// Reads one request head and parses it strictly (RFC 9112): CRLF line ends only, no control
/// characters, no obsolete line folding, header names that are tokens with no whitespace
/// before the colon. Anything a lenient upstream could read differently is refused.
async fn read_request(client: &mut TcpStream) -> Result<Request, String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    let end = loop {
        if let Some(i) = find(&buf, b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            return Err("request head too large".into());
        }
        let n = tokio::time::timeout(CONNECT_TIMEOUT, client.read(&mut chunk))
            .await
            .map_err(|_| "client sent no request".to_string())?
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("client closed before a request".into());
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    check_head_bytes(&buf[..end])?;
    let head = std::str::from_utf8(&buf[..end - 4])
        .map_err(|_| "request head is not UTF-8".to_string())?;
    let mut lines = head.split("\r\n");
    let parts: Vec<&str> = lines.next().unwrap_or_default().split(' ').collect();
    let [method, target, version] = parts[..] else {
        return Err("malformed request line".into());
    };
    if method.is_empty()
        || !method.bytes().all(is_tchar)
        || target.is_empty()
        || !(version == "HTTP/1.1" || version == "HTTP/1.0")
    {
        return Err("malformed request line".into());
    }
    let mut headers = Vec::new();
    for line in lines {
        if line.starts_with([' ', '\t']) {
            return Err("obsolete line folding is refused".into());
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| "a header line has no colon".to_string())?;
        if name.is_empty() || !name.bytes().all(is_tchar) {
            return Err(format!("invalid header name {name:?}"));
        }
        headers.push((
            name.to_string(),
            value.trim_matches([' ', '\t']).to_string(),
        ));
    }
    Ok(Request {
        method: method.to_string(),
        target: target.to_string(),
        version: version.to_string(),
        headers,
        rest: buf[end..].to_vec(),
    })
}

/// Refuses a bare LF or CR and every control character but HTAB in a request head.
fn check_head_bytes(head: &[u8]) -> Result<(), String> {
    for (i, &b) in head.iter().enumerate() {
        match b {
            b'\n' if i == 0 || head[i - 1] != b'\r' => {
                return Err("a bare LF in the request head is refused".into());
            }
            b'\r' if head.get(i + 1) != Some(&b'\n') => {
                return Err("a bare CR in the request head is refused".into());
            }
            b'\t' | b'\r' | b'\n' => {}
            0..=0x1f | 0x7f => {
                return Err("a control character in the request head is refused".into());
            }
            _ => {}
        }
    }
    Ok(())
}

/// An RFC 9110 token character.
fn is_tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// The rules for an absolute-form `http://` request: a crawler only reads, so GET or HEAD,
/// no body, one request per connection, and a Host header, if sent, that names the target.
fn check_plain_request(request: &Request, host: &str, port: u16) -> Result<(), String> {
    if !(request.method == "GET" || request.method == "HEAD") {
        return Err("only GET and HEAD are proxied for http://".into());
    }
    if !request.rest.is_empty() {
        return Err("pipelined requests are refused: one request per connection".into());
    }
    let mut hosts = 0;
    for (name, value) in &request.headers {
        match name.to_ascii_lowercase().as_str() {
            "host" => {
                hosts += 1;
                if !host_header_names(value, host, port) {
                    return Err(format!("the Host header {value:?} names another site"));
                }
            }
            "transfer-encoding" => {
                return Err("request bodies are refused (Transfer-Encoding)".into());
            }
            "content-length" if value != "0" => {
                return Err("request bodies are refused (Content-Length)".into());
            }
            _ => {}
        }
    }
    if hosts > 1 {
        return Err("more than one Host header".into());
    }
    Ok(())
}

/// `host` with brackets if it is an IPv6 literal, and `:port` unless `port` is `default`.
fn authority(host: &str, port: u16, default: u16) -> String {
    let h = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    if port == default {
        h
    } else {
        format!("{h}:{port}")
    }
}

/// Whether a Host header value names `host:port`.
fn host_header_names(value: &str, host: &str, port: u16) -> bool {
    let value = value.trim_end_matches('.');
    value.eq_ignore_ascii_case(&authority(host, port, 80))
        || value.eq_ignore_ascii_case(&authority(host, port, 0))
}

/// The head sent upstream: origin-form, the proxy's own Host, the client's end-to-end headers
/// only, and `Connection: close`.
fn forward_head(request: &Request, host: &str, port: u16, path: &str) -> String {
    let mut head = format!(
        "{} {path} HTTP/1.1\r\nHost: {}\r\n",
        request.method,
        authority(host, port, 80)
    );
    for (name, value) in &request.headers {
        let lower = name.to_ascii_lowercase();
        if lower == "host"
            || lower.starts_with("proxy-")
            || matches!(
                lower.as_str(),
                "connection"
                    | "keep-alive"
                    | "te"
                    | "trailer"
                    | "upgrade"
                    | "content-length"
                    | "transfer-encoding"
            )
        {
            continue;
        }
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("Connection: close\r\n\r\n");
    head
}

fn parse_target(request: &Request) -> Result<Target, String> {
    if request.method == "CONNECT" {
        let (host, port) = split_host_port(&request.target, None)?;
        return Ok(Target::Tunnel { host, port });
    }
    let Some(rest) = request.target.strip_prefix("http://") else {
        return Err("only CONNECT tunnels and absolute http:// requests are proxied".into());
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if authority.contains('@') {
        return Err("credentials in URLs are refused".into());
    }
    let (host, port) = split_host_port(authority, Some(80))?;
    Ok(Target::Http {
        host,
        port,
        path: path.to_string(),
    })
}

fn split_host_port(authority: &str, default_port: Option<u16>) -> Result<(String, u16), String> {
    let (host, port, bracketed) = if let Some(rest) = authority.strip_prefix('[') {
        let (h, tail) = rest.split_once(']').ok_or("malformed IPv6 authority")?;
        if !(tail.is_empty() || tail.starts_with(':')) {
            return Err(format!("malformed authority {authority}"));
        }
        (h, tail.strip_prefix(':'), true)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p), false),
            None => (authority, None, false),
        }
    };
    let port = match (port, default_port) {
        (Some(p), _) => p.parse().map_err(|_| format!("bad port in {authority}"))?,
        (None, Some(d)) => d,
        (None, None) => return Err(format!("missing port in {authority}")),
    };
    let valid = if bracketed {
        host.parse::<std::net::Ipv6Addr>().is_ok()
    } else {
        !host.is_empty()
            && host.len() <= 253
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    };
    if !valid {
        return Err(format!("invalid host in {authority}"));
    }
    Ok((host.to_string(), port))
}

async fn resolve(host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let addrs: Vec<SocketAddr> =
        tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::lookup_host((host, port)))
            .await
            .map_err(|_| format!("DNS for {host} timed out"))?
            .map_err(|e| format!("DNS for {host} failed: {e}"))?
            .collect();
    if addrs.is_empty() {
        return Err(format!("{host} has no addresses"));
    }
    Ok(addrs)
}

async fn connect(addrs: &[SocketAddr]) -> Result<TcpStream, String> {
    let mut last = String::from("no address to connect to");
    for addr in addrs {
        match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(e)) => last = format!("connect to {addr} failed: {e}"),
            Err(_) => last = format!("connect to {addr} timed out"),
        }
    }
    Err(last)
}

/// How long the room may keep sending after the destination has finished.
const HALF_CLOSE_GRACE: Duration = Duration::from_secs(2);

/// Copies both directions with a byte cap each and an overall lifetime; returns
/// `(bytes up, bytes down, timed out)`.
///
/// Bytes are counted as they flow, so the counts stay exact when a cap or the lifetime cuts a
/// connection short. When the destination finishes first, the room gets
/// [`HALF_CLOSE_GRACE`] to finish sending before the tunnel closes, so a half-open client
/// cannot hold a connection (and its receipt) open indefinitely.
async fn pipe(
    client: &mut TcpStream,
    upstream: TcpStream,
    max_bytes: u64,
    max_secs: u64,
) -> (u64, u64, bool) {
    let (mut cr, mut cw) = client.split();
    let (mut ur, mut uw) = upstream.into_split();
    let (up_n, down_n) = (AtomicU64::new(0), AtomicU64::new(0));
    let up = async {
        copy_counted(&mut cr, &mut uw, max_bytes, &up_n).await;
        let _ = uw.shutdown().await;
    };
    let down = async {
        copy_counted(&mut ur, &mut cw, max_bytes, &down_n).await;
        let _ = cw.shutdown().await;
    };
    tokio::pin!(up, down);
    let lifetime = tokio::time::sleep(Duration::from_secs(max_secs));
    tokio::pin!(lifetime);
    let (mut up_done, mut down_done) = (false, false);
    let timed_out = loop {
        tokio::select! {
            () = &mut up, if !up_done => up_done = true,
            () = &mut down, if !down_done => down_done = true,
            () = &mut lifetime => break true,
        }
        if up_done && down_done {
            break false;
        }
        if down_done {
            let _ = tokio::time::timeout(HALF_CLOSE_GRACE, &mut up).await;
            break false;
        }
    };
    (
        up_n.load(Ordering::Relaxed),
        down_n.load(Ordering::Relaxed),
        timed_out,
    )
}

/// Copies `reader` to `writer` until EOF, an error, or `max` bytes, counting into `counter`.
async fn copy_counted<R, W>(reader: &mut R, writer: &mut W, max: u64, counter: &AtomicU64)
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let sent = counter.load(Ordering::Relaxed);
        if sent >= max {
            return;
        }
        let room = usize::try_from(max - sent)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let n = match reader.read(&mut buf[..room]).await {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        if writer.write_all(&buf[..n]).await.is_err() {
            return;
        }
        counter.fetch_add(n as u64, Ordering::Relaxed);
    }
}

/// Copies the destination's answer to the client, with a byte cap and a lifetime, and
/// forwards nothing from the client; returns `(bytes down, timed out)`.
async fn pipe_down(
    client: &mut TcpStream,
    upstream: TcpStream,
    max_bytes: u64,
    max_secs: u64,
) -> (u64, bool) {
    let (_client_reads, mut cw) = client.split();
    // The write half is kept, not dropped: dropping it would half-close the destination.
    let (mut ur, _upstream_writes) = upstream.into_split();
    let n = AtomicU64::new(0);
    let copy = async {
        copy_counted(&mut ur, &mut cw, max_bytes, &n).await;
        let _ = cw.shutdown().await;
    };
    let timed_out = tokio::time::timeout(Duration::from_secs(max_secs), copy)
        .await
        .is_err();
    (n.load(Ordering::Relaxed), timed_out)
}

/// Largest ClientHello the proxy reads before refusing the tunnel.
const MAX_HELLO: usize = 32 * 1024;

/// A tunnel's first TLS message, as read from the client.
struct Hello {
    /// Every byte read, to forward unchanged.
    bytes: Vec<u8>,
    /// Its server name.
    sni: Option<String>,
}

/// Reads the client's ClientHello, reassembled across TLS records, starting with `buf` (bytes
/// that arrived with the CONNECT head).
async fn read_client_hello(client: &mut TcpStream, mut buf: Vec<u8>) -> Result<Hello, String> {
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(sni) = client_hello_sni(&buf)? {
            return Ok(Hello { bytes: buf, sni });
        }
        if buf.len() > MAX_HELLO + 5 * 8 {
            return Err("the TLS ClientHello is too large".into());
        }
        let n = tokio::time::timeout(CONNECT_TIMEOUT, client.read(&mut chunk))
            .await
            .map_err(|_| "no TLS ClientHello arrived".to_string())?
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("the tunnel closed before a TLS ClientHello".into());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

const NOT_TLS: &str = "a tunnel must carry TLS, and this one did not start with a TLS handshake";

/// The server name in the ClientHello at the start of `buf`. `Ok(None)`: more bytes are
/// needed; `Ok(Some(None))`: a complete ClientHello with no server name.
fn client_hello_sni(buf: &[u8]) -> Result<Option<Option<String>>, String> {
    let mut handshake = Vec::new();
    let mut pos = 0;
    loop {
        let Some(header) = buf.get(pos..pos + 5) else {
            if buf.get(pos).is_some_and(|&t| t != 0x16) {
                return Err(NOT_TLS.into());
            }
            return Ok(None);
        };
        if header[0] != 0x16 || header[1] != 0x03 {
            return Err(NOT_TLS.into());
        }
        let len = usize::from(u16::from_be_bytes([header[3], header[4]]));
        if len == 0 || len > 16 * 1024 + 256 {
            return Err("malformed TLS record".into());
        }
        let Some(body) = buf.get(pos + 5..pos + 5 + len) else {
            return Ok(None);
        };
        handshake.extend_from_slice(body);
        pos += 5 + len;
        if handshake.len() >= 4 {
            if handshake[0] != 0x01 {
                return Err("the tunnel's first TLS message is not a ClientHello".into());
            }
            let hl = usize::from(handshake[1]) << 16
                | usize::from(handshake[2]) << 8
                | usize::from(handshake[3]);
            if hl > MAX_HELLO {
                return Err("the TLS ClientHello is too large".into());
            }
            if handshake.len() >= 4 + hl {
                return parse_hello(&handshake[4..4 + hl]).map(Some);
            }
        }
    }
}

/// A cursor over TLS's length-prefixed fields.
struct Fields<'a> {
    b: &'a [u8],
}

impl<'a> Fields<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.b.len() < n {
            return Err("truncated TLS ClientHello".into());
        }
        let (head, tail) = self.b.split_at(n);
        self.b = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<usize, String> {
        Ok(usize::from(self.take(1)?[0]))
    }

    fn u16(&mut self) -> Result<usize, String> {
        let t = self.take(2)?;
        Ok(usize::from(u16::from_be_bytes([t[0], t[1]])))
    }

    fn is_empty(&self) -> bool {
        self.b.is_empty()
    }
}

/// The server name in a ClientHello body. Encrypted ClientHello (ECH) hides the real name,
/// so it is refused.
fn parse_hello(body: &[u8]) -> Result<Option<String>, String> {
    let mut f = Fields { b: body };
    f.take(2 + 32)?; // version, random
    let n = f.u8()?;
    f.take(n)?; // session id
    let n = f.u16()?;
    f.take(n)?; // cipher suites
    let n = f.u8()?;
    f.take(n)?; // compression methods
    if f.is_empty() {
        return Ok(None);
    }
    let n = f.u16()?;
    let mut ext = Fields { b: f.take(n)? };
    let mut sni = None;
    while !ext.is_empty() {
        let kind = ext.u16()?;
        let n = ext.u16()?;
        let data = ext.take(n)?;
        match kind {
            0x0000 => {
                if sni.is_some() {
                    return Err("two server_name extensions".into());
                }
                sni = Some(parse_server_name(data)?);
            }
            0xfe0d => {
                // Chromium sends a GREASE ECH extension by default, indistinguishable from real
                // ECH; rooms start it with the flag below. Real ECH could carry an inner server
                // name for another site behind the same outer name.
                return Err(
                    "ECH is refused; start Chromium with --disable-features=EncryptedClientHello"
                        .into(),
                );
            }
            _ => {}
        }
    }
    Ok(sni)
}

fn parse_server_name(data: &[u8]) -> Result<String, String> {
    let mut f = Fields { b: data };
    let n = f.u16()?;
    let mut list = Fields { b: f.take(n)? };
    let mut name = None;
    while !list.is_empty() {
        let kind = list.u8()?;
        let n = list.u16()?;
        let value = list.take(n)?;
        if kind == 0 {
            if name.is_some() {
                return Err("two host names in server_name".into());
            }
            name = Some(value);
        }
    }
    let value = name.ok_or_else(|| "server_name has no host name".to_string())?;
    std::str::from_utf8(value)
        .ok()
        .filter(|s| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        })
        .map(str::to_string)
        .ok_or_else(|| "the TLS server name is not a host name".to_string())
}

/// Host names compared without case or a trailing dot.
fn same_host(a: &str, b: &str) -> bool {
    a.trim_end_matches('.')
        .eq_ignore_ascii_case(b.trim_end_matches('.'))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
