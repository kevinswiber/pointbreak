# Substrate Thesis Summary

## Status

Source-facing summary. Pointbreak's substrate thesis is supported by the current code-review system and
one headless agent task-supervision prototype. Treat the framing as stable internal architecture
language, not as a universal product claim.

The full architecture still evolves through small implementation plans, code review, and tests.
This summary explains why the current code uses substrate-shaped event, projection, identity,
freshness, and advisory/operative patterns.

## Thesis

Pointbreak is a durable shared medium for software work objects. The same substrate pattern can support
more than one software-work domain without growing a central workflow controller:

- append-only event log;
- stable work-object identity addressed through one non-optional subject;
- attributed assertions with provenance;
- actor- or purpose-specific projections;
- explicit interpretive, attention, and executive policy;
- stale-state detection through captured identity and fingerprints.

The current evidence supports this thesis **for the agent-work code-review ledger**, plus a first
task-supervision prototype that reused the pattern. That is the proven scope. Generality — that the
substrate carries an *arbitrary* software-work domain — is a **named, currently-open claim with a
falsification criterion**, not a demonstrated result. Future domains should be treated as new stress
tests, not as automatic extensions.

## The Open Generality Claim

The honest reading of the evidence is **abstraction-down, not abstraction-up**. The substrate was
abstracted *down* from a working code-review ledger that earned each primitive; it was not designed
*up* from a universal coordination platform that domains then plug into. So the load-bearing claim is
narrow and testable:

- **Claim:** a new software-work domain can adopt the event-log + projection + identity + advisory
  pattern at low marginal cost — reusing the envelope, the projection discipline, and the identity and
  freshness rules — without adding a scheduler, a hard lease, a write gate, or a global current-state
  scalar.
- **Falsification (the marginal-cost bar):** the claim is **falsified** the first time a genuinely new
  domain forces a controller-shaped primitive (an executive write gate, a global current-state field, a
  daemon-owned workflow state) to fit at all. One such domain is enough to retire "general substrate"
  back to "two proven domains."

The task-supervision prototype is one data point toward the claim, not proof of it. Treat the
vocabulary as legitimate internal architecture language for the domains it already carries, and treat
each additional domain as an experiment that can fail.

## What The Prototype Exercised

The task-supervision prototype added a second domain without replacing the review-domain model:

- `Revision` (the review-domain work object) and `TaskAttempt` both address through one non-optional
  `TargetRef` subject. The pre-reshape `WorkObjectId` claim — that the two domains shared an identity
  field — was aspirational: review kept its bespoke identity until the reshape, and identity is now
  realized by the **subject** (`EventTarget.subject`), not by a separate `WorkObjectId` sibling field.
- `TargetRef` carries domain-specific target shapes without forcing one serialization layout.
- Task-domain events use the same `ShoreEvent` envelope as review-domain events.
- The adapter maps Claude Code session JSON into deterministic task intents and then into
  `ShoreEvent`s.
- Task projections are sibling read-side views, not extensions of `SessionState` or review
  history.
- The resumption decision lives in one named projection, with explicit diagnostics and fail-closed
  behavior.
- No scheduler, hard lease, write gate, or global `current_task_attempt_id` was needed.

The important result is not that task supervision is now a product surface. The result is that the
event-log and projection substrate carried a second domain cleanly enough to promote the vocabulary
to source-facing internal docs — one more data point under the open generality claim, no more.

## What The Prototype Supports — And Doesn't

Within its scope, the prototype is consistent with the substrate pattern:

| Statement | Standing |
|---|---|
| The same event-log / projection pattern serves both revisions and task attempts. | Supported within the two tested domains. |
| Humans and agents coordinate asynchronously through recorded facts, not direct calls. | Supported. |
| Stale resolutions can be detected with work-object identity and fingerprints. | Supported at the substrate-mechanism level. |
| Task state can be understood from projections without raw transcripts. | Supported within the tested fixture set. |
| Assertions stay advisory by default; operative status is policy-derived. | Supported. |
| No scheduler, hard leases, or controller-like state are required **for these two domains**. | Supported; this is the marginal-cost bar, not a universal guarantee. |
| Real agent output maps with acceptable loss. | Supported within prototype scope. |

Two qualifications matter:

- The prototype exercised a single second domain. That supports the internal language; it does not
  prove every future domain will fit, and the marginal-cost bar above is exactly what a third domain
  tests.
- Claude-session adapter events do not yet populate real code-state fingerprints. The
  fingerprint-based stale-resolution mechanism is proven by tests that populate fingerprints, while
  real imported Claude logs currently fall back to checkpoint-identity freshness when fingerprints
  are absent.

## Two Tools, Not One

The substrate is **not** the right shape for every review-shaped activity, and the docs should stop
implying that the ledger ought to absorb all of them. There are two distinct coordination tools, and
the boundary between them is a design decision, not a gap to close:

- **The lightweight review loop** — a stateless author/reviewer JSON exchange — is correct for
  **current-state, single-author prose**: a plan, a research write-up, or an ADR that one author edits
  to a fixpoint. There is one evolving document, one writer at a time, and no durable multi-actor
  history to preserve. Folding this into the substrate would add ceremony without buying anything.
- **The Pointbreak substrate** is correct for **multi-actor, durable-record review of evolving work
  objects**: code changes that multiple actors (human, agent, import) assert facts about over time,
  where supersession, attribution, freshness, and advisory/operative policy all need to be durable and
  replay-stable.

Keeping these separate is the point. The substrate earns its weight exactly where durable
multi-actor coordination is real; it should not grow to swallow the cases the JSON loop already
handles well.

## Load-Bearing Decisions

### No Hidden Controller

Pointbreak should not silently become a workflow engine. Executive policy belongs in named projections or
explicit ADRs. A projection may answer whether an actor can proceed, but the rule must be visible,
testable, and diagnostic-rich.

### No Global Current Task

Some review-domain projections have natural current-state values. Task supervision does not assume
that shape. Multiple attempts, checkpoints, or resolutions may be valid facts at the same time; the
projection should preserve ambiguity instead of introducing a scalar `current_task_attempt_id`.

### Advisory First

Recorded assertions are advisory by default. A projection can treat a fact as operative only under
explicit policy, such as a user-authored operative approval targeted at a fresh task checkpoint.

See [ADR-0003](adr/adr-0003-agent-resource-claims-advisory-first.md) for the corresponding
resource-claim decision.

### Fingerprints Are Opaque

Freshness checks compare fingerprints with equality. They should not parse domain meaning out of
the fingerprint string. If either side lacks a fingerprint, projections fall back to the relevant
identity rule.

### Domain Terms Stay At The Surface

The substrate gives contributors shared architecture language. It should not leak into user-facing
commands unless the term is genuinely clearer than the domain term.

## What This Does Not Authorize

- Productizing task supervision.
- Adding a `shore task` command family.
- Factoring a substrate crate or SDK.
- Adding hard leases, schedulers, write gates, or daemon-owned workflow state.
- Renaming review-domain code and commands outside a focused follow-up plan.
- Treating the generality claim as settled, or expanding the substrate to absorb the single-author
  review loop.

## Related Docs

- [Substrate Language](substrate-language.md)
- [ADR-0003: Agent Resource Claims Are Advisory by Default](adr/adr-0003-agent-resource-claims-advisory-first.md)
