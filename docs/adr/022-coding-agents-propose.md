# ADR-022: Coding agents propose; pipelines dispose

Status: Proposed, 7 Nov 2026 (control plane, [08 §6.4](../08-control-plane.md#64-keeping-coding-agents-governed-near-production))

## Context

Zavora builds with coding agents (Zavora CLI and others). An agent that can apply, promote, hold or approve in production would turn a prompt injection in a repository into a production incident.

## Decision

- No coding agent holds production credentials, signing keys or approval rights. Operator tokens are short-lived and bound to a person who signed in with a passkey, or with MFA; Vultr keys exist only in the gateways.
- A coding agent's output is a pull request. `agentrix mcp serve` exposes read tools and proposal tools only, never apply, promote, hold or approve.
- A mutating `agentrix` call an agent makes becomes a card that a person approves on another device, with a passkey or the MFA step-up. The agent never sees or relays the code. CI never auto-approves; pipeline definitions that disable approvals are rejected.
- Signing keys for packs, themes and releases are hardware-backed and need a touch.
- Agent-written code passes the same trains, gates and automatic halts as human code ([ADR-020](020-release-trains-and-rings.md)). Run records are attached to the pull request.

## Consequences

Coding agents speed up the work without widening production's attack surface. Releasing needs people at hardware keys. The `agentrix-guard` hook profile for Zavora CLI and the zavora-cli gaps (08 §12) are prerequisites.
