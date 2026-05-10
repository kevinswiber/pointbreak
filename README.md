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

The current executable surfaces are `shore show`, `shore dump`, and
`shore review publish`.

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

`shore review publish` is the first durable local-state command:

```bash
shore review publish [--repo <path>] [--review-notes <path> | --legacy-hunk-agent-context <path>]
```

Behavior:

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it;
  durable state is created at the Git worktree root.
- The command creates and uses local `.shore/` storage and adds `.shore/` to the worktree
  `.gitignore` when needed.
- `.shore/events/` stores immutable local event files. `.shore/state.json` is a rebuildable
  projection, not the authority.
- `--review-notes <path>` and `--legacy-hunk-agent-context <path>` are recorded as sidecar
  observation provenance. Sidecars remain transport/import inputs; they are not Shore-owned
  persisted session state.
- Explicit sidecar inputs and `--log-file <path>` are command helpers and are excluded from the
  published reviewed file set and content-derived revision fingerprint for that command.
- Output is compact `shore.publish` JSON with IDs, event counts, diagnostics, and the `statePath`.

`shore review publish` does not add a daemon, delivery queue, acknowledgement flow, intervention
runtime, async or remote storage backend, or note mutation. `.shore/events/` is the local
authoritative event log, not a mailbox or retry queue.

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
