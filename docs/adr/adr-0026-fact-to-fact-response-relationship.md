# ADR-0026: Fact-to-Fact Response Relationship — Advisory Acknowledgment Without a Terminal Status

**Status:** Accepted (owner-approved 2026-07-01); implemented in-repo for #100.
**Date:** 2026-07-01
**See also:** **ADR-0018** (event-borne supersession — the proposer-asserted per-writer forward-pointer +
status-tagging fork-tolerant projection this reuses; C1/C2), **ADR-0014** (association/withdrawal — the
writer-*excluded* converging edge this decision deliberately does **not** use; §3 discriminator),
**ADR-0019** (blackboard liveness — D6 "attention, never authority": the advisory-annotate-never-gate rule),
**ADR-0008** (cross-peer conflict — surface-both / withhold-the-headline / pick-no-winner),
**ADR-0017** (identity layering — the fact-vs-request split and substrate-vocabulary-internal / surface-
domain-named rule §A4). Grounding issue: **#100** (observation acknowledge/resolve lifecycle). Source of
record: research 0023, synthesis Finding 3.

## Context

The plan-0076 substrate reshape (ADR-0017/0018/0019) shipped the *mechanism* most of the Track A lifecycle
cluster asked for — but it added **no fact-to-fact response relationship**. There is today no way for one
observation to *respond to*, *acknowledge*, or *dispose of* another; the linkage lives only as prose (a
reviewer-observation-id string) inside an author's observation body, which is exactly the complaint in #100.
The gap is confirmed and genuinely open — it is the lifecycle cluster's only genuinely-new design.

What exists today, and why each is the wrong tool for an acknowledgment:

- **The observation payload's sole relationship field is `supersedes_observation_ids: Vec<ObservationId>`**
  (`src/session/event/observation.rs:28`). `--supersedes` (`src/cli/review/observation.rs:71`) is a
  **destructive correction**: the projection collects every superseded id and status-tags each target
  `Superseded`, removing it from the *active/current* set (`src/session/workflow/observation/view.rs:60,75`
  → `:148-152`). The superseded observation is **still listed**, tagged — it is *not* dropped
  (`view.rs:148-152`); only its membership in the active set is removed. An acknowledgment must do the
  opposite on that axis: the fact it acknowledges stays `Active`. `--supersedes` "removes-from-current-set";
  an ack "notes atop and leaves standing."
- **`ObservationStatus` is binary** — `{ Active, Superseded }` (`view.rs:48-53`); there is no `Addressed`
  state, and the only status transition is the supersede fold above. So "is this follow-up handled, and by
  whom?" is unanswerable from the ledger.
- **`--related-observation` is assessment evidence, not a fact-to-fact lifecycle.** It links an observation
  *onto an assessment* (`src/session/event/assessment.rs`, `related_observation_ids`); it gives the
  observation no lifecycle and is authored by the assessor, mirroring the input-request rule that "that
  relationship is evidence, not lifecycle" (`docs/input-request-model.md:124-126`).

The substrate already draws the line #100 gropes for. An **observation is a fact** ("here is what I see,"
needs no response); an **input request** is the thing that "seeks a response with a lifecycle" — a
decision/answer/approval/clarification with `open → responded`, and it can **already target an observation**
(`InputRequestTargetSelector::observation`, `src/session/workflow/input_request/target.rs`;
`ReviewTargetRef::Observation`, `src/model/revision.rs:76-79`;
`docs/input-request-model.md:24-31,45-51,74-83`). #100's field scenario is two sub-cases straddling that line:

1. **Decision-seeking follow-up** ("fix now or track separately?") — this *is* an input request. It needs no
   new mechanism.
2. **Non-decision disposition / acknowledgment** ("noted — tracking as issue #N") — nothing is being
   *requested*, so an input request would fabricate an `open` obligation for a fact that carries none. This
   residual case is the genuinely-new gap, and it wants a lightweight, non-destructive, attribution-
   preserving back-reference.

The in-tree pattern to reuse is ADR-0018's: a proposer-asserted, **per-writer** forward-pointer folded by a
fork-tolerant, status-*surfacing* projection (`supersedes`/`replaces`; the `SupersessionView.superseded_by`
reverse map at `src/session/projection/supersession.rs:37,159`). This ADR applies that idiom to a third,
**non-destructive** relation and records the decisions that are expensive to reverse — the per-writer
attribution choice, the derived-not-stored posture, and the fact-vs-request routing. Per-surface shaping
(inspector render, exact JSON key layout, #234 graph edge styling) is left to the implementation plan and
the #234 inspector work.

## Decision

### D1. Route decision-seeking follow-ups through advisory input requests (documentation, zero new code)

A follow-up on which the author expects a **disposition or decision** is an **advisory input request**, not a
plain observation. Input requests already target an observation/range and carry the full `respond` + `status`
(`open → responded`, with `ambiguous` when responses diverge) lifecycle and attribution
(`docs/input-request-model.md:45-51,74-83`). The reviewer opens an advisory input request against the
observation/range; the author closes it with
`shore review input-request respond --outcome {dismissed|superseded|abandoned} --reason …`. This is the
**primary** answer for the decision case and ships as guidance only — no new event, id, or field. The
shoreline-reviewer / shoreline-author skills and `docs/input-request-model.md` state the rule: **reserve
plain observations for facts that need no response; use an advisory input request when you expect a
disposition.**

### D2. A minimal, non-destructive `responds_to` observation forward-pointer for the acknowledgment case

Add one optional field to the observation payload, structurally cloning `supersedes_observation_ids`:

```
// src/session/event/observation.rs — ReviewObservationRecordedPayload
responds_to_observation_ids: Vec<ObservationId>   // empty = a standalone fact; ≥1 = responds to those observations
```

Contract, identical to the existing `supersedes_observation_ids` observation forward-pointer — which
behaves differently from ADR-0018's *revision*-level `supersedes`, so ground it in the observation write
path, not the revision one:

- **Proposer-asserted at propose time, immutable thereafter.** Like every observation field it rides the
  event `payload_hash` (`src/session/event/mod.rs:135`).
- **Folded into the observation id, exactly as `supersedes` is today.** The observation id already folds
  `revisionId`, `trackId`, `writerActorId`, *and* `supersedesObservationIds`
  (`src/session/workflow/observation/add.rs:341-366`); `responds_to_observation_ids` joins that material,
  **sorted** — mirroring the existing supersedes fold, which sorts the id material (`add.rs:344-349`) while
  the payload stores the caller's list as authored (`add.rs:289`). Two observations that differ only in what
  they respond to are therefore distinct facts, and the id stays **per-writer, per-track** — which is the
  whole point (see D3). This mirrors the observation-supersede precedent exactly; it is *not* ADR-0018's
  revision-level rule (where `supersedes` is kept *out* of the revision id to keep filenames succession-
  independent — a different constraint that does not apply to fact ids).
- **It participates in the default idempotency key — no "never the idempotency key" exception.** The
  observation id *is* the default idempotency source key (`add.rs:251-254`), and the event id is
  `sha256(idempotency_key)` (`src/session/event/mod.rs:136-139`); so anything folded into the observation id
  — `supersedes` today, `responds_to` under this ADR — flows into the default idempotency key too (an
  explicit `--idempotency-key` overrides it). **Convergence for set-equal (reordered) `responds_to` inputs
  holds only at the id layer, not the event layer.** Because the payload stores the caller's list *as
  authored* (mirroring the observation `supersedes_observation_ids` fold, `add.rs:289`), two set-equal but
  reordered inputs yield the **same** observation id and idempotency key but a **different** `payload_hash` —
  `record_event_once` (`src/session/store/event_store.rs:107-140`) returns `Existing` only on an equal
  `payload_hash`, so a reordered write raises a **hard conflict** (`event conflict for idempotency key …`),
  not an idempotent collapse, exactly as the observation `supersedes_observation_ids` pointer behaves today.
  (This differs from ADR-0018's *revision* `supersedes` and the assessment `replaces_*` fold, both of which
  `sorted_unique` the **stored payload** — `capture.rs:212→245`, `assessment/add.rs:227→280` — and so *do*
  converge byte-equal; `responds_to` mirrors the observation fact-pointer family, which stores as-authored.)
  A byte-identical re-record remains an idempotent no-op. `responds_to` inherits this behavior unchanged and
  adds no new `sigVersion`.

  > **Erratum (opaque-coded signed-identity store break).** The paragraph above describes the pre-break
  > *as-authored → reorder hard-conflict* behavior. As of the signed-store break that re-mints every
  > `payload_hash`, the observation fact-pointer family (`supersedes_observation_ids` and
  > `responds_to_observation_ids`) now `sorted_unique`-normalizes its **stored payload** and feeds the same
  > normalized lists to both the observation id and the payload — mirroring the revision `supersedes` and
  > assessment `replaces_*`/`related_*` families. So a set-equal-but-reordered *or* duplicate-bearing
  > re-write now converges byte-equal (`Existing`) instead of raising a hard conflict (GH #324). D2's
  > *contract* is otherwise unchanged — `responds_to` is still proposer-asserted and folded into the
  > observation id — and this adds no new `sigVersion`; only the stored-payload normalization changed.
- **`responds_to` references an `ObservationId`** (a fact), carried in the payload — never the envelope
  `EventTarget`, never a `ReviewTargetRef`. The observation's own `target` (what it is *about*) is unchanged
  and orthogonal.

CLI: a new flag, distinct from `--supersedes`:

```bash
shore review observation add --responds-to <observation-id> [--responds-to <observation-id> …] …
```

Scope is **obs → obs, within one revision, across tracks** (see D4). It is **non-destructive**: the target
observation stays `Active`; nothing is removed from any set.

### D3. Attribution is preserved — a per-writer forward-pointer, NOT ADR-0014's converging edge

`responds_to` is a **per-writer, proposer-asserted** pointer (the `supersedes`/`replaces` family), **not**
ADR-0014's writer-/track-*excluded* converging edge. ADR-0014's converging id is for a *structural fact about
the world that any peer independently asserts and must converge to one edge* ("this revision landed as commit
X"); its litmus (ADR-0018's rejected-B rationale) is "a third party asserts the relationship later,
convergently." "B responds-to A" fails that litmus: the responding observation *is itself an authored
judgement/disposition* ("I decided to track this separately"), and two actors responding to the same
follow-up should produce **different** dispositions that must **not** collapse to one converged edge.
Accordingly the responding observation's id continues to fold `writerActorId` + `trackId` (`add.rs:341-366`),
so "addressed, **and by whom?**" is answered structurally by the `writer` / `track_id` already on each
responding observation (`ObservationView.writer`, `ObservationView.track_id`, `view.rs:33-46`). A
writer-excluded converging edge would *erase* the "by whom," which is #100's entire value.

### D4. "Addressed" is a derived, advisory `responded_by` reverse-map — never a stored status, never a gate

The observation projection computes a reverse map from the collected `responds_to` edges, mirroring
`SupersessionView.superseded_by` (`supersession.rs:37,159`):

- **Forward field on the view:** `ObservationView.responds_to: Vec<ObservationId>` — the pointer as authored,
  the peer of the existing `supersedes: Vec<ObservationId>` (`view.rs:42`).
- **Derived reverse map:** target observation → the set of responding observation ids
  (`responded_by: BTreeMap<ObservationId, BTreeSet<ObservationId>>` in the projection, surfaced per-view as
  `responded_by: Vec<ObservationId>`, skip-when-empty). The "by whom" is **not** stored on the edge; it is
  read structurally from each responding observation's own `writer` / `track_id` view fields (D3). The read
  surface reads "responded-to by {track T: obs B, …}" as advisory `derived attention state` (ADR-0019 D6).
- **The target stays `Active`.** No `ObservationStatus::Addressed` variant is added; `ObservationStatus`
  stays `{ Active, Superseded }`. A stored terminal `Addressed` would collapse ambiguity (two actors can
  respond with *different* dispositions) and behave like a head/gate — exactly the null-head mistake ADR-0018
  retired.
- **Cross-track, computed before the track filter.** The projection collects `responds_to` edges across all
  tracks of the revision, alongside the existing `superseded_ids` collection that already runs *before* the
  track filter (`view.rs:60,75` vs the track filter at `:81-87`). A reviewer-track acknowledgment of an
  author-track observation therefore surfaces. Scope is within one revision (the observation projection is
  revision-scoped, `view.rs:67-70`); cross-**revision** fact chains are the #131 (`stale_by_superseding_revision`)
  / #234 axis, out of scope here.
- **Never gates anything.** An "unaddressed" follow-up blocks nothing; an "addressed" one auto-clears
  nothing (ADR-0019 D6). Divergent responses surface **both** (ADR-0008); a dangling `responds_to` target
  self-heals on backfill (the `supersession_target_missing` precedent, `supersession.rs:12`).

### D5. Assessments are their own lane — addressing a follow-up does not touch an assessment

The `responded_by` signal **never** alters an assessment. An `accepted-with-follow-up` assessment
(`ReviewAssessment::AcceptedWithFollowUp`, `src/session/event/assessment.rs:13`) stays exactly that until
*its own author* records a replacing assessment (`--replaces`, `replaces_assessment_ids`,
`src/session/workflow/assessment/view.rs`). Assessments are their own facts on their own lane; resolving a
follow-up observation does not rewrite a signed judgement, and the assessment lane does not close a
`responds_to` edge — the converse of `docs/input-request-model.md:124-126` ("an assessment does not close an
input request"). The two lanes stay decoupled by design.

### D6. Surface vocabulary

The field/flag/annotation names are fixed as `responds_to_observation_ids` / `--responds-to` /
`responded_by`. `resolve`/`resolves`/`Addressed` are **rejected** as surface terms because they imply closure
or a gate; `responds-to` (informally "acknowledges") reads as an advisory disposition. This is a
**domain-named** surface (ADR-0017 §A4) — no substrate vocabulary leaks. The owner fixed the verb as
`responds-to` at approval (2026-07-01); a later surface rename remains cheap (it is domain-named, not a
mechanism change — see Revisit Triggers). The *mechanism* (per-writer, non-destructive, derived) is what
this ADR fixes.

## Consequences

### Accepted

- **The #100 gap closes with the substrate's own idiom.** `responds_to` is the ADR-0018 forward-pointer /
  status-*surfacing* fold applied to a non-destructive relation — fork-tolerant, convergence-safe,
  per-writer, and already the substrate's way for "one fact to point at another." No new id type, no new
  container, no new lifecycle event.
- **Attribution ("by whom") is preserved.** Because the id stays per-writer, the derived `responded_by` map
  names *which track/actor* responded — the feature's core value.
- **Two disjoint answers, each fit to its case.** Decision-seeking follow-ups reuse the landed
  input-request lifecycle (D1, zero code); pure acknowledgments get a minimal additive pointer (D2). Neither
  fabricates an obligation the other case does not have.
- **Advisory throughout.** The signal annotates, never gates (ADR-0019 D6), surfaces all dispositions
  (ADR-0008), and leaves the assessment lane untouched (D5). No invariant is weakened.
- **Costs accepted:** one optional payload field + one CLI flag + one projection reverse-map + one skip-when-
  empty view field, plus a one-line addition to the observation-id fold material (existing observations,
  which never carry `responds_to`, keep byte-identical ids — no fixture re-bless for the empty case). The
  #234 fact-level graph (not yet built) gains a third edge kind to render — and research 0023 already
  directs the #234 plan to model edges as a **tagged list** precisely so this edge slots in without a
  redesign. The derived `responded_by` is one more read-time projection to keep advisory.

### Rejected

- **A stored `ObservationStatus::Addressed` terminal state (option (a) in #100).** Accept the *relationship*
  half, reject the *stored-status* half. A terminal `Addressed` collapses two divergent dispositions into one
  and behaves like a gate/head — the ambiguity-collapse ADR-0018/0008 exist to prevent. "Addressed" must be
  derived and advisory, never a stored flip.
- **ADR-0014's writer-/track-excluded converging edge.** It would erase "by whom," which is the whole point;
  and a response is a competing judgement, not a convergent structural fact (D3). Precisely the wrong
  precedent here.
- **Reusing `--supersedes` for acknowledgments.** It is destructive on the active-set axis (removes the
  target from `current`); an ack must leave the target `Active`. Different axis, different verb.
- **Input-request routing *alone* (guidance-only, no field).** Cheaper, and it covers the decision case (D1).
  But the field-tested non-decision case ("tracking as issue #N") requests nothing, so routing alone leaves
  it as prose inside a body — the exact #100 complaint. Ship both: D1 for decisions, D2 for acknowledgments.
- **Widening `responds_to` to target assessments or input requests.** Kept obs → obs, matching #100 and
  avoiding overlap with the input-request lane. A broader target set is a possible later widening (see
  Revisit Triggers), not this decision.
- **A stored `responded_by` index or a new relationship event.** The reverse map is derived from the same
  events, like `superseded_by`. A cached index is a permissible later read-time optimization, never an
  authoritative stored basis.

## Revisit Triggers

- A concrete need emerges for `responds_to` to target an **assessment or input request** (not just an
  observation) — reopen the obs→obs scoping in D2, naming the case, and check it does not duplicate the
  input-request lane.
- A cross-**revision** fact-response chain is genuinely needed (a fact on revision R responding to a fact on
  R′) — that is the #131 / #234 revision-level axis; revisit whether `responds_to` should ever cross a
  revision boundary rather than adding it here.
- The advisory posture is pressured (someone wants `responded_by` to gate an acceptance, a merge, or an
  assessment) — refuse via ADR-0019 D6 / ADR-0017 §A5; a derived signal never becomes executive.
- `responded_by` reads become hot enough that per-read derivation is too slow — add a *derived* cache
  (mirroring the ADR-0018 revisit trigger), never a stored authoritative basis.
- The surface verb (`responds-to` vs `acknowledges`) proves confusing in dogfooding — it is a domain-named
  surface rename (ADR-0017 §A4), not a mechanism change.

## Related Docs

- Research 0023 synthesis (Finding 3, the three-relationship taxonomy).
- [ADR-0018](./adr-0018-event-borne-supersession-replaces-lineage.md) (the reused
  forward-pointer / status-fold pattern), [ADR-0014](./adr-0014-reviewunit-commit-range-lifecycle.md)
  (the converging edge deliberately not used),
  [ADR-0019](./adr-0019-blackboard-liveness-attention-without-executive-controller.md)
  (D6 advisory rule), [ADR-0008](./adr-0008-cross-peer-conflict-policy.md) (surface-both),
  [ADR-0017](./adr-0017-eventtarget-identity-layering-and-engagement-naming.md) (fact-vs-
  request split; §A4 domain-named surface).
- `docs/input-request-model.md` (the lifecycle D1 routes through).

## Amendment: `responds_to` Is Generation-Local; Cross-Generation Continuity Uses Fact Ports (2026-07-19)

ADR-0026's non-destructive `responds_to` relationship stands within one exact generation. It continues to
derive advisory `responded_by` state, preserve target observations as active, and leave assessment and
request lifecycles independent.

[ADR-0037](./adr-0037-immutable-review-generations-and-fact-continuity.md) closes the previously deferred
cross-revision case with `ReviewFactPortedV1`. A port preserves the origin generation and fact, names an
exact target generation, and records only `context_only`, `reanchored_as`, `carried_open_as`, or
`resolved_by`. Reanchoring and carrying open require separately recorded target-generation facts/requests;
ports do not mutate anchors or make the origin current.

Validation and assessment never port or cross-generation-replace. After reviewed content changes, relied-on
checks and the accepting assessment target the new generation. Missing or conflicting ports remain visible
and self-heal on backfill; no timestamp or thread-level winner is inferred.
