# Storage Model

## Status

This is architecture guidance for Shore's durable review/session state. It describes constraints the
first `.shore/` persistence slice should preserve, even when the implementation starts small.

## Goal

Shore should make durable state boring: write facts once, rebuild projections, and keep output,
storage, and notification side effects behind explicit seams. The storage model should avoid the
common failure modes of long-running coordination tools: hidden in-memory authority, direct delivery
before persistence, shared mutable JSON files, unbounded retries, and helper bypasses.

## Storage Authority

Shore V1 intentionally uses filesystem-backed `.shore/events/` and `.shore/artifacts/` as the
authoritative local store. This is a deliberate split between canonical immutable facts and derived
projections, not a temporary gap waiting to be replaced by a database.

**Authoritative facts.** Durable history lives in two places:

- `.shore/events/` — append-only, immutable per-fact event files. Events are independently written
  and never moved, retried in place, or rewritten on read.
- `.shore/artifacts/` — immutable or content-addressed support records, including captured
  ReviewUnit snapshots and large bodies for imported notes, native observations, interventions, and
  dispositions.

These are the only authoritative durable storage in V1. Everything else is a cache or projection.

**Rebuildable projections.** `state.json`, command-output views such as `shore.review-history` and
`shore.review-unit`, and any future read indexes are derived from durable events and artifacts.
They may be deleted and regenerated. Freshness against the current event set is verified through
`eventSetHash`, not through the projection's existence or `eventCount` alone.

**Consumer contract.** Stable automation should depend on Shore commands and named JSON documents,
not on raw storage paths. Commands and documents expose semantic IDs, content hashes, and freshness
metadata as the public surface. Event filenames, artifact paths, fan-out layout, the internal shape
of `state.json`, raw storage envelopes, and row or hunk identifier formatting are Shore-owned
storage details. They may change without a deprecation cycle unless a later design explicitly
promotes them to a stable contract.

**Deferred options.** SQLite-backed read indexes, content-address fan-out, snapshot compaction or
delta packs, store manifests, and retention policy are implementation choices Shore may add later
as derived layers. None of them are current authority, and none of them are part of the consumer
contract until a later design explicitly promotes them.

## Storage Layers

Use distinct storage concepts for distinct semantics:

```text
.shore/
  events/       immutable event log
  state.json    rebuildable projection
  artifacts/    immutable or content-addressed support records
    notes/      optional content-addressed note-body records
    snapshots/  immutable captured ReviewUnit snapshots
```

`events/` is the authoritative log. Events are immutable, independently written, and never moved to
`failed/`, retried in place, or rewritten on read.

`state.json` is a cache/projection. It must be rebuildable from durable records. If it is missing,
stale, or invalid, Shore should rebuild it rather than treating it as authority.

ReviewUnit capture should follow the same authority split:

- `review_unit_captured` events in `events/` carry durable capture facts
- a ReviewUnit is the base endpoint, target endpoint, and captured diff snapshot
- V1 captures the local Git worktree from `HEAD` to the working tree
- full captured snapshots live as Shore-owned immutable artifacts under `artifacts/snapshots/`
- `review_unit_captured` events bind to the internal snapshot artifact's canonical `contentHash`
- bounded `state.json` may summarize ReviewUnit count and current unambiguous ReviewUnit ID, but it
  is not the source of ReviewUnit identity or snapshot content

`shore review capture` returns `shore.review-capture` JSON as the command-output contract. The
command reports ReviewUnit, revision, and snapshot IDs plus the snapshot artifact content hash,
without making snapshot artifact paths a user-facing API.

`SnapshotArtifact.contentHash` is a canonical hash of the artifact body excluding the
self-referential `contentHash` field. It covers the source, endpoints, ReviewUnit identity, and
captured snapshot rows; it is not a raw JSON file checksum.

Imported review notes should follow the same split:

- immutable `review_note_imported` events in `events/` carry durable imported-note facts
- bounded `state.json` may summarize imported-note state, such as `noteCount`
- large note bodies may live in content-addressed `artifacts/notes/` records instead of expanding
  event payloads or the projection without bound

On the read path, Shore reconstructs imported notes by replaying `review_note_imported` events and
loading any optional note-body artifacts under `artifacts/notes/`. `state.json` remains a bounded
projection and is not the durable source of note content.

Native observations follow the ReviewUnit ledger model:

- immutable `review_observation_recorded` events in `events/` carry durable observation facts
- each observation targets a ReviewUnit plus an optional file or line range in that captured
  snapshot
- each observation belongs to a required track; tracks are review lanes, while actor/tool provenance
  remains in the event writer envelope
- bounded `state.json` may summarize observation state, such as `observationCount`, but it does not
  embed observation history or body content

Observations are append-only. Corrections are new `review_observation_recorded` events that name
older observations through `supersedesObservationIds`; standalone retraction is deferred.

Observation read projections use `observationId` as the logical identity. If multiple durable
events carry the same observation ID, Shore preserves those events but returns one observation row
and emits a duplicate semantic diagnostic.

Observation bodies use the same inline-or-artifact mechanics as imported notes. Small bodies remain
inline in the event payload. Larger bodies use the current `shore.note-body` envelope under
`artifacts/notes/`, keeping `state.json` bounded and avoiding unbounded event payload growth.

The direct read surface is `shore review observation list`, which replays events and can optionally
hydrate bodies. Body artifact paths, event filenames, and `state.json` paths are internal storage
details, not command-output API. Native observations also appear in the composite
`shore review unit show` projection, but they are not projected into `shore dump` or `shore show`.

Native interventions follow the same ReviewUnit ledger model:

- immutable `intervention_requested` events in `events/` carry durable request facts
- immutable `intervention_resolved` events in `events/` carry durable resolution facts
- each request targets a ReviewUnit, captured file or range, or native observation in that same
  ReviewUnit
- each request belongs to a required track; actor/tool provenance remains in the event writer
  envelope
- bounded `state.json` summarizes intervention state with `interventionCount`,
  `openInterventionCount`, and `openBlockingInterventionCount`, but it does not embed intervention
  history, resolution history, body content, or reason content

Request `reasonCode` and resolution `outcome` are intentionally separate classification axes.
Multiple different resolution events remain append-only facts; read surfaces report that
intervention as ambiguous instead of choosing a timestamp winner.

Intervention read projections use semantic IDs rather than event filenames as logical identity.
Multiple `intervention_requested` events with the same `interventionId` collapse to one request row
with a duplicate semantic diagnostic. Multiple `intervention_resolved` events with the same
`interventionResolutionId` collapse to one resolution row and do not make the intervention
ambiguous. Distinct resolution IDs remain distinct facts and can still make the intervention
ambiguous.

Intervention bodies and resolution reasons use the shared inline-or-artifact mechanics. Small text
stays inline in the event payload. Larger text uses the current `shore.note-body` envelope under
`artifacts/notes/`, keeping `state.json` bounded and avoiding unbounded event payload growth.

The direct read surfaces are `shore review intervention list` and `shore review intervention fetch`,
which replay events and can optionally hydrate bodies. Body artifact paths, reason artifact paths,
event filenames, and `state.json` paths are internal storage details, not command-output API. Native
interventions also appear in the composite `shore review unit show` projection, but they are not
projected into `shore dump` or `shore show`.

Native dispositions follow the same ReviewUnit ledger model:

- immutable `review_disposition_recorded` events in `events/` carry durable disposition facts
- each disposition targets a ReviewUnit, captured file or range, native observation, native
  intervention, or native disposition in that same ReviewUnit
- each disposition belongs to a required track; actor/tool provenance remains in the event writer
  envelope
- bounded `state.json` summarizes disposition state with `dispositionCount`, but it does not embed
  disposition history, summaries, relationship graphs, or current-disposition candidates

Disposition values are closed in V1: `accepted`, `accepted_with_follow_up`, `needs_changes`,
`needs_clarification`, `overridden`, `deferred`, `split_out`, and `superseded`.

Disposition replacement is explicit. `replacesDispositionIds` is the only V1 relationship that
removes an older disposition from the current set. Override references are metadata; they record
that one fact overrides another but do not change current/replaced status unless the older
disposition is also named in `replacesDispositionIds`.

Disposition read projections use semantic IDs rather than event filenames as logical identity.
Multiple `review_disposition_recorded` events with the same `dispositionId` collapse to one
disposition row with a duplicate semantic diagnostic. Multiple unreplaced disposition IDs remain
append-only facts; read surfaces report the current state as ambiguous instead of choosing a
timestamp winner.

Disposition summaries use the shared inline-or-artifact mechanics. Small summaries stay inline in
the event payload. Larger summaries use the current `shore.note-body` envelope under
`artifacts/notes/`, keeping `state.json` bounded and avoiding unbounded event payload growth.

The direct read surface is `shore review disposition show`, which replays events and can optionally
hydrate summaries. Summary artifact paths, event filenames, and `state.json` paths are internal
storage details, not command-output API. Native dispositions also appear in the composite
`shore review unit show` projection, but they are not projected into `shore dump` or `shore show`.

Review history is the chronological read surface over durable events:

- `shore review history` returns `shore.review-history` JSON derived from a validated scan of
  `.shore/events/`
- `eventSetHash` and `eventCount` describe the full event set read for the command, not only the
  returned entries after filters
- `historyCount` describes the filtered entry count
- entries are sorted by `occurredAt`, then `eventId`, as display chronology only
- ReviewUnit, track, and event-type filters narrow entries without changing freshness metadata or
  suppressing full-event-set diagnostics
- `--include-body` hydrates body-like text from inline payloads or `artifacts/notes/`, while the
  default output keeps large text omitted

History preserves raw append-only facts. It does not collapse duplicate semantic events, choose
current dispositions, resolve interventions, or build the full ReviewUnit row projection. Shared
state diagnostics are still included so callers can see duplicate semantic facts while inspecting
the underlying events. Raw event files, artifact paths, event filenames, and `state.json` are
storage details, not history output API.

ReviewUnit show is the composite read surface for one captured ReviewUnit:

- `shore review unit show` returns `shore.review-unit` JSON derived from a validated scan of
  `.shore/events/` plus the bound immutable snapshot artifact for the selected ReviewUnit
- `eventSetHash` and `eventCount` describe the full event set read for the command, not only the
  selected ReviewUnit's returned narrative facts
- the output includes ReviewUnit identity, filters, summary counts, current disposition, native
  observations, interventions, dispositions, imported adapter notes, projection rows, and
  diagnostics
- rows are narrative-first plus snapshot-complete: reviewed ledger material appears first, and the
  snapshot remainder still includes every captured file, metadata row, hunk header, and diff row
- track filters narrow narrative facts only; they do not mutate ReviewUnit selection, freshness
  metadata, or captured snapshot completeness
- `--include-body` hydrates body-like text from inline payloads or `artifacts/notes/`, while the
  default output keeps large text omitted

`shore.review-unit` is command-output API. Snapshot artifacts, note body artifacts, event files,
event filenames, and `state.json` remain Shore-owned storage details and are not exposed as stable
paths.

The review stream also surfaces stale and orphan notes as dedicated rows so reviewers can park the
cursor on them; the stream emits an additional synthetic file header for orphan notes when at least
one is present.

Reload is a read-side projection refresh. The durable event log remains immutable; reload re-runs
the order-independent projection against the current worktree state and lowers anchor-stale
conditions into the read surface via `reload_diagnostics`. If reload encounters a parse or ingest
error partway through, the prior projection survives because the read-side primitive never mutates
`.shore/`.

A future delivery queue is a separate subsystem. Queue concepts such as `pending/`, `failed/`,
retry counts, backoff, and circuit breakers do not belong in `.shore/events/`.

## Event Files

Every durable event must carry a non-null `idempotencyKey`. The key should be derived from canonical
event content, not generated randomly at the call site.

Use a hash of the idempotency key as the event filename:

```text
events/<sha256(idempotencyKey)>.json
```

Keep the readable idempotency key inside the event envelope. The filename is fixed-width and safe;
the event remains inspectable.

Event creation should be exclusive. If the file already exists for the same idempotency key, the
write is idempotent. If the filename exists with conflicting content, that is a corruption or
conflict error, not a merge.

Same-key retry checks should compare the canonical event payload hash, not the full event bytes.
Envelope fields such as `occurredAt` may differ across attempts while the durable fact is still the
same. A matching `payloadHash` is idempotent; a different `payloadHash` is a conflict.

Idempotency keys control write identity. Semantic IDs control logical projection identity. A caller
that repeats the same logical fact with different idempotency keys creates multiple durable events,
not a storage overwrite. Read projections collapse same-semantic-ID events to one logical row and
surface a duplicate semantic diagnostic so the raw append-only history remains inspectable.

Any hash that contributes to durable identity should use Shore's canonical JSON path, with object
keys sorted recursively before hashing. Do not rely on incidental serde_json map ordering or local
construction order for event payload hashes, revision fingerprints, snapshot fingerprints, or future
content-derived IDs.

Do not add a global sequence number until Shore has a concrete allocator that does not create a
shared mutable counter. Deterministic event ordering can start from event metadata and filenames.

## Atomic Writes

All durable writes should go through one storage helper. The helper owns:

- temp file in the same directory as the target
- deterministic temp filename prefix
- file mode suitable for local review/session data
- temp file fsync for durable writes
- atomic rename into place
- parent directory fsync for durable writes
- stale temp file sweep

Any helper that can create temp files must also participate in sweeping them. Cleanup should not be
limited to queue code. On load, Shore should remove stale temp files matching its known prefixes and
older than the configured safety threshold.

Rebuildable projections may use a non-durable write mode that skips fsync, but they still should use
the same temp/rename path to avoid partial reads.

## Bounded Projections

`state.json` must stay bounded. It should summarize current state, cursors, and active projections;
it should not grow linearly with the event log.

If a projection needs unbounded history, split it into paged or content-addressed records under
`artifacts/` and keep `state.json` as an index or summary. A large `state.json` is a design smell
because it becomes a shared mutable file, a slow health check, and a crash-recovery hazard.

Imported review-note bodies follow this rule directly: small bodies may stay inline in the durable
event payload, but oversized bodies should move to content-addressed `artifacts/notes/` records so
the authoritative event and rebuildable projection remain bounded.

## Projection Freshness

`state.json` records `eventSetHash` as derived freshness metadata for the event set used to build
the projection. `eventCount` remains a cheap count, but it does not prove that a cached projection
matches the current `.shore/events/` set.

`eventSetHash` is computed from Shore's canonical JSON hash path over sorted `(eventId,
payloadHash)` pairs. It intentionally excludes the full event JSON, event filenames, sequence
numbers, writer metadata, storage paths, and `occurredAt`. The hash describes which durable facts
the projection saw; it is not a causal ordering primitive or a raw event-file checksum.

If a cached projection's `eventSetHash` does not match a fresh scan of `.shore/events/`, the
projection is stale and should be rebuilt from the event files. The event files remain authoritative;
`state.json` is still safe to delete and regenerate. `shore review history` and
`shore review unit show` reuse this freshness primitive, and future derived-index projections should
do the same rather than inventing per-projection hashes.

## Shared Mutable Files

Authoritative facts should not live in read-modify-write shared JSON documents. Per-event files are
a deliberate defense against metadata clobbering:

- two writers can write different events without merging a shared object
- one failed event does not roll back unrelated events
- a projection can be rebuilt after partial failure
- stale projections are recoverable

Shared JSON files are acceptable only for rebuildable projections or configuration whose merge rules
are explicit and tested.

## V1 Writer Contract

V1 uses a single-writer workflow contract: one active Shore writer per `.shore/` directory at a
time. Shore does not coordinate writers with lockfiles, leases, a daemon, IPC, or filesystem
notifications yet.

Event files remain the append-only authority. They are created with exclusive file creation:
same-key and same-payload retries are idempotent, while same-key and different-payload attempts are
conflicts. Different event files can be written independently, but reducers and projections decide
whether the resulting event set is valid, ambiguous, or conflicting.

`state.json` writes are projection cache writes. If projection writers race, events remain
authoritative and the projection can be rebuilt.

Workflow startup cleanup removes only Shore temp files older than the workflow startup threshold.
Preserving fresh `.shore-write.*.tmp` files avoids clobbering an in-flight write, but it is not a
lock or lease and does not make long-running multi-process writes a supported coordination model.

## Projection Ordering

Event filenames are derived from idempotency-key hashes. Listing event files therefore does not imply
causal publication order.

Reducers should be order-independent unless the model has introduced an explicit ordering primitive
for the events they consume. A projection may collect facts and derive state at the end, but it
should not depend on "apply this event before that event" just because one filename sorts earlier.
If a future feature needs causal order, add the ordering mechanism first and test the projection
against shuffled event input.

## Storage API Shape

Keep the primitive storage API bytes-shaped first, with JSON as a convenience layer:

```text
storage::read_bytes
storage::read_bytes_if_exists
storage::write_bytes_atomic
storage::create_file_exclusive
storage::list_dir
storage::sweep_temp_files

storage::read_json
storage::write_json_atomic

event_store::write_event
event_store::read_event
event_store::list_events
event_store::event_exists
```

This keeps the lower layer useful for manifests, JSON, future binary artifacts, and exact conflict
checks. Event filename construction should live in `event_store`, not in command handlers.

The first local durable-state slice should stay synchronous and local. Do not introduce async traits
or a runtime until a remote backend, subscription API, or second storage backend forces that
decision.

## Output Boundary

CLI output is also a side effect and should have a seam.

Domain, storage, and workflow code should return values, diagnostics, or events. CLI code should
decide how to write those values to stdout and stderr. Avoid burying `println!` or `eprintln!`
inside workflow logic.

A small boundary such as `run_with_io(args, stdout: &mut dyn Write, stderr: &mut dyn Write)` is
enough. This is not a multi-channel delivery framework; it is a testability and side-effect
boundary.

Machine-readable commands should work through ordinary pipes without terminal allocation. Formatting
should be explicit for automation-oriented JSON output; do not make command semantics depend on
ambient process TTY state unless the command is inherently interactive and fails clearly without one.

## Notifications And Delivery

Notifications are hints, not authority. The durable event must land before any notification fires.
Clients that receive a notification should re-read durable state before acting.

If Shore later adds a delivery queue, every retry path must have:

- a maximum attempt count
- backoff policy
- permanent vs. transient failure classification
- a terminal failed state that removes the entry from active rotation
- target-liveness checks before resume or apply actions

The local durable-state slice should not implement this queue. Local event writes should fail loudly
rather than loop.

## Migrations And Doctor

Runtime code should read canonical storage. Legacy repair and migration belong in a future
`shore doctor` or equivalent explicit command.

Migration and repair work should commit independently. One successful fix should not be rolled back
because an unrelated later validation failed. This mirrors the event-log rule: one durable fact, one
independent commit.

## Lock Discipline

The first local event-store implementation should not need locks. If a future change introduces
locks, follow these constraints:

- keep critical sections short
- do not perform long I/O while holding a lock when it can be avoided
- use lock-acquisition timeouts
- record enough state on disk to recover after process death
- do not rely on process-exit cleanup for correctness

## Health And Status

Health checks and status commands should exercise the real path:

- load the manifest or storage root
- list `events/`
- read event envelopes through the event store
- derive fresh state
- compare or refresh the projection

A lightweight probe that bypasses event loading and state derivation can report healthy while the
real workflow is broken. The health path should be the same code path users depend on.

## Non-Goals

This document does not require:

- a daemon
- remote storage
- async storage
- a delivery queue
- filesystem locks
- global event sequence allocation
- committed `.shore/` state

The point is to keep the first storage slice small while making the safe path the easiest path to
use.
