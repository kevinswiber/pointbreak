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
| `show_review_unit` + `ReviewUnitShowOptions` / `ReviewUnitShowResult` | The ReviewUnit projection (identity, summary, rows, observations, input requests, assessments, validation checks). |
| `list_review_units` + `ReviewUnitListOptions` / `ReviewUnitListResult` | Enumerate captured ReviewUnits. |
| `list_input_requests` + `InputRequestListOptions` / `InputRequestListResult` | List input requests; defaults to open. Filter with `InputRequestStatusFilter`. |
| `fetch_input_request` + `InputRequestFetchOptions` / `InputRequestFetchResult` | Fetch one input request, optionally hydrating its body. |
| `list_observations` / `show_assessments` / `list_validation_checks` / `review_history` (+ their options/results) | Observations, current assessment, validation evidence, and the review history projection. |
| `store_status` + `StoreStatusOptions` / `StoreStatusResult` | Store inventory and sensitivity diagnostics. |
| `InputRequestView`, `InputRequestResponseView`, `ObservationView`, `AssessmentView`, `ValidationCheckView`, `ReviewUnitProjection*` | Public-field result types. |
| `InputRequestStatus`, `InputRequestStatusFilter`, `ObservationStatus`, `ValidationStatus`, `ValidationTrigger`, `CurrentAssessmentStatus`, `ReloadOutcome` | Status/value enums consumers branch on. |

### Writes — `shoreline::session`

| Item | Purpose |
| ---- | ------- |
| `capture_worktree_review` + `CaptureOptions` | Capture a Git working tree into a ReviewUnit. |
| `open_input_request` / `respond_input_request` (+ options/results) | Open and operatively respond to input requests. |
| `record_observation` / `record_assessment` / `record_validation_check` (+ options/results) | Record observations, the review assessment, and advisory validation evidence. |

**Per-call writer attribution.** Each write-options builder exposes
`with_actor_id(ActorId)`. Precedence is **explicit override > `SHORE_ACTOR_ID` env var > local Git
identity**; a malformed id is ignored and falls through to the next source, and `None` reproduces the
default resolution exactly. This lets an in-process, concurrent consumer attribute each write to the
correct actor without mutating the process-global `SHORE_ACTOR_ID` (which is `unsafe` and racy under
edition 2024). The chosen actor is part of a fact's content-addressed identity, so distinct actors
produce distinct facts.

Validation checks target a captured ReviewUnit only. `ValidationAddOptions` resolves either an
explicit ReviewUnit, a lineage head, or the single current ReviewUnit, then constructs the
ReviewUnit validation target internally. `ValidationListOptions` filters by ReviewUnit, track, and
status, and `with_include_body(true)` hydrates validation summaries. Validation evidence is
advisory; it does not accept, reject, merge, block, or replace a review assessment.

### Event signatures — `shoreline::session` / `shoreline::crypto`

Per-event Ed25519 signatures are optional. Unsigned events remain valid and continue to omit
`signer` and `signature`; all event-producing write options (`CaptureOptions`,
`InputRequestOpenOptions`, `InputRequestRespondOptions`, `ObservationAddOptions`,
`AssessmentAddOptions`, and `ValidationAddOptions`) expose `sign_with(...)` for callers that want
the event signed at write time.

| Item | Purpose |
| ---- | ------- |
| `EventSignature` | The event envelope signature object: `{ alg: "ed25519", sigVersion: 1, sig: "..." }`. |
| `EventSigner` / `EventSignatureBytes` / `SignerId` | Implement signer integrations and carry base64 Ed25519 signature bytes plus `did:key` signer identity. |
| `event_to_be_signed` / `EventToBeSigned` | Build the canonical to-be-signed producer-fact view for a signed event. |
| `event_signature_pre_authentication_encoding` | Build the Dead Simple Signing Envelope (DSSE) pre-authentication encoding bytes that Ed25519 signs. |
| `verify_event_signature` | Verify one event against a `TrustSet` and return an `EventVerificationStatus`. |
| `EventVerificationPolicy` | Choose advisory, integrity-strict, or trusted-strict ingest/read policy. |
| `EventVerificationStatus` | The public status enum: `valid`, `invalid`, `untrusted_key`, or `unsigned`. |
| `TrustSet` / `event_signature_trust_set` | Authorize friendly `actor:*` identities to one or more `did:key` signers. |
| `EventVerificationView` / `ArtifactAvailability` / `verification_view` | Combine authenticity status with artifact availability without conflating them. |

Signed friendly-actor events carry both `writer.actorId` and top-level `signer`:

```text
writer.actorId = actor:git-email:alice@example.com
signer         = did:key:z6Mk...
signature      = { alg: "ed25519", sigVersion: 1, sig: "base64-ed25519-signature" }
```

Self-certifying events may use a `did:key` actor id directly and omit the top-level `signer`:

```text
writer.actorId = did:key:z6Mk...
signature      = { alg: "ed25519", sigVersion: 1, sig: "base64-ed25519-signature" }
```

These forms are not aliases. `writer.actorId = did:key:z6Mk...` and
`writer.actorId = actor:git-email:alice@example.com` signed by `did:key:z6Mk...` are different
claims and remain different events.

`EventToBeSigned` is not "the whole event minus `signature`." It is the canonical producer-fact
view described by [ADR-0004](adr/adr-0004-event-signatures.md): schema, version, event type, event
id, payload hash, target, writer actor id, effective signer, occurrence timestamp, and
assertion mode. `sourceRef` is unsigned hop metadata; bridges may add or change it without changing
the producer signature. The protocol media type still contains `event-tbs.v1` for
"event to be signed" compatibility, but public Rust names spell out `EventToBeSigned`.

Verification is advisory by default. Callers can select:

- `EventVerificationPolicy::advisory()` — report status and accept `invalid`, `untrusted_key`, and
  `unsigned`;
- `EventVerificationPolicy::integrity_strict()` — reject `invalid`, accept `untrusted_key` and
  `unsigned`;
- `EventVerificationPolicy::trusted_strict()` — reject `invalid`, `untrusted_key`, and `unsigned`
  unless `with_allow_unsigned(true)` is set.

`IngestEventsOptions`, `ImportEventOptions`, and `ReviewHistoryOptions` can carry a verification
policy and trust set. Ingest evaluates the policy before committing any event in the batch. Read
surfaces such as `review_history` report verification status only when requested; they do not
persist that status into event files or `state.json`.

An idempotent re-ingest keeps the first stored event. If a later event has the same idempotency key
and payload hash but a different signer or signature, Shoreline reports the
`divergent_signature_existing_event` diagnostic and keeps the first stored event. Other metadata
differences with the same payload hash remain an idempotent existing event. Signatures authenticate
the producer facts; they do not choose an automatic conflict winner.

### Event ingest — `shoreline::session`

| Item | Purpose |
| ---- | ------- |
| `ingest_events` + `IngestEventsOptions` / `IngestEventsResult` | Ingest pre-formed `ShoreEvent`s (forwarded over a network or merged from another clone), preserving append-only / content-addressed / idempotent + conflict semantics. |
| `import_event` + `ImportEventOptions` | Single-event convenience over `ingest_events`. |
| `shoreline::session::event::ShoreEvent` (+ `EventType`, `Writer`, payload types) | The event envelope; `Serialize` + `Deserialize`, so events can be forwarded as JSON. |

Ingest validates each envelope (`eventId`/`payloadHash`/schema) and validates the whole batch's
attribution before any write. Per-event Ed25519 signatures are optional and additive: signed events
carry top-level `signer` and `signature = { alg, sigVersion, sig }` fields, while unsigned historical
events remain valid. The signed bytes use `application/vnd.shore.event-tbs.v1+json` with literal
Dead Simple Signing Envelope (DSSE) pre-authentication encoding over canonical `EventToBeSigned`
to-be-signed bytes, as defined in [ADR-0004](adr/adr-0004-event-signatures.md).

Signature verification is advisory by default, preserving the reader-owned policy boundary from
[ADR-0003](adr/adr-0003-agent-resource-claims-advisory-first.md). Verification reports
`valid`, `invalid`, `untrusted_key`, or `unsigned`; policy presets named `advisory`,
`integrity-strict`, and `trusted-strict` let callers choose whether invalid signatures, untrusted
keys, or unsigned events should remain diagnostics or reject ingest. Read surfaces expose requested
verification status without changing the stored event or projection. A re-ingest of an
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
rather than constructing paths. A remote bridge derives those refs from the forwarded events it
already has, fetches bytes by `ArtifactRef::content_hash()`, and loops over `import_artifact`;
callers do not construct refs from a raw hash alone. This byte-level transfer path complements
`link_clone_local_store`: linked local clones can share one store, while remote or networked
consumers can fetch and import the required blobs by content hash.

Signature validity does not imply artifact availability. A signed event can be `valid` while its
referenced snapshot or note-body artifact is unavailable, and an available artifact can be attached
to an unsigned event. Consumers that need a full-fidelity mirror should verify both event signatures
and artifact transfer separately.

### Documents — `shoreline::documents`

The `shoreline::documents` module produces the documented `shore.review-*` command-output documents,
**byte-identical** to the `shore` CLI:

- Envelopes: `DiagnosticDocument<T>`, `EventWriteDocument<T>` (schema/version/diagnostics, plus
  event-write counts).
- Per-command builders: `unit_show_document`, `unit_list_document`, `capture_document`,
  `observation_add_document`, `observation_list_document`, `input_request_open_document`,
  `input_request_list_document`, `input_request_fetch_document`, `input_request_respond_document`,
  `assessment_add_document`, `assessment_show_document`, `validation_add_document`,
  `validation_list_document`, `history_document`, and the body/view document types they return.

A consumer that wants exactly the documented JSON contract calls a read/write workflow, passes the
typed result to the matching builder, and serializes the document with `serde_json`.

### Identifiers — `shoreline::model`

The content-addressed id newtypes (`ActorId`, `ReviewUnitId`, `InputRequestId`,
`InputRequestResponseId`, `ObservationId`, `AssessmentId`, `ValidationCheckId`, `EventId`,
`TrackId`, …) are public and serialize transparently as strings.

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
