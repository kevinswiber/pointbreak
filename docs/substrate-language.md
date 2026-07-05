# Substrate Language

## Status

Stable internal language. Shoreline's substrate framing is supported by the code-review domain and one
agent task-supervision prototype. That is enough to use this vocabulary in source docs, architecture
discussion, and code review; it is not a claim that the substrate generalizes to every possible
software-work domain. See [Substrate Thesis Summary](substrate-thesis-summary.md) for the open
generality claim and its falsification criterion.

This document names the internal coordination vocabulary. It does not authorize a separate
substrate crate, SDK, product surface, or public API. User-facing command names and output documents
should stay domain-named.

## Purpose

Shoreline is built around durable software work objects. Different actors can read, write, and interpret
facts about those objects without requiring one central controller to decide the workflow.

The substrate language keeps that architecture legible. It separates:

- substrate concepts, which describe the durable shared medium and its coordination rules;
- domain concepts, which describe the current product shape, such as revisions or task attempts;
- product language, which should stay clear to users and should not expose implementation jargon.

The important line is naming versus factoring. Naming the substrate is a vocabulary discipline.
Factoring it into a separate module or product boundary is a later architectural decision that needs
its own design and review.

## Core Model

Shoreline coordinates through these pieces:

- **Append-only event log.** Recorded facts are immutable. Corrections, supersession, retraction,
  and resolution are new facts, not edits to old facts.
- **Work objects.** Durable subjects that actors coordinate around. A revision is one work
  object. A task attempt is another.
- **Targets within work objects.** A file range, observation, or task checkpoint can be addressed
  without becoming a peer work object.
- **Actor-attributed assertions.** Events are claims by actors, not unqualified global truth.
- **Purpose-built projections.** Read views derive from the log for a specific purpose. They can
  summarize, group, flag ambiguity, and apply explicit policy.
- **Named coordination policy.** Interpretive, attention, and executive policy should live in
  locatable code and docs, not as hidden defaults spread across readers.

## Layering Vocabulary

The substrate names four nested layers. Two of them — **Journal** and **Engagement** — are internal
architecture vocabulary and never surface in commands or JSON; the other two — **Revision** and
**Object** — are permitted domain terms because the CLI already addresses them.

| Layer | What it is | Surface? |
|---|---|---|
| **Journal** | The durable container that scopes one coordinated body of work within the append-only store. | Internal only. |
| **Engagement** | The activity, typed by **one domain axis** (`Review` or `Task`). It groups an object's revisions. | Internal only. |
| **Revision** | The captured **work object** — the addressed, fact-carrying unit that observations, input requests, assessments, and validation evidence attach to, and the thing supersession operates over. The pre-reshape `ReviewUnit` folds into this layer. | `revision` is a permitted domain term. |
| **Object** | A **content-only identity** that is a sub-layer of `Revision`: a hash of the captured content alone, git-optional. Many revisions can share one object (two clones capturing identical content converge on the same object id), so it is a dedup/grouping key, **not** a peer work-object kind. | `object` is a permitted domain term (listing/grouping only). |

The verb **`shore review`** stays the surface name for the activity; "review" is the engagement type,
distinct from the captured unit, which is a **revision**. So "review" surfaces as exactly one thing,
and `revision` / `object` name the unit and its content identity.

**Single-domain-axis guard.** The domain appears structurally in two places — the `Engagement` type
(`Review` | `Task`) and the subject's `TargetRef` outer variant (`Review` | `Task`). These must never
disagree: a `Review` engagement addresses `Revision` subjects, never a `TaskAttempt`. The subject's
domain is **derived from / type-checked against** its engagement, never asserted as an independent wire
field. That is what keeps the layering from re-creating the diverged-identity trap one layer up — the
exact failure the reshape exists to fix (see the thesis summary's note on the aspirational
`WorkObjectId` claim).

## Substrate Concepts

| Term | Meaning |
|---|---|
| event | An atomic recorded fact. Authoritative as a record, not necessarily as world truth. |
| event log | The append-only stream of events. |
| projection | A derived read view over events for a specific purpose. |
| work object | A durable thing actors coordinate around, with stable identity and captured state. |
| target | A more specific address inside a work object. |
| actor | Any participant: human, coding agent, import pipeline, automation, or tool. |
| assertion | An attributed claim about a target. |
| advisory | Assertion mode meaning non-binding by default. |
| operative | Assertion mode meaning binding only under explicit projection policy. |
| stale | An action against captured state that has moved since the actor read it. |
| fingerprint | Opaque identity for a captured state. Equality is meaningful; internal structure is not. |
| supersedes | A forward pointer recorded by a new capture naming the earlier revisions it evolves past. |
| supersession DAG | The fork-tolerant succession graph built from `supersedes` pointers. A thread is a connected component; competing heads are **surfaced**, never nulled or tie-broken. |
| provenance | Source information for a fact or imported assertion. |
| evidence | Material that supports an assertion, such as test output, logs, traces, or artifacts. |
| supersession | A new event that replaces an earlier event without mutating it. |
| retraction | A new event that withdraws an earlier event without mutating it. |
| representative selection | Stable choice of one representative for duplicate semantic facts. |
| projection diagnostic | A first-class signal that a projection saw ambiguity, conflict, drift, or data loss. |
| ambiguity preservation | The discipline of carrying disagreement as explicit state instead of picking a winner. |
| derived attention state | Projection output used to guide attention, not to authorize or block writes. |

### Supersession Replaces Lineage

Earlier drafts named a `lineage` term — a predecessor relationship between related captures, declared
as its own fact with a scalar current head. That model is **retired**. Succession is now the
**supersession DAG**: a capture records `supersedes` forward pointers to the revisions it evolves past,
and the thread is the connected component those pointers form. The DAG is **fork-tolerant** — two
captures may both supersede the same predecessor, producing competing heads that the projection
**surfaces as competing**, rather than nulling the head or picking a timestamp winner. This is a
*strengthening* of the old ambiguity-preservation discipline, not a relaxation: where lineage resolved
to a single `headReviewUnitId` (and went null when malformed), supersession keeps every live head
visible. Content grouping is a separate lens: the **object** id groups revisions by identical content,
which may span threads, so it is a listing aid and never a head selector.

## Policy Vocabulary

| Term | Meaning |
|---|---|
| interpretive policy | Rules that define what events or projection values mean. |
| attention policy | Rules that decide what is surfaced, hidden, grouped, sorted, or highlighted. |
| executive policy | Rules that decide what is allowed to proceed or treated as operative. |
| coordination semantics | The complete set of rules by which actors coordinate through the substrate. |

Executive policy is allowed, but it must be explicit and locatable. A projection that answers "may
this actor continue?" is acceptable when the rule is named, testable, diagnostic-rich, and derived
from events. A scheduler, hard lease system, write gate, or hidden global current-state field is a
different architecture.

### Attention Is Not Execution

"No controller" means **no executive controller** — no scheduler, lease, write gate, or daemon-owned
workflow state. It does **not** ban attention. Attention and notification are allowed and load-bearing:
a projection may surface what needs a human's or agent's attention, and a reader may poll for liveness
changes. The split is the discipline — attention output guides where to look; it never authorizes or
blocks a write.

**Notification-independence invariant.** A notification is **never a write precondition**. Every
durable write succeeds without anyone being notified first, and every reader re-reads durable state
before acting rather than trusting a delivered hint. There is no daemon and no filesystem-watch
dependency on the write path; freshness is pull-only. The input-request model states this boundary
directly — its "no filesystem notification" / "no daemon or notification service" Non-Goals are the
seam this invariant rests on (see [input-request-model.md](input-request-model.md)). The freshness and
liveness mechanics live in the same place.

## Wire Naming: When An Event Name Is Abstract

Event names follow one rule that unifies the abstract and domain halves of the vocabulary:

> An event name uses the abstract **`WorkObject`** term **iff** it spans both work-object kinds
> (`Revision` and `TaskAttempt`) **and** that cross-domain symmetry is load-bearing. Otherwise it names
> the domain work object (`Review*` / `Task*`, including the `Revision*` association family) or its own
> concept (`InputRequest*`, `ValidationCheckRecorded`, and so on).

`WorkObjectProposed` is the **sole** event that satisfies the rule. It is the one generative move —
proposing a captured work object — and it is genuinely polymorphic over both kinds: one event collapses
what used to be a review-domain capture and a task-domain capture, carrying `supersedes`, a
write-derived `engagement_id`, and the advisory-generative default. Crucially, the domain is **not** a
separate field on the event: it rides `EventTarget.subject` (the `TargetRef` variant), the single
source of truth for which kind the subject is. A reader recovers the domain from the subject, never
from a parallel discriminator that could disagree with it.

Everything else names its domain directly: observations, assessments, and notes stay `Review*`; the
commit/ref association family is `Revision*` (`revision_commit_associated`, `revision_ref_withdrawn`,
…); input requests and validation evidence name their own concept. The surface verb is unchanged:
`shore capture` is still how an author proposes a work object, even though the internal event is
`work_object_proposed`.

**Generative moves default and stay Advisory.** The highest-stakes rule for the generative move is that
proposing a work object — like every other recorded assertion — is **advisory by default**. A proposal
becomes operative only under explicit projection policy, never by being written. This is the same
advisory-first rule that governs resource claims; see
[ADR-0003](adr/adr-0003-agent-resource-claims-advisory-first.md).

## Current Domain: Code Review

| Domain term | Substrate concept | Notes |
|---|---|---|
| revision | work object | Captured working-tree (or committed-range) state for review; the `ReviewUnit` that folded into the `Revision` layer. |
| object | content identity of a revision | The content-only hash a revision carries; groups revisions with identical content. |
| review observation | assertion | An attributed claim about a revision or target inside it. |
| review assessment | assertion | A formal review-domain call. |
| review note | imported assertion | A note imported from an external review source. |
| input request | assertion | A durable request for another actor's input, such as attention, decision, or explicit response. |
| input request response | assertion | A durable answer to an input request. |
| validation evidence | evidence | A completed check's facts attached to an exact captured revision. |

Review-domain terms remain correct in command names and user-facing JSON. Do not rename user-facing
surfaces to substrate terms just because the underlying pattern generalizes.

The domain surface is `shore review validation` and documents such as
`shore.review-validation-list`. Internally, those records are evidence supporting an assertion, but
public commands and JSON stay review-domain named.

## Current Prototype Domain: Task Supervision

| Domain term | Substrate concept | Notes |
|---|---|---|
| task attempt | work object | A bounded attempt by an agent to perform a software task. |
| checkpoint | target within a task attempt | A captured assistant-turn boundary, not a peer work object. |
| task observation | assertion | An attributed claim or captured signal about a task attempt or checkpoint. |
| resumption projection | executive-policy projection | A read view that decides whether an agent may resume under explicit rules. |

The task-supervision prototype validates that these shapes can reuse the same event-log and
projection pattern without adding scheduler, lease, or controller primitives. It does not by itself
authorize a public task command surface.

## Carry-Forward Rules

- Keep substrate vocabulary internal unless a user-facing term is clearly better than the domain
  term. Prefer `shore review show` over exposing "projection" in the command surface; keep `Journal`
  and `Engagement` out of the command surface entirely.
- Preserve ambiguity in projections. Multiple distinct facts should surface as ambiguous or
  diagnostic state, not disappear behind timestamp tie-breakers. Competing supersession heads are the
  canonical case: surface them, never null or auto-pick.
- Treat event IDs and filenames as storage addresses, not causal order. Use explicit fields for
  semantic ordering or succession (the `supersedes` pointers, not filename sort order).
- Keep advisory and operative separate. Stored assertion mode is an actor's claim; treated-as-
  operative is a projection rule.
- Keep resource claims advisory by default. See
  [ADR-0003](adr/adr-0003-agent-resource-claims-advisory-first.md).
- Compare fingerprints as opaque values. Equality is the stable contract; domain-specific parsing
  belongs outside the substrate rule.
- Add substrate primitives only when a real projection or writer needs them. Do not fill out a
  speculative platform vocabulary.

### Two Conscious Relaxations

This vocabulary keeps every original guardrail, with two deliberate refinements worth naming:

- **"Named, not factored" now permits *internal* factoring.** Naming the substrate has always been the
  discipline; the relaxation is that internal modules may be organized around these layers
  (`Revision`, `Object`, projections) without that counting as "factoring." What stays forbidden is a
  *separate* substrate crate, SDK, or product boundary — an external factoring with its own design and
  review.
- **The current-state-scalar suspicion is *strengthened*, not loosened.** The old guardrail was wary of
  a global current-state field; supersession replaces that wariness with a concrete fork-tolerant
  succession DAG that surfaces competing heads. It serves the guardrail better than a scalar head ever
  did — there is no single "current" to be wrong about.

## What This Does Not Authorize

- A separate substrate package, crate, or SDK.
- A scheduler, lease manager, daemon broker, or write-side gate.
- A global "current task" or "current actor" field for domains where ambiguity is normal.
- Productizing task supervision or any additional domain.
- Renaming existing review-domain commands simply to match the substrate vocabulary.
- Surfacing `Journal` or `Engagement` in any command, flag, or JSON document.
