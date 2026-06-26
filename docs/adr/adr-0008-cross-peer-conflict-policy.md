# ADR-0008: Cross-Peer Conflict Policy

**Status:** Accepted 2026-06-11 — **REVISED 2026-06-16** per the owner's co-signature direction:
conflict **class (b) divergent-signature dissolves** into signature-set union, and signature-exclusive
`eventRecordHash` is **re-affirmed** as the precondition that makes the union well-defined. **Revision
re-approved (owner-approved 2026-06-17);** landed in-repo with the co-signature event family whose ADR-0004
amendment this revision depends on. **Correction owner-ratified
2026-06-17:** `eventRecordHash` excludes `ingest` (per-hop metadata) in addition to
`signer`/`signature`/`sourceRef` — see §"Event-Set Root And Signature Divergence" (without it the
section's own convergence claim fails); this is part of the accepted decision.
**Date:** 2026-06-10 (revised 2026-06-16)
**See also:** [ADR-0003](./adr-0003-agent-resource-claims-advisory-first.md),
[ADR-0004](./adr-0004-event-signatures.md) + its **detached co-signature amendment** (the
`## Amendment: Detached Co-Signature Event Family` section of ADR-0004),
[ADR-0005](./adr-0005-review-unit-lineage.md),
[ADR-0009](./adr-0009-resumption-binding-trust-source.md)

> **Revision note (2026-06-16).** This draft predates the owner direction that "signatures are
> attestations tied to their content-identity, and multiple signatures over one fact are
> co-signatures, not a conflict." Under that direction the four-class taxonomy becomes **three conflict
> classes plus one dissolved class**: class (b) is no longer a conflict at all — divergent signatures
> over one fact are a **convergent signature set** (ADR-0004's co-signature amendment), reconciled by
> set-union rather than reported. The dissolution is folded into the text below; the spine allocation
> rule and classes (a)/(c)/(d) are unchanged.

## Context

Shoreline stores are append-only event logs whose projections are pure functions of the stored event
set. When autonomous peers mirror and exchange events, one logical review accumulates facts written
concurrently on different stores, and the union of those facts can disagree. ADR-0003 fixed the
posture — conflicts are surfaced, never auto-picked — and ADR-0004 added that signatures do not select
a conflict winner. What neither ADR says is *which component detects each kind of cross-peer conflict
and where it surfaces*. That allocation must exist before mirrors ship, so event-sync work does not
invent conflict semantics inline.

Prior art, informed by cross-project federation research, supports the posture: every federated system
examined keeps all facts durably and names conflicts in a derived layer. Jujutsu's "divergent"
representation — keep both embodiments, label the logical fact, give each side a stable address, leave
resolution to an explicit act — is the representation borrowed here. Gerrit's multi-site design
contributes the park-and-retry treatment of events whose referents have not yet replicated. Fossil's
silent timestamp last-write-wins over an otherwise identical append-only artifact bag is the failure
mode this ADR exists to prevent: durable retention is not sufficient if the projection auto-picks
without a diagnostic.

## Decision

Adopt one allocation rule as the policy's spine:

> A conflict computable from the union of stored events alone is a shoreline projection diagnostic. A
> conflict visible only by comparing what two stores hold — or by recording what a store *refused* to
> store — is a sync-plane report owned by the relay: ephemeral, never persisted.

Shoreline owns store-content conflicts because projections are deterministic over the unioned log: once
both peers' events are in one store, detection needs no sync-plane state. The sync plane owns
sync-process facts because no single store can see them; they are re-derived on each sync and never
written into store content. The relay never "repairs" a store's copy to make mirrors agree —
overwriting either side would select a winner.

## The Conflict Classes (three conflicts, one dissolved)

The taxonomy originally listed four classes. Under the 2026-06-16 co-signature direction, **class (b)
is no longer a conflict** — it is a convergent signature set — so it is retained in the table for
continuity but marked dissolved.

| Class | Two-peer trigger | Detection point | Surfacing home |
| ----- | ---------------- | --------------- | -------------- |
| (a) Same semantic fact from two peers | Two reviewers answer the same operative input request on different peers; distinct response ids mean distinct idempotency keys, so both events store cleanly | Projection: more than one response yields `ambiguous` status; agent resumption fails closed | Shoreline projection diagnostics (shipped) |
| ~~(b) Divergent signature, same payload, across stores~~ **— DISSOLVED** | Each mirror first-stored the same event under a different attestation; sync re-offers the other copy | No longer a conflict: the two signatures are **co-signers** of one fact | **Signature-set union** — convergent metadata, not a conflict; converges via the co-signature event family (ADR-0004 amendment). No diagnostic, no report. |
| (c) Supersede/retract races (reserved) | A retraction or supersession arrives before its referent has backfilled; or two peers supersede the same unit divergently | Projection over the unioned log, via reserved diagnostics | Shoreline projection diagnostics (reserved), plus a sync-plane gap report |
| (d) Lineage-head disagreement | Both peers extend the same lineage head concurrently; sync unions the round facts | Lineage projection: forked successor, multiple heads, head withheld | Shoreline projection diagnostics (shipped) |

### Class (a) — same semantic fact from two peers

Concurrent responses to one operative input request carry distinct response ids and therefore distinct
idempotency keys; both store cleanly with no write-time collision. Detection is projection-side and
already shipped: `status_for_responses` reports `ambiguous` for more than one response, and the task
projection emits `agent_resumption_ambiguous_input_request_responses` and refuses to pick a winner,
blocking agent resumption. Because the projection is a pure function of the event set, this class is
cross-peer-correct as shipped. Re-keyed duplicates of one semantic fact collapse to one fact with a
`duplicate_semantic_*` hygiene diagnostic.

### Class (b) — divergent signature, same payload, across stores — **DISSOLVED**

**This class no longer exists as a conflict.** Under the 2026-06-16 co-signature direction (ADR-0004's
detached co-signature amendment), the inline `signer`/`signature` is **attestation #1** of an event's
co-signature set, and a divergent signature over the same fact is simply **another co-signer**. Two
mirrors that each first-stored a different attestation hold two *valid* attestations of one fact;
neither wins, and there is nothing to pick — which is the whole point of the dissolution.

What used to be reported is now **reconciled by transcription** at the ingest seam: when ingest sees an
event whose `eventId` and `payloadHash` match a stored event but whose inline attestation differs
(`EventWriteOutcome::ExistingDivergentSignature`), the store keeps its first-stored copy **and records
the incoming attestation as a co-signature event**, converging the set to both signatures. Because the
co-signature set is a grow-only set whose union is commutative, associative, and idempotent (ADR-0004
amendment D3), both mirrors converge to the same set with no winner-selection. The importer transcribes
a signature it *received and can verify* — it never mints one (ADR-0009: the relay never signs as the
reviewer).

Consequently the `divergent_signature_existing_event` diagnostic is **retired as a divergence signal**.
The only residual diagnostic at this seam fires when a newly merged co-signer is *untrusted for the
claimed actor* (ADR-0004 `untrusted_key`) — an authorization observation, never a "your stores
disagree" report. The relay's former divergent-signature *report* becomes expected **signature-set
reconciliation** (relay ADR-0002, narrowed). The earlier "enrich the diagnostic with kept/offered
signer fields" library ask is obsolete — there are no two sides to name, only a set to union.

This dissolution is what the spine allocation rule predicts once identity is signature-exclusive: a
property that was "visible only by comparing what two stores hold" becomes, under set-union semantics, a
property **computable from the union of stored events alone** (the co-signature events are events) — so
it crosses from the sync plane back to deterministic projection, and stops being a conflict in the
process.

### Class (c) — supersede/retract races (reserved)

Neither retraction tombstones nor unit supersession has landed; this ADR reserves their conflict
vocabulary so those features implement against it, mirroring how ADR-0004 reserved `eventSetRoot`.
Reserved diagnostics:

```text
retraction_target_missing
ambiguous_supersession
```

They follow the two patterns the lineage projection already ships: a dangling referent
(`lineage_round_missing_review_unit`) and a forked relationship (`lineage_forked_successor`). Both
sub-cases are computable from the unioned log, so they are shoreline projection diagnostics, with one
rule stated now: retraction and supersession effects are computed at projection time, so a fact whose
referent is missing simply has no effect yet, and the projection self-heals when the referent later
arrives through backfill — the diagnostic describes the current event set, not a persisted error. A
divergent supersession surfaces both claims and leaves the canonical choice unset. The sync plane keeps
a gap twin: only the relay can distinguish "referent pending backfill, expected to heal" from
"genuinely dangling," and that distinction stays in sync-plane reporting.

### Class (d) — lineage-head disagreement

Already correct as shipped. When sync unions concurrent rounds extending the same predecessor, the
lineage projection emits `lineage_forked_successor` and `lineage_multiple_heads` and withholds the head
— `headReviewUnitId` is unset whenever any lineage diagnostic exists — so lineage-scoped current reads
fail clearly per ADR-0005. No new vocabulary is needed. One constraint binds the sync plane: sync and
arrival order carry zero lineage meaning, and no component may report or infer a head from delivery
order.

## Vocabulary

Cross-peer conflict diagnostics extend the live families — `duplicate_semantic_*`, `lineage_*`, and
`agent_resumption_ambiguous_*` — plus a `divergent_*` family adopted from Jujutsu's vocabulary for one
logical identity with more than one durable embodiment. The representation rules for divergent facts:

- both sides remain durably stored; the event log is never mutated to record a conflict;
- the diagnostic labels the logical *fact* as divergent, not either event as wrong;
- each side is stably addressable by its `eventId`, which is content-addressed and order-free;
- resolution is an explicit recorded act, never an automatic pick.

The `divergent_*` family now covers only the genuinely-divergent classes (c) and (d). The
`divergent_signature_existing_event` diagnostic that originally seeded this family is **retired** under
the 2026-06-16 revision: divergent signatures are a convergent co-signature set, not a divergent fact,
so no `divergent_*` diagnostic describes them. A diagnostic at the signature seam fires only for an
*untrusted* merged co-signer (ADR-0004 `untrusted_key`), which is an authorization observation, not a
divergence label.

## Framing Corrections

Two earlier internal framings are corrected by this ADR:

- "Same idempotency key with a different payload hash produces an ambiguous state" is wrong. That case
  is a **hard write rejection** at `record_event_once`, which aborts the ingest batch and leaves no
  trace in the store. It is a sync-process fact: only the ingest caller can report that an offered event
  permanently conflicts with a stored event under the same key, and that report lives in the sync plane,
  never in store content.
- The ambient `ambiguous_current_review_unit` diagnostic is not the vocabulary to extend; it was retired
  by ADR-0005. Routine multi-capture stores stay quiet, and ambiguity is a selection error at
  unscoped-current boundaries.

## Event-Set Root And Signature Divergence

The shipped `eventSetHash` (`shore.event-set.v1`) hashes only `eventId` and `payloadHash`, so it is
signature-blind and converges regardless of which attestation a mirror first-stored. ADR-0004's reserved
`shore.event-set.canonical-map.v1` entry shape includes an undefined `eventRecordHash`; if that hash
covered signature bytes, mirrors holding different attestations could never agree on a signed head.

This ADR **re-affirms** (this is now the load-bearing precondition for the class-(b) dissolution):
**`eventRecordHash` is signature-exclusive *and hop-exclusive*.** It is computed over the stored event
record excluding `signer`, `signature`, `sourceRef`, **and `ingest`**.

> **Correction (2026-06-17, post-approval).** The original list excluded only
> `signer`/`signature`/`sourceRef`. It must **also exclude `ingest`** — the per-hop ingest-provenance
> stamp. `ingest_events` stamps `ingest` (with a per-hop `received_at`) *before* storage, and the store
> preserves first-stored stamp differences, so a locally-authored (unstamped) copy and an ingested
> (stamped) copy of one fact would otherwise compute *different* `eventRecordHash` — directly
> contradicting the convergence this very section claims (and breaking class-(b) transcription, which
> matches the kept local copy against the incoming stamped copy by `eventRecordHash`). Excluding
> `ingest` is what the section's own logic requires; it is grouped with `sourceRef` as hop metadata.
> Surfaced by external review of the `eventRecordHash` implementation; **ratified by
> the owner 2026-06-17.** The rule is now: `eventRecordHash` excludes **the signature and all
> per-hop/per-mirror metadata** (`signer`, `signature`, `sourceRef`, `ingest`). This is part of the
> accepted decision.

Two consequences, both required by the co-signature direction:

- **The target event's identity converges regardless of its inline attestation *or its hop stamp*.**
  Mirrors that first-stored different signatures over one fact — or that stamped it with different ingest
  provenance — compute the *same* `eventId`, `payloadHash`, and `eventRecordHash`, so they agree on the
  event-set root with no winner-selection. Signature-and-hop-exclusive identity is what makes the
  co-signature *set* well-defined; without it, "the set of attestations over event E" would not even have
  a stable key.
- **Co-signature events converge as ordinary records.** A co-signature is itself an event (ADR-0004
  amendment D2), so it carries its own `eventId`/`payloadHash`/`eventRecordHash` and is covered by
  `eventSetHash`/`eventSetRoot` like any other event. A mirror missing an attestation is missing that
  *event* and backfills it on the next sync. **Signature-set convergence is therefore subsumed by
  event-set convergence** — there is no separate signature-reconciliation channel for the M2 sync plane
  to build. The M2.3 plan confirms only that co-signature events flow through the same cursor/gap/replay
  machinery as every event.

The earlier framing — "semantically converged, signature-divergent" as a terminal, reportable sync
*condition* — is superseded: there is no terminal divergence to report, because the attestations union.
ADR-0004 remains Accepted; its detached co-signature amendment and this clause together define the names
ADR-0004 reserved (`eventRecordHash`, `eventSetRoot`, multi-signature envelopes).

## Consequences

### Accepted

- One sentence allocates every conflict class between shoreline and the sync plane.
- Classes (a) and (d) need no new machinery; their projections are already pure functions of the event
  set and therefore cross-peer-correct.
- Class (c) features implement against reserved codes and a stated self-healing rule instead of designing
  conflict semantics inline.
- The durable layer keeps all facts; conflict status lives only in derived layers — projections for
  store-content facts, ephemeral sync reports for sync-process facts.
- Signature divergence stops threatening future signed-head convergence.
- **The taxonomy shrinks to three conflict classes (2026-06-16 revision).** Class (b) dissolves:
  divergent signatures over one fact are a convergent co-signature set (ADR-0004 amendment), not a
  conflict, so there is one fewer cross-store comparison the relay must report. The signature-exclusive
  `eventRecordHash` re-affirmation is what makes the union well-defined.

### Rejected

- Picking conflict winners by timestamp, arrival order, signature trust, or authority weight.
- **Treating divergent signatures over one fact as a conflict at all** (the former class (b)) — they are
  co-signers; the set unions and converges (ADR-0004 amendment).
- Relay "repair" of a mirror's stored copy to force byte convergence.
- Persisting sync-process facts (refused writes, cross-store comparisons) into store content; a persisted
  divergence witness would re-fire on every re-sync and is revisited only under a concrete audit
  requirement.
- An ambient store-wide conflict warning, per ADR-0005's retirement of `ambiguous_current_review_unit`.
- Defining `eventRecordHash` over signature bytes **or per-hop metadata** — `signer`, `signature`,
  `sourceRef`, and `ingest` are all excluded (the 2026-06-17 correction above).
- Designing the retraction or supersession features themselves here; this ADR reserves their conflict
  vocabulary only.

## Revisit Triggers

Reopen this ADR if the retraction or supersession designs cannot implement against the reserved codes,
if per-side addressing of divergent facts needs more than `eventId` plus projection-supplied display
order, or if external consumers need an explicit version marker on the conflict-diagnostic vocabulary.
The former "audit requirement demands a persisted witness for **signature** divergence" trigger is
**removed**: under the co-signature direction, every attestation is already a durable, persisted event,
so the audit record exists by construction. (A persisted witness for the genuinely-ephemeral classes —
refused writes — remains relay ADR-0002's concern, not this ADR's.)
