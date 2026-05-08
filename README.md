# Shore

Shore is an experimental Rust terminal review tool for understanding what a coding agent changed
and why.

It is inspired by [hunk](https://github.com/modem-dev/hunk), but it is not intended to be a direct clone or fork. The goal is to build a
small, Rust-native agent-review core with a data model that is easy to reason about, test, and
eventually expose to other tools.

## Name

The name connects to Pointbreak and the idea of reviewing the wake of agent work once it reaches
shore. It also works as a verb: `shore up` can become the command that reviews and hardens an agent
changeset.

The metaphor should stay light. Command names should remain mostly plain and practical:

- `shore diff`
- `shore show`
- `shore up`
- `shore notes`
- `shore session`
- `shore dump`

## Product Intent

Shore is for agent-aware code review in a terminal. It should help a human reviewer inspect:

- the actual diff an agent left behind
- the rationale the agent attached to files and hunks
- the hunk stream in the order the reviewer should read it
- which notes are attached to which code rows
- enough recoverable session state that review context is not lost when a UI is restarted

The first version should be a focused terminal review tool, not a generic "AI diff" product.

## Inspiration And Lessons

Hunk is the practical inspiration: a terminal-first diff viewer with agent-context sidecars,
hunk-level notes, live review sessions, and keyboard navigation across notes.

The most important lesson from maintaining a [hunk fork](https://github.com/kevinswiber/hunk) is that the hard part is not simply drawing a
diff in a TUI. The hard part is keeping these behaviors aligned:

- file order
- hunk order
- row geometry
- scroll position
- selected hunk
- note anchors
- annotated-hunk navigation
- terminal resize behavior
- saved or live agent context

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
- annotated-hunk navigation cursors
- serializable review/session state

The TUI should be a projection of that model. Widgets may render state, but they should not become
the authoritative owners of scroll, selection, or navigation semantics.

## Initial Scope

The first milestone should be deliberately smaller than hunk.

Build:

- a Rust CLI binary
- working-tree `diff` support
- tracked and untracked file support
- unified-diff parsing into Shore's own file/hunk/row model
- an `agent-context.json` sidecar loader
- a split terminal diff view
- `[` and `]` navigation through the full hunk stream
- `{` and `}` navigation through annotated hunks
- snapshot and acceptance fixtures for the review model

Prefer shelling out to `git` at first. A VCS abstraction can come later if the model earns it.

## Explicit V1 Deferrals

Do not start by rebuilding all of hunk.

Defer:

- daemon and multi-session brokering
- external IPC protocol
- live comment mutation
- stash/show/pager/difftool modes
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

## Agent Context

The sidecar should stay review-oriented and concise:

- one changeset summary
- file summaries in narrative order
- hunk-level or line-level annotations with real rationale

Agent context belongs beside the code. The first UI should render notes spatially near the annotated
hunk or row, and note navigation should move through hunk-specific notes in the review stream.

The sidecar file order is intentional. Shore should preserve that order when it differs from the raw
Git diff order.

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
- current hunk not annotated, then `{` or `}` should resolve relative to the full stream
- current hunk past the last annotation, then `}` should clamp to the last annotated hunk rather
  than wrap
- terminal resize causing geometry recomputation
- large synthetic changesets

TUI tests should come after the review-stream model can prove geometry, navigation, and note
placement without a terminal.

## Non-Goals

Shore should not initially try to be:

- a general Git porcelain
- a complete nunk replacement
- a web review UI
- an AI summarizer detached from the code
- a terminal framework experiment

The narrow goal is a reliable terminal review surface for agent-produced changesets.

