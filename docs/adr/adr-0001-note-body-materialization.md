# ADR-0001: Note Body Materialization Policy

**Status:** Accepted
**Date:** 2026-05-15
**Issue:** [#17](https://github.com/kevinswiber/shore/issues/17)

## Context

Shore stores note-shaped event bodies (observations, intervention bodies, intervention resolution
reasons, assessment summaries, imported review notes) using a threshold split: small bodies live
inline in the event payload, larger bodies are externalized to `artifacts/notes/<sha256(body)>.json`
under the `shore.note-body` envelope. Issue #17 asked whether note bodies should always materialize
as durable artifacts even when small, or stay on the current threshold model.

## Decision

Stay on the threshold model. Replay is authoritative.

- Bodies of byte length at most `BODY_INLINE_LIMIT` (4096 bytes today; see
  `src/session/store/body_artifact.rs`) remain inline in the event payload.
- Bodies above the threshold are externalized to `artifacts/notes/<sha256(body)>.json` with envelope
  `{"schema":"shore.note-body","version":1,"body":"..."}`.
- The event under `.shore/events/` is the authoritative durable record of every note. The artifact,
  when present, is a content-addressed sidecar.
- `.shore/artifacts/notes/` is an overflow store, not a complete inventory of note bodies. Tooling
  that wants a complete list of note bodies must replay events.
- `state.json` projects bounded counts (`note_count`, `observation_count`, `assessment_count`,
  `intervention_count`, plus open / blocking-intervention counts and projection diagnostics). It does
  not project body content, body sizes, or artifact paths.
- The 4096-byte threshold is internal storage tuning. It may change without a deprecation cycle.
  The inline-or-artifact bifurcation itself is a stable contract for storage consumers.

### Body-hash availability (asymmetric)

- Native-recorded payloads carry a hash: `ReviewObservationRecordedPayload.body_content_hash`,
  `InterventionRequestedPayload.body_content_hash`,
  `InterventionResolvedPayload.reason_content_hash`,
  `ReviewAssessmentRecordedPayload.summary_content_hash`.
- `ReviewNoteImportedPayload` does **not** carry a payload-level body hash. Imported-note artifact
  identity is content-addressed by `sha256(body)` in the artifact filename, but the event payload
  does not duplicate that hash.
- `load_body_artifact` validates the relative-path shape and the `shore.note-body` envelope's
  `schema` / `version` fields. It does **not** verify the loaded body against any event-payload
  hash. Hash-based cross-validation, where available, is a caller's responsibility.

## Consequences

### Adopted

- `.shore/artifacts/notes/` is an overflow store and is not a complete inventory of note bodies.
- Artifact-only tooling is not a supported authority; tools must replay `.shore/events/`.
- `state.json` continues to project counts only — never body content.
- The 4096-byte threshold is a tuning parameter and may move without a deprecation cycle.
- `body_byte_size` (and, where available, `body_content_hash` / `reason_content_hash` /
  `summary_content_hash`) gives tooling replay-derivable handles to body length and — for
  native-recorded payloads — identity, without re-reading artifacts. The materialization
  discriminator is `body` vs `body_artifact_path`, not `body_byte_size`: native ledger payloads
  currently set `body_byte_size = Some(_)` on the inline arm via the shared `staged_body` helper,
  while imported-note payloads leave it `None` inline.
- No migration is required. Existing repos may have any mix of inline and artifact-backed bodies;
  both are already supported on the read path.

### Implementation invariants pinned by tests

- `body_artifact.rs::body_of_exactly_inline_limit_bytes_returns_inline`
- `body_artifact.rs::body_of_inline_limit_plus_one_bytes_returns_artifact`
- `body_artifact.rs::body_inline_limit_is_the_documented_4096_bytes`
- `workflow/import.rs::extracted_note_records_keep_inline_body_at_threshold_and_externalize_at_threshold_plus_one`
- `tests/acceptance/session_state.rs::artifacts_notes_directory_is_not_a_complete_note_body_inventory`

## Alternatives Considered

### Option (b): always materialize every note body as a durable artifact

Rejected for these reasons:

1. **Doubled durable writes per body-bearing event.** Every observation, intervention body,
   intervention resolution reason, assessment summary, and imported note would emit an additional
   `Durability::Durable` write (one fsync per artifact, in addition to the event's existing fsync).
   Shore's expected workload has many short observations and assessment summaries; this multiplies
   file-count and fsync growth roughly 1:1 with the event log.
2. **No simplification on the read path.** Every consumer already handles both arms via a uniform
   `inline-or-load_body_artifact` fallback (see `observation_body`, `intervention_body`,
   `assessment_summary`, `optional_text`, `replay_note_entry`, `adapter_notes`). Switching to
   option (b) removes a one-arm match in those helpers; it does not eliminate the JSON-envelope
   deserialize.
3. **Materialization alone does not deliver artifact-authoritative tooling.** An artifact under
   `artifacts/notes/<sha256>.json` is anonymous — it carries body bytes and envelope
   schema / version, but no referrer ID. Without joining back to the event ledger, an
   artifact-only tool cannot answer "which note / observation / assessment does this body belong
   to?". Real artifact-authoritative tooling would require enlarging `NoteBodyEnvelope` to carry
   referrer IDs and reconciling consistency between the envelope and the event payload that names
   it — a significantly larger change than threshold tuning.
4. **No real consumer is currently requesting artifact-only enumeration.** Every read surface today
   (history, review-unit show, observation list, intervention list / fetch, assessment show,
   dump's adapter notes) replays events first. There is no `shore notes artifacts ls` CLI or
   library entry point that walks `artifacts/notes/` directly.

### Option (c): tune the threshold (e.g., to 16 KiB or to 0)

Out of scope for this ADR. The threshold is explicitly declared a tunable storage parameter;
raising or lowering it is a future decision that does not require revisiting the inline-or-artifact
bifurcation. Setting the threshold to `0` would in effect be option (b) and is rejected for the
same reasons.

## Future Reversal

If a future workload makes option (b) attractive — e.g., a tool that must enumerate all note bodies
without event replay — the migration shape is:

1. Extend `NoteBodyEnvelope` (or a new envelope) to carry referrer identity.
2. Walk existing `.shore/events/`; for every body-bearing event whose payload still carries an
   inline `body`, emit a new artifact under `artifacts/notes/` and an updated event (or a "body
   migration" event) that replaces the inline body with a `body_artifact_path`.
3. Update `stage_body_artifact` to always materialize (or invert the threshold).
4. Invert the relevant tests pinned by this ADR.

This migration is intentionally **not** in scope for the current issue. Recording the shape here
keeps the option open without committing to it.
