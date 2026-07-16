# Review Workflow

This document describes the intended end-to-end workflow for reviewing a
tool-assisted change with Pointbreak today. Command reference details live in the
`README.md`; this is the narrative version that explains *when* to run each
command and *why*. If the change was authored by a coding agent, start with
[Agent authoring handoffs](agent-authoring.md) for the capture-at-end-of-work loop.

## What Pointbreak reviews

Pointbreak reviews a **revision**: the base endpoint, the target endpoint, and a
captured diff snapshot taken at a single moment. Capturing a revision is the one **generative move**
in the workflow — proposing a captured work object for others to assert facts about — while "review"
stays the surface verb. V1 captures several Git-backed shapes: the local Git worktree from `HEAD` to
the working tree (the default, excluding untracked files unless `--include-untracked` is passed); the
committed range between two resolved commits with `pointbreak capture --base <rev>`
(`<rev>..--target`, target defaulting to `HEAD`), read as a tree diff with no working-tree
involvement; or explicit index-boundary captures with `pointbreak capture --staged` and
`pointbreak capture --unstaged`.

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
2. Capture a revision with `pointbreak capture`.
3. Inspect what was captured with `pointbreak revision show` and
   `pointbreak history`.
4. Record review facts as you read the diff:
   - **Observations** are notes you want preserved.
   - **Input requests** are durable pause/decision requests for someone else.
   - **Assessments** are the current review call for the revision (or for a
     file, range, or specific fact within it).

The rest of this document walks through each step.

## 1. Start in a worktree with the change

Pointbreak runs inside a Git worktree. For the default capture the working tree
must differ from `HEAD`; the change can come from anywhere — a coding agent, a
teammate's WIP branch, your own edits — but it must be present in the working
tree before capture. Pointbreak reads the diff from `git`; it does not summarize
prior commits on its own. If the change is already committed, capture the
committed range directly with `pointbreak capture --base <rev>` (see
[section 2](#2-capture-a-revision)) instead of recreating it in the working
tree.

```bash
cd path/to/worktree
git status        # confirm the changes you expect are present
```

By default the store is the shared common-dir store at `<git-common-dir>/pointbreak`, under the clone's Git common
directory — every worktree of the clone resolves the same store, and because it lives inside `.git`
it never appears in `git status`, so ordinary captures never touch the working tree at all. Opting
into an ephemeral worktree (`pointbreak store mode ephemeral`) or writing a `--local` identity override
generates a committed `.pointbreak/.gitignore` (two lines: `data/` + `*.local.json`) that keeps the
worktree-local store and the private overrides out of `git status`; the file is visible, meant to
be committed, and survives clone. If the paths are already ignored — for example by a project
`.gitignore` entry — Pointbreak generates nothing and leaves your ignore files untouched. Nothing
writes the hidden `.git/info/exclude` anymore.

## 2. Capture a revision

```bash
pointbreak capture
pointbreak capture --include-untracked
```

`pointbreak capture` records a `work_object_proposed` event and writes the
captured snapshot as an immutable Pointbreak-owned object artifact. The output document is
`pointbreak.review-capture` JSON and includes:

- the revision ID
- the object ID (the content-only identity)
- the object artifact's canonical content hash

You can pin later commands to the captured revision with `--revision
<id>`. When only one revision exists in the store, commands that need a
current revision pick it automatically. When multiple exist, list them with
`pointbreak revision list` and pass either the exact revision ID or seed a
supersession thread with `--revision <id>`.

The snapshot is now frozen. Re-running `pointbreak capture` later creates a
new revision; it does not mutate the previous one.

Default worktree capture is a combined `HEAD` to working-tree capture: staged
and unstaged tracked changes are both included because both differ from `HEAD`.
It follows `git diff HEAD` for untracked files, so untracked paths are ignored
unless you opt in with `--include-untracked`. In a repository with no commits
yet, `pointbreak capture --include-untracked` captures untracked initial files from
Git's empty tree to the working tree; `--root` is for capturing a committed
target from the empty tree.

If the selected source has no changed files, capture returns an error instead
of recording an accidental empty revision. The message suggests relevant flags
such as `--include-untracked`, `--staged`, or `--unstaged`. Pass
`--allow-empty` only when an empty revision is the intended review object.

When the index boundary matters, capture it explicitly:

```bash
pointbreak capture --staged
pointbreak capture --unstaged
pointbreak capture --unstaged --include-untracked
```

`--staged` records the current commit (or Git's empty tree in a repository with
no commits) to the captured index tree. `--unstaged` records the captured index
tree to the working tree, matching plain `git diff`: staged changes are in the
base endpoint, and untracked files are excluded unless `--include-untracked` is
present.

### Capturing a committed range

When the change is already committed and the working tree is clean, capture the
landed range instead of recreating a working-tree diff:

```bash
pointbreak capture --base <commit-before-the-change>   # target defaults to HEAD
pointbreak capture --base <rev> --target <rev>         # explicit range
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
pointbreak capture --path packages/foo

# same, over an explicit range
pointbreak capture --base v1.2.0 --target HEAD --path docs/spec
```

`--path` takes a native git pathspec, is repeatable, and scopes tracked files and
any enabled untracked-file synthesis alike; a scope that matches no changed
files is an error unless `--allow-empty` is passed. See
[`pointbreak capture`](./cli-reference.md#pointbreak-capture) for the full
semantics.

Record a succession round by naming the revisions a new capture supersedes:

```bash
pointbreak capture --supersedes <revision-id>
pointbreak capture --supersedes <revision-id> --supersedes <other-revision-id>
```

The `supersedes` set is order-independent and may name more than one predecessor. There is no
separate lineage command or declared lineage id — the thread is the connected component of the
`supersedes` graph, and its current **head** is derived from that graph. When two captures supersede
the same predecessor, the resulting **competing heads** are surfaced as competing, never collapsed to
a single winner.

Write commands such as `pointbreak observation add`,
`pointbreak input-request open`, and `pointbreak assessment add` accept
`--revision <id>`. When more than one captured revision is current, pass
the ID from capture output or `pointbreak revision list`; otherwise writes fail
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
pointbreak revision list     # what revisions exist in the store
pointbreak revision show          # composite revision view (narrative + snapshot)
pointbreak history       # chronological raw event listing
```

For a visual, cross-linked view of the whole store — an event timeline, composite per-revision
pages, supersession-thread pages, and captured diffs annotated with their review facts — run
`pointbreak inspect` to open a local web UI (see the [CLI reference](cli-reference.md)). The commands
below remain the scriptable surface.

### `pointbreak revision list`

`pointbreak revision list` projects every `work_object_proposed` event into a
flat directory of revisions. It is the discovery surface — start here when
`pointbreak revision show` errors with `multiple captured revisions; pass
--revision`, or whenever you need to pick an ID for `--revision <id>`.
Git reachability enriches each entry's status but never removes a recorded
revision from this unfiltered directory, even if its commit objects later disappear.

It returns `pointbreak.review-revision-list` JSON with `eventSetHash`, `eventCount`,
`revisionCount`, and an `entries` array whose elements include
`revisionId`, `capturedAt`, `objectId`, `source`, `base`,
`target`, and `objectArtifactContentHash`. Entries are sorted by capture
time so the newest revision appears last. Pass `--object <object-id>` to list only the revisions
that share one content object — a listing lens that may span threads, never a head selector.

```bash
pointbreak revision list --format json-pretty
```

When a revision supersedes another, the list/read projections build the supersession DAG and surface
each thread's competing heads. That view is a thread over immutable captures, not an interdiff
renderer; this release has no interdiff or stack DAG. Capture facts remain signable through the
generic `EventToBeSigned` producer-fact view and ADR-0004's Dead Simple Signing Envelope (DSSE)
pre-authentication encoding.

### `pointbreak revision show`

`pointbreak revision show` is the composite view of one revision. It returns
`pointbreak.review-revision` JSON containing:

- revision identity and event-set freshness metadata
- summary counts and current assessment status
- native observations, input requests, and assessments
- projection rows (narrative-first, then snapshot-complete)
- diagnostics

Narrative rows (native facts) appear before the snapshot
remainder, but the snapshot remainder still includes every captured file,
metadata row, hunk header, and diff row. Track filters narrow narrative facts
without changing snapshot completeness.

```bash
pointbreak revision show --format json-pretty
pointbreak revision show <revision-id>
pointbreak revision show --track agent:codex
pointbreak revision show --include-body
```

Passing a revision id seeds head selection on that revision and resolves its thread's current
head; an intra-thread fork is reported as competing revisions.

### `pointbreak history`

`pointbreak history` is the chronological raw-event listing across the
entire `events/` log — across revisions if there is more than one.
It is the place to answer "what happened, in what order?" rather than
"what does this revision look like right now?".

```bash
pointbreak history --format json-pretty
pointbreak history --event-type review-observation-recorded
pointbreak history --revision <id> --include-body
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
pointbreak observation add \
  --track agent:codex \
  --title "Check error handling near IO boundary"

# File-targeted observation
pointbreak observation add \
  --track agent:codex \
  --title "Untrusted input flows here" \
  --file src/lib.rs

# Range-targeted observation, with a body from a file
pointbreak observation add \
  --track human:kevin \
  --title "Worth a unit test" \
  --file src/lib.rs --start-line 42 --end-line 58 \
  --body-file notes/lib-42.md

# Replay observations for one track
pointbreak observation list --track agent:codex --format json-pretty

# Include bodies on read
pointbreak observation list --include-body
```

Bodies may come from `--body`, `--body-file`, or `--body-stdin`. Large bodies
are stored as Pointbreak-owned content-addressed artifacts; command output never
exposes those paths.

### Input requests

An input request is a durable pause/decision request. Use it when a reviewer
or tool needs an explicit answer before proceeding. `--mode` defaults to
`operative`; `advisory` requests are still durable and visible but do not
imply that a cooperative client must pause.

```bash
pointbreak input-request open \
  --track human:kevin \
  --title "Need approval to land schema change" \
  --reason manual-decision-required

pointbreak input-request list                 # defaults to open
pointbreak input-request list --status all
pointbreak input-request show <input-request-id> --include-body

pointbreak input-request respond <input-request-id> \
  --outcome approved \
  --reason "discussed in chat, ok to land"
```

`--reason` on the request is the classification axis (`manual-decision-required`,
`ambiguous-state`, `unsafe-action`, `insufficient-evidence`, etc.). `--outcome` on
the response is a separate axis (`approved`, `rejected`, `dismissed`, `superseded`,
`abandoned`).

Multiple different response events are preserved as append-only facts and
make the input request `ambiguous` rather than picking a timestamp winner.

### Assessments

An assessment is the current review call for a revision, a file, a range,
or a specific native observation/input request/assessment in the same
revision. CLI input and human-facing display use `accepted`,
`accepted-with-follow-up`, `needs-changes`, and `needs-clarification`; command
JSON output uses the matching `snake_case` values.

```bash
pointbreak assessment add \
  --track human:kevin \
  --assessment accepted \
  --summary "looks good, ship it"

# Assessment that replaces an older one
pointbreak assessment add \
  --track human:kevin \
  --assessment accepted-with-follow-up \
  --summary "supersedes earlier needs-changes after offline discussion" \
  --replaces <older-assessment-id>

pointbreak assessment show --format json-pretty
pointbreak assessment show --all --include-summary
```

`--replaces` is the only V1 relationship that removes an older assessment
from the current set.
`--related-observation` and `--related-input-request` record evidence links;
they do not mutate observations or close input requests (use
`pointbreak input-request respond` for the input-request lifecycle).

State-change outcomes such as deferred, split-out, overridden, and superseded
are recorded as observations tagged with `state-change:*`. Use
`pointbreak assessment` for review calls and `pointbreak observation add`
with a concrete tag such as `--tag state-change:deferred` for state-change
evidence.

### Attention

`pointbreak attention list` is the read that surfaces what still needs an actor's
judgment across the review record: open asks (including evidence requests),
ambiguous assessments, competing supersession heads, stale decisions on
superseded revisions, failed checks on current heads, and outstanding
follow-ups. It is a projection over the same durable facts the commands above
record — nothing new is written by reading it.

```bash
pointbreak attention list
pointbreak attention list --revision <revision-id>
```

Attention *guides, never gates* (ADR-0019): the list is derived attention
state, never a write precondition; a cooperative actor uses it to decide where
to look next, and Pointbreak never blocks a write on it. "Attention" is
promoted here from internal substrate vocabulary (ADR-0019's notification
seam) to a product-level term for this surface.

How items clear — every clearing action is a durable fact about the work,
never a fact about the queue (ADR-0019's judgment-subsumption amendment):

- `open_input_request` / `follow_up_outstanding` — respond with
  `pointbreak input-request respond` (the `dismissed` outcome closes a moot ask).
- `ambiguous_assessment` — record an assessment that `--replaces` the
  competing records; a superseded revision's ambiguity also resolves once
  every successor head has been re-judged.
- `stale_assessment` — replace the assessment, or re-judge every successor
  head (any assessment value counts as re-judged).
- `failed_validation` — record a strictly-later passing run of the same
  check (`skipped` never clears), supersede the revision, or record a later,
  unanimously accepting judgment on it: a reviewer who accepts a revision
  with the failure in evidence has rendered the judgment the item was
  waiting for.
- `competing_heads` — consolidate the fork with a capture that supersedes
  the competing heads.

## 5. Concepts you need to know

### Durable event facts vs. rebuildable projections

Pointbreak separates **authoritative facts** from **derived views**. The paths below are relative to
the resolved store directory — `<git-common-dir>/pointbreak` by default, or a worktree-local `.pointbreak/data/` when the
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

If `state.json` looks stale or inconsistent, Pointbreak rebuilds it from
the event log. Do not write to `state.json` yourself, and do not depend on
its internal shape.

### Command-output JSON is the integration surface

The stable surface for automation is **command-output JSON documents**:
`pointbreak.review-capture`, `pointbreak.review-history`, `pointbreak.review-revision`,
`pointbreak.review-observation-add` / `-list`,
`pointbreak.review-input-request-open` / `-list` / `-show` / `-respond`,
and `pointbreak.review-assessment-add` / `-show`.

These documents expose semantic IDs, content hashes, and freshness metadata.
Raw event files, event filenames, artifact paths, and `state.json` are
Pointbreak-owned storage details. They can change without a deprecation cycle.

### Tracks

Every observation, input request, and assessment belongs to a required
`--track`. Tracks are **review lanes**, such as `agent:codex` or
`human:kevin`. They are not actor identity. Writer provenance — who actually
ran the command — is recorded separately in the event envelope: the writer
`actorId` (from local Git config, or an explicit `actor:agent:<name>` set via
`POINTBREAK_ACTOR_ID`) and the `producer` that wrote the event. The human a resolved
agent acts on behalf of comes from the checked-in `.pointbreak/delegates.json` map at
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

Pointbreak exposes several kinds of IDs in its output: revision IDs, object
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

## 6. A small realistic walkthrough

The block below captures the typical sequence: confirm the change, capture
the revision, inspect it, record a couple of observations, open an
input request, respond to it, and land an assessment.

```bash
# 0. Confirm the worktree has the changes you want to review.
cd ~/src/myproject
git status

# 1. Capture a revision. This freezes the current diff as a snapshot.
#    `pointbreak capture` emits compact JSON only; pipe through jq if you
#    want to read it.
pointbreak capture | jq .

# 2. Read the captured revision (composite view, narrative + snapshot).
pointbreak revision show --format json-pretty | less

# 3. Record observations as you read the diff.
pointbreak observation add \
  --track agent:codex \
  --title "Check error handling near IO boundary" \
  --file src/io.rs --start-line 88 --end-line 104 \
  --body "The new branch swallows io::ErrorKind::Interrupted silently."

pointbreak observation add \
  --track human:kevin \
  --title "Unit test for the new retry path" \
  --file src/io.rs --start-line 120 --end-line 135

pointbreak observation list --format json-pretty

# 4. Open an input request when you need a decision from someone else.
pointbreak input-request open \
  --track human:kevin \
  --title "Approve schema migration before landing" \
  --reason manual-decision-required \
  --file db/migrations/0042_users.sql

# Someone reads the open queue and responds to it.
pointbreak input-request list --status open
pointbreak input-request respond <input-request-id> \
  --outcome approved \
  --reason "verified backfill plan with on-call DBA"

# 5. Record the final assessment for the revision.
pointbreak assessment add \
  --track human:kevin \
  --assessment accepted-with-follow-up \
  --summary "ship it; follow up on the retry-path unit test"

# 6. Verify the durable record.
pointbreak assessment show --format json-pretty
pointbreak history --format json-pretty | less
```

That is the full V1 workflow. Anything beyond it — notifications, daemons,
multi-writer coordination, automatic delivery — is intentionally out of
scope and will be addressed by future, separately-designed subsystems.
