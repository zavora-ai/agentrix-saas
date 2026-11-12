//! The sandbox plane's broker client (design §3, §4.2).
//!
//! Used by `agentrix-node` to send control-signed `JobSpec`s to `agentrix-runner` on the
//! sandbox VM over the VPC and stream back `JobEvent`s and the `JobResult`. The runner binary
//! lives in `src/bin/agentrix-runner.rs`. Tasks S1-01 (runner) and S1-05 (broker client).
//!
//! Scaffold only.
