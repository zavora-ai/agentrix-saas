# ADR-009: "Staff" becomes "WebAgents"; the lead WebAgent is the Chief WebAgent

Status: **Accepted**, 7 Nov 2026 (James): the rename ("Call them WebAgents, never staff") and the role name, "Chief WebAgent".

## Context

The product called its AI workers "staff" and their lead the "Chief of Staff". The product is the Agentrix WebAgent Platform, so its workers should carry its name. "Staff" also suggests employees and a staff hierarchy, which invites the reading that the AI is a person. Customers must always know they are talking to AI ([03-trust.md](../03-trust.md)).

## Decision

- The AI workers are **WebAgents** (one **WebAgent**), always written as one capitalised word. The product is the **Agentrix WebAgent Platform** and the company is **Zavora**. The console tab "Staff" becomes "WebAgents", and the tagline becomes "Move your website. Meet your WebAgents."
- The lead WebAgent is the **Chief WebAgent** (James). Owners read a chief as the one who leads the others, and the title carries the product's own word.
- Considered and not chosen: **Coordinator**, proposed first because it matches adk-rust's term for the role (`TeamSpec::coordinator`, the partition coordinator), but plain; **Steward**, adk-website's name for the role, because it collides with Zavora's Steward payments and receipts engine (the fleet release); **Manager**, because it collides with the human "manager" team role; keeping **Chief of Staff**, because it keeps the old word.
- Runtime principals keep the role's technical name: the Chief WebAgent is `urn:adk:agent:{site}/coordinator`, because in the runtime it is the team's coordinator. Only surfaces and documents say "Chief WebAgent".

## Consequences

README, docs, diagrams and UI concepts use the new terms; `grep -i staff` finds only the naming record in this ADR and in [05-webagents.md §2.2](../05-webagents.md#22-the-role-name-chief-webagent). Because the principal is unchanged, the title can change again without touching receipts, standing rules or stores.
