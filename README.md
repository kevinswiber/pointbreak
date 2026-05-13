# Shore

Shore is an experimental Rust terminal review tool for understanding what changed and why,
especially in tool-assisted changesets.

It is inspired by [hunk](https://github.com/modem-dev/hunk), but it is not intended to be a direct clone or fork. The goal is to build a
small, Rust-native review core with a data model that is easy to reason about, test, and eventually
expose to other tools.

## Name

The name connects to Pointbreak and the idea of reviewing the wake of tool-assisted work once it
reaches shore. It also works as a verb: `shore up` can become the command that reviews and hardens a
changeset.

The metaphor should stay light. Command names should remain mostly plain and practical:

- `shore diff`
- `shore show`
- `shore up`
- `shore notes`
- `shore dump`
- `shore review`
- `shore session`

## Product Intent

Shore is for code review in a terminal. It should help a reviewer inspect:

- the actual working-tree diff
- the review notes a reviewer or tool attached to files and hunks
- the hunk stream in the order the reviewer should read it
- which notes are attached to which code rows
- enough recoverable session state that review context is not lost when a UI is restarted

The first version should be a focused terminal review tool, not a generic summarizer.

## Inspiration And Lessons

Hunk is the practical inspiration: a terminal-first diff viewer with Hunk-compatible
`agent-context.json` sidecars, hunk-level notes, live review sessions, and keyboard navigation
across notes.

Detailed field notes from a real Hunk review session are captured in
[docs/hunk-feedback.md](docs/hunk-feedback.md). Treat those notes as product input, especially
around persistence, reload semantics, stable comment anchors, and separating long-lived reviews
from individual diff snapshots.

The most important lesson from maintaining a [hunk fork](https://github.com/kevinswiber/hunk) is that the hard part is not simply drawing a
diff in a TUI. The hard part is keeping these behaviors aligned:

- file order
- hunk order
- row geometry
- scroll position
- selected hunk
- note anchors
- note-hunk navigation
- terminal resize behavior
- saved or live review-note context

Shore should avoid parallel sources of truth. Rendering, scrolling, and navigation should derive
from one explicit review-stream model.

## Core Architecture Principle

Build the review stream as a pure, headless data layer before building the TUI.

That model should own:

- file identity, status, old path, and new path
- file order, including sidecar-provided narrative order
- hunk identity and hunk order
- hunk header spans, including context rows
- rendered review rows
- row and section geometry
- note anchors and resolved note targets
- hunk navigation cursors
- note-hunk navigation cursors
- serializable review/session state

The TUI should be a projection of that model. Widgets may render state, but they should not become
the authoritative owners of scroll, selection, or navigation semantics.

Durable workflow guidance is captured in [docs/storage-model.md](docs/storage-model.md) and
[docs/intervention-model.md](docs/intervention-model.md). Treat those as architecture guidance for
storage, event, interruption, and escalation design, not current V1 implementation scope.

## Initial Scope

The first milestone should be deliberately smaller than hunk.

Build:

- a Rust CLI binary
- working-tree `diff` support
- tracked and untracked file support
- unified-diff parsing into Shore's own file/hunk/row model
- a native `review-notes.json` sidecar loader
- a split terminal diff view
- `[` and `]` navigation through the full hunk stream
- `{` and `}` navigation through hunks with review notes
- snapshot and acceptance fixtures for the review model

Prefer shelling out to `git` at first. A VCS abstraction can come later if the model earns it.

## Current CLI

The current executable surfaces are `shore show`, `shore dump`, `shore review capture`,
`shore review observation add/list`, `shore review intervention request/list/fetch/resolve`,
`shore notes apply`, and transitional `shore review publish` / `verdict` / `ack` scaffolding.

All commands accept optional tracing flags:

```bash
--log <filter>
--log-format <compact|pretty|json>
--log-file <path>
```

Tracing writes to stderr by default. If stdout is being piped into JSON tools, prefer
`--log-file <path>` instead of `2>&1`; mixing stderr into stdout will corrupt the JSON stream.
`shore show` requires `--log-file` when tracing is enabled so trace lines do not scribble over the
raw-mode TUI. When `--log-file <path>` points inside the repository, Shore treats that path as a
command helper for the current command and excludes it from the reviewed snapshot and fingerprint.

`shore show` opens the first read-only terminal review view over the same headless review stream
used by the JSON dump command:

```bash
shore show [--repo <path>] [--review-notes <path> | --legacy-hunk-agent-context <path>]
```

Behavior:

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--review-notes <path>` loads Shore-native `review-notes.json`.
- `--legacy-hunk-agent-context <path>` imports a Hunk-compatible `agent-context.json` through the
  explicit legacy adapter.
- When no explicit sidecar is supplied, repo-only `shore show` auto-loads durable imported notes
  from `.shore/` if the store exists.
- When `review_artifacts` data is present in the underlying dump document, `shore show` renders a
  verdict status banner above the main view (for example, `verdict: pass | acks: 1/1`). If multiple
  unreplaced verdicts exist for the current revision, the banner reports `ambiguous` with a
  candidate count instead of inventing a tie-breaker.
- Press `r` to re-ingest the working tree and reload the projection without losing your cursor
  position. Reload preserves the cursor by row ID when possible, then falls back to file+hunk,
  file, or the first row in the refreshed stream.
- Stale and orphan review notes appear as dedicated rows you can park the cursor on. The detail
  pane labels the row with its resolution status and the original target path and line range.
- Explicit sidecar inputs are command helpers and are not themselves included in the reviewed
  snapshot for that command. Other unrelated tracked and untracked files remain visible.
- The view is read-only: it renders the working-tree diff, resolved review notes, and recoverable
  diagnostics, but it does not mutate notes or write session state.
- Keybindings are intentionally small: `q`/Esc/Ctrl+C quits, `j`/`k` or Up/Down moves by row, `[` and
  `]` move through hunks, and `{` and `}` move through hunks with review notes.

`shore dump` remains the JSON contract over the headless model so other frontends and tests can
consume the same review stream:

```bash
shore dump [--repo <path>] [--review-notes <path> | --legacy-hunk-agent-context <path>] [--pretty | --compact]
```

Behavior:

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--review-notes <path>` loads Shore-native `review-notes.json`.
- `--legacy-hunk-agent-context <path>` imports a Hunk-compatible `agent-context.json` through the
  explicit legacy adapter.
- When no explicit sidecar is supplied, repo-only `shore dump` auto-loads durable imported notes
  from `.shore/` if the store exists.
- When `.shore/` exists, `shore dump` also emits a `review_artifacts` section containing published
  verdicts, acknowledgements, and a `current_verdict` summary. The section is omitted entirely when
  the durable store is absent.
- When durable verdicts or notes no longer match the current revision, `shore dump` emits an
  optional `reload_diagnostics` section containing the staleness entries (each with a `code` and
  `message`). Individual verdicts and acknowledgements also carry a `stale: true` flag when their
  revision no longer matches `current_revision_id`. Both the `reload_diagnostics` section and the
  `stale` flags are omitted entirely when there is no reload-time staleness.
- The review stream emits a `stale_note` row variant when a durable note's anchor no longer matches
  the current diff (the file is present but the line range is unsatisfiable). When a note's file is
  absent from the snapshot entirely, the stream emits a synthetic `<orphaned notes>` file header
  followed by one `stale_note` row per orphan note. Stale-note rows carry `note_id`, `title`,
  `resolution_status` (`stale` or `orphaned`), `target_path`, and `target_line_range`. The
  synthetic header is omitted when there are no orphan notes.
- Explicit sidecar inputs and `--log-file <path>` are command helpers and are not themselves
  included in the reviewed snapshot for that command. Other unrelated tracked and untracked files
  remain visible.
- Output is compact by default for scripts. Use `--pretty` for human-readable formatting;
  `--compact` is accepted as an explicit compact-format request.
- Recoverable review-note diagnostics stay in the JSON document and the command exits successfully.
- Fatal errors, such as unreadable files or malformed JSON, are written to stderr and exit
  non-zero; unreadable sidecar errors include the attempted path.

The dump output is Shore introspection JSON and uses snake_case fields. Native `review-notes.json`
input keeps its schema-defined camelCase fields such as `oldPath`, `startLine`, and `createdAt`.

`shore review capture` records the current V1 ReviewUnit:

```bash
shore review capture [--repo <path>]
```

Behavior:

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it;
  durable state is created at the Git worktree root.
- The ReviewUnit is the base endpoint, target endpoint, and captured diff snapshot. V1 captures the
  local Git worktree from `HEAD` to the working tree, including untracked files.
- The command creates and uses local `.shore/` storage and adds `.shore/` to the worktree
  `.gitignore` when needed.
- `.shore/events/` stores immutable local event files. `.shore/state.json` is a rebuildable
  projection, not the authority.
- `.shore/` is local, synchronous storage. V1 uses a single-writer contract: one active Shore writer
  at a time per `.shore/` directory. Event writes use per-file durable facts and rebuildable
  projections rather than a daemon or shared mutable JSON authority.
- Full captured snapshots are Shore-owned immutable artifacts under `.shore/artifacts/snapshots/`.
  The `review_unit_captured` event binds to the snapshot artifact's canonical content hash, so
  replay can detect changed artifact facts. The output exposes ReviewUnit, revision, and snapshot
  IDs plus that content hash, but does not expose artifact paths as user-facing API.
- `--log-file <path>` is command-helper plumbing and is excluded from the captured snapshot and
  content-derived ReviewUnit fingerprint for that command. Other unrelated tracked and untracked
  files remain part of the capture unless the caller keeps them out of the worktree.
- Sidecar inputs are not part of the capture contract. Native `review-notes.json` and legacy Hunk
  `agent-context.json` remain optional import/transport adapters for read/import commands.
- Output is compact `shore.review-capture` JSON. Command output documents are the external contract
  for automation; `.shore/state.json` is only a rebuildable projection, and artifact paths remain
  Shore-owned storage details.

`shore review capture` does not add a daemon, delivery queue, acknowledgement flow, async or remote
storage backend, or note mutation. `.shore/events/` is the local authoritative event log, not a
mailbox or retry queue.

`shore review observation` records and reads append-only reviewer observations for a captured
ReviewUnit:

```bash
shore review capture
shore review observation add --track agent:codex --title "Check error handling" --file src/lib.rs
shore review observation list --track agent:codex
```

Behavior:

- `shore review observation add` requires `--track` and `--title`.
- Tracks are review lanes, not actor or tool provenance. Shore still records writer provenance from
  local Git config and the Shore tool identity in the event envelope.
- Without `--file`, the observation is review-wide and targets the whole ReviewUnit.
- With `--file <path>`, the observation targets a file in the captured snapshot.
- With `--file <path> --start-line <n> [--end-line <n>]`, the observation targets a range on the
  selected side (`--side <old|new>`, default `new`).
- Bodies may come from `--body`, `--body-file`, or `--body-stdin`. Large bodies are stored as
  Shore-owned `shore.note-body` artifacts while command output keeps artifact paths private.
- `--supersedes <observation-id>` records a correction by appending a new observation that names the
  older observation. Standalone retraction is deferred.
- `shore review observation list` replays durable events for the ReviewUnit. Bodies are omitted by
  default and hydrated only with `--include-body`; `--track` and `--file` filter the returned rows.
- Output is compact `shore.review-observation-add` or `shore.review-observation-list` JSON by
  default. `observation list` also accepts `--pretty`.
- Native observations are not yet projected into `shore dump` or `shore show`; that belongs to the
  later ledger projection slice.

`shore review intervention` records and reads durable pause/decision requests for a captured
ReviewUnit:

```bash
shore review intervention request --track human:kevin --title "Need approval" \
  --reason manual-decision-required [--mode blocking|advisory]
shore review intervention list [--status open|resolved|ambiguous|all]
shore review intervention fetch <intervention-id> [--include-body]
shore review intervention resolve <intervention-id> --outcome approved [--reason "approved"]
```

Behavior:

- `intervention request` requires `--track`, `--title`, and `--reason`. `--mode` defaults to
  `blocking`; `advisory` requests are durable but do not imply a cooperative client must pause.
- Request targets mirror observations: review-wide by default, `--file <path>` for a captured file,
  `--file <path> --start-line <n> [--end-line <n>]` for a range, or `--observation
  <observation-id>` for an existing native observation in the same ReviewUnit.
- Request bodies may come from `--body`, `--body-file`, or `--body-stdin`. Large bodies reuse
  Shore-owned `shore.note-body` artifacts while command output keeps artifact paths private.
- `intervention list` is the V1 polling read surface. It replays `.shore/events/`, defaults to open
  interventions, and can filter by `--track`, `--mode`, `--file`, and `--status`.
- `intervention fetch <id> --include-body` returns one intervention and hydrates the body when
  requested.
- `intervention resolve <id>` appends an `intervention_resolved` event with an `--outcome` of
  `approved`, `rejected`, `dismissed`, `superseded`, or `abandoned`. The optional reason may come
  from `--reason`, `--reason-file`, or `--reason-stdin`.
- Multiple different resolution events are preserved as append-only facts. Read surfaces report the
  intervention as `ambiguous` instead of picking a timestamp winner.
- Output documents are compact `shore.review-intervention-request`,
  `shore.review-intervention-list`, `shore.review-intervention-fetch`, and
  `shore.review-intervention-resolve` JSON by default. Read commands also accept `--pretty`.
- V1 is durable and polling-friendly. It does not add a daemon, filesystem watch mode, TUI prompt,
  notification transport, cancellation/escalation event, or final review disposition.

`shore review publish` is older durable scaffolding kept temporarily while the ReviewUnit ledger
model takes over. New capture-oriented workflows should start with `shore review capture`; later
plans will replace or remove the publish/verdict/ack vocabulary rather than treating it as the
stable ReviewUnit API.

`shore review verdict` records a reviewer verdict against the current or named revision:

```bash
shore review verdict [--repo <path>] --decision <pass|pass-minor-nit|request-changes> \
  [--summary <text> | --summary-file <path>] [--target-revision <revision-id>] \
  [--replaces <review-artifact-id>...] [--reviewer-id <opaque>]
```

Behavior:

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--decision` is required and uses reviewer-oriented verdict vocabulary.
- `--summary` and `--summary-file` are mutually exclusive.
- `--target-revision <revision-id>` overrides the current durable revision; otherwise Shore uses the
  current revision projected from `.shore/state.json`.
- `--replaces <review-artifact-id>` marks older verdict artifacts as superseded inline in the new
  verdict event.
- `--reviewer-id <opaque>` overrides the reviewer actor ID; otherwise Shore derives reviewer
  identity from local Git config.
- Large summaries are externalized as content-addressed `shore.note-body` artifacts under
  `.shore/artifacts/notes/`; small summaries remain inline in the durable event payload.
- Output is compact `shore.review-verdict` JSON with the returned `reviewArtifactId`, event counts,
  and diagnostics.

`shore review ack` records an acknowledgement against a published review artifact:

```bash
shore review ack [--repo <path>] --review-artifact <review-artifact-id> \
  --next-action <accept|address|defer|obsolete> [--reason <text> | --reason-file <path>] \
  [--actor-id <opaque>]
```

Behavior:

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--review-artifact <review-artifact-id>` is required and must reference an existing published
  review artifact in the local event store.
- `--next-action` is required and records the acknowledgement decision.
- `--reason` and `--reason-file` are mutually exclusive.
- `--actor-id <opaque>` overrides the acknowledging actor ID; otherwise Shore derives author-side
  identity from local Git config.
- Large reasons reuse the shared `shore.note-body` artifact path under `.shore/artifacts/notes/`.
- Output is compact `shore.review-ack` JSON with the returned `acknowledgementId`, event counts,
  and diagnostics.

`shore notes apply` imports review notes into Shore-owned durable state without publishing a
revision:

```bash
shore notes apply --repo <path> --review-notes <path>
shore notes apply --repo <path> --legacy-hunk-agent-context <path>
```

Behavior:

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it;
  durable state is created at the Git worktree root.
- Exactly one of `--review-notes <path>` or `--legacy-hunk-agent-context <path>` is required.
- The command initializes local `.shore/` storage when needed, records one immutable durable event
  per imported note, and rebuilds `.shore/state.json`.
- Native `review-notes.json` and legacy Hunk `agent-context.json` remain import/transport inputs.
  They are not the authoritative persisted Shore store.
- Large note bodies may be stored as content-addressed note-body artifacts under
  `.shore/artifacts/notes/`; small note bodies remain inline in the imported-note event payload.
- Output is compact `shore.notes-apply` JSON with note counts, diagnostics, and the `statePath`.

## Explicit V1 Deferrals

Do not start by rebuilding all of hunk.

Defer:

- daemon and multi-session brokering
- external IPC protocol
- live comment mutation
- stash/pager/difftool modes and any write-capable evolution of `shore show`
- full config layering
- menus and extensive chrome
- syntax highlighting
- word-level intra-line diff
- advanced mouse behavior
- package distribution

These are useful, but they should wrap a proven review model rather than shape it prematurely.

## Git And Diff Requirements To Account For Early

Some diff cases should be represented in the model from the start, even if the UI initially renders
them plainly:

- untracked files: `git diff` does not include them, so use `git ls-files --others --exclude-standard`
  and synthesize diffs against `/dev/null`
- renames and copies: use `git diff -M` and model both `old_path` and `new_path`
- binary files
- submodules
- mode-only changes
- deleted files and new files
- hunks with zero lines on one side
- context-row note anchors
- large changesets where rendering must not require every row to be visible at once

Line-level diff is acceptable for the first version. If word-level diff is deferred, make that an
honest product constraint.

## Review Notes Sidecar

Shore's native sidecar is `review-notes.json`. It is a transport/import file for ordered review
notes, not a persisted `.shore/` session-state format.

The sidecar should stay review-oriented and concise:

- one changeset summary
- file summaries in narrative order
- hunk-level or line-level review notes with clear title and body text

Review notes belong beside the code. The first UI should render notes spatially near the targeted
hunk or row, and note navigation should move through hunk-specific notes in the review stream.

The sidecar file order is intentional. Shore should preserve that order when it differs from the raw
Git diff order.

Example native sidecar shape:

```json
{
  "schema": "shore.review-notes",
  "version": 1,
  "summary": "Review notes for the current change",
  "files": [
    {
      "path": "src/model/mod.rs",
      "notes": [
        {
          "id": "note:decode-json",
          "title": "decode_json keeps the error boundary explicit",
          "body": "Full review note body in markdown.",
          "target": {
            "side": "new",
            "startLine": 9,
            "endLine": 9
          },
          "author": "reviewer",
          "source": "codex",
          "createdAt": "2026-05-09T00:04:07.818Z",
          "tags": ["parser"],
          "confidence": "high"
        }
      ]
    }
  ]
}
```

Shore can import Hunk-compatible `agent-context.json` through a legacy adapter, but Shore's native
sidecar is `review-notes.json`.

When Shore imports these sidecars through `shore notes apply`, it persists immutable imported-note
events under `.shore/events/`. For large note bodies, Shore may store the body text as a
content-addressed artifact under `.shore/artifacts/notes/` while keeping the event payload bounded.

## Future Session Model

Daemon/session coordination is not v1, but the review model should be ready for it.

Future external tools should be able to ask for operations like:

- get current review state
- navigate to next or previous hunk
- navigate to next or previous note
- select a file or hunk
- add a live comment
- clear live comments
- dump the current review context

The model should use stable IDs and serializable state so this can be added without rewriting the
core.

## Candidate Rust Stack

Likely starting point:

- `ratatui` plus `crossterm` for the terminal UI
- `serde` and `serde_json` for sidecar and state JSON
- shelling out to `git` for repository data
- snapshot tests for the headless model

Be careful with stateful widget idioms. Shore should keep model state authoritative and make TUI
widgets render from it.

## Testing Strategy

Start with headless tests before TUI tests.

Useful fixtures:

- multi-file diffs with narrative sidecar ordering
- untracked file diffs
- rename diffs
- binary and mode-only changes
- notes on context rows inside a hunk
- current hunk has no notes, then `{` or `}` should resolve relative to the full stream
- current hunk past the last note, then `}` should clamp to the last hunk with notes rather than
  wrap
- terminal resize causing geometry recomputation
- large synthetic changesets

TUI tests should come after the review-stream model can prove geometry, navigation, and note
placement without a terminal.

## Non-Goals

Shore should not initially try to be:

- a general Git porcelain
- a complete nunk replacement
- a web review UI
- a summarizer detached from the code
- a terminal framework experiment

The narrow goal is a reliable terminal review surface for tool-assisted or review-heavy changesets.
