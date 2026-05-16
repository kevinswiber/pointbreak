# ADR-0002: Large Snapshot Artifact Policy

**Status:** Accepted
**Date:** 2026-05-15
**Issue:** [#64](https://github.com/kevinswiber/shore/issues/64)
**See also:** [ADR-0001](./adr-0001-note-body-materialization.md) (note-body materialization, parallel decision)

## Context

Shore stores captured review-unit diffs as identifier-hashed artifacts under
`artifacts/snapshots/<sha256(snapshotId)>.json`. The artifact body carries the full row
inventory inline: `snapshot.files[].hunks[].rows[]` is one row object per added, removed,
or context line. A newly added 10,000-line text file produces one artifact with roughly
10,000 row objects. The artifact also carries a self-describing canonical `contentHash`
that the read path recomputes and compares on every load
(`src/session/store/snapshot_artifact.rs::validate_snapshot_artifact_content_hash`).

Issue #64 asked Shore to decide V1 policy on five concrete questions about how the
snapshot artifact should behave at scale.

## Decision

V1 keeps the current behavior. The snapshot artifact remains the authoritative carrier of
every captured row; no elision, no generated-file detection, and no new content-omission marker
beyond the existing `FileMetadataKind::BinarySummary`. The other V1 metadata-row variants
(`RenameSummary`, `ModeChange`, `SubmoduleSummary`) describe file-level Git facts and continue
to be emitted as today; they are not content-omission markers. Each of the five issue-#64
questions receives an explicit answer below.

- **Q1: Should large text files be elided past a size or line-count threshold?**
  No. V1 stores every captured row inline. A real workload producing snapshot artifacts
  too large to load on the read path is the trigger for a V2 elision plan; absent that
  trigger, ratifying the simplest possible model keeps `shore review unit show`'s
  "snapshot-complete" contract honest.
- **Q2: Should generated files be detected and marked metadata-only?**
  No. Shore does not parse `.gitattributes`, look at `linguist-generated` markers, or
  apply any heuristic. Detection rules and their interaction with review-stream noise
  are V2 design decisions.
- **Q3: Should binary / too-large / elided files produce explicit metadata rows?**
  Partial. `FileMetadataKind::BinarySummary` is the V1 content-omission marker — ingest
  emits it for binary patches and leaves `hunks` empty (`src/git/ingest.rs`). The remaining
  V1 variants (`RenameSummary`, `ModeChange`, `SubmoduleSummary`) carry file-level Git
  facts rather than mark omitted content, even though ingest also leaves `hunks` empty
  for them. No new variants for "too-large" or "elided" — both would require V1 to commit
  to a threshold and a marker shape that V2 may want to redesign.
- **Q4: Should `shore review unit show` preserve file presence while omitting row bodies?**
  No. All captured rows continue to flow through `build_snapshot_rows`
  (`src/session/workflow/review_unit_projection/rows.rs`) into the
  `snapshot_remainder` projection phase. A `--omit-row-bodies` flag (or an automatic
  mode keyed off snapshot byte size) is a CLI surface change; it belongs in a separate
  issue that owns the `shore.review-unit` JSON contract impact.
- **Q5: What does this mean for snapshot `contentHash` stability?**
  `SnapshotArtifact.contentHash` covers the full row inventory today. Any V2 elision must
  either bump `SNAPSHOT_ARTIFACT_VERSION` (currently `1`) or add a separate
  `contentHashScope` field, so consumers can tell whether a given hash was computed under
  the V1 (full row inventory) or a V2 (elided) scope. Silently changing the hash scope
  under the same version is rejected.

## Consequences

### Adopted

- `artifacts/snapshots/<sha256(snapshotId)>.json` is the authoritative carrier of every
  captured row. There is no overflow store for snapshot rows; the artifact is a single
  file per snapshot.
- `SnapshotArtifact.contentHash` covers the full row inventory under V1. The hash is
  recomputed on every read and rejects tampered or partially written artifacts.
- The current set of `FileMetadataKind` variants —
  `{ BinarySummary, ModeChange, RenameSummary, SubmoduleSummary }` — is the V1 surface.
  Adding a new variant is a V2 decision.
- `shore review unit show` continues to return narrative-first plus snapshot-complete
  rows. Every captured file, metadata row, hunk header, and diff row appears in the
  `snapshot_remainder` phase.
- No migration is required.

### Implementation invariants pinned by tests

- `snapshot_artifact.rs::snapshot_artifact_schema_is_pinned_at_shore_snapshot_v1`
- `snapshot_artifact.rs::captured_text_rows_remain_inline_in_snapshot_artifact`
- `tests/acceptance/git_ingestion.rs::ingest_only_emits_v1_file_metadata_kinds`, together
  with its sibling `variant_is_v1` helper, an exhaustive `match` over `FileMetadataKind`
  with **no wildcard arm**. The match is the load-bearing compile-time tripwire: adding
  any new variant to `FileMetadataKind` will fail to compile here, and that compile
  error is the entry point for revisiting this ADR.
- `tests/cli_review_capture.rs::capture_preserves_inline_rows_for_normal_added_file`

## Alternatives Considered

### Option (b): adopt a row-count elision threshold in V1

Rejected. Choosing a threshold without a real workload to anchor it commits Shore to a
number (1k? 10k? 50k?) that V2 will almost certainly want to change. Worse, an
elision-bearing artifact under the current `SNAPSHOT_ARTIFACT_VERSION = 1` and the
current `contentHash` scope would change what `snapshotArtifactContentHash` means for
downstream consumers (`shore.review-capture` already exposes it; see
`tests/cli_review_capture.rs`). The honest path is to declare the V1 scope explicitly
and require a versioning decision in V2.

### Option (c): add `FileMetadataKind::ElidedFile` and `GeneratedFile` variants now, kept dormant

Rejected. Dormant enum variants are dead code; they would emerge through `serde` into
`shore.review-unit` JSON the moment any caller exercised them, and the V2 plan would
inherit a variant whose semantics it had not yet decided. Better to land the variant in
the same V2 plan that emits it.

### Option (d): expose a `--omit-row-bodies` flag on `shore review unit show` now

Rejected. The flag would have no behavior under V1 (there is nothing to omit beyond what
the snapshot already inlines) and would foreclose V2 design choices about the relationship
between elision (storage-side) and omission (display-side).

## Future Reversal

If a future workload makes elision attractive, the migration shape is:

1. Bump `SNAPSHOT_ARTIFACT_VERSION` from `1` to `2` (or add an explicit
   `contentHashScope: String` field; choice is a V2 design decision).
2. Add the new `FileMetadataKind` variant(s) for elided/generated/too-large files, and
   emit them from `src/git/ingest.rs::metadata_rows` under the new threshold rule(s).
3. Recompute `SnapshotArtifact.contentHash` for new artifacts under the new scope; decide
   whether legacy v1 artifacts get a dual-read path or a one-time recapture, and document
   the choice in the V2 plan.
4. Update the `shore.review-unit` command-output contract if the V2 mechanism is
   user-visible (e.g., a new `--omit-row-bodies` flag or an `elided` summary field).
5. Invert the relevant tests pinned by this ADR. Each pinned test names the V1 invariant
   in its assertion, so the migration's test-side diff is small and reviewable.

This migration is intentionally **not** in scope for the current issue. Recording its
shape here keeps the option open without committing to any specific V2 design.
