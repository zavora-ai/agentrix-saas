# ADR-003: Untrusted work runs only on separate, secret-free sandbox hosts

Status: Proposed, 7 Nov 2026

## Context

The proof of concept rendered arbitrary URLs with headless Chrome on the production host, using `--no-sandbox` and inheriting the server's environment. Its fetcher did not refuse private addresses. Migration necessarily processes hostile input: compromised WordPress sites, hidden instructions to AI, redirects into internal networks.

## Decision

All fetch, render, replay, exec and build work runs as typed jobs on runner hosts in their own VPC with no secrets. Each job gets gVisor by default, a microVM on VX1 for legacy replays and high-risk exec, or a throwaway Vultr VM for the largest cases. Egress goes only through a per-host proxy that pins DNS, refuses private and metadata ranges, enforces a per-job origin allowlist and logs every request. Artifacts leave only as content-addressed blobs. Every job ends in a signed receipt.

Amended 7 Nov 2026: the proxy is custom code, and every function behind an allowed origin is attack surface ([02 §5](../02-architecture.md#5-the-sandbox-plane-replacing-the-proofs-of-concepts-in-process-chrome)). Allowlists never name a service where an attacker can hold an account, and the proxy gets a hard security review before the first release ships and before each change to it.

## Consequences

A sandbox escape reaches a host with nothing to steal and no route to the control plane. It costs one more VM in the single-node profile, and a broker and scheduler to build.
