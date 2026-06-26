# ADR-0003: Agent Resource Claims Are Advisory by Default

**Status:** Accepted
**Date:** 2026-05-19

## Context

Shoreline coordinates software work through durable facts and derived projections, not through a
central workflow controller. As agent workflows become more common, agents may need to communicate
intent such as "I am editing this file" or "I am working on this task checkpoint."

Hard leases and reservations would prevent some conflicts, but they would also introduce executive
policy with write-side force: a scheduler, lock manager, daemon broker, or first-claimer-wins rule.
That is not Shoreline's V1 architecture. Shoreline's substrate should preserve facts, surface conflicts,
and let readers apply explicit policy.

The existing writer contract already follows this posture. `.shore/data/` assumes one active Shoreline
writer per store at a time, but it does not coordinate broader multi-agent work through lockfiles,
leases, daemon brokering, IPC, or filesystem notifications.

## Decision

Agent resource claims are advisory by default.

An agent may record an intent to edit, hold, inspect, or otherwise act on a target. That fact is an
attributed assertion in the event log. It is not a lease grant, reservation token, scheduler command,
or write-side gate.

Concretely:

- Resource-claim assertions use advisory mode unless a specific projection policy treats them as
  operative.
- Projections surface conflicting claims as explicit conflict or ambiguity state.
- Readers decide how to behave from the projection they read.
- Recovery from stale or conflicting claims happens through later events: supersession, retraction,
  input request response, human review, or a projection-specific authority rule.
- Shoreline does not block event writes because a competing advisory claim exists.

## Consequences

### Accepted

- Some conflicts can happen. Shoreline records and surfaces them; it does not prevent all of them.
- Agents need reader-side discipline. An agent that ignores advisory conflict projections may still
  produce collisions.
- Human review and later corrective facts remain the recovery path for conflicts that cannot be
  resolved mechanically.
- A future projection can summarize active resource claims, stale claims, or conflicting claims
  without changing the storage authority.

### Rejected

- `LeaseGranted` / `LeaseExpired` event types for V1.
- A first-claim-wins idempotency rule for competing resource claims.
- A write-side gate that rejects events while a conflicting claim is open.
- A central scheduler, daemon-owned workflow state, or lock manager as part of the substrate.

## Revisit Triggers

Reopen this decision if one of these occurs:

- Advisory-only coordination produces unrecoverable state, not merely inconvenient cleanup.
- Supersession or human review is structurally inadequate for a real recurring conflict.
- Resource-claim volume becomes a noise floor that makes projections unreadable.
- A concrete multi-agent workflow needs scope-bounded authority that cannot be expressed through
  actor, target, assertion mode, source provenance, and projection policy.
- A remote or multi-process storage backend introduces measured write-conflict behavior that cannot
  be handled by event idempotency and replay.

If this decision is reopened, the next design should still name the executive-policy exception
directly. It should not silently introduce lock behavior as ordinary metadata.

## Related Docs

- [Substrate Language](../substrate-language.md)
- [Substrate Thesis Summary](../substrate-thesis-summary.md)

## Amendment: Advisory-First Extends to Generative Moves (2026-06-19)

**The original decision stands** — agent resource claims are advisory by default, and the substrate has no
write-side gate. This amendment **extends that scope** to a new kind of fact the substrate
re-architecture introduces, exactly the case ADR-0003's **Revisit Trigger 4** anticipated ("a concrete
multi-agent workflow needs scope-bounded authority that cannot be expressed through actor, target,
assertion mode, source provenance, and projection policy").

**Context.** The substrate re-architecture (ADR-0017) reframes the review activity as three attributed
move kinds — **generative** (propose/capture a revision), **evaluative** (observation, assessment,
validation), and **coordinative** (input request / response). `capture` becomes the generative move,
performable by *any* actor, and it carries a `supersedes` pointer (ADR-0018, the supersession-replaces-lineage
decision). A generative move is therefore a **proposal**: "actor X proposes revision R, superseding R-1."
A proposal is the most tempting fact to escalate into "do this," and that escalation is precisely the
workflow engine the substrate refuses.

**Decision (amendment).**

- A **generative move defaults to, and remains, Advisory.** It is **never promotable to Operative**. There
  is no "this proposal is approved — implement it" path in the substrate: no projection may treat a
  generative move as operative via a write-gate, scheduler, lease, or global current-state field.
- `operative` remains, as in the original ADR-0003, a **named, locatable, testable, diagnostic-rich
  projection policy** — never an intrinsic property of a move. A projection may answer "which revision is
  the current head?" (including, under a fork, "these competing heads exist" per ADR-0018), but it never
  *authorizes execution*.
- **As-built flag (for the implementing plan):** `ReviewAssessmentRecorded` currently defaults *Operative*
  (`src/session/event/mod.rs:73-78`). The generative move must default and **stay Advisory** — a different
  default from assessments; the reshape must not let the generative move inherit the operative default.
- The blackboard's attention/notification layer (ADR-0019) carries the **fact** of a generative move and its
  derived attention state ("competing heads exist"), but **never an instruction and never a single-head
  presumption**. This amendment is the write-side guarantee; ADR-0019 §D6 is the delivery-side guarantee; they
  state the same rule from two sides.

**Why this is an extension, not a re-decision.** ADR-0003 already forbids write-side force and keeps
recorded assertions advisory; it simply predates the generative move. Without this amendment, a
`supersedes`-carrying proposal a projection could mark operative would be one step from the scheduler/
write-gate ADR-0003 rejects. The amendment closes that step. The recovery path for a stale or contested
proposal is unchanged: later events — a superseding revision, a withdrawal, an input-request response, or
human review — exactly as ADR-0003 already prescribes.

**Revisit trigger (additional).** If a real multi-agent workflow genuinely needs a generative proposal to
be treated as operative (e.g. an auto-apply gate), reopen via ADR-0003's existing executive-policy
exception, **naming the executive behavior directly** and giving it a locatable, testable, diagnostic-rich
home — never introducing it as ordinary advisory metadata on the move.
