# ADR-020: Release trains with rings; ring 0 is Zavora's own sites

Status: Proposed, 7 Nov 2026 (control plane, [08 §3.5](../08-control-plane.md#35-rollouts-release-trains-rings-and-waves); [05 §6.2](../05-webagents.md#62-agency-fleets))

## Context

Platform binaries, adk-rust pins, packs, playbooks, themes, model routing, detectors and infrastructure all change. Every change must reach thousands of sites without breaking any, and without overriding what owners chose ([ADR-012](012-autonomy-per-job.md), [05 §6.3](../05-webagents.md#63-cross-site-learning-without-leakage)).

## Decision

- Every change ships as a signed artefact on a **release train**. A train moves through **rings**, and inside a ring through **waves** (by cell, then by share):
  - ring 0: Zavora's own sites plus synthetic canaries per archetype;
  - ring 1: owners who opted in (an agency may propose its clients; the owner opts in);
  - rings 2–4: everyone else, in growing shares, with more human approvers at each step.
- **Gates:** the scenarios, the containment gauntlet, the cross-tenant and integrity suites and the WebAgent evaluation suites with no regression on any baseline ([ADR-016](016-evaluation-gates.md)) before ring 0; canary analysis with `adk-eval`'s `ExperimentAnalyzer` afterwards. A breach halts the train automatically.
- **Rollback is by pin.** Schema migrations expand first and contract only after the whole train completes.
- **Consent:** platform releases and detector updates roll under the operator charter only when nothing an owner can see changes. Model-routing changes roll after their suites pass and are disclosed. Pack upgrades, playbook revisions and behaviour changes need each owner's decision. Theme updates publish only within the owner's standing grant. Security fixes take expedited trains with two approvers from the start.

## Consequences

Change is continuous but bounded, and every wave leaves a receipt. Zavora feels every regression first. Owners keep the last word on behaviour, so some improvements reach sites slowly.
