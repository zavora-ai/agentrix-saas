# ADR-010: A WebAgent is a governed kernel process with its own principal

Status: Proposed, 7 Nov 2026

## Context

adk-rust v3 offers the pieces: `adk-harness`'s `HarnessAgent` with one governed tool path and the website kits; `adk-kernel`'s durable processes, wakes and charter ladder; and `adk-website`'s prebuilt team of eight. `adk-website`'s roster is a closed enum (`AgentRole`), and its roles are broader than the platform's: its `Growth` agent publishes pages, posts to social media and runs experiments. The product's roster (Chief WebAgent, Concierge, Sales, Editor, Reputation, Cashier, Guardian, Caretaker, Mover) cuts the work more finely.

## Decision

- Each WebAgent is one or more runtime agents. Each is an `adk_harness::HarnessAgent` with its own principal `urn:adk:agent:{site}/{webagent}`, its own `JobBinding`s, playbooks and run budget. The Concierge has three desks (`concierge`, `agent-desk`, `inbox`); the others have one runtime agent each.
- The platform builds WebAgents directly on `adk-harness`'s website kits (`website_public`, `website_back_office`) and the open ladder (`JobBinding::new(job, agent, tools)`), reusing `adk-website`'s jobs, playbooks, edge, inbox, alarms, review and learning. Making `adk-website`'s roster data is upstream enhancement U1.
- A WebAgent runs as kernel processes: woken by its wake set, admitted under quotas, quiet hours and budget, checkpointed, and asleep with no lease and no model call between wakes (K-I2).
- Every tool call passes the harness's policy stack composed with `StrictestPolicy`, plus a platform layer that checks the capability token and the charter's red lines. Consequential tools are thin clients of the Effects Gateway.

## Consequences

Each WebAgent holds only its own jobs' tools, so an injected Editor cannot post to social media and an injected Sales cannot refund. Receipts name one principal per action. The platform carries a small amount of team-building code until U1 lands. See [05-webagents.md §2–3](../05-webagents.md#2-the-roster).
