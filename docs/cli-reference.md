# CLI Reference

This reference covers the public `shore` command surface provided by the `shoreline` crate.

Command output JSON is the integration surface. Raw event files, artifact paths, event filenames,
and `.shore/data/state.json` are internal storage details unless a command explicitly returns them.

## Global Tracing Flags

Most commands accept optional tracing flags:

```bash
--log <filter>
--log-format <compact|pretty|json>
--log-file <path>
```

Tracing writes to stderr by default. When stdout is piped into JSON tools, prefer
`--log-file <path>` so trace lines do not corrupt the JSON stream. `shore show` requires
`--log-file` when tracing is enabled because it runs a raw-mode TUI.

When `--log-file <path>` points inside the repository, Shoreline treats that path as command-helper
plumbing for the current command and excludes it from the reviewed snapshot and fingerprint.

## Actor Identity and Delegation

Every write records a writer `actorId`. By default it derives from the local Git identity
(`actor:git-email:<email>`, then `actor:git-name:<name>`, then `actor:local`). Set
`SHORE_ACTOR_ID` to write under an explicit identity — agents use `actor:agent:<agent-name>`:

```bash
export SHORE_ACTOR_ID="actor:agent:claude-code"
```

`SHORE_ACTOR_ID` outranks the Git identity on every CLI write path, including paths without a
per-call override; a malformed value is ignored and falls through rather than corrupting
provenance.

Review read commands (`history`, the `observation` / `input-request` / `assessment` / `validation`
list and show commands, `unit show`, and the inspector) discover a checked-in delegation map at
`<repo>/.shore/delegates.json` and resolve the human principal an agent wrote on behalf of,
rendering it beside the writer as `claude-code (for kevin@swiber.dev)`. Discovery is presence-based
— absent file, no change. A malformed `.shore/delegates.json` prints a single warning to stderr and
the read proceeds with no resolution (advisory, never blocking). The file format is documented in
[storage-model.md](./storage-model.md).

## `shore show`

```bash
shore show [--repo <path>] [--review-notes <path>]
```

`shore show` opens a read-only terminal review view over the same headless review stream used by
`shore dump`.

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--review-notes <path>` loads Shoreline-native `review-notes.json`.
- Without an explicit sidecar, repo-only `shore show` auto-loads durable imported notes from
  `.shore/data/` when the store exists.
- Press `r` to reload the working-tree projection without losing cursor position when possible.
- Stale and orphan review notes appear as dedicated rows.
- Explicit sidecar inputs and `--log-file` are command helpers and are excluded from the reviewed
  snapshot for that command.
- The view is read-only; it does not mutate notes or write session state.

Keybindings: `q`/Esc/Ctrl+C quits, `j`/`k` or Up/Down moves by row, `[` and `]` move through diff
sections, and `{` and `}` move through noted sections.

## `shore dump`

```bash
shore dump [--repo <path>] [--review-notes <path>] [--pretty | --compact]
```

`shore dump` emits the JSON contract over the headless review stream.

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--review-notes <path>` loads Shoreline-native `review-notes.json`.
- Without an explicit sidecar, repo-only `shore dump` auto-loads durable imported notes from
  `.shore/data/` when the store exists.
- Output is compact by default; use `--pretty` for human-readable formatting.
- Recoverable review-note diagnostics stay in the JSON document and the command exits
  successfully.
- Fatal errors, such as unreadable files or malformed JSON, are written to stderr and exit
  non-zero.

Each `stream.rows[*].kind` value is the externally tagged JSON representation of the row-kind enum:
a single-key object such as `file_header`, `hunk_header`, `diff`, `metadata`, `note`,
`stale_note`, or `empty_state`.

Row IDs are opaque strings. They are stable and unique within one built review stream and safe as
keys, but callers must not parse them, derive ordering from them, or rely on their prefix/width. Use
the sibling `ordinal` field for numeric position.

## `shore inspect`

```bash
shore inspect [--repo <path>] [--host <addr>] [--port <n>] [--open]
```

`shore inspect` starts a small local web server that visualizes a `.shore/data` store for tracing event
timelines and outcomes — the kind of inspection that is awkward against the raw JSON files or
per-command output.

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--host <addr>` defaults to `127.0.0.1` (loopback). `--port <n>` defaults to `7878`; use
  `--port 0` to bind an ephemeral port, which is then printed.
- `--open` launches the inspector in the default browser after the server starts.
- The server runs until interrupted with Ctrl-C.

The page provides a chronological event timeline (filterable by track, ReviewUnit, lineage, and
event type, newest-first by default), a per-event detail view, a composite per-ReviewUnit page showing
the current-assessment status plus grouped observations, input requests, assessments, and lineage/head
badges, a ReviewUnit lineage list/detail view showing heads, threaded rounds, diagnostics, and stale
older-round facts, and the captured diff for a ReviewUnit annotated with the review facts anchored to
each line. Validation checks recorded with `shore review validation add` appear throughout — as a
labeled timeline event type, a "Validation checks" section on the ReviewUnit page (with the check
name, status, trigger, and exit code), and on lineage round cards — shown for context only; they do
not affect the current assessment and carry no merge or acceptance authority. Long IDs render as
truncated, clickable references that navigate to the resource they name, and the page auto-refreshes
when the store changes or a freshness diagnostic appears or clears.

The inspector is a read-only, single-store, localhost developer tool. It reads through the same
validated projections as `shore review history` and `shore review unit show` rather than parsing raw
storage, and it serves over a synchronous, dependency-free HTTP server with no async runtime. The
small JSON API the page consumes (`/api/history`, `/api/units`, `/api/unit`, `/api/lineages`,
`/api/lineage`, `/api/snapshot`, `/api/freshness`) is an internal surface for the bundled page, not
a stable contract.

In a linked worktree the inspector serves the clone-local store, so it can render snapshots
captured in sibling worktrees. The snapshot payload therefore omits the captured worktree path:
`target.worktreeRoot` is removed from the response after content-hash validation, with
`worktreeRootRedacted: true`, `contentHashScope: "stored-artifact"`, and a path-private
`targetDisplay` block marking the redaction. The stored snapshot artifact and its content hash are
unchanged, so re-validating the hash means fetching the artifact, not hashing the response JSON.
`shore review unit list` JSON still carries `target.worktreeRoot`, unchanged.

## `shore review capture`

```bash
shore review capture [--repo <path>] [--base <rev> [--target <rev>]] \
  [--lineage <lineage-id>] [--predecessor <review-unit-id>] [--change-id <change-id>]
```

`shore review capture` records the current V1 ReviewUnit: the base endpoint, target endpoint, and
captured diff snapshot. By default V1 captures the local Git worktree from `HEAD` to the working
tree, including untracked files (source `git_worktree`).

- With `--base <rev>`, capture instead records the committed range from `<rev>` to `--target`
  (default `HEAD`) as a `git_commit_range` source. Both revs are resolved with `git rev-parse` to
  commit OIDs at capture time: annotated tags peel to their commit, and a rev that does not exist or
  does not name a commit (a blob or tree) is rejected with an error that names the rev. The snapshot
  is the `base..target` tree diff with no working-tree, index, or untracked involvement, so both
  endpoints serialize as `git_commit` and no worktree path appears in the output. `--target`
  requires `--base`. Re-capturing the same range is idempotent and reports `eventsExisting`, and an
  equivalent rev spelling (`HEAD~1` versus the resolved OID) captures the same ReviewUnit because
  rev spellings are never stored. The `clone_local_capture_batch_only` diagnostic applies the same
  as for worktree capture.
- Durable state is created at the Git worktree root under `.shore/data/`. A legacy flat `.shore/`
  store from before this layout is upgraded with `just migrate-store [<repo>]` (an owner-run
  one-off driver, not a `shore` subcommand); see
  [storage-model.md](./storage-model.md#migrations-and-doctor).
- The command registers `.shore/data/` in the repository-local `.git/info/exclude`
  when it is not already ignored, so it never modifies a tracked `.gitignore` or
  dirties the working tree. This applies to every writer-initializing command
  (capture, observation, input-request, assessment, validation), not just `capture`.
  Committed config siblings (`.shore/delegates.json`, `.shore/allowed-signers.json`) stay tracked;
  only `.shore/data/` and the private `.shore/delegates.local.json` override are excluded.
- `.shore/data/events/` stores immutable local event files.
- `.shore/data/state.json` is a rebuildable projection, not the authority.
- Full captured snapshots are Shoreline-owned immutable artifacts under
  `.shore/data/artifacts/snapshots/`.
- The `review_unit_captured` event binds to the snapshot artifact's canonical content hash.
- Output is compact `shore.review-capture` JSON and includes ReviewUnit, revision, and snapshot IDs
  plus `snapshotArtifactContentHash`.
- With `--lineage`, capture immediately records lineage declaration/round facts for the newly
  captured ReviewUnit. `--predecessor` is allowed only with `--lineage`.

V1 `.shore/data/` storage is local and synchronous. It assumes one active Shoreline writer per `.shore/data/`
directory and does not add a daemon, delivery queue, approval flow, async storage, remote storage,
or note mutation.

When a worktree is linked to a clone-local store, `shore review capture` still writes the capture to
that worktree's local `.shore/data/` store. The command includes a
`clone_local_capture_batch_only` diagnostic that tells callers to run `shore store link` to copy the
new local facts into the linked clone-local store.

The native review write commands — `shore review observation add`, `shore review input-request open`
and `respond`, `shore review assessment add`, `shore review validation add`, and `shore review
lineage attach` — behave the same way in a linked worktree. They validate against the linked family's
review record plus your unsynced local facts, so you can record a fact against a review unit (or
related observation, assessment, or request) captured in a sibling worktree, but the fact is written
to your worktree-local `.shore/data/` store. In linked mode the result carries the
`clone_local_fact_batch_only` diagnostic; run `shore store link` to copy the fact into the
clone-local store so other checkouts can see it. The diagnostic is linked-mode only — unlinked output
is unchanged.

## `shore store`

```bash
shore store status [--repo <path>] [--pretty]
shore store link [--repo <path>] [--pretty]
```

`shore store` commands inspect or connect the current Git worktree to a clone-local store. A
clone-local store is shared by Git linked worktrees from the same clone; it is not a user-level
multi-repository store or remote sync service.

`shore store status` resolves the selected store and emits `shore.store-status` JSON.

- `mode` is `local` for the default worktree-local `.shore/data/` store and `linked` when the worktree
  has been registered with a clone-local store.
- `storeRef` is `worktree-local` in local mode and an opaque `store:random:*` ref in linked mode.
  Linked output also includes opaque `cloneRef` and `repositoryFamilyRef` values.
- `inventory` reports `eventCount`, `eventBytes`, `artifactCount`, `artifactBytes`, `totalBytes`,
  optional `untrackedBytes`, `largestArtifacts`, and `reviewUnitSnapshots`. Artifact entries use
  opaque artifact refs rather than filesystem paths.
- `sensitivity` reports `policyOutcome` plus redacted findings. Finding references use
  `file:sha256:*` refs, and command output does not print secret values or source file paths.

`shore store link` registers the current worktree with the clone-local store and imports the
worktree-local `.shore/data/` events and artifacts into that store. It emits `shore.store-link` JSON with
the selected opaque refs and `eventsCreated`, `eventsExisting`, `artifactsCreated`, and
`artifactsExisting` counters. It also includes the same redacted `sensitivity` object as
`shore store status`. The import is idempotent for already-present matching facts.

Sensitivity scanning happens before data movement. For this clone-local release, findings are
reported but do not abort `shore store link`; hard-blocking policy and explicit override controls are
deferred until movement can target a wider user-level or remote store. Blocking findings still name
only safe finding kinds, such as `known_token`, and command output does not print the secret text or
file path.

Linked capture is batch-only in this release: capture writes local facts first, emits the
`clone_local_capture_batch_only` diagnostic when the worktree is linked, and `shore store link`
copies those facts into the clone-local store. Every review read command resolves the linked store:
`review unit list` and `unit show`, `history`, the observation, input-request, and validation
lists, `assessment show`, and `lineage show` read the clone-local store from any linked worktree,
including hydrated bodies and the captured snapshot, so their `eventCount` and `eventSetHash`
reflect the linked store. Linked reads are store-only — local facts not yet copied by
`shore store link` do not appear in results; read commands report them with the
`clone_local_unsynced_local_events` diagnostic, and `shore store link` copies them and clears it.
Run `shore store link` before removing a worktree whose review record should survive for its
siblings.

Command output is the stable integration surface. Raw clone-local store paths, event files, artifact
paths, `.git` paths, `.shore/data` paths, and `state.json` remain internal storage details.

## `shore review observation`

```bash
shore review observation add --track <track-id> --title <title> \
  [--review-unit <review-unit-id> | --lineage <lineage-id>] [target options]
shore review observation list [--review-unit <review-unit-id> | --lineage <lineage-id>] [--track <track-id>] \
  [--file <path>] [--tag <tag>] [--include-body] [--pretty|--compact]
```

Observations are append-only review notes for a captured ReviewUnit.

- `observation add` requires `--track` and `--title`.
- `--review-unit` pins the observation to one captured ReviewUnit. `--lineage` targets the current
  lineage head. Without either, the command defaults to the single captured unit and errors if
  multiple captured ReviewUnits exist.
- Tracks are review lanes, not actor or producer provenance.
- Without `--file`, the observation targets the whole ReviewUnit.
- With `--file <path>`, it targets a captured file.
- With `--file <path> --start-line <n> [--end-line <n>]`, it targets a range on `--side <old|new>`
  where the default side is `new`.
- Bodies may come from `--body`, `--body-file`, or `--body-stdin`.
- Large bodies are stored as Shoreline-owned `shore.note-body` artifacts while command output keeps
  artifact paths private.
- `--supersedes <observation-id>` records a correction by appending a new observation that names the
  older observation.
- `observation list` replays durable events for the ReviewUnit and may filter by ReviewUnit, track,
  file, or tag. It hydrates body text only with `--include-body`.

Output is compact `shore.review-observation-add` or `shore.review-observation-list` JSON by
default. `observation list` also accepts `--pretty` and `--compact`.

## `shore review input-request`

```bash
shore review input-request open --track <track-id> --title <title> --reason <reason> \
  [--review-unit <review-unit-id> | --lineage <lineage-id>] [--mode operative|advisory]
shore review input-request list [--review-unit <review-unit-id> | --lineage <lineage-id>] [--track <track-id>] \
  [--mode operative|advisory] [--file <path>] [--status open|responded|ambiguous|all] \
  [--include-body] [--pretty|--compact]
shore review input-request fetch <input-request-id> [--include-body]
shore review input-request respond <input-request-id> --outcome <outcome> [reason options]
```

Input requests are durable pause or decision requests for a captured ReviewUnit.

- `input-request open` requires `--track`, `--title`, and `--reason`.
- `--review-unit` pins the request to one captured ReviewUnit. `--lineage` targets the current
  lineage head. Without either, the command defaults to the single captured unit and errors if
  multiple captured ReviewUnits exist.
- `--mode` defaults to `operative`; `advisory` requests are durable and visible but do not imply a
  cooperative client must pause.
- Targets mirror observations: review-wide by default, captured file, captured range, or an
  existing native observation through `--observation <observation-id>`.
- Request bodies may come from `--body`, `--body-file`, or `--body-stdin`.
- Large request bodies reuse Shoreline-owned `shore.note-body` artifacts while command output keeps
  artifact paths private.
- `input-request list` is the V1 polling read surface and defaults to open requests. It may filter
  by ReviewUnit, track, mode, file, or status, and hydrates body text only with `--include-body`.
- `input-request fetch <id> --include-body` returns one request and hydrates the body when
  requested.
- `input-request respond <id>` appends an `input_request_responded` event.
- Response outcomes are `approved`, `rejected`, `dismissed`, `superseded`, and `abandoned`.

Output documents are compact `shore.review-input-request-open`,
`shore.review-input-request-list`, `shore.review-input-request-fetch`, and
`shore.review-input-request-respond` JSON by default. Read commands also accept `--pretty` and
`--compact`.

V1 is durable and polling-friendly. It does not add a daemon, filesystem watch mode, TUI prompt,
notification transport, or cancellation/escalation event.

## `shore review assessment`

```bash
shore review assessment add --track <track-id> --assessment <assessment> \
  [--review-unit <review-unit-id> | --lineage <lineage-id>] [target options]
shore review assessment show [--review-unit <review-unit-id> | --lineage <lineage-id>] [--all] [--track <track-id>] \
  [--include-summary] [--pretty|--compact]
```

Assessments record review calls for a captured ReviewUnit.

- `assessment add` requires `--track` and `--assessment`.
- `--review-unit` pins the assessment to one captured ReviewUnit. `--lineage` targets the current
  lineage head. Without either, the command defaults to the single captured unit and errors if
  multiple captured ReviewUnits exist.
- V1 assessment values are `accepted`, `accepted-with-follow-up`, `needs-changes`, and
  `needs-clarification`.
- Targets mirror the ReviewUnit ledger: review-wide by default, captured file, captured range,
  native observation, native input request, or another assessment.
- Summaries may come from `--summary`, `--summary-file`, or `--summary-stdin`.
- Large summaries reuse Shoreline-owned `shore.note-body` artifacts while command output keeps
  artifact paths private.
- `--replaces <assessment-id>` is the only V1 relationship that removes an older assessment from
  the current set.
- `--related-observation` and `--related-input-request` record evidence links; they do not mutate
  observations or close input requests.
- `assessment show` reports current status as `unassessed`, `resolved`, or `ambiguous`. It may
  filter by ReviewUnit or track, include replaced assessments with `--all`, and hydrate summaries
  with `--include-summary`.

Output documents are compact `shore.review-assessment-add` and `shore.review-assessment-show` JSON
by default. `assessment show` also accepts `--pretty` and `--compact`.

State-change outcomes such as deferred, split-out, overridden, and superseded are ordinary review
observations when needed.

## `shore review validation`

```bash
shore review validation add --track <track-id> --check-name <name> --status <status> \
  [--review-unit <review-unit-id> | --lineage <lineage-id>] [validation options]
shore review validation list [--review-unit <review-unit-id> | --lineage <lineage-id>] \
  [--track <track-id>] [--status <status>] [--include-body] [--pretty | --compact]
```

Validation checks record local test, lint, build, or other verification evidence for a captured
ReviewUnit. They are advisory review context only: they do not accept, reject, merge, block, or
replace a review assessment.

- `validation add` requires `--track`, `--check-name`, and `--status`.
- `--review-unit` pins the check to one captured ReviewUnit. `--lineage` targets the current
  lineage head. Without either, the command defaults to the single captured unit and errors if
  multiple captured ReviewUnits exist.
- Validation targets are ReviewUnit-only. There are no file or path target flags.
- Status values are `passed`, `failed`, `errored`, and `skipped`.
- `--command`, `--exit-code`, `--source-fingerprint`, `--started-at`, `--completed-at`, and
  repeatable `--log-content-hash` record evidence metadata without exposing artifact paths.
- `--trigger` defaults to `manual`; accepted values are `manual`, `push`, and `pull-request`.
- Summaries may come from `--summary`, `--summary-file`, or `--summary-stdin`.
- Large summaries reuse Shoreline-owned `shore.note-body` artifacts while command output keeps
  artifact paths private.
- `validation list` replays durable events for the ReviewUnit and may filter by ReviewUnit, track,
  or status. It hydrates summaries only with `--include-body`.

Output documents are compact `shore.review-validation-add` and
`shore.review-validation-list` JSON by default. `validation list` also accepts `--pretty` and
`--compact`.

## `shore review history`

```bash
shore review history [--repo <path>] [--review-unit <id>] [--track <track-id>] \
  [--event-type <event-type>]... [--include-body] [--pretty | --compact]
```

`shore review history` reads the chronological ledger of durable Shoreline events.

- History replays `.shore/data/events/` and emits compact `shore.review-history` v1 JSON by default.
- `eventSetHash` and `eventCount` describe the full validated event set used to build the output,
  even when filters return only a subset of entries.
- `historyCount` is the number of returned entries after filters.
- Entries are sorted by `occurredAt`, then `eventId`, as display chronology.
- `--review-unit`, `--track`, and repeated `--event-type` narrow the returned entries.
- Lineage event filters are `review-unit-lineage-declared` and
  `review-unit-lineage-round-recorded`.
- Body-like text is omitted by default. `--include-body` hydrates observation bodies, input request
  bodies, input request response reasons, assessment summaries, validation summaries, and
  imported-note bodies.
- Duplicate semantic events remain visible as separate entries while shared duplicate diagnostics
  are included in the document.

History is not the full ReviewUnit row projection. Use `shore review unit show` for the composite
narrative-first plus snapshot-complete view of one captured ReviewUnit.

## `shore review unit`

```bash
shore review unit list [--repo <path>] [--pretty | --compact]
shore review unit show [--repo <path>] [--review-unit <id> | --lineage <lineage-id>] [--track <track-id>] \
  [--include-body] [--pretty | --compact]
```

`shore review unit list` is the discovery surface for captured ReviewUnits. It emits
`shore.review-unit-list` JSON with `eventSetHash`, `eventCount`, `reviewUnitCount`, and entries
sorted by capture time.

ReviewUnit lineage metadata is reported by lineage-aware read surfaces. The lineage round event is
`review_unit_lineage_round_recorded`; it links an already-stored captured ReviewUnit into an ordered
thread. Change-Id optional enrichment only: it is not required and is not the lineage identity.

`shore review unit show` is the composite view for one ReviewUnit. It emits compact
`shore.review-unit` v1 JSON by default.

- When exactly one ReviewUnit has been captured, Shoreline selects it automatically.
- If multiple ReviewUnits exist, pass `--review-unit <id>` or `--lineage <lineage-id>`.
- The output includes ReviewUnit identity, event-set freshness metadata, filters, summary counts,
  current assessment status, native observations, input requests, assessments, validation checks,
  imported adapter notes, projection rows, and diagnostics.
- Rows are narrative-first, then snapshot-complete.
- `--track <track-id>` filters narrative facts without changing the selected ReviewUnit,
  event-set freshness metadata, or captured snapshot completeness.
- Body-like text is omitted by default. `--include-body` hydrates observation bodies, input request
  bodies and response reasons, assessment summaries, validation summaries, and imported-note bodies.

Lineage-scoped current selection resolves to `headReviewUnitId`; no implicit newest capture globally
wins. Unscoped current selection with multiple captured ReviewUnits still errors at the selection
boundary, but routine list, history, exact ReviewUnit, and lineage-scoped reads should have no
always-on ambiguous-current warning for routine multi-capture reads. Thread-level lineage reads may
surface `stale_by_newer_round` for facts attached to older rounds. This release has no interdiff or
stack DAG.

Lineage event families stay signable under ADR-0004's generic `EventToBeSigned` contract with the
Dead Simple Signing Envelope (DSSE) and pre-authentication encoding rules.

`shore review unit show` is distinct from `shore review history`: history is the chronological raw
event listing, while unit show is the composite ReviewUnit view for agents and future frontends.

## `shore review lineage`

```bash
shore review lineage attach --repo <path> --lineage <lineage-id> --review-unit <id> \
  [--predecessor <id>] [--change-id <change-id>]
shore review lineage show --repo <path> --lineage <lineage-id> [--pretty | --compact]
```

`shore review lineage attach` records path-free lineage declaration and round facts over
already-stored `review_unit_captured` events. The output is compact
`shore.review-lineage-attach` JSON with `lineageId`, `headReviewUnitId`, event write counts, and
diagnostics. If the write leaves the lineage malformed, `headReviewUnitId` is `null` and diagnostics
describe the malformed lineage.

`shore review lineage show` emits compact `shore.review-lineage` JSON by default. The document
includes `eventSetHash`, `eventCount`, `lineageId`, `headReviewUnitId`, `rounds`, and diagnostics.
Round entries include `reviewUnitId`, optional `predecessorReviewUnitId`, `roundIndex`, and
`isHead`. Thread-level diagnostics may include `stale_by_newer_round` when facts target an older
round than the lineage head. This release does not render interdiffs, stack graphs, or a
stacked-work DAG.

`shore review capture --lineage <lineage-id> [--predecessor <id>]` is a convenience that captures a
new ReviewUnit and then attaches that captured ReviewUnit to the lineage. Its `shore.review-capture`
output keeps capture event counts at the top level and places lineage attach counts under
`lineageAttach`, including the post-attach lineage head and lineage diagnostics.

## `shore notes apply`

```bash
shore notes apply --repo <path> --review-notes <path>
```

`shore notes apply` imports review notes into Shoreline-owned durable state without publishing a
revision.

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--review-notes <path>` is required.
- The command initializes local `.shore/data/` storage when needed, records one immutable durable event
  per imported note, and rebuilds `.shore/data/state.json`.
- Native `review-notes.json` is an import/transport input, not the authoritative persisted
  Shoreline store.
- Large note bodies may be stored as content-addressed note-body artifacts under
  `.shore/data/artifacts/notes/`; small note bodies remain inline in the imported-note event payload.
- Output is compact `shore.notes-apply` JSON with note counts, diagnostics, and `statePath`.
