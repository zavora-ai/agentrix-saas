# ADR-016: Evaluation gates promotions, playbook revisions and model routing

Status: Proposed, 7 Nov 2026

## Context

The ladder promotes a job on approval history alone (20 unchanged approvals in 30 days, no breach). A playbook revision or a model change can alter a WebAgent's behaviour without changing that history. `adk-eval` provides `Evaluator`, `ToolTrajectoryScorer`, `LlmJudge`, `BaselineStore` regressions and `ExperimentAnalyzer`; adk-rust's `examples/website` runs scenarios S1–S19 scripted and live.

## Decision

- Each WebAgent has an evaluation suite: golden conversations for the Concierge (grounded answers, honest "I don't know", refusals), tool trajectories for each back-office job, and safety and voice rubrics, with baselines in `BaselineStore`.
- The suite, scenarios S1–S19 and the containment gauntlet run on every pull request (scripted) and nightly on Vultr models (live).
- **Gates:** a promotion offer needs the ladder's evidence and a passing suite at the current playbook and model; a playbook revision (a site's own or a fleet one) must not regress any baseline; the Model Gateway changes a purpose's model only after its suites pass, then through a canary.
- Online, corrections per 100 actions, unchanged-approval rates, KPIs and guardrail breaches are tracked per WebAgent and job.

## Consequences

Autonomy rises only when the evidence and the tests agree. Model and playbook changes become releases with a pass/fail record. Running the live set costs inference, which is accounted to Zavora, not to sites. See [05-webagents.md §9.4](../05-webagents.md#94-evaluation).
