# ADR-0039: Exact Logical Bundles and Separate Import Receipts

**Status:** Accepted (owner-approved 2026-07-19)
**Date:** 2026-07-19
**See also:** [ADR-0004](./adr-0004-event-signatures.md),
[ADR-0016](./adr-0016-content-targeted-artifact-removal-and-compaction.md),
[ADR-0020](./adr-0020-durable-storage-backend-seam.md),
[ADR-0027](./adr-0027-at-rest-encryption-boundaries.md),
[ADR-0037](./adr-0037-immutable-review-generations-and-fact-continuity.md), and
[ADR-0038](./adr-0038-relation-proof-and-auxiliary-document-resources.md).

## Context

The current bundle importer records local possession by changing imported event bytes. That is useful local
history but it prevents exact replication: identity-bearing decoded bytes and destination provenance occupy
the same document. The qualification prototype separates them and proves complete preflight, exact event
bytes, content-first publication, hard conflicts, and idempotent retry
(`src/bench_support/foundation/bundle_v2.rs:14-35,233-286`; `src/bench_support/foundation/receipt.rs:30-49`).

Logical transfer is also distinct from physical backup. A logical bundle must cross storage profiles without
row IDs, WAL state, paths, segment generations, offsets, or encoded-carrier identity. A physical backup must
restore one coherent candidate root with all of its physical state.

## Decision

### D1. Exact bundle v2 carries selected decoded records and closure

The logical document is `pointbreak.exact-bundle.v2`:

```text
ExactBundleManifestV2 {
  schema,
  source_manifest_sha256,
  required_capabilities,
  events[],
  content[],
  closure[],
  event_set_sha256,
  bundle_sha256,
}
```

Each record carries a domain logical key, record kind, exact decoded bytes, and decoded SHA-256. Events and
content are sorted by logical key. Closure is sorted by event key and lists every required child content key.
`event_set_sha256` covers exact event records; `bundle_sha256` covers the schema, source manifest, logical
capabilities, events, content, closure, and event-set hash. Neither hash covers destination-local receipt
data.

The manifest contains no physical path, filename, SQLite coordinate, WAL/SHM state, segment offset or
generation, encoded carrier, backup marker, or destination receipt. Encoded-byte equality is not logical
identity; every destination verifies decoded bytes and re-encodes through its own strict writer.

### D2. Reconciliation is backend-neutral and first-stored-wins

Backend-neutral reconciliation keys every record by `(logical_key, decoded_sha256)` and confirms decoded
bytes directly before returning idempotent `AlreadyExists`. A same logical key with a different hash or bytes
is a divergent-key hard conflict. It is never overwritten, timestamp-selected, or last-writer-wins.

Omission never deletes destination state. Exact bundle v2 is selected disclosure, not a whole-store desired-
state manifest. Deletion and physical erasure continue through their explicit event and maintenance
contracts.

### D3. Preflight is complete; publication is content-first and event-last

Before the first target write, import validates schema and both hashes, logical capabilities, record kinds,
decoded hashes/bytes, key uniqueness, recursive closure, and every existing-key conflict. Any failure writes
zero content and zero events.

After successful preflight, all required content publishes before the first event. Events publish only after
content succeeds. A retry after ambiguous acknowledgement is idempotent under the exact same-key/same-bytes
rule. Missing semantic targets that the logical cohort explicitly permits may remain visible for federation
backfill; missing required content closure may not.

### D4. Import provenance is a separate durable operational receipt

The selected receipt policy is a separate import receipt in durable operational state, not a synthesized
journal event and not a field added to imported events. It records:

```text
ImportReceiptV1 {
  schema,
  source_bundle_sha256,
  source_event_set_sha256,
  source_event_sha256[],
  local_import_context,
  receipt_sha256,
}
```

The source event hashes are sorted. The receipt digest covers every field except itself. Local context is
never exported as source event identity. The durable-store owner exposes receipt lookup, includes receipts
in candidate-native backup, restore, inventory, and copy-out repair, and may prune them only under an
explicit operational retention policy.

A local provenance event is rejected for v1 because it changes the destination event set and recursively
creates provenance on re-export. If future product behavior needs shareable possession claims, that is a
new attributed event family and not an import side effect.

### D5. Logical replication, physical backup, and archive copy remain distinct

- Logical replication transfers exact decoded records through this backend-neutral contract.
- Candidate-native backup captures one coherent physical root, including profile descriptor, journal side
  state, content, receipts, and operational metadata.
- An immutable archive copy moves the completed backup as opaque carriers to a fresh prefix, verifies its
  carrier manifest, and publishes the manifest-bound completion marker last.

Physical files are not a synchronization protocol. A writable local-filesystem live root is the only
eligible live-root policy for this cohort. Network and synchronizing filesystems fail or advise before
mutation according to platform consequences fixed by the eventual physical profile. Remote archival may
copy a completed backup opaquely; remote multi-writer convergence, a broker, a sync server, and a cloud
control plane remain separate decisions.

### D6. Capability and legacy boundaries fail closed

The bundle advertises the complete logical capability epoch/set. Import rejects an incomplete reader or
writer before committing any member. Export/import, removal, repair, backup, and migration must understand
generation continuations, relation attestations, fact ports, independent relation proof, and independent
auxiliary document closure before the production writer emits them.

Migration requires legacy-byte preservation of events, identities, hashes, signatures, and attribution.
There is no semantic backfill: migration does not invent continuations, attestations, ports, documents, or
content equality. The source remains recoverable; after the first new-format write, rollback is forward
repair rather than silently reopening the old source.

### D7. This decision does not select a physical profile

SQLite WAL and bounded segments remain qualification implementations only. A physical profile, format,
limits, sync policy, backup mechanism, repair details, dependency version, and platform consequences may be
frozen only after one candidate passes every required hard gate. No runtime selector, public migration
command, live-root cutover, or production new-event writer follows from this ADR.

## Consequences

### Accepted

- Imported events remain byte-identical, so IDs, signatures, record hashes, and event-set roots survive.
- Local import provenance remains queryable without contaminating transferred truth.
- The same logical bundle works across future physical profiles.
- Transfer failures are atomic before events; retry, conflict, closure, and omission behavior are explicit.
- Backup and off-site archive can remain profile-native without becoming logical identity.

### Rejected

- Mutating imported events to stamp local provenance.
- Modulo-field or parse/reserialize equivalence for exact transfer.
- Raw selective transfer of physical database or segment carriers.
- Last-writer-wins, overwrite, or deletion by omission.
- A mutable archive prefix, physical live-root mirroring, or physical files as remote convergence.

## Revisit Triggers

- A shareable possession/receipt claim earns a separately reviewed attributed event family.
- A future remote adapter can admit exact records only through the same strict logical writer and conflict
  contract.
- A selected physical profile needs additional operational receipt fields without changing source-event or
  bundle identity.
