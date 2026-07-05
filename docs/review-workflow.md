# Review Workflow

This document describes the intended end-to-end workflow for reviewing a
tool-assisted change with Shoreline today. Command reference details live in the
`README.md`; this is the narrative version that explains *when* to run each
command and *why*. If the change was authored by a coding agent, start with
[Agent authoring handoffs](agent-authoring.md) for the capture-at-end-of-work loop.

## What Shoreline reviews

Shoreline reviews a **revision**: the base endpoint, the target endpoint, and a
captured diff snapshot taken at a single moment. Capturing a revision is the one **generative move**
in the workflow — proposing a captured work object for others to assert facts about — while "review"
stays the surface verb. V1 captures one of two shapes:
the local Git worktree from `HEAD` to the working tree, including untracked files
(the default); or, with `shore capture --base <rev>`, the committed range
between two resolved commits (`<rev>..--target`, target defaulting to `HEAD`),
read as a tree diff with no working-tree involvement.

Each revision binds an immutable **object artifact** by content hash. The artifact body is
content-only, so two revisions capturing the same change in different worktrees share one
byte-identical artifact — they converge on a single **object** — rather than each owning a distinct
copy. The revision is the captured unit's identity; the object is a hash of its content alone.
Anything you record afterwards — observations, input requests, assessments — attaches to that
revision and lives in the durable `events/` log.

A later capture can record that it **supersedes** one or more earlier revisions, forming a
fork-tolerant succession graph. A reviewer is free to counter-propose by capturing their own revision
that supersedes yours. Successive rounds never mutate the captured snapshots; the thread's current
**head** is derived from the supersession graph, and when two captures supersede the same predecessor
the **competing heads** are surfaced rather than one silently winning.

## The workflow at a glance

1. Start from a Git worktree containing the change you want to review.
2. Capture a revision with `shore capture`.
3. Inspect what was captured with `shore review show` and
   `shore history`.
4. Record review facts as you read the diff:
   - **Observations** are notes you want preserved.
   - **Input requests** are durable pause/decision requests for someone else.
   - **Assessments** are the current review call for the revision (or for a
     file, range, or specific fact within it).
5. Optionally use `shore notes apply`, `shore dump`, and `shore show` for
   import and read-only inspection of the older review-stream surface.

The rest of this document walks through each step.

## 1. Start in a worktree with the change

Shoreline runs inside a Git worktree. For the default capture the working tree
must differ from `HEAD`; the change can come from anywhere — a coding agent, a
teammate's WIP branch, your own edits — but it must be present in the working
tree before capture. Shoreline reads the diff from `git`; it does not summarize
prior commits on its own. If the change is already committed, capture the
committed range directly with `shore capture --base <rev>` (see
[section 2](#2-capture-a-revision)) instead of recreating it in the working
tree.

```bash
cd path/to/worktree
git status        # confirm the changes you expect are present
```

By default the store is the shared common-dir store at `.git/shore`, under the clone's Git common
directory — every worktree of the clone resolves the same store, and because it lives inside `.git`
it never appears in `git status`, so ordinary captures never touch the working tree at all. Opting
into an ephemeral worktree (`shore store mode ephemeral`) or writing a `--local` identity override
generates a committed `.shore/.gitignore` (two lines: `data/` + `*.local.json`) that keeps the
worktree-local store and the private overrides out of `git status`; the file is visible, meant to
be committed, and survives clone. If the paths are already ignored — for example by a project
`.gitignore` entry — Shoreline generates nothing and leaves your ignore files untouched. Nothing
writes the hidden `.git/info/exclude` anymore.

## 2. Capture a revision

```bash
shore capture
```

`shore capture` records a `work_object_proposed` event and writes the
captured snapshot as an immutable Shoreline-owned object artifact. The output document is
`shore.review-capture` JSON and includes:

- the revision ID
- the object ID (the content-only identity)
- the object artifact's canonical content hash

You can pin later commands to the captured revision with `--revision
<id>`. When only one revision exists in the store, commands that need a
current revision pick it automatically. When multiple exist, list them with
`shore review revisions` and pass either the exact revision ID or seed a
supersession thread with `--revision <id>`.

The snapshot is now frozen. Re-running `shore capture` later creates a
new revision; it does not mutate the previous one.

### Capturing a committed range

When the change is already committed and the working tree is clean, capture the
landed range instead of recreating a working-tree diff:

```bash
shore capture --base <commit-before-the-change>   # target defaults to HEAD
shore capture --base <rev> --target <rev>         # explicit range
```

`--base`/`--target` resolve any rev (a branch, tag, `HEAD~N`, or commit OID) to a
commit; annotated tags peel, and a non-commit or unknown rev is rejected with an
honest error. The capture is the `base..target` tree diff — both endpoints are
`git_commit`, no working-tree or untracked state is read, and no worktree path
appears in the output. This is the supported way to review after landing: never
rewrite history (for example `git reset --soft`) to manufacture a worktree diff.

A post-landing range capture is a second current revision alongside any
worktree capture, so disambiguate later reads and writes with `--revision
<id>`, exactly as for any multi-capture store. Recording the
landed commit, choosing a canonical capture, and revision lifecycle remain open
follow-ups.

### Scoping a capture to a subtree

In a monorepo a review is usually about one subtree, but the worktree and the
commits interleave changes across many. Scope the capture with `--path` so
unrelated changes stay out of the reviewer's diff and out of the revision's
identity:

```bash
# review only what changed under packages/foo, in the current worktree
shore capture --path packages/foo

# same, over an explicit range
shore capture --base v1.2.0 --target HEAD --path docs/spec
```

`--path` takes a native git pathspec, is repeatable, and scopes tracked and
untracked files alike; a scope that matches no changed files is an error. See
[`shore capture`](./cli-reference.md#shore-review-capture) for the full
semantics.

Record a succession round by naming the revisions a new capture supersedes:

```bash
shore capture --supersedes <revision-id>
shore capture --supersedes <revision-id> --supersedes <other-revision-id>
```

The `supersedes` set is order-independent and may name more than one predecessor. There is no
separate lineage command or declared lineage id — the thread is the connected component of the
`supersedes` graph, and its current **head** is derived from that graph. When two captures supersede
the same predecessor, the resulting **competing heads** are surfaced as competing, never collapsed to
a single winner.

Write commands such as `shore observation add`,
`shore input-request open`, and `shore assessment add` accept
`--revision <id>`. When more than one captured revision is current, pass
the ID from capture output or `shore review revisions`; otherwise writes fail
with an ambiguity error.

Supersession makes that ambiguity contextual. A revision-scoped read seeds on `--revision <id>` and
resolves that revision's thread head; unscoped current selection remains ambiguous when multiple
unrelated captures exist. Routine list, history, and exact-revision reads have no always-on
ambiguous-current warning. A thread-level read may report `stale_by_superseding_revision` when a fact
targets a revision that a newer revision supersedes, but exact-revision reads remain valid for
superseded revisions.

## 3. Inspect what was captured

Three read surfaces describe revisions, and they answer different questions:

```bash
shore review revisions     # what revisions exist in the store
shore review show          # composite revision view (narrative + snapshot)
shore history       # chronological raw event listing
```

For a visual, cross-linked view of the whole store — an event timeline, composite per-revision
pages, supersession-thread pages, and captured diffs annotated with their review facts — run
`shore inspect` to open a local web UI (see the [CLI reference](cli-reference.md)). The commands
below remain the scriptable surface.

### `shore review revisions`

`shore review revisions` projects every `work_object_proposed` event into a
flat directory of revisions. It is the discovery surface — start here when
`shore review show` errors with `multiple captured revisions; pass
--revision`, or whenever you need to pick an ID for `--revision <id>`.

It returns `shore.review-revision-list` JSON with `eventSetHash`, `eventCount`,
`revisionCount`, and an `entries` array whose elements include
`revisionId`, `capturedAt`, `objectId`, `source`, `base`,
`target`, and `objectArtifactContentHash`. Entries are sorted by capture
time so the newest revision appears last. Pass `--object <object-id>` to list only the revisions
that share one content object — a listing lens that may span threads, never a head selector.

```bash
shore review revisions --pretty
```

When a revision supersedes another, the list/read projections build the supersession DAG and surface
each thread's competing heads. That view is a thread over immutable captures, not an interdiff
renderer; this release has no interdiff or stack DAG. Capture facts remain signable through the
generic `EventToBeSigned` producer-fact view and ADR-0004's Dead Simple Signing Envelope (DSSE)
pre-authentication encoding.

### `shore review show`

`shore review show` is the composite view of one revision. It returns
`shore.review-revision` JSON containing:

- revision identity and event-set freshness metadata
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
shore review show --pretty
shore review show --revision <revision-id>
shore review show --track agent:codex
shore review show --include-body
```

Passing `--revision <id>` seeds head selection on that revision and resolves its thread's current
head; an intra-thread fork is reported as competing revisions.

### `shore history`

`shore history` is the chronological raw-event listing across the
entire `events/` log — across revisions if there is more than one.
It is the place to answer "what happened, in what order?" rather than
"what does this revision look like right now?".

```bash
shore history --pretty
shore history --event-type review-observation-recorded
shore history --revision <id> --include-body
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

An observation is a durable note for a revision, a file, or a line range.
Observations are append-only; corrections are new observations that name the
older observation through `--supersedes`.

```bash
# Review-wide observation
shore observation add \
  --track agent:codex \
  --title "Check error handling near IO boundary"

# File-targeted observation
shore observation add \
  --track agent:codex \
  --title "Untrusted input flows here" \
  --file src/lib.rs

# Range-targeted observation, with a body from a file
shore observation add \
  --track human:kevin \
  --title "Worth a unit test" \
  --file src/lib.rs --start-line 42 --end-line 58 \
  --body-file notes/lib-42.md

# Replay observations for one track
shore observation list --track agent:codex --pretty

# Include bodies on read
shore observation list --include-body
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
shore input-request open \
  --track human:kevin \
  --title "Need approval to land schema change" \
  --reason manual-decision-required

shore input-request list                 # defaults to open
shore input-request list --status all
shore input-request show <input-request-id> --include-body

shore input-request respond <input-request-id> \
  --outcome approved \
  --reason "discussed in chat, ok to land"
```

`--reason` on the request is the classification axis (`manual-decision-required`,
`ambiguous-state`, `unsafe-action`, etc.). `--outcome` on the response is a
separate axis (`approved`, `rejected`, `dismissed`, `superseded`, `abandoned`).

Multiple different response events are preserved as append-only facts and
make the input request `ambiguous` rather than picking a timestamp winner.

### Assessments

An assessment is the current review call for a revision, a file, a range,
or a specific native observation/input request/assessment in the same
revision. CLI input and human-facing display use `accepted`,
`accepted-with-follow-up`, `needs-changes`, and `needs-clarification`; command
JSON output uses the matching `snake_case` values.

```bash
shore assessment add \
  --track human:kevin \
  --assessment accepted \
  --summary "looks good, ship it"

# Assessment that replaces an older one
shore assessment add \
  --track human:kevin \
  --assessment accepted-with-follow-up \
  --summary "supersedes earlier needs-changes after offline discussion" \
  --replaces <older-assessment-id>

shore assessment show --pretty
shore assessment show --all --include-summary
```

`--replaces` is the only V1 relationship that removes an older assessment
from the current set.
`--related-observation` and `--related-input-request` record evidence links;
they do not mutate observations or close input requests (use
`shore input-request respond` for the input-request lifecycle).

State-change outcomes such as deferred, split-out, overridden, and superseded
are recorded as observations tagged with `state-change:*`. Use
`shore assessment` for review calls and `shore observation add`
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
Imported notes appear in `shore review show` as adapter notes, and in
`shore dump` and `shore show` as note rows in the review stream.

### `shore dump`

`shore dump` emits the headless review-stream JSON for the current working
tree. It is a useful integration surface for non-Shoreline frontends and tests.

```bash
shore dump --pretty
shore dump --review-notes review-notes.json
```

`shore dump` operates on the **working tree at run time**, not on a captured
revision. It does not include native observations, input requests, or
assessments — use `shore review show` for those.

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

Shoreline separates **authoritative facts** from **derived views**. The paths below are relative to
the resolved store directory — `.git/shore` by default, or a worktree-local `.shore/data/` when the
worktree is ephemeral:

- `events/` is the authoritative append-only log. Each file is one
  immutable durable fact. Events are never moved, retried in place, or
  rewritten on read.
- `artifacts/` holds the immutable support records that events bind to:
  captured revision object artifacts, and the optional content-addressed bodies
  for large observation, input request, and assessment payloads.
- `state.json` is a **rebuildable projection**, not the authority. It
  may be deleted and regenerated; freshness against the current event set is
  verified through `eventSetHash`.

If `state.json` looks stale or inconsistent, Shoreline rebuilds it from
the event log. Do not write to `state.json` yourself, and do not depend on
its internal shape.

### Command-output JSON is the integration surface

The stable surface for automation is **command-output JSON documents**:
`shore.review-capture`, `shore.review-history`, `shore.review-revision`,
`shore.review-observation-add` / `-list`,
`shore.review-input-request-open` / `-list` / `-fetch` / `-respond`,
`shore.review-assessment-add` / `-show`, and `shore.notes-apply`.

These documents expose semantic IDs, content hashes, and freshness metadata.
Raw event files, event filenames, artifact paths, and `state.json` are
Shoreline-owned storage details. They can change without a deprecation cycle.

### Old dump/show stream vs. revision ledger

There are two overlapping read surfaces today:

- The **review-stream surface** (`shore dump`, `shore show`) operates on the
  working tree at run time and renders the unified diff plus imported notes.
  It is the older surface and is well-suited to import workflows and quick
  read-only viewing.
- The **revision ledger** (`shore capture` plus the
  `shore observation`, `input-request`, `assessment`, `history`, and
  `show` commands) operates on a frozen captured snapshot plus the
  durable event log. It is the surface for recording review facts.

Native observations, input requests, and assessments appear in
`shore review show` but are not yet projected into `shore dump` or
`shore show`. If you need a single view that combines a captured snapshot
with all ledger facts, use `shore review show`.

### Tracks

Every observation, input request, and assessment belongs to a required
`--track`. Tracks are **review lanes**, such as `agent:codex` or
`human:kevin`. They are not actor identity. Writer provenance — who actually
ran the command — is recorded separately in the event envelope: the writer
`actorId` (from local Git config, or an explicit `actor:agent:<name>` set via
`SHORE_ACTOR_ID`) and the `producer` that wrote the event. The human a resolved
agent acts on behalf of comes from the checked-in `.shore/delegates.json` map at
read time. Pick track names that group facts the way you want to read them back,
then let provenance take care of itself.

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

Shoreline exposes several kinds of IDs in its output: revision IDs, object
IDs, observation IDs, input request IDs, input request response
IDs, assessment IDs, event IDs, and review-stream row IDs. **Treat them all
as opaque strings.** They are stable and safe to use as keys or to pass back
into other commands, but their internal format is an implementation detail.
This opacity is load-bearing: a content id is derived from content, so two clones
capturing identical content converge on the same revision and object IDs without
coordinating, and a store migration may rename the on-disk files that hold them
while the IDs themselves stay valid and unchanged. Read the IDs; never parse them.
In particular:

- Do not parse review-stream `row.id` values, derive ordering from them
  lexically, or assume any particular width or prefix. Use the sibling
  `ordinal` field if you need a numeric position.
- Do not parse storage filenames. Event filenames, object artifact
  filenames, and note-body artifact filenames are derived from internal
  hashes and may change without a deprecation cycle.
- Do not depend on artifact paths or the internal shape of
  `state.json`.

## 7. A small realistic walkthrough

The block below captures the typical sequence: confirm the change, capture
the revision, inspect it, record a couple of observations, open an
input request, respond to it, and land an assessment.

```bash
# 0. Confirm the worktree has the changes you want to review.
cd ~/src/myproject
git status

# 1. Capture a revision. This freezes the current diff as a snapshot.
#    `shore capture` emits compact JSON only; pipe through jq if you
#    want to read it.
shore capture | jq .

# 2. Read the captured revision (composite view, narrative + snapshot).
shore review show --pretty | less

# 3. Record observations as you read the diff.
shore observation add \
  --track agent:codex \
  --title "Check error handling near IO boundary" \
  --file src/io.rs --start-line 88 --end-line 104 \
  --body "The new branch swallows io::ErrorKind::Interrupted silently."

shore observation add \
  --track human:kevin \
  --title "Unit test for the new retry path" \
  --file src/io.rs --start-line 120 --end-line 135

shore observation list --pretty

# 4. Open an input request when you need a decision from someone else.
shore input-request open \
  --track human:kevin \
  --title "Approve schema migration before landing" \
  --reason manual-decision-required \
  --file db/migrations/0042_users.sql

# Someone reads the open queue and responds to it.
shore input-request list --status open
shore input-request respond <input-request-id> \
  --outcome approved \
  --reason "verified backfill plan with on-call DBA"

# 5. Record the final assessment for the revision.
shore assessment add \
  --track human:kevin \
  --assessment accepted-with-follow-up \
  --summary "ship it; follow up on the retry-path unit test"

# 6. Verify the durable record.
shore assessment show --pretty
shore history --pretty | less
```

That is the full V1 workflow. Anything beyond it — notifications, daemons,
multi-writer coordination, automatic delivery — is intentionally out of
scope and will be addressed by future, separately-designed subsystems.
