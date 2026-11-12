//! # agentrix-egress
//!
//! The only way out of a sealed room.
//!
//! Every Agentrix sandbox job (fetching a legacy site, rendering a page in a browser, replaying
//! a compromised WordPress) runs on a network whose only route is this proxy. Vultr firewall
//! groups filter inbound traffic only, so outbound control has to be ours, and it has to hold
//! even when the page inside the room is hostile.
//!
//! The proxy allows a request only when:
//!
//! | Check | Why |
//! |---|---|
//! | The destination is on the job's origin allowlist | A migration of `kilele.example` may reach `kilele.example`, nothing else |
//! | Every DNS answer is a public unicast address ([`addr::is_public`]) | Stops SSRF into `169.254.169.254` (instance metadata), the host, the VPC and loopback |
//! | The connection goes to the address that was checked | No DNS rebinding between check and connect |
//! | Bytes and lifetime stay under the job's caps | A room cannot become a download mirror or an open relay |
//! | The connection comes from the job's own room (`Policy::peers`) | Rooms share one bridge; no room may use another job's allowlist |
//! | The request head parses strictly; `http://` is GET or HEAD, one per connection, with any Host naming the target | No request smuggling, no second site on the same address |
//! | A tunnel opens with a TLS ClientHello whose server name is the tunnel's host, without ECH | No domain fronting, no plain HTTP inside a tunnel |
//!
//! Each request, allowed or refused, becomes one [`proxy::Event`] in the job's receipt.
//!
//! ## Limits and residual risks (S2-09)
//!
//! - **Host names** are `[A-Za-z0-9.-]` only, so names with an underscore are refused. A crawler
//!   of business sites does not need them.
//! - **Chromium must start with `--disable-features=EncryptedClientHello`.** It otherwise sends
//!   a GREASE ECH extension the proxy cannot tell from real ECH, and every HTTPS tunnel is
//!   refused with that instruction as the reason.
//! - **Only the first ClientHello is checked.** After a HelloRetryRequest, a second ClientHello
//!   could name another server; RFC 8446 forbids it, but not every server enforces that.
//!   Checking handshake records until the first application-data record is planned.
//! - **The HTTP Host inside TLS is invisible** without breaking TLS. On a provider that serves
//!   several sites on one address and does not enforce that SNI and Host agree, a room could
//!   reach another site there. The per-job allowlist and the provider's own checks are the
//!   boundary; this is an accepted residual, not a pass.
//!
//! ```no_run
//! use agentrix_egress::{policy::{Origin, Policy}, proxy};
//! # async fn run() -> anyhow::Result<()> {
//! let policy = Policy::only([Origin::parse("kilele.example")?]);
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:3128").await?;
//! proxy::serve(listener, policy, "job-42", 64, proxy::stdout_sink()).await?;
//! # Ok(()) }
//! ```

pub mod addr;
pub mod policy;
pub mod proxy;
