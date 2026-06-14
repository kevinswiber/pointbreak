# Storage Model

## Status

This is architecture guidance for Shoreline's durable review/session state. It describes constraints
the first `.shore/` persistence release should preserve, even when the implementation starts small.

## Goal

Shoreline should make durable state boring: write facts once, rebuild projections, and keep output,
storage, and notification side effects behind explicit seams. The storage model should avoid the
common failure modes of long-running coordination tools: hidden in-memory authority, direct delivery
before persistence, shared mutable JSON files, unbounded retries, and helper bypasses.

## Storage Authority

Shoreline V1 intentionally uses filesystem-backed `.shore/events/` and `.shore/artifacts/` as the
authoritative local store. This is a deliberate split between canonical immutable facts and derived
projections, not a temporary gap waiting to be replaced by a database.

**Authoritative facts.** Durable history lives in two places:

- `.shore/events/` — append-only, immutable per-fact event files. Events are independently written
  and never moved, retried in place, or rewritten on read.
- `.shore/artifacts/` — immutable or content-addressed support records, including captured
  ReviewUnit snapshots and large bodies for imported notes, native observations, input requests, and
  assessments.

These are the only authoritative durable storage in V1. Everything else is a cache or projection.

**Rebuildable projections.** `state.json`, command-output views such as `shore.review-history` and
`shore.review-unit`, and any future read indexes are derived from durable events and artifacts.
They may be deleted and regenerated. Freshness against the current event set is verified through
`eventSetHash`, not through the projection's existence or `eventCount` alone.

**Consumer contract.** Stable automation should depend on Shoreline commands and named JSON documents,
not on raw storage paths. Commands and documents expose semantic IDs, content hashes, and freshness
metadata as the public surface. Event filenames, artifact paths, fan-out layout, the internal shape
of `state.json`, raw storage envelopes, and row or hunk identifier formatting are Shoreline-owned
storage details. They may change without a deprecation cycle unless a later design explicitly
promotes them to a stable contract.

**Deferred options.** SQLite-backed read indexes, content-address fan-out, snapshot compaction or
delta packs, store manifests, and retention policy are implementation choices Shoreline may add later
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
stale, or invalid, Shoreline should rebuild it rather than treating it as authority.

ReviewUnit capture follows the same authority split:

- `review_unit_captured` events in `events/` carry durable capture facts
- a ReviewUnit is the base endpoint, target endpoint, and captured diff snapshot
- V1 captures the local Git worktree from `HEAD` to the working tree
- full captured snapshots live as Shoreline-owned immutable artifacts under `artifacts/snapshots/`
- `review_unit_captured` events bind to the internal snapshot artifact's canonical `contentHash`
- bounded `state.json` may summarize ReviewUnit count and current unambiguous ReviewUnit ID, but it
  is not the source of ReviewUnit identity or snapshot content

`shore review capture` returns `shore.review-capture` JSON as the command-output contract. The
command reports ReviewUnit, revision, and snapshot IDs plus the snapshot artifact content hash,
without making snapshot artifact paths a user-facing API.

ReviewUnit lineage follows the same event/projection split. A lineage links already-stored
`review_unit_captured` events through path-free lineage facts; it never edits captured ReviewUnit
payloads or snapshot artifacts. The lineage event family includes
`review_unit_lineage_declared` and `review_unit_lineage_round_recorded`. Derived read documents use
domain fields such as `lineageId`, `roundIndex`, and `headReviewUnitId` to describe the thread and
its current head. `shore review lineage show` emits the compact `shore.review-lineage` document;
`shore review lineage attach` emits `shore.review-lineage-attach`. The capture convenience
`shore review capture --lineage <id> [--predecessor <review-unit-id>]` keeps capture counts at the
top level and reports lineage attach counts, the post-attach head, and lineage diagnostics in a
nested `lineageAttach` object. Malformed lineages have no head, so `headReviewUnitId` is `null`
until the lineage projection is well formed again.

Lineage identity must not depend on worktree paths, raw `.git` layout, raw `.shore` paths, or
clone-local store paths. Change-Id optional enrichment only: it may help readers display or correlate
rounds, but it is not required and is not the lineage identity. Lineage events remain ordinary
producer facts signable by the generic `EventToBeSigned` contract from
[ADR-0004](./adr/adr-0004-event-signatures.md), including its Dead Simple Signing Envelope (DSSE)
and pre-authentication encoding rules.

Lineage introduces scoped current semantics. A lineage-scoped read resolves to the lineage's
`headReviewUnitId`; no implicit newest capture globally wins. Routine list, history, exact
ReviewUnit, and lineage-scoped projections have no always-on ambiguous-current warning for routine
multi-capture reads. Unscoped current selection still fails clearly when the caller asks for one
current ReviewUnit in a store with multiple captures. The `stale_by_newer_round` diagnostic is a
thread-level freshness fact for older lineage rounds, not an exact-ReviewUnit read error.

The first lineage release has no interdiff or stack DAG. Public export, relay/network forwarding,
visual stack rendering, and stacked-work graph semantics remain out of scope.

When the inspector lists captured ReviewUnits, it shows a derived label for each working-tree
target — the worktree's name together with the short base commit — instead of a generic
"working tree". This label is computed at read time from the capture's existing endpoint data; the
captured record itself is unchanged, and the full worktree path is not shown. Opening the complete
detail for a unit captured in a different worktree is future work.

`SnapshotArtifact.contentHash` is a canonical hash of the artifact body excluding the
self-referential `contentHash` field. Under V1 it covers the source, endpoints, ReviewUnit
identity, and the **full captured row inventory** — every `DiffFile`, every `FileMetadataRow`,
every `ReviewHunk`, and every `DiffRow`. The hash is not a raw JSON file checksum, and its scope
includes data that a hypothetical V2 might elide. Any future elision plan must bump
`SNAPSHOT_ARTIFACT_VERSION` or introduce a separate `contentHashScope` field so consumers can
tell which scope produced a given hash; see
[ADR-0002](./adr/adr-0002-large-snapshot-artifact-policy.md).

Imported review notes should follow the same split:

- immutable `review_note_imported` events in `events/` carry durable imported-note facts
- bounded `state.json` may summarize imported-note state, such as `noteCount`
- bodies under or equal to `BODY_INLINE_LIMIT` (4096 bytes today) stay inline in the event payload;
  bodies above the threshold are externalized to `artifacts/notes/<sha256(body)>.json` with the
  `shore.note-body` envelope (schema `shore.note-body`, version `1`)

On the read path, Shoreline reconstructs imported notes by replaying `review_note_imported` events and
loading any optional note-body artifacts under `artifacts/notes/`. `state.json` remains a bounded
projection and is not the durable source of note content.

Native observations follow the ReviewUnit ledger model:

- immutable `review_observation_recorded` events in `events/` carry durable observation facts
- each observation targets a ReviewUnit plus an optional file or line range in that captured
  snapshot
- each observation belongs to a required track; tracks are review lanes, while actor/producer provenance
  remains in the event writer envelope
- bounded `state.json` may summarize observation state, such as `observationCount`, but it does not
  embed observation history or body content

Observations are append-only. Corrections are new `review_observation_recorded` events that name
older observations through `supersedesObservationIds`; standalone retraction is deferred.

Observation read projections use `observationId` as the logical identity. If multiple durable
events carry the same observation ID, Shoreline preserves those events but returns one observation row
and emits a duplicate semantic diagnostic.

Observation bodies use the same inline-or-artifact mechanics as imported notes. Bodies under or
equal to `BODY_INLINE_LIMIT` (4096 bytes today) stay inline in the event payload; bodies above the
threshold are externalized to `artifacts/notes/<sha256(body)>.json` with the `shore.note-body`
envelope (schema `shore.note-body`, version `1`), keeping `state.json` bounded and avoiding
unbounded event payload growth.

The direct read surface is `shore review observation list`, which replays events and can optionally
hydrate bodies. Body artifact paths, event filenames, and `state.json` paths are internal storage
details, not command-output API. Native observations also appear in the composite
`shore review unit show` projection, but they are not projected into `shore dump` or `shore show`.

Native input requests follow the same ReviewUnit ledger model:

- immutable `input_request_opened` events in `events/` carry durable request facts
- immutable `input_request_responded` events in `events/` carry durable response facts
- each request targets a ReviewUnit, captured file or range, or native observation in that same
  ReviewUnit
- each request belongs to a required track; actor/producer provenance remains in the event writer
  envelope
- bounded `state.json` summarizes input request state with `inputRequestCount`,
  `openInputRequestCount`, and `openOperativeInputRequestCount`, but it does not embed request
  history, response history, body content, or reason content

Request `reasonCode` and response `outcome` are intentionally separate classification axes.
Multiple different response events remain append-only facts; read surfaces report that
input request as ambiguous instead of choosing a timestamp winner.

Input request read projections use semantic IDs rather than event filenames as logical identity.
Multiple `input_request_opened` events with the same `inputRequestId` collapse to one request row
with a duplicate semantic diagnostic. Multiple `input_request_responded` events with the same
`inputRequestResponseId` collapse to one response row and do not make the input request
ambiguous. Distinct response IDs remain distinct facts and can still make the input request
ambiguous.

Input request bodies and response reasons use the shared inline-or-artifact mechanics. Text under
or equal to `BODY_INLINE_LIMIT` (4096 bytes today) stays inline in the event payload; text above
the threshold is externalized to `artifacts/notes/<sha256(body)>.json` with the `shore.note-body`
envelope (schema `shore.note-body`, version `1`), keeping `state.json` bounded and avoiding
unbounded event payload growth.

The direct read surfaces are `shore review input-request list` and `shore review input-request fetch`,
which replay events and can optionally hydrate bodies. Body artifact paths, reason artifact paths,
event filenames, and `state.json` paths are internal storage details, not command-output API. Native
input requests also appear in the composite `shore review unit show` projection. They are not
projected into `shore dump` or `shore show`.

Native assessments follow the same ReviewUnit ledger model:

- immutable `review_assessment_recorded` events in `events/` carry durable assessment facts
- each assessment targets a ReviewUnit, captured file or range, native observation, native input
  request, or native assessment in that same ReviewUnit
- each assessment belongs to a required track; actor/producer provenance remains in the event writer
  envelope
- bounded `state.json` summarizes assessment state with `assessmentCount`, but it does not embed
  assessment history, summaries, relationship graphs, or current-assessment candidates

Assessment values are closed in V1: `accepted`, `accepted_with_follow_up`, `needs_changes`, and
`needs_clarification`.

Assessment replacement is explicit. `replacesAssessmentIds` is the only V1 relationship that
removes an older assessment from the current set. Related observation and input-request references
are evidence links; they do not change current/replaced status.

Assessment read projections use semantic IDs rather than event filenames as logical identity.
Multiple `review_assessment_recorded` events with the same `assessmentId` collapse to one
assessment row with a duplicate semantic diagnostic. Multiple unreplaced assessment IDs remain
append-only facts; read surfaces report the current state as ambiguous instead of choosing a
timestamp winner.

Assessment summaries use the shared inline-or-artifact mechanics. Summaries under or equal to
`BODY_INLINE_LIMIT` (4096 bytes today) stay inline in the event payload; summaries above the
threshold are externalized to `artifacts/notes/<sha256(body)>.json` with the `shore.note-body`
envelope (schema `shore.note-body`, version `1`), keeping `state.json` bounded and avoiding
unbounded event payload growth.

The direct read surface is `shore review assessment show`, which replays events and can optionally
hydrate summaries. Summary artifact paths, event filenames, and `state.json` paths are internal
storage details, not command-output API. Native assessments also appear in the composite
`shore review unit show` projection, but they are not projected into `shore dump` or `shore show`.

State-change outcomes such as deferred, split-out, overridden, and superseded are represented as
native observations tagged with `state-change:*`, not as assessment values.

Validation evidence follows the same ReviewUnit ledger model:

- immutable `validation_check_recorded` events in `events/` carry durable facts about completed
  checks
- each validation check targets one exact captured ReviewUnit through opaque, content-addressed
  ReviewUnit identity
- each validation check belongs to a required track; actor/producer provenance remains in the event
  writer envelope
- bounded `state.json` summarizes validation evidence with `validationCheckCount`, but it does not
  embed validation history, summary content, logs, or reports

Validation evidence is advisory. It may support review judgment in `shore review unit show`,
`shore review history`, and `shore review validation list`, but it never grants review acceptance,
merge authority, or write authority. It never changes `currentAssessment`, assessment ambiguity,
operative input-request counts, or any other operative projection.

Validation identity is path-free. Event targets, validation targets, and stable identity fields carry
opaque IDs such as `reviewUnitId`, `trackId`, and `validationCheckId`; they must not derive from
worktree paths, raw `.git` layout, raw `.shore` paths, clone-local store paths, raw artifact paths,
or machine-local route names.

Validation summaries use the shared inline-or-artifact mechanics. Summaries under or equal to
`BODY_INLINE_LIMIT` (4096 bytes today) stay inline in the event payload; summaries above the
threshold are externalized to `artifacts/notes/<sha256(body)>.json` with the `shore.note-body`
envelope (schema `shore.note-body`, version `1`). Large logs and reports are referenced by
`sha256:<hex>` content hashes only; they are never inlined in validation events.

Validation events remain ordinary producer facts signable by the generic `EventToBeSigned` contract
from [ADR-0004](./adr/adr-0004-event-signatures.md). The validation family adds no signing payload
type, `sigVersion`, or family-specific signing path. See
[ADR-0006](./adr/adr-0006-validation-evidence.md) for the accepted validation evidence contract.

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
current assessments, resolve input-request lifecycles, or build the full ReviewUnit row projection.
Shared state diagnostics are still included so callers can see duplicate semantic facts while
inspecting the underlying events. Raw event files, artifact paths, event filenames, and `state.json`
are storage details, not history output API.

ReviewUnit show is the composite read surface for one captured ReviewUnit:

- `shore review unit show` returns `shore.review-unit` JSON derived from a validated scan of
  `.shore/events/` plus the bound immutable snapshot artifact for the selected ReviewUnit
- `eventSetHash` and `eventCount` describe the full event set read for the command, not only the
  selected ReviewUnit's returned narrative facts
- the output includes ReviewUnit identity, filters, summary counts, current assessment, native
  observations, input requests, assessments, imported adapter notes, projection rows, and
  diagnostics
- rows are narrative-first plus snapshot-complete: reviewed ledger material appears first, and the
  snapshot remainder still includes every captured file, metadata row, hunk header, and diff row
- track filters narrow narrative facts only; they do not mutate ReviewUnit selection, freshness
  metadata, or captured snapshot completeness
- `--include-body` hydrates body-like text from inline payloads or `artifacts/notes/`, while the
  default output keeps large text omitted

`shore.review-unit` is command-output API. Snapshot artifacts, note body artifacts, event files,
event filenames, and `state.json` remain Shoreline-owned storage details and are not exposed as stable
paths.

The review stream also surfaces stale and orphan notes as dedicated rows so reviewers can park the
cursor on them; the stream emits an additional synthetic file header for orphan notes when at least
one is present.

## Clone-Local Store Selection

The default durable store remains worktree-local `.shore/`. A Git worktree can also be registered
with a clone-local store associated with the clone's Git common directory, allowing linked
worktrees from the same clone to share imported Shoreline facts.

Clone-local stores are selected through a worktree-local registration file. Public commands expose
the result as command JSON with opaque store, clone, and repository-family refs; callers must not
depend on raw clone-local store paths, event filenames, artifact paths, `.git` paths, `.shore` paths,
or `state.json` layout.

The current linked writer contract is batch-only for durability, with linked-aware validation.
Review capture and native review write commands continue to write the worktree-local `.shore/`
store. What changed is what those write commands *validate against*: in a linked checkout, recording
a review fact — an observation, an input request open or response, an assessment, validation
evidence, or a lineage round — validates against the linked family's review record **plus** any local
facts you have not yet synced. That writer-visible union lets you attach a fact to a review unit (or
relate it to an observation, assessment, or request) captured in a sibling worktree. The fact itself
is still written to your worktree-local `.shore/` store and stays invisible to other checkouts until
`shore store link` copies it; the write result reports the `clone_local_fact_batch_only` diagnostic
to signal the pending sync, mirroring `clone_local_capture_batch_only` for capture. `shore store
link` is the explicit movement step: it scans the worktree for sensitivity findings before data
movement, reports redacted findings in the command document, and then imports local events and
artifacts into the clone-local store with strict content-hash validation. In this clone-local
release, sensitivity findings warn rather than abort; hard-blocking policy and explicit override
controls are deferred until movement can target a wider user-level or remote store.

`shore store status` is the public health and inventory surface for the selected store. It reports
event and artifact byte counts, total bytes, optional Git untracked bytes, largest artifact refs,
ReviewUnit snapshot byte accounting, and redacted sensitivity scan findings. Sensitivity references
are hashed `file:sha256:*` values and do not disclose secret contents or source file paths.

Linked reads resolve the selected store on every review read surface. `shore review unit list` and
`unit show`, `shore review history`, the observation, input-request, and validation lists,
`shore review assessment show`, `shore review lineage list` and `show`, and the inspector API all
read the clone-local store when the worktree is registered, including snapshot artifacts and large
note-shaped bodies. Linked reads are store-only: events written locally since the last
`shore store link` are not unioned into results. When the worktree holds local events that are not
yet in the linked store, read surfaces append the `clone_local_unsynced_local_events` diagnostic
naming the local event count, and `shore store link` copies those facts and clears it. Run
`shore store link` before removing a worktree whose review record should survive for its siblings.

The read-side `clone_local_unsynced_local_events` diagnostic and the write-side
`clone_local_fact_batch_only` diagnostic are two views of the same asymmetry: a fact you write in a
linked checkout lands worktree-local, so it is invisible to every read surface (the write result
names it at write time, later reads name it on the count) until `shore store link` shares it.

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

Any hash that contributes to durable identity should use Shoreline's canonical JSON path, with object
keys sorted recursively before hashing. Do not rely on incidental serde_json map ordering or local
construction order for event payload hashes, revision fingerprints, snapshot fingerprints, or future
content-derived IDs.

Do not add a global sequence number until Shoreline has a concrete allocator that does not create a
shared mutable counter. Deterministic event ordering can start from event metadata and filenames.

## Ingest Provenance

Events that enter a store through a foreign-event seam carry an optional top-level envelope
sibling stamped by the local importer
([ADR-0009](adr/adr-0009-resumption-binding-trust-source.md)):

```json
"ingest": { "via": "ingest-events", "receivedAt": "unix-ms:1760000000000" }
```

`via` is a bounded vocabulary naming the seam: `ingest-events` (the `ingest_events` /
`import_event` workflow) or `bundle-apply` (store bundle import). `receivedAt` uses the store's
`unix-ms:` timestamp format. Consumers read presence; `via` and `receivedAt` are operator-facing
detail.

Both import seams stamp unconditionally and **overwrite** any inbound stamp. A stamp in arriving
bytes is some other store's bookkeeping — the same honesty rule that applies to `sourceRef`: hop
metadata from elsewhere is not a fact.

The stamp participates in nothing that identifies or authenticates the event. It is excluded from
the to-be-signed view, so stamping a signed event cannot invalidate its signature, and it
contributes to neither idempotency keys nor `eventId`. Exclusive event creation gives the stamp
first-stored-wins mechanically: a locally authored stored event can never acquire a stamp after
the fact, and an ingested event can never lose or swap its first stamp on re-ingest.

The marker is local bookkeeping written by the store owner's own importer. It is trustworthy to
this store under the single-writer contract; it is never a signed fact, and it is never
trustworthy to a third party reading a mirrored or copied store. Note the seam boundary: bundle
apply stamps, but a wholesale filesystem copy (`cp -r`) carries unstamped events into the new
store. Stores whose possession does not imply authorship should prefer the `verified-only`
binding posture described in
[ADR-0009](adr/adr-0009-resumption-binding-trust-source.md).

Events imported before the marker landed are unstamped and indistinguishable from local-authored
events — a store owner who imported events earlier possesses a store whose history they chose.
The marker discriminates from its landing forward.

## Artifact Files

Artifact filenames follow two deliberate rules, paired to what the file represents:

- **Identifier-hashed artifacts** use a hash of a stable opaque identifier as the filename stem.
  Snapshot artifacts live at `artifacts/snapshots/<sha256(snapshotId)>.json`. The readable ID stays
  inside the artifact body; the hash exists only so the filename is fixed-width, filesystem-safe,
  and free of the characters that appear in semantic IDs (such as the `:` separators in
  `snap:git:sha256:…`). This is the same rule events use, applied to a different identifier.
  Snapshot artifacts also carry their own canonical `contentHash` field that the read path
  recomputes and compares, so tamper or transcription errors are caught at load time. Under V1
  the artifact body inlines every captured row; the `contentHash` therefore covers the full row
  inventory. See [ADR-0002](./adr/adr-0002-large-snapshot-artifact-policy.md) for the V1 policy and
  the V2 reversal shape.
- **Content-addressed artifacts** use a hash of the artifact body as the filename stem. Note-body
  artifacts live at `artifacts/notes/<sha256(body)>.json`. Hashing the body gives deterministic
  addressing and deduplication across observations, input requests, and assessments that share
  text. Native-recorded payloads may carry a payload-level body hash
  (`body_content_hash` / `reason_content_hash` / `summary_content_hash`) so future read paths or
  repair tools can verify the artifact against the event ledger; imported-note payloads do not
  carry such a hash and instead rely on the content-addressed filename plus the referring event's
  `body_artifact_path`. Identifier-hashed artifacts do not gain the same dedup benefit, because
  their underlying ID is already unique.

The asymmetry is intentional: identifier-hashed naming protects filenames from arbitrary ID
characters, while content-addressed naming earns its keep through deterministic dedup. Read paths
should not mix the two rules — locate snapshot artifacts by their `snapshotId` and note-body
artifacts by the relative path recorded in the referencing event.

Artifact filenames remain Shoreline-owned storage details. The consumer contract is the command-output
JSON (`shore.review-capture`, `shore.review-unit`, and friends), which exposes semantic IDs and the
snapshot artifact's canonical `contentHash`. Filename derivation rules may change without a
deprecation cycle, but artifacts are V1 authority alongside events — the event log alone cannot
reconstruct snapshot rows or large note bodies. A future rule change must therefore rename or
migrate the affected files in place, keep a compatibility read path during transition, or
regenerate the directory from the original source (worktree capture, sidecar import) where that is
possible. Shoreline does not promise dual-read of legacy filenames implicitly.

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
limited to queue code. On load, Shoreline should remove stale temp files matching its known prefixes and
older than the configured safety threshold.

Rebuildable projections may use a non-durable write mode that skips fsync, but they still should use
the same temp/rename path to avoid partial reads.

## Bounded Projections

`state.json` must stay bounded. It should summarize current state, cursors, and active projections;
it should not grow linearly with the event log.

If a projection needs unbounded history, split it into paged or content-addressed records under
`artifacts/` and keep `state.json` as an index or summary. A large `state.json` is a design smell
because it becomes a shared mutable file, a slow health check, and a crash-recovery hazard.

Imported review-note bodies follow this rule directly: bodies under or equal to `BODY_INLINE_LIMIT`
(4096 bytes today) stay inline in the event payload; bodies above the threshold are externalized to
`artifacts/notes/<sha256(body)>.json` with the `shore.note-body` envelope (schema `shore.note-body`,
version `1`), so the authoritative event and rebuildable projection remain bounded.

## Note Body Materialization

Shoreline stores note-shaped event bodies (observations, input request bodies, input request response
reasons, assessment summaries, imported review notes) using a threshold split, not as a uniform
artifact-per-body materialization.

- **Inline path.** Bodies whose byte length is at most `BODY_INLINE_LIMIT` (4096 bytes today, defined
  at `src/session/store/body_artifact.rs:8`) remain inline in the event payload. The on-disk event
  carries the body bytes verbatim under its `body` (or `summary` / `reason`) field.
  `body_artifact_path` stays `None`. The materialization discriminator is `body` vs
  `body_artifact_path`, not `body_byte_size`: native ledger payloads (observations, input requests,
  assessments) currently set `body_byte_size = Some(_)` on the inline arm via the shared
  `staged_body` helper, while imported-note payloads leave `body_byte_size = None` inline. Consumers
  that need an inline length should read it from the inline string directly.
- **Artifact path.** Bodies above the threshold are externalized to
  `artifacts/notes/<sha256(body)>.json` under the `shore.note-body` envelope
  (`{"schema":"shore.note-body","version":1,"body":"..."}`). The event payload's `body` field is
  `None`; its `body_artifact_path` carries the relative path and `body_byte_size` carries the body's
  length. Native-recorded payloads (observations, input requests, input request responses,
  assessments) additionally carry `body_content_hash` / `reason_content_hash` /
  `summary_content_hash`; imported-note payloads do not. `load_body_artifact` validates the path
  shape and the envelope's `schema` / `version` fields, not the body bytes themselves — hash-based
  cross-validation against the event payload, where available, is a caller's responsibility.

### What `artifacts/notes/` is — and isn't

- It is a content-addressed **overflow store**, not a complete inventory of note bodies. Small-body
  notes have no corresponding file in this directory. The directory may be empty for a repo that
  has only small notes.
- The authoritative durable record of every note is its event under `.shore/events/`. Replay
  (`EventStore::list_events()` followed by `load_body_artifact` for any `body_artifact_path`) is
  the only supported read primitive for note state.
- Tooling that wants a complete list of note bodies must replay events; walking `artifacts/notes/`
  alone is not sufficient and is not a supported authority.

### Why threshold-based

- Most observations and assessment summaries are short. Externalizing every body would emit one
  additional fsync per body-bearing event (both inline event and artifact use `Durability::Durable`
  writes), with proportional file-count growth.
- The body's identity does not depend on materialization: native-recorded payloads (observations,
  input requests, assessments) already carry a `*_content_hash`, and imported-note artifacts are
  content-addressed by `sha256(body)` in their filenames. Materializing every body would not
  strengthen those guarantees, only change where canonical bytes live.
- Artifact-only enumeration is not a supported read path. Even if all bodies were materialized, an
  artifact file under `artifacts/notes/<hash>.json` carries the body and the envelope schema /
  version but no referrer identity — it cannot answer "which note / observation / assessment does
  this body belong to?" without joining back to the event ledger.

### Threshold is tunable

The 4096-byte threshold is internal storage tuning and may change without a deprecation cycle. The
**inline-or-artifact bifurcation itself** is the stable contract: storage consumers must accept
that any given note-shaped body may be either inline or referenced by a `body_artifact_path`, and
resolve both arms.

See [ADR-0001](./adr/adr-0001-note-body-materialization.md) for the decision rationale.

## Large Snapshot Artifact Policy

Shoreline stores captured review-unit diffs inline in identifier-hashed artifacts under
`artifacts/snapshots/<sha256(snapshotId)>.json`. The artifact body is one JSON object per snapshot
and carries every captured file, every metadata row, every hunk, and every diff row. There is no
elision threshold, no generated-file detection, and no metadata-only marker for "too-large" or
"elided" files.

- **Row inventory.** A captured snapshot for a newly added 10,000-line text file produces one
  artifact with roughly 10,000 inline `DiffRow` objects. V1 does not elide.
- **Metadata rows.** `FileMetadataKind` is `{ BinarySummary, ModeChange, RenameSummary,
  SubmoduleSummary }` today. `BinarySummary` is the V1 *content-omission* marker — binary
  patches set `is_binary = true`, get a `BinarySummary` row, and leave `hunks` empty. The other
  three variants carry file-level Git facts (rename, mode change, submodule pointer change) and
  also leave `hunks` empty, but they are not content-omission markers. There is no `ElidedFile`
  or `GeneratedFile` variant.
- **Read surface.** `shore review unit show` is narrative-first plus snapshot-complete: reviewed
  ledger material appears first, and the snapshot remainder includes every captured file, metadata
  row, hunk header, and diff row. No flag omits row bodies.
- **Content-hash scope.** `SnapshotArtifact.contentHash` covers the full row inventory under V1.
  Any future elision must change the hash scope explicitly (either bump
  `SNAPSHOT_ARTIFACT_VERSION` from `1` to `2`, or add a separate `contentHashScope` field), so a
  consumer can tell V1 hashes (full inventory) from V2 hashes (elided) on inspection.

The V1 policy is intentionally minimal: every question issue #64 asks ("elide?", "detect
generated?", "metadata-only rows?", "omit-on-show?", "hash scope?") receives an explicit answer in
[ADR-0002](./adr/adr-0002-large-snapshot-artifact-policy.md). Each answer's reversal — what would
have to change to flip it — is recorded in the ADR's "Future Reversal" section.

## Projection Freshness

`state.json` records `eventSetHash` as derived freshness metadata for the event set used to build
the projection. `eventCount` remains a cheap count, but it does not prove that a cached projection
matches the current `.shore/events/` set.

`eventSetHash` is computed from Shoreline's canonical JSON hash path over sorted `(eventId,
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

V1 uses a single-writer workflow contract: one active Shoreline writer per `.shore/` directory at a
time. Shoreline does not coordinate writers with lockfiles, leases, a daemon, IPC, or filesystem
notifications yet.

Clone-local linking does not change that direct-write contract. Shared clone-local writes are
limited to the explicit `shore store link` batch import path, which performs sensitivity scanning
before movement, reports redacted findings, and imports artifacts before events. Direct shared
capture remains unsupported until Shoreline has a storage-level serializer for multi-file
publication.

Event files remain the append-only authority. They are created with exclusive file creation:
same-key and same-payload retries are idempotent, while same-key and different-payload attempts are
conflicts. Different event files can be written independently, but reducers and projections decide
whether the resulting event set is valid, ambiguous, or conflicting.

`state.json` writes are projection cache writes. If projection writers race, events remain
authoritative and the projection can be rebuilt.

Workflow startup cleanup removes only Shoreline temp files older than the workflow startup threshold.
Preserving fresh `.shore-write.*.tmp` files avoids clobbering an in-flight write, but it is not a
lock or lease and does not make long-running multi-process writes a supported coordination model.

## Legacy Writer Role Events

Earlier development versions of Shoreline wrote a `role` field inside each event's writer
envelope. Current Shoreline does not store a writer role: the review act is derived from
`eventType`, and the conversation speaker is recorded by adapters as a `sourceSpeaker` payload
field. Store reads reject stored events whose writer carries `role`. Because Shoreline has not
released this storage contract, the supported migration is to discard the old local `.shore/`
directory and recapture the review.

## Legacy Writer Tool Events

Earlier development versions of Shoreline wrote a `tool` object inside each event's writer
envelope. Current Shoreline names the producing software under `producer` (`{name, version}`); per
the [ADR-0010](./adr/adr-0010-actor-identity-and-delegation.md) vocabulary rule, "agent" names
acting software, "producer" names software that writes events, and the word "tool" is reserved for
the model-API/MCP sense and is no longer an envelope field. The rename rides the pre-adoption
hard-break policy ([ADR-0007](./adr/adr-0007-writer-act-vocabulary.md)): the golden to-be-signed
bytes, the embedded signatures, and `sigVersion: 1` are all untouched.

Store reads reject stored events whose writer carries `tool` with a typed
`UnsupportedEventEnvelope` error naming the replacement field (`writer.producer`) and this anchor,
rather than an opaque missing-field error. Because Shoreline has not released this storage
contract, the supported migration is to discard the old local `.shore/` directory and recapture
the review.

## Actor Identity and Delegation

Every event's writer carries an `actorId`. Human writers derive theirs from Git identity
(`actor:git-email:<email>` or `actor:git-name:<name>`); agents write under their own
`actor:agent:<agent-name>` id, set with `SHORE_ACTOR_ID` (see
[agent-authoring.md](./agent-authoring.md)). The actor id is reported in projections but is never
the basis of a binding decision — a writer cannot make a claim trustworthy by asserting it
([ADR-0007](./adr/adr-0007-writer-act-vocabulary.md)).

Who an agent acts *on behalf of* is answered by a checked-in delegation map at
`.shoreline/delegates` — a sibling of `.shoreline/allowed-signers`, deliberately separate so that
key rotation never touches delegation. It is human-committed JSON:

```json
{
  "delegates": {
    "actor:agent:claude-code": [
      {
        "principal": "actor:git-email:kevin@swiber.dev",
        "validFrom": "2026-06-10T00:00:00Z",
        "validUntil": null,
        "comment": "claude-code, enrolled by Kevin"
      }
    ]
  }
}
```

- The top-level key is `delegates`; unknown top-level keys are ignored for forward compatibility.
- Each key is an `actor:agent:<name>` id mapping to an array of windowed records.
- `principal` must be a valid **non-agent** actor id in v1 (delegation chains have depth 0).
- `validFrom` is required and `validUntil` is null or an RFC 3339 UTC instant (`Z` offset only,
  e.g. `2026-06-10T00:00:00Z`); the window is half-open `[validFrom, validUntil)`.
- `comment` is free text for diff readers and is never authority.

Resolution is projection-time, replay-stable, and git-free: it selects the record whose window
contains the event's `occurredAt`. **Revocation closes a window** (`validUntil` set) — events
inside the closed window keep resolving, so history stays stable — while **deleting a record is
disavowal**: events that previously resolved deliberately resolve to nothing. `git log -p
.shoreline/delegates` is the audit trail; the file's history, not a mechanism, records who was
enrolled when.

Resolution config is reader-supplied, exactly like the allowed-signers trust set. A consumer
without the map — a mirror, an exported bundle — degrades to `principal: none`, never a wrong
answer. The CLI discovers `.shoreline/delegates` at the worktree root; a malformed file warns once
to stderr and proceeds with no map (advisory, never blocking). Overlapping windows with distinct
principals resolve as `ambiguous` and are surfaced, never auto-picked.

In this release, delegation entries are created by editing `.shoreline/delegates` directly (or by
an agent proposing a working-tree edit); the human's review-and-commit is the authorization. A
`shore keys`-staged enrollment flow is a separate, later key-custody plan; this release documents
no unshipped commands.

Pre-cutover honesty: agent events written before the `actor:agent:` cutover carry the human's
git-email id and remain exactly what they claimed at write time. The `agent:*` *track* name is a
heuristic ("written on an agent track"), never re-attribution; recapture is the hard-break escape
hatch.

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

The first local durable-state stage should stay synchronous and local. Do not introduce async traits
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

If Shoreline later adds a delivery queue, every retry path must have:

- a maximum attempt count
- backoff policy
- permanent vs. transient failure classification
- a terminal failed state that removes the entry from active rotation
- target-liveness checks before resume or apply actions

The local durable-state stage should not implement this queue. Local event writes should fail loudly
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

The point is to keep the first storage stage small while making the safe path the easiest path to
use.
