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
| `show_revision` + `RevisionShowOptions` / `RevisionShowResult` | The revision projection (identity, summary, rows, observations, input requests, assessments, validation checks). |
| `list_revisions` + `RevisionListOptions` / `RevisionListResult` | Enumerate captured revisions. |
| `list_input_requests` + `InputRequestListOptions` / `InputRequestListResult` | List input requests; defaults to open. Filter with `InputRequestStatusFilter`. |
| `fetch_input_request` + `InputRequestFetchOptions` / `InputRequestFetchResult` | Fetch one input request, optionally hydrating its body. |
| `list_observations` / `show_assessments` / `list_validation_checks` / `review_history` (+ their options/results) | Observations, current assessment, validation evidence, and the review history projection. |
| `store_status` + `StoreStatusOptions` / `StoreStatusResult` | Store inventory and sensitivity diagnostics. |
| `InputRequestView`, `InputRequestResponseView`, `ObservationView`, `AssessmentView`, `ValidationCheckView`, `RevisionProjection*` | Public-field result types. |
| `InputRequestStatus`, `InputRequestStatusFilter`, `ObservationStatus`, `ValidationStatus`, `ValidationTrigger`, `CurrentAssessmentStatus` | Status/value enums consumers branch on. |

### Writes — `shoreline::session`

| Item | Purpose |
| ---- | ------- |
| `capture_review` + `CaptureOptions` | Canonical capture entry point; records a `WorkObjectProposed` event carrying a `WorkObjectProposedPayload` (the one generative move), and dispatches on the options' source spec (worktree by default, or a commit range via `CaptureOptions::with_commit_range`). The function name is unchanged. |
| `CommitRangeSpec` | Commit-range capture input: a base rev and an optional target rev (default `HEAD`), resolved to commit endpoints at capture time. |
| `capture_worktree_review` + `CaptureOptions` | Worktree-source convenience entry point; delegates to `capture_review`. The function name is unchanged. |
| `open_input_request` / `respond_input_request` (+ options/results) | Open and operatively respond to input requests. |
| `record_observation` / `record_assessment` / `record_validation_check` (+ options/results) | Record observations, the review assessment, and advisory validation evidence. |

**Per-call writer attribution.** Each write-options builder exposes
`with_actor_id(ActorId)`. Precedence is **explicit override > `SHORE_ACTOR_ID` env var > local Git
identity**; a malformed id is ignored and falls through to the next source, and `None` reproduces the
default resolution exactly. This lets an in-process, concurrent consumer attribute each write to the
correct actor without mutating the process-global `SHORE_ACTOR_ID` (which is `unsafe` and racy under
edition 2024). The chosen actor is part of a fact's content-addressed identity, so distinct actors
produce distinct facts.

Validation checks target a captured revision only. `ValidationAddOptions` resolves either an
explicit revision, a supersession-thread head, or the single current revision, then constructs the
revision validation target internally. `ValidationListOptions` filters by revision, track, and
status, and `with_include_body(true)` hydrates validation summaries. Validation evidence is
advisory; it does not accept, reject, merge, block, or replace a review assessment.

**Shared-store write resolution.** By default every worktree of a clone resolves the same shared
common-dir store (`.git/shore`) for both reads and writes, with no setup step. Every review write
workflow above (`record_observation`, `open_input_request`, `respond_input_request`,
`record_assessment`, and `record_validation_check`) validates and derives against
that store, and the write itself lands directly in it (`resolve_write_store` resolves the same store
the reads do), so a consumer can record a fact against a revision, observation, assessment, or
input request captured in a sibling worktree, and the fact is visible to reads from every worktree in
place. An `ephemeral` worktree instead resolves its own discardable worktree-local `.shore/data/`
store; the validation set reduces to that store's event list.

`respond_input_request` answers **revision** input requests (the reviewer-to-author loop). Agent
**task-attempt** input requests — the resumption domain that feeds ADR-0009 binding — are a separate
input-request flavour, authored and answered by the agent session / relay rather than by this
review-fact command; passing a task-attempt request id to `respond_input_request` is rejected with a
domain-boundary error. A future task-attempt response writer that wants cross-worktree validation can
route through the same `resolve_write_validation_store` seam — it is domain-agnostic.

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
assertion mode. `sourceRef` and `ingest` are unsigned hop metadata; bridges may add or change
`sourceRef`, and import seams stamp `ingest`, without changing the producer signature. The protocol media type still contains `event-tbs.v1` for
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
and payload hash but a different signer or signature, Shoreline keeps the first stored event and,
when the incoming copy carries a resolvable attestation, transcribes it into a detached
co-signature carrier (an unsigned divergent duplicate transcribes nothing); the affected input row
reports `write_outcome: existing_divergent_signature`. Other metadata differences with the same
payload hash remain an idempotent existing event. Signatures authenticate the producer facts; they
do not choose an automatic conflict winner.

`FileEd25519Signer` (`shoreline::keys`) is the production `EventSigner`: an Ed25519 key loaded from
the user-level keystore (`shoreline::keys::{generate_key, load_signer, list_keys}`). Signing over a
loaded key is infallible — the only fallible work (resolving the key home, reading and decoding the
key file) happens at load time, before the signer exists. **Signer resolution lives in the CLI
layer, not the library**: the library seam `sign_event_if_requested` returns `Result` and propagates
errors via `?`, so all fallible resolve/load/validate work happens CLI-side *before* `.sign_with(...)`
and the workflow only ever signs with a known-good signer. That placement is why **signing never
gates a write** — every resolution failure degrades to an unsigned write at exit 0 with a named
diagnostic. The library seam is unchanged; there is no library entry point for resolution.

`SshAgentSigner` (`shoreline::keys`) is the **second** production `EventSigner`: it signs by shipping
the DSSE PAE bytes to ssh-agent, so its `sign_event_message` is the **fallible** (network) one, unlike
the file signer's infallible local sign. `sign_with` and the `EventSigner` trait are **unchanged** —
the CLI resolution layer carries either signer as a boxed `dyn EventSigner` (a blanket impl lets the
unchanged generic `sign_with` accept it), and a tightly-scoped sign-time degrade keeps never-gates true
for the network signer. See [ADR-0010](./adr/adr-0010-actor-identity-and-delegation.md).

### Actor identity and delegation — `shoreline::session`

Verification answers "is this event authentic?"; delegation answers the orthogonal question "whose
responsibility is this agent's write?". The two are independent — an unsigned local agent event can
resolve a principal, and signing never gates a write.

| Symbol | Purpose |
| --- | --- |
| `DelegationMap` / `delegation_map_from_value` / `DelegationMap::from_delegates_file` | Parse a checked-in `.shore/delegates.json` map (top-level `delegates` key, unknown keys ignored), reader-supplied like `TrustSet`. |
| `DelegationMap::resolve` / `PrincipalResolution` / `UnresolvedReason` | Resolve an agent actor's principal at an event `occurredAt` over half-open validity windows: `Resolved` / `None(reason)` / `Ambiguous`. |
| `PrincipalView` / `PrincipalStatus` / `PrincipalSource` | The serialized principal object `{actorId, status, source}` that rides beside `writer` in projections; `principal_view_for` builds it (only for `actor:agent:*` writers), `principal_display_label` renders `claude-code (for kevin@swiber.dev)`. |
| `with_delegation_map` | Thread a `DelegationMap` into a read — on `ReviewHistoryOptions` and `RevisionShowOptions`, and as a parameter to the leaf document builders — beside `with_trust_set`. |
| `PrincipalPolicy` / `principal_sufficient` | Reader-side principal-sufficiency policy (`none` default / `prefer` / `require-resolvable-principal`), composed conjunctively beneath ADR-0009's resumption binding predicate — narrowing only. |

Every event's envelope carries `writer.producer` (`{name, version}`), the producing software that
wrote the event. (Pre-release stores that used a `writer.tool` object are rejected on read with a
typed migration error; see [storage-model.md](./storage-model.md).) The delegation map and principal
policy are reader-supplied config the agent does not control, so consuming them never trusts a
self-asserted field. See [ADR-0010](./adr/adr-0010-actor-identity-and-delegation.md).

### Event ingest — `shoreline::session`

| Item | Purpose |
| ---- | ------- |
| `ingest_events` + `IngestEventsOptions` / `IngestEventsResult` | Ingest pre-formed `ShoreEvent`s (forwarded over a network or merged from another clone), preserving append-only / content-addressed / idempotent + conflict semantics. |
| `import_event` + `ImportEventOptions` | Single-event convenience over `ingest_events`. |
| `IngestEventVerification` | One row per verified event: `event_id`, `status`, `message`, and `write_outcome: Option<EventWriteOutcome>` — how the store resolved that event's write. |
| `EventWriteOutcome` | The public per-event write resolution: `created`, `existing`, or `existing_divergent_signature`; serde, `as_str()`, and `Display` all use the snake_case wire strings. |
| `shoreline::session::event::ShoreEvent` (+ `EventType`, `Writer`, payload types) | The event envelope; `Serialize` + `Deserialize`, so events can be forwarded as JSON. |
| `IngestProvenance` / `IngestVia` | The optional `ingest: { via, receivedAt }` envelope sibling stamped by import seams ([ADR-0009](adr/adr-0009-resumption-binding-trust-source.md)). |

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

When a strict policy rejects an event, `ingest_events` / `import_event` return
`ShoreError::EventVerificationRejected { event_id, status }` before anything is written, so
consumers classify the rejection on the variant and read the offending event id and
`EventVerificationStatus` without parsing message text. The rendered message is unchanged:
`event signature verification rejected event <event_id> with status <status>`.

`IngestEventsResult.verification` rows appear in input order for non-carrier events; a stored
detached co-signature carrier appends its row when the write loop stores it, and a dropped
carrier has no row. On a successful ingest every row's `write_outcome` is populated, so a
forwarding consumer can answer both questions per row — did it verify, and what did the store do
with it — without parsing diagnostics. Divergent-signature semantics are unchanged (first stored
wins; this surface is observability only).

Every event written through `ingest_events` / `import_event` or store bundle import is stamped
with ingest provenance — `ingest: { via, receivedAt }`, with `via` naming the seam
(`ingest-events` or `bundle-apply`) — overwriting any inbound stamp. The stamp is outside the
to-be-signed view, so stamping never invalidates a signature, and re-ingest keeps the first
stored stamp state ([ADR-0009](adr/adr-0009-resumption-binding-trust-source.md)).

Events folded into the shared common-dir store by `import_store_bundle` (the seam `shore store
migrate` uses) carry that same ingest provenance (`via: "bundle-apply"`), and binding decisions are a
pure function of the events actually read. An unsigned input-request response binds via possession
only inside the store that locally wrote it: once it is read back as a bundle-stamped copy, the
response projects as non-binding with reason `ingested_unsigned`. The predicate never consults which
workflow or worktree produced the event, only its stamp and signature, so a response signed by a
verified, authorized signer binds identically from any store. Sign responses that must stay binding
after a migration into the shared store.

### Sensitivity vocabulary — `shoreline::session`

`store_status` reports a redacted worktree sensitivity scan. The scan's vocabulary is a typed
public contract so downstream boundaries (for example a relay's egress classification gate) can
classify findings without vendoring string literals:

| Item | Purpose |
| ---- | ------- |
| `SensitivityKind` | The five finding classes: `known_token`, `private_key`, `high_entropy`, `sensitive_filename`, `generated_path`. `severity()` and `policy_outcome()` pin each kind's scanner assignment. |
| `SensitivitySeverity` | `medium` < `high` (derived ordering). |
| `SensitivityPolicyOutcome` | The combined-outcome lattice `allow` < `warn` < `block`; `combine(...)` folds finding outcomes with `max` (identity `allow`), so a repository's combined outcome is `block` > `warn` > `allow` by dominance. |

All three enums serialize to those snake_case wire strings (serde and `as_str()` agree), parse
back via `parse(...)`, and enumerate via `ALL`. The scan document itself
(`StoreStatusSensitivity` / `StoreStatusSensitivityFinding`) keeps plain-`String` fields on the
wire; the enums are the vocabulary those strings are drawn from, and
`tests/fixtures/sensitivity/conformance-vectors.json` pins the agreement: seeded vectors, the
expected `(kind, severity, policyOutcome)` rows for a positive (`block`) and a negative (`allow`)
repository, exercised against the real scanner by `tests/sensitivity_conformance.rs`. Boundary
implementations can assert identical per-kind behavior from the same file.

One deliberate divergence is part of the contract: shoreline's `known_token` detector matches the
`sk-`/`ghp_`/`github_pat_`/`AKIA` prefixes only at the start of a token, while shoreline-relay's
egress gate matches the prefix anywhere in a token (its threat model is "the secret must not
cross the wire", not "the token starts with a known prefix"). The fixture's embedded-prefix
vector records both expectations: no shoreline finding, `known_token` at the relay.

### Artifacts — `shoreline::session`

| Item | Purpose |
| ---- | ------- |
| `referenced_artifacts` | Enumerate the content-addressed artifacts required by a set of forwarded `ShoreEvent`s. |
| `ArtifactRef` / `ArtifactKind` | Opaque artifact references. Consumers can branch on kind and fetch by `content_hash()` without depending on store paths. |
| `export_artifact` | Read and hash-verify one referenced artifact's bytes from a source store. |
| `import_artifact` + `ImportArtifactOptions` / `ImportArtifactResult` / `ImportArtifactOutcome` | Hash-verify and idempotently write one referenced artifact into a destination store. |

`ingest_events` transfers events only. Events can reference object artifacts and large note-shaped
body artifacts, and those blobs must be transferred separately before reads that need them. A full
mirror flow is:

1. read or receive a batch of `ShoreEvent`s;
2. call `referenced_artifacts(&events)` to learn which artifact hashes are required;
3. fetch or `export_artifact` those blobs from the source store;
4. `ingest_events` into the destination store;
5. `import_artifact` each fetched blob into the destination store.

After events and artifacts are present, `show_revision` can load the bound object artifact and
`fetch_input_request(...with_include_body(true))` / include-body projections can hydrate large
bodies. The store layout remains private; callers should keep and pass around `ArtifactRef` values
rather than constructing paths. A remote bridge derives those refs from the forwarded events it
already has, fetches bytes by `ArtifactRef::content_hash()`, and loops over `import_artifact`;
callers do not construct refs from a raw hash alone. This byte-level transfer path complements the
shared common-dir store: every worktree of a clone already shares one local store, while remote or
networked consumers can fetch and import the required blobs by content hash.

Signature validity does not imply artifact availability. A signed event can be `valid` while its
referenced object or note-body artifact is unavailable, and an available artifact can be attached
to an unsigned event. Consumers that need a full-fidelity mirror should verify both event signatures
and artifact transfer separately.

### Documents — `shoreline::documents`

The `shoreline::documents` module produces the documented `shore.review-*` command-output documents,
**byte-identical** to the `shore` CLI:

- Envelopes: `DiagnosticDocument<T>`, `EventWriteDocument<T>` (schema/version/diagnostics, plus
  event-write counts).
- Per-command builders: `revision_show_document`, `revision_list_document`, `capture_document`,
  `observation_add_document`, `observation_list_document`, `input_request_open_document`,
  `input_request_list_document`, `input_request_fetch_document`, `input_request_respond_document`,
  `assessment_add_document`, `assessment_show_document`, `validation_add_document`,
  `validation_list_document`, `history_document`, and the body/view document types they return.

A consumer that wants exactly the documented JSON contract calls a read/write workflow, passes the
typed result to the matching builder, and serializes the document with `serde_json`.

### Identifiers — `shoreline::model`

The content-addressed id newtypes (`ActorId`, `RevisionId`, `ObjectId`, `InputRequestId`,
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
