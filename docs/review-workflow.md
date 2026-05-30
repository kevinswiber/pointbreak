# Review Workflow

This document describes the intended end-to-end workflow for reviewing a
tool-assisted change with Shoreline today. Command reference details live in the
`README.md`; this is the narrative version that explains *when* to run each
command and *why*. If the change was authored by a coding agent, start with
[Agent authoring handoffs](agent-authoring.md) for the capture-at-end-of-work loop.

## What Shoreline reviews

Shoreline reviews a **ReviewUnit**: the base endpoint, the target endpoint, and a
captured diff snapshot taken at a single moment. V1 captures the local Git
worktree from `HEAD` to the working tree, including untracked files.

Each ReviewUnit gets its own immutable snapshot artifact. Anything you record
afterwards — observations, input requests, assessments — attaches to that
ReviewUnit and lives in the durable `.shore/events/` log.

## The workflow at a glance

1. Start from a Git worktree containing the change you want to review.
2. Capture a ReviewUnit with `shore review capture`.
3. Inspect what was captured with `shore review unit show` and
   `shore review history`.
4. Record review facts as you read the diff:
   - **Observations** are notes you want preserved.
   - **Input requests** are durable pause/decision requests for someone else.
   - **Assessments** are the current review call for the ReviewUnit (or for a
     file, range, or specific fact within it).
5. Optionally use `shore notes apply`, `shore dump`, and `shore show` for
   import and read-only inspection of the older review-stream surface.

The rest of this document walks through each step.

## 1. Start in a worktree with the change

Shoreline expects to run inside a Git worktree where the working tree differs from
`HEAD`. The change can come from anywhere — a coding agent, a teammate's WIP
branch, your own edits — but it must be present in the working tree before
capture. Shoreline reads the diff from `git`; it does not summarize prior commits
on its own.

```bash
cd path/to/worktree
git status        # confirm the changes you expect are present
```

The first Shoreline command run in the worktree creates local `.shore/`
storage and registers `.shore/` in the repository-local `.git/info/exclude`
when it is not already ignored. This keeps `.shore/` out of `git status`
without modifying your tracked `.gitignore` or dirtying the working tree. If
`.shore/` is already ignored — for example by a project `.gitignore` entry —
Shoreline leaves the ignore files untouched.

## 2. Capture a ReviewUnit

```bash
shore review capture
```

`shore review capture` records a `review_unit_captured` event and writes the
captured snapshot as an immutable Shoreline-owned artifact. The output document is
`shore.review-capture` JSON and includes:

- the ReviewUnit ID
- the revision ID
- the snapshot ID
- the snapshot artifact's canonical content hash

You can pin later commands to the captured ReviewUnit with `--review-unit
<id>`. When only one ReviewUnit exists in `.shore/`, the read commands pick
it automatically. When multiple exist, list them with
`shore review unit list` and pass the ID you want.

The snapshot is now frozen. Re-running `shore review capture` later creates a
new ReviewUnit; it does not mutate the previous one.

Write commands such as `shore review observation add`,
`shore review input-request open`, and `shore review assessment add` accept
`--review-unit <id>`. When more than one captured ReviewUnit is current, pass
the ID from capture output or `shore review unit list`; otherwise writes fail
with an ambiguity error.

## 3. Inspect what was captured

Three read surfaces describe ReviewUnits, and they answer different questions:

```bash
shore review unit list     # what ReviewUnits exist in .shore/
shore review unit show     # composite ReviewUnit view (narrative + snapshot)
shore review history       # chronological raw event listing
```

For a visual, cross-linked view of the whole store — an event timeline, composite per-ReviewUnit
pages, and captured diffs annotated with their review facts — run `shore inspect` to open a local
web UI (see the [CLI reference](cli-reference.md)). The commands below remain the scriptable surface.

### `shore review unit list`

`shore review unit list` projects every `review_unit_captured` event into a
flat directory of ReviewUnits. It is the discovery surface — start here when
`shore review unit show` errors with `multiple captured review units; pass
--review-unit`, or whenever you need to pick an ID for `--review-unit <id>`.

It returns `shore.review-unit-list` JSON with `eventSetHash`, `eventCount`,
`reviewUnitCount`, and an `entries` array whose elements include
`reviewUnitId`, `capturedAt`, `revisionId`, `snapshotId`, `source`, `base`,
`target`, and `snapshotArtifactContentHash`. Entries are sorted by capture
time so the newest ReviewUnit appears last.

```bash
shore review unit list --pretty
```

### `shore review unit show`

`shore review unit show` is the composite view of one ReviewUnit. It returns
`shore.review-unit` JSON containing:

- ReviewUnit identity and event-set freshness metadata
- summary counts and current assessment status
- native observations, input requests, and assessments
- imported adapter notes
- projection rows (narrative-first, then snapshot-complete)
- diagnostics

Narrative rows (native facts and imported notes) appear before the snapshot
remainder, but the snapshot remainder still includes every captured file,
metadata row, hunk header, and diff row. Track filters narrow narrative facts
without changing snapshot completeness.

```bash
shore review unit show --pretty
shore review unit show --track agent:codex
shore review unit show --include-body
```

### `shore review history`

`shore review history` is the chronological raw-event listing across the
entire `.shore/events/` log — across ReviewUnits if there is more than one.
It is the place to answer "what happened, in what order?" rather than
"what does this ReviewUnit look like right now?".

```bash
shore review history --pretty
shore review history --event-type review-observation-recorded
shore review history --review-unit <id> --include-body
```

`eventSetHash` and `eventCount` describe the full validated event set used to
build the document, even when filters return only a subset of entries.
History preserves duplicate semantic events as separate entries; it does not
collapse them or pick "winners".

## 4. Record review facts

The three event families below are append-only. Each writes one durable event
per call. Read surfaces collapse same-semantic-ID writes to one logical row
and surface a duplicate diagnostic.

### Observations

An observation is a durable note for a ReviewUnit, a file, or a line range.
Observations are append-only; corrections are new observations that name the
older observation through `--supersedes`.

```bash
# Review-wide observation
shore review observation add \
  --track agent:codex \
  --title "Check error handling near IO boundary"

# File-targeted observation
shore review observation add \
  --track agent:codex \
  --title "Untrusted input flows here" \
  --file src/lib.rs

# Range-targeted observation, with a body from a file
shore review observation add \
  --track human:kevin \
  --title "Worth a unit test" \
  --file src/lib.rs --start-line 42 --end-line 58 \
  --body-file notes/lib-42.md

# Replay observations for one track
shore review observation list --track agent:codex --pretty

# Include bodies on read
shore review observation list --include-body
```

Bodies may come from `--body`, `--body-file`, or `--body-stdin`. Large bodies
are stored as Shoreline-owned content-addressed artifacts; command output never
exposes those paths.

### Input requests

An input request is a durable pause/decision request. Use it when a reviewer
or tool needs an explicit answer before proceeding. `--mode` defaults to
`operative`; `advisory` requests are still durable and visible but do not
imply that a cooperative client must pause.

```bash
shore review input-request open \
  --track human:kevin \
  --title "Need approval to land schema change" \
  --reason manual-decision-required

shore review input-request list                 # defaults to open
shore review input-request list --status all
shore review input-request fetch <input-request-id> --include-body

shore review input-request respond <input-request-id> \
  --outcome approved \
  --reason "discussed in chat, ok to land"
```

`--reason` on the request is the classification axis (`manual-decision-required`,
`ambiguous-state`, `unsafe-action`, etc.). `--outcome` on the response is a
separate axis (`approved`, `rejected`, `dismissed`, `superseded`, `abandoned`).

Multiple different response events are preserved as append-only facts and
make the input request `ambiguous` rather than picking a timestamp winner.

### Assessments

An assessment is the current review call for a ReviewUnit, a file, a range,
or a specific native observation/input request/assessment in the same
ReviewUnit. V1 values: `accepted`, `accepted-with-follow-up`, `needs-changes`,
and `needs-clarification`.

```bash
shore review assessment add \
  --track human:kevin \
  --assessment accepted \
  --summary "looks good, ship it"

# Assessment that replaces an older one
shore review assessment add \
  --track human:kevin \
  --assessment accepted-with-follow-up \
  --summary "supersedes earlier needs-changes after offline discussion" \
  --replaces <older-assessment-id>

shore review assessment show --pretty
shore review assessment show --all --include-summary
```

`--replaces` is the only V1 relationship that removes an older assessment
from the current set.
`--related-observation` and `--related-input-request` record evidence links;
they do not mutate observations or close input requests (use
`shore review input-request respond` for the input-request lifecycle).

State-change outcomes such as deferred, split-out, overridden, and superseded
are recorded as observations tagged with `state-change:*`. Use
`shore review assessment` for review calls and `shore review observation add`
with a concrete tag such as `--tag state-change:deferred` for state-change
evidence.

## 5. Compatibility and import surfaces (optional)

The older review-stream surface is still useful for working with sidecar
review notes from other tools, or for a quick read-only look at the diff.

### `shore notes apply`

`shore notes apply` imports a native `review-notes.json` sidecar into durable
storage without publishing a revision.

```bash
shore notes apply --repo . --review-notes review-notes.json
```

It writes one immutable `review_note_imported` event per imported note.
Imported notes appear in `shore review unit show` as adapter notes, and in
`shore dump` and `shore show` as note rows in the review stream.

### `shore dump`

`shore dump` emits the headless review-stream JSON for the current working
tree. It is a useful integration surface for non-Shoreline frontends and tests.

```bash
shore dump --pretty
shore dump --review-notes review-notes.json
```

`shore dump` operates on the **working tree at run time**, not on a captured
ReviewUnit. It does not include native observations, input requests, or
assessments — use `shore review unit show` for those.

### `shore show`

`shore show` is the read-only terminal review view over the same review
stream. It opens a split pane and supports a small set of keybindings:

```bash
shore show
shore show --review-notes review-notes.json
```

- `q` / Esc / Ctrl+C — quit
- `j` / `k` or Up / Down — move by row
- `[` / `]` — move through diff sections
- `{` / `}` — move through noted sections
- `r` — re-ingest the working tree and reload

Like `shore dump`, `shore show` does not yet project native observations,
input requests, or assessments; it renders the diff, imported notes, and any
stale/orphan note rows.

## 6. Concepts you need to know

### Durable event facts vs. rebuildable projections

Shoreline separates **authoritative facts** from **derived views**:

- `.shore/events/` is the authoritative append-only log. Each file is one
  immutable durable fact. Events are never moved, retried in place, or
  rewritten on read.
- `.shore/artifacts/` holds the immutable support records that events bind to:
  captured ReviewUnit snapshots, and the optional content-addressed bodies
  for large observation, input request, and assessment payloads.
- `.shore/state.json` is a **rebuildable projection**, not the authority. It
  may be deleted and regenerated; freshness against the current event set is
  verified through `eventSetHash`.

If `.shore/state.json` looks stale or inconsistent, Shoreline rebuilds it from
the event log. Do not write to `state.json` yourself, and do not depend on
its internal shape.

### Command-output JSON is the integration surface

The stable surface for automation is **command-output JSON documents**:
`shore.review-capture`, `shore.review-history`, `shore.review-unit`,
`shore.review-observation-add` / `-list`,
`shore.review-input-request-open` / `-list` / `-fetch` / `-respond`,
`shore.review-assessment-add` / `-show`, and `shore.notes-apply`.

These documents expose semantic IDs, content hashes, and freshness metadata.
Raw event files, event filenames, artifact paths, and `.shore/state.json` are
Shoreline-owned storage details. They can change without a deprecation cycle.

### Old dump/show stream vs. ReviewUnit ledger

There are two overlapping read surfaces today:

- The **review-stream surface** (`shore dump`, `shore show`) operates on the
  working tree at run time and renders the unified diff plus imported notes.
  It is the older surface and is well-suited to import workflows and quick
  read-only viewing.
- The **ReviewUnit ledger** (`shore review capture` plus the
  `shore review observation`, `input-request`, `assessment`, `history`, and
  `unit show` commands) operates on a frozen captured snapshot plus the
  durable event log. It is the surface for recording review facts.

Native observations, input requests, and assessments appear in
`shore review unit show` but are not yet projected into `shore dump` or
`shore show`. If you need a single view that combines a captured snapshot
with all ledger facts, use `shore review unit show`.

### Tracks

Every observation, input request, and assessment belongs to a required
`--track`. Tracks are **review lanes**, such as `agent:codex` or
`human:kevin`. They are not actor identity. Writer provenance — who actually
ran the command, with which tool — is recorded separately in the event
envelope from local Git config and the Shoreline tool identity. Pick track names
that group facts the way you want to read them back, then let provenance
take care of itself.

### Bodies

Observation bodies, input request bodies, input request response
reasons, and assessment summaries all share the same input mechanics:
`--body` / `--body-file` / `--body-stdin` (or `--summary*` /
`--reason*`). Read commands omit body-like text by default and hydrate it
only when `--include-body` is passed. Small bodies stay inline in the event
payload; larger bodies move to content-addressed artifacts. From a user
perspective, the difference is invisible — read commands return the same
shape either way.

### IDs are opaque

Shoreline exposes several kinds of IDs in its output: ReviewUnit IDs, revision
IDs, snapshot IDs, observation IDs, input request IDs, input request response
IDs, assessment IDs, event IDs, and review-stream row IDs. **Treat them all
as opaque strings.** They are stable and safe to use as keys or to pass back
into other commands, but their internal format is an implementation detail.
In particular:

- Do not parse review-stream `row.id` values, derive ordering from them
  lexically, or assume any particular width or prefix. Use the sibling
  `ordinal` field if you need a numeric position.
- Do not parse storage filenames. Event filenames, snapshot artifact
  filenames, and note-body artifact filenames are derived from internal
  hashes and may change without a deprecation cycle.
- Do not depend on artifact paths or the internal shape of
  `.shore/state.json`.

## 7. A small realistic walkthrough

The block below captures the typical sequence: confirm the change, capture
the ReviewUnit, inspect it, record a couple of observations, open an
input request, respond to it, and land an assessment.

```bash
# 0. Confirm the worktree has the changes you want to review.
cd ~/src/myproject
git status

# 1. Capture a ReviewUnit. This freezes the current diff as a snapshot.
#    `shore review capture` emits compact JSON only; pipe through jq if you
#    want to read it.
shore review capture | jq .

# 2. Read the captured ReviewUnit (composite view, narrative + snapshot).
shore review unit show --pretty | less

# 3. Record observations as you read the diff.
shore review observation add \
  --track agent:codex \
  --title "Check error handling near IO boundary" \
  --file src/io.rs --start-line 88 --end-line 104 \
  --body "The new branch swallows io::ErrorKind::Interrupted silently."

shore review observation add \
  --track human:kevin \
  --title "Unit test for the new retry path" \
  --file src/io.rs --start-line 120 --end-line 135

shore review observation list --pretty

# 4. Open an input request when you need a decision from someone else.
shore review input-request open \
  --track human:kevin \
  --title "Approve schema migration before landing" \
  --reason manual-decision-required \
  --file db/migrations/0042_users.sql

# Someone reads the open queue and responds to it.
shore review input-request list --status open
shore review input-request respond <input-request-id> \
  --outcome approved \
  --reason "verified backfill plan with on-call DBA"

# 5. Record the final assessment for the ReviewUnit.
shore review assessment add \
  --track human:kevin \
  --assessment accepted-with-follow-up \
  --summary "ship it; follow up on the retry-path unit test"

# 6. Verify the durable record.
shore review assessment show --pretty
shore review history --pretty | less
```

That is the full V1 workflow. Anything beyond it — notifications, daemons,
multi-writer coordination, automatic delivery — is intentionally out of
scope and will be addressed by future, separately-designed subsystems.
