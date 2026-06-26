# ADR-0018: Event-Borne Supersession Replaces the Lineage Sub-Model

**Status:** Accepted (owner-approved 2026-06-19); the §C4 HEAD-selection wording clarified + re-approved
2026-06-20 — the head-selection seed is `--revision`/thread and `--object` is a listing/grouping lens only
(a content-`Object` can span supersession threads). Landed via the substrate-reshape implementation work.
**Supersedes in-repo ADR-0005** (ReviewUnit Lineage). Sequenced **after ADR-0017** (it depends on
ADR-0017's `Revision` layer).
**Date:** 2026-06-19
**See also:** [ADR-0017](./adr-0017-eventtarget-identity-layering-and-engagement-naming.md) and
[substrate language](../substrate-language.md). **Depends on ADR-0017** (identity layering — supplies `Revision`/`Object`, the
generative move, and the advisory-generative rule this ADR builds on). Coupled set: ADR-0017 (anchor), ADR-0019
(blackboard, independent), and the one-time owner-run store migration (translates existing lineage edges to
`supersedes`). In-repo `docs/adr/`: **ADR-0005** ReviewUnit lineage (superseded here), **ADR-0014**
association/withdrawal (its §3 discriminator justifies the forward-pointer choice; its withdrawal family is
untouched), **ADR-0008** cross-peer conflict policy (the surface-both/pick-no-winner diagnostic posture
reused), **ADR-0016** content-targeted artifact removal — **landed in-repo** at
`docs/adr/adr-0016-content-targeted-artifact-removal-and-compaction.md` — the third retraction verb, kept distinct.

## Interface contract with ADR-0017 (shared seam — must read identically in both)

- **`Revision`** — the distinct logical layer ADR-0017 §A3/§A4 separates from `Object`; it folds the object
  content id plus optional git provenance and is carried in the generative move's payload. `supersedes`
  references **`Revision`** (a position), never `Object` (content).
- **Generative move** — `capture` is the generative move, performed by any actor (ADR-0017 §A5); `supersedes`
  is a field on its payload, alongside `engagement_id` (ADR-0017 §A4).
- **Advisory-generative** — a `supersedes`-carrying generative move defaults to, and stays, **Advisory**;
  it is never promotable to Operative (ADR-0017 §A5; amends ADR-0003). This ADR adds no exception.

## Context

*(Implementation status: the substrate-reshape implementation has since **retired the lineage event
families** per this ADR — `src/session/event/kind.rs` no longer carries any lineage `EventType`. The lineage
descriptions below are the **pre-retirement baseline** that motivated the decision, not current source.)*

Lineage (ADR-0005) tracks one conceptual change across multiple captured revisions. It is a declared,
git-endpoint-keyed, linear-biased, fork-**intolerant** sub-model layered over already-stored captures:

- **2 event families** — `review_unit_lineage_declared` + `review_unit_lineage_round_recorded`
  (`src/session/event/lineage.rs:9-51`).
- **3 id types** — `ReviewUnitLineageId`, `ReviewUnitLineageRoundId`, and the basis
  `ReviewUnitLineageBasisV1` (`src/model/lineage.rs:12-44`).
- **1 basis** = `{source, base}`, literally the git endpoints copied from the first attached capture
  (`src/session/workflow/lineage/attach.rs:90`) — so the evolution relationship is keyed on *git
  identity*, contradicting ADR-0017's git-optional, content-addressed `Object`.
- **1 actual supersession edge** — the optional `predecessor_review_unit_id` on each round
  (`event/lineage.rs:35`), threaded manually via `--predecessor`. Everything else is scaffolding around
  threading that one edge and deriving a single HEAD.

The fork anti-pattern is the decisive flaw: when two captures name the same predecessor (a fork), the
projection fires a `lineage_forked_successor` diagnostic and then **nulls `head_review_unit_id` the moment
`diagnostics` is non-empty** (`src/session/projection/lineage.rs:212-216`); downstream that reads as
"lineage is malformed" (`workflow/observation/target.rs:96-100` turns any diagnostic into a hard error).
Competing proposals — the legitimate "two candidate revisions" state the substrate prizes elsewhere — are
treated as corruption.

Two precedents bracket the design choice:
- **Forward-pointer in the move's payload, status-tagged (A).** Observations carry
  `supersedes_observation_ids` (`event/observation.rs:26`) and assessments `replaces_assessment_ids`
  (`event/assessment.rs:33`); the projection collects the superseded ids and **status-tags** each
  superseded item while every survivor stays active (`workflow/observation/view.rs:142-146`) — no nulling,
  no error, fork-tolerant by construction.
- **Separate ids-only terminal retraction event (B).** The withdrawal family (`event/association.rs:78-119`,
  projection set-subtraction `projection/commit_range.rs:360-381`), which ADR-0014 §3/§10 deliberately chose
  for *associations* and forbade from touching supersession.

ADR-0017 established the `Revision` layer with distinct cardinality (one `Object`, many `Revision`s over an
`Engagement`), which is what makes an inter-revision supersession edge expressible at all. This ADR is
written abstraction-down: this is the agent-work code-review journal's evolution model, not a general
graph primitive.

## Decision

### C1. A `supersedes` forward-pointer on the generative move (not a separate event)

The generative (capture/propose) move's payload carries:

```
supersedes: Vec<RevisionId>   // empty = a root revision; ≥1 = supersedes those revisions
```

It is the obs/assessment forward-pointer (`event/observation.rs:26`) lifted from intra-revision facts to
inter-revision evolution. Placement and contract:

- **In the generative payload, not the envelope `EventTarget`** — consistent with ADR-0017 (object/revision
  identity and `engagement_id` also ride the payload), and so it is covered by `payload_hash`
  (`event/mod.rs:135`), not the idempotency key. The revision id therefore stays **succession-independent**
  (it folds the content-only `Object` id + optional git provenance per ADR-0017, never the `supersedes` set):
  adding a successor never perturbs the predecessor's revision id or filename.
- **Proposer-asserted at propose time, immutable thereafter** — the same contract as the existing
  forward-pointers. Sorted + deduped before hashing (as `assessment/add.rs` already does for
  `replaces_*`) so set-equal `supersedes` sets are byte-equal and converge across peers. No new
  `sigVersion`.
- A revision may supersede several (consolidating competing branches); a later third-party correction is
  itself just another generative move (propose C, `supersedes: [B]`) — no second mechanism needed.

### C2. A fork-tolerant, status-tagging supersession projection (competing heads, never nulled)

The projection is the observation fold (`observation/view.rs:142-146`) generalized to revisions:

- `superseded: BTreeSet<RevisionId>` = the union of every revision's `supersedes`.
- A revision is a **current head** iff it is *not* in `superseded`; **superseded** iff it is — status-tagged,
  never deleted, never an error.
- **Forks are first-class:** if A is superseded by both B and C, the current-head set is `{B, C}` — surfaced
  as `competing_revisions`, not nulled. This is ADR-0008's surface-both / withhold-the-headline /
  pick-no-winner posture applied to revision evolution.
- **Cycles** → a `supersession_cycle` diagnostic that withholds a headline, affecting only the revisions in
  the cycle (there is no lineage container to null).
- **Dangling `supersedes`** (a referenced revision not yet in the store) → a `supersession_target_missing`
  diagnostic that self-heals on backfill (the ADR-0008/0016 `retraction_target_missing` precedent). Never
  reject the write.

### C3. Retire the lineage sub-model; supersede ADR-0005

Delete the two lineage event families (`ReviewUnitLineageDeclared`, `ReviewUnitLineageRoundRecorded`), the
two id types (`ReviewUnitLineageId`, `ReviewUnitLineageRoundId`), and the basis (`ReviewUnitLineageBasisV1`),
along with the `change_id` enrichment (decision-dead, though it is an output-only field in
`shore review history` — its removal is a visible output diff taken at the break).
There is no `lineage_id`, no `declared` event, no round-id: two revisions are in the same thread iff a
`supersedes` path connects them — the relationship is intrinsic to the edges, derived, not declared. This
**supersedes ADR-0005**, whose core decisions (the lineage event family, `headReviewUnitId`, the
malformed-lineage-nulls-head rule, `stale_by_newer_round`) are all replaced here.

### C4. Recover lineage's three real affordances — two as supersession projections, one separate

- **HEAD selection** (`ReviewUnitSelection::LineageHead`, `observation/target.rs:88-108`) → "target the
  current head of a revision's supersession **thread** (its connected component)." The selection **seed is a
  `Revision`, not an `Object`**: head is a per-thread property, and post-§A4 an `Object` is a content-dedup
  key that can span *multiple* threads (§A3 clone convergence — two independent reviews of identical content
  share an `object_id` across separate engagements), so an object has no single "object DAG" to take a head
  of. Within the seed revision's thread, a **fork** (≥2 un-superseded heads) **force-disambiguates** (errors,
  naming the competing revisions and asking the caller to name one) rather than picking a winner — strictly
  better than today's "malformed lineage" hard error. **`--object` is a listing/grouping lens only**
  (`revisions --object` lists every revision sharing the content, grouped by thread, each marked with its
  head status), **never a head-selector**; a content-`Object` that spans threads is *coincident content*,
  surfaced as a multi-thread listing, and is **never** `competing_revisions` (which denotes only intra-thread
  forks). (The exact CLI shape is a surface decision.)
- **`stale_by_newer_round`** → renamed **`stale_by_superseding_revision`**: a fact targeting a revision that
  is in the `superseded` set is flagged, naming *all* superseding successors (improves on today's single
  HEAD). A direct membership test against the `superseded` set; no container needed.
- **Base auto-grouping** ("everything derived from base X") → a **separate** `revisions_by_base` projection
  that buckets revisions by their *optional* base endpoint, read directly off the capture/generative
  payload. The base data was never lineage's to own — it was copied from the capture. **Supersession does
  not subsume base-grouping**: they are two orthogonal projections, and base-grouping is
  git-provenance-only (empty when an object has no git base, e.g. a markdown set).

### C5. Three distinct retraction verbs; `supersedes` references the revision, not the object

Keep all three, distinct: **supersede** (evolve — the superseded revision and its facts remain inspectable;
this ADR), **withdraw** (retract a structural edge — `associated − withdrawn`, ADR-0014), **remove** (delete
content bytes — ADR-0016). `supersedes` references a **`Revision`** (logical position), never an
`Object`/content hash (ADR-0017's separation): superseding is about position, not bytes.

## Consequences

### Accepted

- **Fork-tolerance is gained, by reusing an in-tree pattern.** Competing revisions surface as
  `competing_revisions` (the obs/assessment status-tag fold), replacing lineage's null-head-on-fork bug. This
  is the substrate's ambiguity-preservation discipline, now applied to revision evolution.
- **One mechanism instead of two.** The whole lineage sub-model (2 events, 2 ids, 1 basis, 1 declared
  container) collapses into one optional payload field plus a projection — and the git-endpoint-keyed basis,
  which contradicted ADR-0017's git-optional identity, is gone.
- **Convergence-safe and key-stable.** `supersedes` rides `payload_hash`, sorted+deduped; it never touches
  the idempotency key, so a revision's id stays succession-independent (object id + optional git provenance,
  not the `supersedes` set) and a successor never re-keys its predecessor.
- **Costs accepted:** retiring lineage is a store-format and a `shore review history` output change (the
  lineage event types and `change_id` leave the history document — a visible diff taken at the one-time
  break); the lineage CLI (`lineage attach/show`) and the inspector Lineages tab retire/reshape (a
  separate surface decision, not this one); existing lineage events are migrated by
  re-expressing each `predecessor_review_unit_id` as a `supersedes` pointer on the successor's re-emitted
  generative event (a lossless 1:1 translation handled by the one-time owner-run store migration).

### Rejected

- **A separate supersession event (B).** Rejected by ADR-0014's *own* §3 discriminator: an association is a
  *structural edge, not a competing judgement*, so withdraw-only fits it; a **revision is a competing
  judgement** ("B is the now-current proposal, A is prior"), so the forward-pointer fits. A makes
  propose-and-supersede one atomic signed move; B splits attribution and opens a window where the proposal
  and the supersession assertion can disagree across peers. B's only advantage — a third party asserting the
  supersession later, convergently — is the association use case, not this one (a later "C supersedes B" is
  just another generative move under A).
- **Keeping the single-HEAD fiction under forks.** The one thing genuinely retired. Pretending a forked DAG
  has one head is exactly the ambiguity-collapse this ADR fixes.
- **Folding base auto-grouping into supersession.** They are orthogonal; base-grouping is a separate,
  optional, git-provenance-only projection; it is not subsumed by supersession.
- **A standalone `Lineage`/graph primitive or a stored `supersedes` index.** No new id types or containers;
  the DAG is derived. A cached `revisions_by_base` index is a permissible later read-time optimization but
  must stay derived, never an authoritative declared basis (which is what made lineage git-coupled).

## Revisit Triggers

- Supersession-DAG reads become hot enough that pure derivation is too slow — add a *derived* cache, never a
  stored authoritative basis.
- A real need emerges for a *third party* to assert a supersession that the proposer did not sign (the
  association/convergent-retraction shape) — reopen the A-vs-B choice for that specific case, naming it.
- A revision genuinely needs to supersede by *content* rather than *position* (e.g. dedup-driven
  consolidation) — revisit whether `supersedes` should ever reference an `Object`.
- The advisory-generative line is pressured (a superseding proposal needs to be treated as operative) —
  route through ADR-0003's executive-policy exception (ADR-0017 §A5), never as ordinary metadata.

## Related Docs

- [Substrate language](../substrate-language.md)
- [ADR-0017](./adr-0017-eventtarget-identity-layering-and-engagement-naming.md) (the dependency / shared seam)
- In-repo `docs/adr/`: **ADR-0005** lineage (superseded), **ADR-0014** association/withdrawal
  (the §3 discriminator; withdrawal family untouched), **ADR-0008** cross-peer conflict (diagnostic posture).
- In-repo: **ADR-0016** content-targeted artifact removal
  (`docs/adr/adr-0016-content-targeted-artifact-removal-and-compaction.md`) — the third verb, kept distinct.
