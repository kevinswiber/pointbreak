# ADR-0038: Relation Proof and Auxiliary Document Resources

**Status:** Accepted (owner-approved 2026-07-19)
**Date:** 2026-07-19
**See also:** [ADR-0002](./adr-0002-large-snapshot-artifact-policy.md),
[ADR-0016](./adr-0016-content-targeted-artifact-removal-and-compaction.md),
[ADR-0027](./adr-0027-at-rest-encryption-boundaries.md),
[ADR-0037](./adr-0037-immutable-review-generations-and-fact-continuity.md), and
[ADR-0039](./adr-0039-exact-logical-bundles-and-import-receipts.md).

## Context

A stronger commit relation needs independently auditable inputs, not ancestry, path overlap, or normalized
patch identity. Native editor projection also benefits from complete before/after documents, but those
documents are optional, privacy-sensitive evidence and must never replace the frozen object artifact.

The qualification prototypes already exercise canonical raw entries and six capture modes
(`src/bench_support/foundation/proof.rs:10-129`), plus explicit document retention, typed absence, exact
hash verification, and snapshot-before-diff mutation detection
(`src/bench_support/foundation/documents.rs:59-99,236-267`).

## Decision

### D1. Relation proof is an independent content resource

`RelationProofManifestV1` is canonical JSON with:

```text
schema = pointbreak.relation-proof.v1
algorithm
algorithm_version
generation_revision_id
object_artifact_content_hash
association_id
source: CanonicalProofInputV1
candidate: CanonicalProofInputV1
result: RelationProofResultV1
evidence_sha256
```

Each canonical input binds capture mode (`commit_range | root | staged | unstaged | combined_worktree |
synthetic_untracked`), base/parent, path scope, Git availability, and sorted raw entries. A raw entry binds
path/previous path, status, old/new OIDs, modes, decoded hashes, text/binary/symlink/submodule kind, and
untracked state. The evidence digest covers every field except itself.

The algorithms are versioned `exact_materialization`, `canonical_equivalent_rewrite`,
`content_preserving_extension`, and `attribution_only`. Results carry the semantic relation, proof status,
and exact additions. Candidate signals such as ancestry, path overlap, and stable patch ID always remain
`unknown + unverified` until a canonical algorithm runs.

### D2. Proof lifecycle is separate from relation history

The proof resource has `available | removed | missing` lifecycle states. Only an available, verified proof
is currently reproducible. Removal or loss does not rewrite the relation attestation; it changes the
evidence-availability projection and may withdraw stronger presentation authorization. The proof is never
the captured diff, a complete document source, or reviewable content.

### D3. Auxiliary documents are optional independent content

`AuxiliaryDocumentManifestV1` is canonical JSON with:

```text
schema = pointbreak.auxiliary-documents.v1
generation_revision_id
object_artifact_content_hash
retention_policy
entries[]
child_content_hashes[]
retained_decoded_bytes
manifest_sha256
```

Each entry names one canonical raw entry and `before | after` side, then records either:

- `retained { decoded_sha256, decoded_bytes, content_kind, encoding }`; or
- `absent { reason }`, where reason is `added | deleted | unavailable | policy_not_retained |
  file_limit_exceeded | capture_limit_exceeded | mutable_target_changed | not_applicable`.

Retained blobs are separate decoded-hash-addressed resources. Text uses UTF-8 only after validation;
binary and symlink content use raw bytes. Submodules have no document blob. Manifest and child hashes are
independent of physical paths or carrier encodings.

### D4. Retention is explicit, bounded, and privacy classified

Retention requires explicit consent, sensitivity (`public | internal | confidential | restricted`), a
finite per-file limit, and a finite per-capture limit. Capture snapshots a document before diff
materialization and verifies the same bytes afterward; a changed mutable target is absent/failing evidence,
never silently recaptured under the old generation. Non-retention is valid and produces typed absence.

An independent auxiliary document improves native presentation but never replaces the authoritative object
artifact, expands assessment scope, or becomes required for review truth. Evidence and emitted diagnostics
expose hashes, counts, kinds, states, and sanitized summaries only; private decoded bytes and private paths
do not enter logs or receipts.

### D5. Closure and logical capabilities are fail-closed

The capability cohort includes `relation_proof_v1`, `auxiliary_document_manifest_v1`, and
`auxiliary_document_blob_v1`. A bundle that includes a relation attestation or document manifest declares
its complete required child closure. Missing capability, missing child, hash mismatch, unsupported kind,
or bound violation fails before publication. Omission is not an assertion that an optional document is
absent; absence is explicit in the manifest.

Removal, repair, export/import, and migration preserve `available | removed | missing` distinctions and
recursive closure. No derived read index becomes the evidence or erasure authority.

## Consequences

### Accepted

- Content-qualified relation labels are reproducible across Git loss when retained proof inputs suffice.
- Complete documents can support native views without weakening the captured-diff authority.
- Privacy, consent, bounds, mutation races, typed absence, and lifecycle are part of the resource contract.
- Proof manifests, document manifests, and blobs transfer as ordinary independent logical records.

### Rejected

- Stable patch ID, ancestry, or path overlap as verified relation proof.
- Reconstructed hunks or live worktree files as exact document endpoints.
- Document packs, deltas, or shared plaintext carriers as logical identity.
- Silent document backfill, unbounded capture, or retention without explicit consent.
- Treating removed proof as if it were still reproducible.

## Revisit Triggers

- A new capture mode needs canonical status/blob/mode semantics not expressible by `CanonicalRawEntryV1`.
- A new content kind requires a privacy, encoding, closure, and native-projection contract.
- Proof retention cost warrants a smaller manifest that remains independently auditable.
