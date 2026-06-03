# Library API

Shoreline ships as a library (`shoreline`) alongside the `shore` binary. This page documents the
**supported, stable library surface** for consumers that read and write durable review facts
in process, without shelling out to `shore`. The motivating consumer is a federation bridge that
forwards review decisions on behalf of remote reviewers.

The `shore` command-output JSON remains a supported integration surface (see
[cli-reference.md](cli-reference.md)). The library surface below is an additional, equally supported
contract: the [`shoreline::documents`](#documents) module produces the **byte-identical**
`shore.*` documents in process.

## What "stable" means

The items listed here are the supported public API. Within a `0.x` line they may still change, but
changes are intentional and called out in the changelog; we do not break them casually. Anything not
listed here — including `pub` items reached through module paths not re-exported from the surfaces
below — is internal and may change without notice. Internal architecture vocabulary
(`docs/substrate-language.md`) is explicitly **not** part of this contract.

## Sync / async boundary

The storage layer is **synchronous** and stays that way until a remote backend, subscription API, or
second storage backend forces otherwise (see [storage-model.md](storage-model.md)). All library
calls below are synchronous and may block on local I/O.

Async consumers (for example a Tokio server) must run these calls on a blocking executor:

```rust
let result = tokio::task::spawn_blocking(move || {
    shoreline::session::respond_input_request(options)
})
.await??;
```

Shoreline does not introduce async traits or a runtime of its own.

## Supported surface

### Reads — `shoreline::session`

| Item | Purpose |
| ---- | ------- |
| `show_review_unit` + `ReviewUnitShowOptions` / `ReviewUnitShowResult` | The ReviewUnit projection (identity, summary, rows, observations, input requests, assessments). |
| `list_review_units` + `ReviewUnitListOptions` / `ReviewUnitListResult` | Enumerate captured ReviewUnits. |
| `list_input_requests` + `InputRequestListOptions` / `InputRequestListResult` | List input requests; defaults to open. Filter with `InputRequestStatusFilter`. |
| `fetch_input_request` + `InputRequestFetchOptions` / `InputRequestFetchResult` | Fetch one input request, optionally hydrating its body. |
| `list_observations` / `show_assessments` / `review_history` (+ their options/results) | Observations, current assessment, and the review history projection. |
| `store_status` + `StoreStatusOptions` / `StoreStatusResult` | Store inventory and sensitivity diagnostics. |
| `InputRequestView`, `InputRequestResponseView`, `ObservationView`, `AssessmentView`, `ReviewUnitProjection*` | Public-field result types. |
| `InputRequestStatus`, `InputRequestStatusFilter`, `ObservationStatus`, `CurrentAssessmentStatus`, `ReloadOutcome` | Status/value enums consumers branch on. |

### Writes — `shoreline::session`

| Item | Purpose |
| ---- | ------- |
| `capture_worktree_review` + `CaptureOptions` | Capture a Git working tree into a ReviewUnit. |
| `open_input_request` / `respond_input_request` (+ options/results) | Open and operatively respond to input requests. |
| `record_observation` / `record_assessment` (+ options/results) | Record observations and the review assessment. |

**Per-call writer attribution.** Each write-options builder exposes
`with_actor_id(ActorId)`. Precedence is **explicit override > `SHORE_ACTOR_ID` env var > local Git
identity**; a malformed id is ignored and falls through to the next source, and `None` reproduces the
default resolution exactly. This lets an in-process, concurrent consumer attribute each write to the
correct actor without mutating the process-global `SHORE_ACTOR_ID` (which is `unsafe` and racy under
edition 2024). The chosen actor is part of a fact's content-addressed identity, so distinct actors
produce distinct facts.

### Event ingest — `shoreline::session`

| Item | Purpose |
| ---- | ------- |
| `ingest_events` + `IngestEventsOptions` / `IngestEventsResult` | Ingest pre-formed `ShoreEvent`s (forwarded over a network or merged from another clone), preserving append-only / content-addressed / idempotent + conflict semantics. |
| `import_event` + `ImportEventOptions` | Single-event convenience over `ingest_events`. |
| `shoreline::session::event::ShoreEvent` (+ `EventType`, `Writer`, payload types) | The event envelope; `Serialize` + `Deserialize`, so events can be forwarded as JSON. |

Ingest validates each envelope (`eventId`/`payloadHash`/schema) and rejects events whose
`writer.actor_id` is not a well-formed `actor:` id, validating the whole batch's attribution before
any write. It validates internal consistency and attribution shape, not authenticity: there are no
signatures, and Shoreline does not prove the writer actually controlled the named actor.
That advisory-first boundary is intentional (see
[ADR-0003](adr/adr-0003-agent-resource-claims-advisory-first.md)). A re-ingest of an
already-present event is a no-op; a conflicting payload under the same idempotency key is rejected.
The projection (`state.json`) is rebuilt once after the batch.

### Artifacts — `shoreline::session`

| Item | Purpose |
| ---- | ------- |
| `referenced_artifacts` | Enumerate the content-addressed artifacts required by a set of forwarded `ShoreEvent`s. |
| `ArtifactRef` / `ArtifactKind` | Opaque artifact references. Consumers can branch on kind and fetch by `content_hash()` without depending on store paths. |
| `export_artifact` | Read and hash-verify one referenced artifact's bytes from a source store. |
| `import_artifact` + `ImportArtifactOptions` / `ImportArtifactResult` / `ImportArtifactOutcome` | Hash-verify and idempotently write one referenced artifact into a destination store. |

`ingest_events` transfers events only. Events can reference snapshot artifacts and large note-shaped
body artifacts, and those blobs must be transferred separately before reads that need them. A full
mirror flow is:

1. read or receive a batch of `ShoreEvent`s;
2. call `referenced_artifacts(&events)` to learn which artifact hashes are required;
3. fetch or `export_artifact` those blobs from the source store;
4. `ingest_events` into the destination store;
5. `import_artifact` each fetched blob into the destination store.

After events and artifacts are present, `show_review_unit` can load the bound snapshot artifact and
`fetch_input_request(...with_include_body(true))` / include-body projections can hydrate large
bodies. The store layout remains private; callers should keep and pass around `ArtifactRef` values
rather than constructing paths. This byte-level transfer path complements `link_clone_local_store`:
linked local clones can share one store, while remote or networked consumers can fetch and import
the required blobs by content hash.

### Documents — `shoreline::documents`

The `shoreline::documents` module produces the documented `shore.review-*` command-output documents,
**byte-identical** to the `shore` CLI:

- Envelopes: `DiagnosticDocument<T>`, `EventWriteDocument<T>` (schema/version/diagnostics, plus
  event-write counts).
- Per-command builders: `unit_show_document`, `unit_list_document`, `capture_document`,
  `observation_add_document`, `observation_list_document`, `input_request_open_document`,
  `input_request_list_document`, `input_request_fetch_document`, `input_request_respond_document`,
  `assessment_add_document`, `assessment_show_document`, `history_document`, and the body/view
  document types they return.

A consumer that wants exactly the documented JSON contract calls a read/write workflow, passes the
typed result to the matching builder, and serializes the document with `serde_json`.

### Identifiers — `shoreline::model`

The content-addressed id newtypes (`ActorId`, `ReviewUnitId`, `InputRequestId`,
`InputRequestResponseId`, `ObservationId`, `AssessmentId`, `EventId`, `TrackId`, …) are public and
serialize transparently as strings.

## Example: read, attribute a write, and forward an event

```rust
use shoreline::model::ActorId;
use shoreline::session::{
    InputRequestListOptions, InputRequestRespondOptions, InputRequestResponseOutcome,
    list_input_requests, respond_input_request,
};

// Read open input requests.
let open = list_input_requests(InputRequestListOptions::new(&repo))?;

// Respond on behalf of a specific remote reviewer.
let result = respond_input_request(
    InputRequestRespondOptions::new(&repo, open.input_requests[0].id.clone())
        .with_outcome(InputRequestResponseOutcome::Approved)
        .with_actor_id(ActorId::new("actor:agent:remote-reviewer")),
)?;

// Produce the documented `shore.review-input-request-respond` JSON in process.
let document = shoreline::documents::input_request_respond_document(result);
let json = serde_json::to_value(&document)?;
```
