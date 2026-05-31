# CLI Reference

This reference covers the public `shore` command surface provided by the `shoreline` crate.

Command output JSON is the integration surface. Raw event files, artifact paths, event filenames,
and `.shore/state.json` are internal storage details unless a command explicitly returns them.

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

## `shore show`

```bash
shore show [--repo <path>] [--review-notes <path>]
```

`shore show` opens a read-only terminal review view over the same headless review stream used by
`shore dump`.

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--review-notes <path>` loads Shoreline-native `review-notes.json`.
- Without an explicit sidecar, repo-only `shore show` auto-loads durable imported notes from
  `.shore/` when the store exists.
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
  `.shore/` when the store exists.
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

`shore inspect` starts a small local web server that visualizes a `.shore` store for tracing event
timelines and outcomes — the kind of inspection that is awkward against the raw JSON files or
per-command output.

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--host <addr>` defaults to `127.0.0.1` (loopback). `--port <n>` defaults to `7878`; use
  `--port 0` to bind an ephemeral port, which is then printed.
- `--open` launches the inspector in the default browser after the server starts.
- The server runs until interrupted with Ctrl-C.

The page provides a chronological event timeline (filterable by track, ReviewUnit, and event type,
newest-first by default), a per-event detail view, a composite per-ReviewUnit page showing the
current-assessment status plus grouped observations, input requests, and assessments, and the
captured diff for a ReviewUnit annotated with the review facts anchored to each line. Long IDs render
as truncated, clickable references that navigate to the resource they name, and the page
auto-refreshes when the store changes.

The inspector is a read-only, single-store, localhost developer tool. It reads through the same
validated projections as `shore review history` and `shore review unit show` rather than parsing raw
storage, and it serves over a synchronous, dependency-free HTTP server with no async runtime. The
small JSON API the page consumes (`/api/history`, `/api/units`, `/api/unit`, `/api/snapshot`,
`/api/freshness`) is an internal surface for the bundled page, not a stable contract.

## `shore review capture`

```bash
shore review capture [--repo <path>]
```

`shore review capture` records the current V1 ReviewUnit: the base endpoint, target endpoint, and
captured diff snapshot. V1 captures the local Git worktree from `HEAD` to the working tree,
including untracked files.

- Durable state is created at the Git worktree root under `.shore/`.
- The command registers `.shore/` in the repository-local `.git/info/exclude`
  when it is not already ignored, so it never modifies a tracked `.gitignore` or
  dirties the working tree. This applies to every writer-initializing command
  (capture, observation, input-request, assessment), not just `capture`.
- `.shore/events/` stores immutable local event files.
- `.shore/state.json` is a rebuildable projection, not the authority.
- Full captured snapshots are Shoreline-owned immutable artifacts under
  `.shore/artifacts/snapshots/`.
- The `review_unit_captured` event binds to the snapshot artifact's canonical content hash.
- Output is compact `shore.review-capture` JSON and includes ReviewUnit, revision, and snapshot IDs
  plus `snapshotArtifactContentHash`.

V1 `.shore/` storage is local and synchronous. It assumes one active Shoreline writer per `.shore/`
directory and does not add a daemon, delivery queue, approval flow, async storage, remote storage,
or note mutation.

When a worktree is linked to a clone-local store, `shore review capture` still writes the capture to
that worktree's local `.shore/` store. The command includes a
`clone_local_capture_batch_only` diagnostic that tells callers to run `shore store link` to copy the
new local facts into the linked clone-local store.

## `shore store`

```bash
shore store status [--repo <path>] [--pretty]
shore store link [--repo <path>] [--pretty]
```

`shore store` commands inspect or connect the current Git worktree to a clone-local store. A
clone-local store is shared by Git linked worktrees from the same clone; it is not a user-level
multi-repository store or remote sync service.

`shore store status` resolves the selected store and emits `shore.store-status` JSON.

- `mode` is `local` for the default worktree-local `.shore/` store and `linked` when the worktree
  has been registered with a clone-local store.
- `storeRef` is `worktree-local` in local mode and an opaque `store:random:*` ref in linked mode.
  Linked output also includes opaque `cloneRef` and `repositoryFamilyRef` values.
- `inventory` reports `eventCount`, `eventBytes`, `artifactCount`, `artifactBytes`, `totalBytes`,
  optional `untrackedBytes`, `largestArtifacts`, and `reviewUnitSnapshots`. Artifact entries use
  opaque artifact refs rather than filesystem paths.
- `sensitivity` reports `policyOutcome` plus redacted findings. Finding references use
  `file:sha256:*` refs, and command output does not print secret values or source file paths.

`shore store link` registers the current worktree with the clone-local store and imports the
worktree-local `.shore/` events and artifacts into that store. It emits `shore.store-link` JSON with
the selected opaque refs and `eventsCreated`, `eventsExisting`, `artifactsCreated`, and
`artifactsExisting` counters. The import is idempotent for already-present matching facts.

Sensitivity scanning happens before data movement. If the scan returns a blocking outcome,
`shore store link` exits non-zero and names only the safe finding kinds, such as `known_token`,
instead of printing the secret text or file path.

Linked capture is batch-only in this release: capture writes local facts first, emits the
`clone_local_capture_batch_only` diagnostic when the worktree is linked, and `shore store link`
copies those facts into the clone-local store. `shore review unit list` resolves the linked store,
so a linked worktree can discover imported ReviewUnits even when it has no local `.shore/events/`.
Sibling read commands such as observation, input-request, assessment, history, and unit show still
read the worktree-local store in this release; use a worktree that owns those facts for full
inspection until those read surfaces are migrated.

Command output is the stable integration surface. Raw clone-local store paths, event files, artifact
paths, `.git` paths, `.shore` paths, and `state.json` remain internal storage details.

## `shore review observation`

```bash
shore review observation add --track <track-id> --title <title> \
  [--review-unit <review-unit-id>] [target options]
shore review observation list [--review-unit <review-unit-id>] [--track <track-id>] \
  [--file <path>] [--tag <tag>] [--include-body] [--pretty|--compact]
```

Observations are append-only review notes for a captured ReviewUnit.

- `observation add` requires `--track` and `--title`.
- `--review-unit` pins the observation to one captured ReviewUnit; without it, the command defaults
  to the single captured unit and errors if multiple captured ReviewUnits are current.
- Tracks are review lanes, not actor or tool provenance.
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
  [--review-unit <review-unit-id>] [--mode operative|advisory]
shore review input-request list [--review-unit <review-unit-id>] [--track <track-id>] \
  [--mode operative|advisory] [--file <path>] [--status open|responded|ambiguous|all] \
  [--include-body] [--pretty|--compact]
shore review input-request fetch <input-request-id> [--include-body]
shore review input-request respond <input-request-id> --outcome <outcome> [reason options]
```

Input requests are durable pause or decision requests for a captured ReviewUnit.

- `input-request open` requires `--track`, `--title`, and `--reason`.
- `--review-unit` pins the request to one captured ReviewUnit; without it, the command defaults to
  the single captured unit and errors if multiple captured ReviewUnits are current.
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
  [--review-unit <review-unit-id>] [target options]
shore review assessment show [--review-unit <review-unit-id>] [--all] [--track <track-id>] \
  [--include-summary] [--pretty|--compact]
```

Assessments record review calls for a captured ReviewUnit.

- `assessment add` requires `--track` and `--assessment`.
- `--review-unit` pins the assessment to one captured ReviewUnit; without it, the command defaults
  to the single captured unit and errors if multiple captured ReviewUnits are current.
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

## `shore review history`

```bash
shore review history [--repo <path>] [--review-unit <id>] [--track <track-id>] \
  [--event-type <event-type>]... [--include-body] [--pretty | --compact]
```

`shore review history` reads the chronological ledger of durable Shoreline events.

- History replays `.shore/events/` and emits compact `shore.review-history` v1 JSON by default.
- `eventSetHash` and `eventCount` describe the full validated event set used to build the output,
  even when filters return only a subset of entries.
- `historyCount` is the number of returned entries after filters.
- Entries are sorted by `occurredAt`, then `eventId`, as display chronology.
- `--review-unit`, `--track`, and repeated `--event-type` narrow the returned entries.
- Body-like text is omitted by default. `--include-body` hydrates observation bodies, input request
  bodies, input request response reasons, assessment summaries, and imported-note bodies.
- Duplicate semantic events remain visible as separate entries while shared duplicate diagnostics
  are included in the document.

History is not the full ReviewUnit row projection. Use `shore review unit show` for the composite
narrative-first plus snapshot-complete view of one captured ReviewUnit.

## `shore review unit`

```bash
shore review unit list [--repo <path>] [--pretty | --compact]
shore review unit show [--repo <path>] [--review-unit <id>] [--track <track-id>] \
  [--include-body] [--pretty | --compact]
```

`shore review unit list` is the discovery surface for captured ReviewUnits. It emits
`shore.review-unit-list` JSON with `eventSetHash`, `eventCount`, `reviewUnitCount`, and entries
sorted by capture time.

`shore review unit show` is the composite view for one ReviewUnit. It emits compact
`shore.review-unit` v1 JSON by default.

- When exactly one ReviewUnit has been captured, Shoreline selects it automatically.
- If multiple ReviewUnits exist, pass `--review-unit <id>`.
- The output includes ReviewUnit identity, event-set freshness metadata, filters, summary counts,
  current assessment status, native observations, input requests, assessments, imported adapter
  notes, projection rows, and diagnostics.
- Rows are narrative-first, then snapshot-complete.
- `--track <track-id>` filters narrative facts without changing the selected ReviewUnit,
  event-set freshness metadata, or captured snapshot completeness.
- Body-like text is omitted by default. `--include-body` hydrates observation bodies, input request
  bodies and response reasons, assessment summaries, and imported-note bodies.

`shore review unit show` is distinct from `shore review history`: history is the chronological raw
event listing, while unit show is the composite ReviewUnit view for agents and future frontends.

## `shore notes apply`

```bash
shore notes apply --repo <path> --review-notes <path>
```

`shore notes apply` imports review notes into Shoreline-owned durable state without publishing a
revision.

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--review-notes <path>` is required.
- The command initializes local `.shore/` storage when needed, records one immutable durable event
  per imported note, and rebuilds `.shore/state.json`.
- Native `review-notes.json` is an import/transport input, not the authoritative persisted
  Shoreline store.
- Large note bodies may be stored as content-addressed note-body artifacts under
  `.shore/artifacts/notes/`; small note bodies remain inline in the imported-note event payload.
- Output is compact `shore.notes-apply` JSON with note counts, diagnostics, and `statePath`.
