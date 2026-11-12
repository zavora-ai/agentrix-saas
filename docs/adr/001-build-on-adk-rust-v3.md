# ADR-001: Build on adk-rust v3 at a pinned git revision

Status: **Accepted**, 7 Nov 2026 (James)

## Context

The published adk-rust 2.3.0 crates have agents, models, tools and a basic sandbox, but not the always-on kernel (`adk-kernel`), the governed harness (`adk-harness`), the prebuilt website (`adk-website`), the site pipeline and migration framework (`adk-site`), the service drivers (`adk-drivers`) or the plain-words surfaces (`adk-surface`). All of those exist on adk-rust `main` (v3.0.0, Apache-2.0, public source, not yet on crates.io), with scenario tests that include migrating an infected WordPress site in isolation.

## Decision

Depend on adk-rust through a git dependency pinned to one `main` commit, recorded in `Cargo.lock`. Move to crates.io v3 when it publishes.

**Enhance v3 where the platform needs it, upstream first.** Improvements land as pull requests on `zavora-ai/adk-rust` from the `feat/webagent-platform` branch, and the platform pins the merged commit. During development a `[patch]` points at the local worktree `~/Developer/projects/adk-rust-wt-webagent`. Known first enhancements: a Vultr Serverless Inference provider preset in `adk-model`; a configurable container runtime (gVisor `runsc`), resource limits and a remote sandbox host for `adk-site`'s replay driver; and `--provider vultr` in the website scenarios.

## Consequences

The platform gets the kernel, migration and website substrate on day one. Upgrades are deliberate commits that bump the pin. Contributors can build without a sibling checkout. We take on tracking v3 changes until it is published, which is acceptable because the same team owns adk-rust.
