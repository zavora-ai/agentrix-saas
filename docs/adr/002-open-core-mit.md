# ADR-002: The platform is MIT open source

Status: **Accepted**, 7 Nov 2026 (James): the whole platform is MIT; Zavora sells the managed service

## Context

Customers must trust a system that acts on their business, and Vultr must trust what runs on its network. lablab requires MIT-compliant entries. Zavora's positioning is founder ownership. Open-source peers (Supabase, Coolify) show that a managed service on top of an open core is a working business and fits Vultr's Marketplace.

## Decision

License this repository MIT (Copyright Zavora Technologies Ltd). Zavora's revenue comes from the managed cloud, the agency tier, premium capability packs and themes, and support. adk-rust stays Apache-2.0 as a dependency.

## Consequences

Anyone can audit what the WebAgents can do and self-host on Vultr in one click. Competitors can fork, so the moat is operating quality, the agent network, packs and brand. The agency fleet view stays in the MIT core.
