//! End-to-end: real sockets through the proxy.

use std::sync::{Arc, Mutex};

use agentrix_egress::policy::{Origin, Policy};
use agentrix_egress::proxy::{self, Decision, Event, Sink};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A tiny upstream that answers every connection with a fixed HTTP response.
async fn upstream() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf).await;
                let _ = s
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
                    )
                    .await;
                let _ = s.shutdown().await;
            });
        }
    });
    port
}

/// Starts a proxy with `policy`; returns its port and the events it records.
async fn proxy_with(policy: Policy) -> (u16, Arc<Mutex<Vec<Event>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink: Sink = {
        let events = Arc::clone(&events);
        Arc::new(move |e| events.lock().unwrap().push(e))
    };
    tokio::spawn(proxy::serve(listener, policy, "job-test", 8, sink));
    (port, events)
}

async fn send(proxy_port: u16, request: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out).await;
    String::from_utf8_lossy(&out).into_owned()
}

async fn last_event(events: &Arc<Mutex<Vec<Event>>>) -> Event {
    // The proxy records a tunnel when it closes: at most the half-close grace (2 s) after the
    // destination finishes, so wait a little longer than that.
    for _ in 0..500 {
        if let Some(e) = events.lock().unwrap().last().cloned() {
            return e;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("no event recorded");
}

fn loopback_policy(port: u16) -> Policy {
    let mut policy = Policy::only([Origin::parse(&format!("127.0.0.1:{port}")).unwrap()]);
    policy.permit_non_public = true; // the test upstream lives on loopback
    policy
}

#[tokio::test]
async fn allowed_tunnel_reaches_the_destination_and_is_recorded() {
    let up = upstream().await;
    let (port, events) = proxy_with(loopback_policy(up)).await;
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(
        format!("CONNECT 127.0.0.1:{up} HTTP/1.1\r\nHost: 127.0.0.1:{up}\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    let mut head = [0u8; 39];
    s.read_exact(&mut head).await.unwrap();
    assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 200"));
    // A tunnel carries TLS whose server name is the tunnel's host.
    s.write_all(&client_hello("127.0.0.1")).await.unwrap();
    let mut body = Vec::new();
    s.read_to_end(&mut body).await.unwrap();
    assert!(String::from_utf8_lossy(&body).ends_with("hello"));
    let e = last_event(&events).await;
    assert_eq!(e.decision, Decision::Allow);
    assert_eq!(e.method, "CONNECT");
    assert_eq!(e.ip.as_deref(), Some("127.0.0.1"));
    assert!(e.bytes_down > 0 && e.bytes_up > 0);
}

#[tokio::test]
async fn allowed_plain_http_is_rewritten_to_origin_form() {
    let up = upstream().await;
    let (port, events) = proxy_with(loopback_policy(up)).await;
    let reply = send(
        port,
        &format!("GET http://127.0.0.1:{up}/menu HTTP/1.1\r\nProxy-Connection: keep-alive\r\n\r\n"),
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 200 OK"), "{reply}");
    assert_eq!(last_event(&events).await.decision, Decision::Allow);
}

#[tokio::test]
async fn instance_metadata_is_refused_even_if_listed() {
    let mut policy = Policy::only([Origin::parse("169.254.169.254").unwrap()]);
    policy.permit_non_public = false;
    let (port, events) = proxy_with(policy).await;
    let reply = send(
        port,
        "GET http://169.254.169.254/v1.json HTTP/1.1\r\nHost: 169.254.169.254\r\n\r\n",
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
    assert!(reply.contains("non-public"));
    let e = last_event(&events).await;
    assert_eq!(e.decision, Decision::Deny);
    assert_eq!(e.bytes_down, 0);
}

#[tokio::test]
async fn destinations_off_the_allowlist_are_refused_before_dns() {
    let (port, events) = proxy_with(Policy::only([Origin::parse("kilele.example").unwrap()])).await;
    let reply = send(port, "CONNECT evil.test:443 HTTP/1.1\r\n\r\n").await;
    assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
    let e = last_event(&events).await;
    assert_eq!(e.decision, Decision::Deny);
    assert_eq!(e.host, "evil.test");
    assert!(e.reason.unwrap().contains("allowlist"));
}

#[tokio::test]
async fn a_name_resolving_to_loopback_is_refused() {
    let (port, events) = proxy_with(Policy::only([Origin::parse("localhost").unwrap()])).await;
    let reply = send(port, "CONNECT localhost:443 HTTP/1.1\r\n\r\n").await;
    assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
    assert!(
        last_event(&events)
            .await
            .reason
            .unwrap()
            .contains("non-public")
    );
}

#[tokio::test]
async fn non_proxy_requests_and_url_credentials_are_refused() {
    let (port, _) = proxy_with(Policy::only([Origin::parse("kilele.example").unwrap()])).await;
    assert!(
        send(port, "GET /etc/passwd HTTP/1.1\r\n\r\n")
            .await
            .starts_with("HTTP/1.1 403")
    );
    assert!(
        send(port, "GET http://user:pw@kilele.example/ HTTP/1.1\r\n\r\n")
            .await
            .starts_with("HTTP/1.1 403")
    );
}

#[tokio::test]
async fn connections_from_another_room_are_refused() {
    // Rooms share one bridge, so each proxy serves only its own room's address. Here the
    // "room" is 127.0.0.2; a client on 127.0.0.1 is another room.
    let up = upstream().await;
    let mut policy = loopback_policy(up);
    policy.peers = vec!["127.0.0.2".parse().unwrap()];
    let (port, events) = proxy_with(policy).await;
    let reply = send(
        port,
        &format!("CONNECT 127.0.0.1:{up} HTTP/1.1\r\nHost: 127.0.0.1:{up}\r\n\r\n"),
    )
    .await;
    assert!(
        reply.is_empty(),
        "the connection is closed without a reply: {reply:?}"
    );
    let e = last_event(&events).await;
    assert_eq!(e.decision, Decision::Deny);
    assert!(e.reason.unwrap().contains("not this job's room"));
}

/// A minimal TLS 1.2 ClientHello naming `name`, as the first record of a tunnel.
fn client_hello(name: &str) -> Vec<u8> {
    let name = name.as_bytes();
    let mut sni = Vec::new();
    sni.extend_from_slice(&((name.len() + 3) as u16).to_be_bytes());
    sni.push(0);
    sni.extend_from_slice(&(name.len() as u16).to_be_bytes());
    sni.extend_from_slice(name);
    let mut extensions = Vec::new();
    extensions.extend_from_slice(&0u16.to_be_bytes());
    extensions.extend_from_slice(&(sni.len() as u16).to_be_bytes());
    extensions.extend_from_slice(&sni);
    let mut body = vec![0x03, 0x03];
    body.extend_from_slice(&[0u8; 32]);
    body.push(0);
    body.extend_from_slice(&[0x00, 0x02, 0x00, 0x2f]);
    body.extend_from_slice(&[0x01, 0x00]);
    body.extend_from_slice(&(extensions.len() as u16).to_be_bytes());
    body.extend_from_slice(&extensions);
    let mut handshake = vec![0x01];
    handshake.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    handshake.extend_from_slice(&body);
    let mut record = vec![0x16, 0x03, 0x01];
    record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
    record.extend_from_slice(&handshake);
    record
}

#[tokio::test]
async fn a_tunnel_carrying_plain_http_or_another_server_name_is_refused() {
    let up = upstream().await;
    let (port, events) = proxy_with(loopback_policy(up)).await;
    let mut req = format!("CONNECT 127.0.0.1:{up} HTTP/1.1\r\n\r\n").into_bytes();
    req.extend_from_slice(&client_hello("evil.example"));
    let reply = send_bytes(port, &req).await;
    assert!(!reply.contains("hello"), "{reply}");
    let e = last_event(&events).await;
    assert_eq!(e.decision, Decision::Deny);
    assert!(e.reason.unwrap().contains("server name"));
}

async fn send_bytes(proxy_port: u16, request: &[u8]) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    s.write_all(request).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), s.read_to_end(&mut out)).await;
    String::from_utf8_lossy(&out).into_owned()
}
