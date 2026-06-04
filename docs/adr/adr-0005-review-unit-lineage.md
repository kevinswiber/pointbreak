# ADR-0005: ReviewUnit Lineage

**Status:** Accepted
**Date:** 2026-06-04
**See also:** [ADR-0004](./adr-0004-event-signatures.md)

## Context

Every `shore review capture` creates an immutable ReviewUnit: one base endpoint, one target
endpoint, and one captured snapshot artifact. Re-running capture later creates another immutable
ReviewUnit. That is the right storage model, but it leaves routine reads with only a store-wide
"current ReviewUnit" question. When more than one capture exists, unscoped current selection is
ambiguous even when the captures are rounds of the same logical review.

Shoreline needs an explicit way to say "these captured ReviewUnits are successive rounds of one
review" without editing earlier captures and without deriving identity from machine-local paths.

## Decision

Add a ReviewUnit lineage contract. A lineage is an ordered thread over already-stored
`review_unit_captured` events. It links captures; it never mutates ReviewUnit payloads, snapshot
artifacts, or ReviewUnit IDs.

Lineage identity is path-free. A `lineageId` is opaque in command output and must not be derived
from `worktreeRoot`, `.git`, `.shore`, clone-local store paths, or any other machine-local
path. Change-Id optional enrichment only: a Change-Id can help display or duplicate heuristics, but
it is not required and is never the lineage identity.

The lineage event family is:

- `review_unit_lineage_declared`, which declares one lineage ID and path-free basis facts when
  available.
- `review_unit_lineage_round_recorded`, which records one captured ReviewUnit as a lineage round.

Round records expose the domain fields `lineageId`, `roundIndex`, and `headReviewUnitId` in derived
command-output documents. `headReviewUnitId` is the latest well-formed round in that lineage,
not a rewrite of any earlier ReviewUnit.

Lineage event families remain signable under ADR-0004's generic `EventToBeSigned` contract. They use
the same Dead Simple Signing Envelope (DSSE) and pre-authentication encoding rules as other signed
events; this ADR does not add a new signing payload type.

## Current Selection

A lineage-scoped current read resolves to that lineage's explicit head. A store-wide unscoped
current read still remains ambiguous when multiple captured ReviewUnits exist and the caller did not
choose a ReviewUnit or lineage.

These invariants are intentional:

- no implicit newest capture globally wins
- no always-on ambiguous-current warning for routine multi-capture reads
- exact ReviewUnit reads continue to work for old rounds
- singleton legacy stores keep automatic current selection
- `stale_by_newer_round` applies to thread-level reads when facts target an older round than the
  lineage head, not to an exact ReviewUnit read that intentionally asks for that older round

Routine list, history, exact ReviewUnit, and lineage-scoped read projections should not report an
ambient ambiguous-current diagnostic merely because the store contains multiple captures. Ambiguity
is a selection error at unscoped-current boundaries.

## Projection Contract

The reducer derives lineage views only from durable events. A round may be recorded only for a
ReviewUnit that exists in the stored `review_unit_captured` set. Timestamp-only recaptures that are
otherwise the same stored payload stay idempotent existing outcomes. Same-payload replays with a
different signer or signature are diagnostics for the unstored candidate event; they do not mint
phantom lineage rounds.

Malformed lineage facts are surfaced as projection diagnostics instead of being silently repaired.
Examples include missing captured ReviewUnits, predecessor references outside the lineage, duplicate
semantic round records, forked successors, and cycles.

## Out Of Scope

This release has no interdiff or stack DAG. It also does not add public export, relay or network
forwarding, visual stack rendering, or a stacked-work graph. Those features can consume lineage facts
later, but they are not part of the first lineage/head linkage contract.

## Consequences

### Accepted

- ReviewUnits remain immutable and independently inspectable.
- Lineage head is explicit and scoped by lineage ID.
- Path-free lineage identity keeps the contract usable across linked worktrees and future stores.
- Legacy stores with only `review_unit_captured` events remain readable.
- Routine projections can be quiet for multi-capture stores while unscoped current selection still
  fails clearly when the caller needs a single target.

### Rejected

- Rewriting earlier ReviewUnit captures to mark them stale.
- Inferring a global current ReviewUnit from newest capture time.
- Making Change-Id mandatory for lineage.
- Embedding local filesystem paths in lineage IDs, payload identity fields, or event-signing
  targets.
- Treating lineage as an interdiff renderer or stacked-work DAG.

## Revisit Triggers

Reopen this ADR if path-free lineage identity cannot represent common local review workflows, if
thread-level stale diagnostics become too noisy for routine reads, or if a future export/sync design
needs stronger signed-head guarantees than per-event `EventToBeSigned` signatures provide.
