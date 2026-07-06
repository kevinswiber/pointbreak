# Manual Testing Playbook

This is a maintainer-facing checklist for spot-checking Shoreline's current workflows by hand. It is
intentionally small: each section creates a disposable scratch repo, runs a few commands, and
describes what to look for. Use it after big changes to confirm the surfaces still behave the way
the docs claim.

It is not a substitute for `just test` — automated tests still own correctness. The point here is to
exercise the end-to-end ergonomics, the JSON contracts, and the storage layout the way a real
caller would see them.

## Conventions

- Use a release build for representative timings: `cargo build --release` and run
  `./target/release/shore`. A debug build works for behavior checks if you prefer.
- All commands below assume `shore` resolves to that binary. Set `SHORE=$(pwd)/target/release/shore`
  in your shell and substitute `"$SHORE"` if you do not want to install it on `PATH`.
- Use a fresh temp directory per test so storage state does not bleed across cases. `shore capture`
  shells out to `git diff … HEAD`, so the repo needs **at least one commit** before
  capture runs; otherwise the underlying git call fails with `fatal: bad revision 'HEAD'`. Include a
  baseline commit in the setup:

  ```bash
  TMP=$(mktemp -d)
  cd "$TMP"
  git init -q
  git config user.email "manual-test@example.com"
  git config user.name "Manual Test"
  git config commit.gpgsign false

  # Baseline commit — required so `shore capture` has a HEAD to diff against.
  echo "placeholder" > README
  git add README && git commit -q -m "baseline"
  ```

  Each section below then layers real changes on top of that baseline (modify tracked files, add
  new ones, stage them, leave them unstaged, etc.) so the captured diff is non-empty.

- `shore capture` and the write commands emit **compact JSON only**. Pipe through `jq` or
  `python3 -m json.tool` if you want to read them. Most read commands accept `--pretty`.
- `shore` writes durable state into `.shore/data/` inside the worktree. After a manual test, you can
  remove the temp directory; nothing escapes it.

## A. Basic capture of tracked changes

**Goal.** Confirm that `shore capture` records a `work_object_proposed` event, writes a
snapshot artifact, and rebuilds `.shore/data/state.json`.

```bash
# Add a tracked file on top of the baseline commit, then modify it so the
# working tree has a real diff against HEAD.
echo -e "alpha\nbeta\ngamma" > src.txt
git add src.txt && git commit -q -m "add src"
echo -e "alpha\nbeta-modified\ngamma\ndelta" > src.txt

shore capture | jq .
ls -la .shore/data/
ls .shore/data/events/ .shore/data/artifacts/objects/
```

**Expect.**

- One JSON document with `schema: "shore.review-capture"`, a `revision.id`, a `revisionId`, an
  `objectId`, and an `objectArtifactContentHash`.
- `.shore/data/events/` contains exactly one event file.
- `.shore/data/artifacts/objects/` contains exactly one snapshot artifact.
- `.shore/data/state.json` exists and reports `revisionCount: 1`.
- No `.git/info/exclude` entry is written; a worktree-local (ephemeral) store is covered by the
  generated, committed `.shore/.gitignore` instead, and the root `.gitignore` is unchanged
  (`git status --short` reports no `.gitignore` line).

## B. Capture with untracked files

**Goal.** Confirm that untracked files appear as `added` in the captured snapshot.

```bash
echo "fresh content" > new-file.txt
shore dump | jq '.stream.rows[] | select(.kind.file_header) | .kind.file_header'
```

**Expect.**

- One `file_header` row per modified, added, or deleted path.
- The untracked `new-file.txt` appears with `status: "added"`.
- No root `.gitignore` row appears: the shared store lives inside `.git/`, and an ephemeral store
  is covered by the generated `.shore/.gitignore` — Shoreline never edits the root `.gitignore`.
- No `.shore/data/` rows appear, because the generated ignore keeps Shoreline's own storage out of
  the captured snapshot.

If you want a fresh capture that *only* sees the untracked file, run this in a different temp repo
that has no other diff.

## C. Observations — add and list

**Goal.** Confirm observations attach to a revision, support review-wide and range targets, and
can be filtered by track or tag on read.

```bash
shore observation add \
  --track agent:codex \
  --title "Check epsilon handling" \
  --tag correctness

shore observation add \
  --track human:kevin \
  --title "Worth a unit test" \
  --file src.txt --start-line 4 --end-line 4 \
  --body "epsilon line was added in this revision"

shore observation list --pretty
shore observation list --pretty --track agent:codex
shore observation list --pretty --tag correctness
shore observation list --pretty --include-body
```

**Expect.**

- Each `add` returns `shore.review-observation-add` JSON with a new `observationId` and
  `eventId`, plus a `bodyContentHash` for the second observation only.
- `observation list` returns both observations under the same `revisionId`. The range-targeted
  observation has `target.kind: "range"` with `filePath`, `side`, `startLine`, `endLine`.
- The `--track agent:codex` filter returns only the first observation.
- The `--tag correctness` filter returns only observations carrying that exact tag.
- The default `observation list` omits body text; `--include-body` hydrates it.

## D. Input requests — open, list, fetch, respond

**Goal.** Confirm the durable pause/decision lifecycle.

```bash
REQUEST_OUT=$(shore input-request open \
  --track human:kevin \
  --title "Need approval before landing" \
  --reason manual-decision-required)
echo "$REQUEST_OUT" | jq .
INPUT_REQUEST_ID=$(echo "$REQUEST_OUT" | jq -r .inputRequestId)

shore input-request list --pretty
shore input-request list --pretty --status all
shore input-request show "$INPUT_REQUEST_ID" --pretty --include-body

shore input-request respond "$INPUT_REQUEST_ID" \
  --outcome approved \
  --reason "verified plan with on-call DBA"

shore input-request list --pretty --status all
```

**Expect.**

- `input-request open` returns an `inputRequestId` and `reasonCode: "manual_decision_required"`
  (snake_case in the output).
- `input-request list` defaults to status `open` and includes the new request.
- `input-request show` returns one input request plus an empty `responses` list before respond.
- `input-request respond` returns an `inputRequestResponseId` and `outcome: "approved"`.
- After respond, `input-request list --status all` shows the request with `status: "responded"`
  and one entry under `responses`. `input-request list` with the default `--status open` returns
  zero entries.

## E. Assessments — add and show

**Goal.** Confirm a review assessment lands, and that `--replaces` is the only thing that removes
an older assessment from the current set.

```bash
shore assessment add \
  --track human:kevin \
  --assessment accepted \
  --summary "looks good, ship it"

shore assessment show --pretty
shore assessment show --pretty --include-summary

# Replacing example
ASSESS_OLD=$(shore assessment show | jq -r '.current.assessmentId')
shore assessment add \
  --track human:kevin \
  --assessment accepted-with-follow-up \
  --summary "second pass; follow-up filed" \
  --replaces "$ASSESS_OLD"

shore assessment show --pretty
shore assessment show --pretty --all
```

**Expect.**

- After the first `add`, `assessment show` reports `current.status: "resolved"` and
  `current.assessment: "accepted"`.
- `--include-summary` adds the summary text inline; without it, only the `summaryContentHash`
  appears.
- After the second `add`, the original assessment is no longer in the current list. It still
  appears under `--all` with `status: "replaced"`.

## F. Review history with filters

**Goal.** Confirm `shore history` is chronological, preserves duplicate semantic events,
and applies filters without changing freshness metadata.

```bash
shore history --pretty | jq '.eventCount, .historyCount'
shore history --pretty --event-type review-observation-recorded \
  | jq '.eventCount, .historyCount'
shore history --pretty --track human:kevin \
  | jq '.eventCount, .historyCount'
shore history --pretty --include-body \
  | jq '.entries[] | select(.eventType=="review_observation_recorded") | .summary.body'
```

**Expect.**

- The two count fields differ when a filter applies: `eventCount` reflects the full validated
  scan; `historyCount` reflects the returned entries. The `eventSetHash` is identical across
  filtered and unfiltered runs of the same event set.
- `--include-body` hydrates observation bodies, input request bodies and response reasons, and
  assessment summaries inline. In a history entry, the event-specific fields (including any
  hydrated body) live under `.summary`, not at the entry root — for example, an observation body
  is `.summary.body`, an assessment summary is `.summary.summary`, and an input request response
  reason is on the responded entry's `.summary.reason`.

## G. Review revisions and show with and without `--include-body`

**Goal.** Confirm the discovery surface lists every captured revision, and the composite
revision view returns narrative facts before the snapshot remainder with body text omitted by
default.

### `shore revision list`

`shore revision list` projects `work_object_proposed` events into a flat directory of
revisions. Reach for it whenever `shore revision show` errors with
`multiple captured revisions; pass --revision`.

```bash
shore revision list --pretty | jq '{eventSetHash, revisionCount, ids: [.entries[].revisionId]}'
shore revision list --pretty | jq '.entries[] | {revisionId, capturedAt, objectArtifactContentHash}'
```

**Expect.**

- `revisionCount` matches the number of `work_object_proposed` events on disk; capturing a new
  revision increments it by one.
- Each entry includes `revisionId`, `capturedAt`, `objectId`, `source`, `base`,
  `target`, and `objectArtifactContentHash` and no event paths, artifact paths, or `statePath`.
- Entries are sorted by `capturedAt`, so the newest revision appears last.

### `shore revision show`

`shore revision show` puts each revision fact in two places:

- top-level `observations[]`, `inputRequests[]`, `assessments[]`, and `adapterNotes[]` carry the
  hydrated facts (including `body` / `summary` / `reason` when `--include-body` is passed).
- `rows[]` carries the projection rendering. Each row has `kind` as a **string**
  (`"observation"`, `"input_request"`, `"assessment"`, `"file_header"`, `"hunk_header"`,
  `"diff"`, `"metadata"`, `"adapter_note"`, etc.) and a `projectionPhase` of either `"narrative"`
  or `"snapshot_remainder"`. Body text is **not** carried on rows.

```bash
shore revision show --pretty | jq '.eventSetHash, .summary'
shore revision show --pretty | jq '[.rows[].kind] | unique'
shore revision show --pretty \
  | jq '[.rows[] | {kind, projectionPhase}] | group_by(.projectionPhase) | map({phase: .[0].projectionPhase, count: length})'

# Bodies are omitted by default and live on the top-level fact lists when hydrated.
shore revision show --pretty | jq '.observations[] | {title, body}'
shore revision show --pretty --include-body | jq '.observations[] | {title, body}'
shore revision show --pretty --include-body | jq '.assessments[] | {assessment, summary}'

# Track filter narrows narrative material but leaves the snapshot remainder intact.
shore revision show --pretty --track agent:codex \
  | jq '{
      observations: [.observations[].trackId] | unique,
      input_requests_count: (.inputRequests | length),
      assessments_count: (.assessments | length),
      narrative_rows: [.rows[] | select(.projectionPhase=="narrative") | .kind],
      snapshot_remainder_count: [.rows[] | select(.projectionPhase=="snapshot_remainder")] | length
    }'
```

**Expect.**

- `[.rows[].kind] | unique` returns a flat list of row-kind strings; the narrative-phase rows
  appear before the snapshot-remainder rows in `rows[]` order.
- Default output has every observation/input-request/assessment object present in the top-level
  lists but with no `body` / `summary` / `reason` field. `--include-body` adds those fields
  inline.
- The `--track agent:codex` filter keeps only `agent:codex` facts in the top-level lists and
  narrows the narrative rows to the matching track (non-`agent:codex` narrative rows are dropped;
  the rows for the kept facts remain). `snapshot_remainder_count` is the same as without the
  filter, and the snapshot remainder still includes every captured file.

## H. Notes apply + dump/show compatibility path

**Goal.** Confirm sidecar import lands a durable `review_note_imported` event, and that
`shore dump` / `shore show` render imported notes alongside the working-tree diff.

```bash
cat > review-notes.json <<'EOF'
{
  "schema": "shore.review-notes",
  "version": 1,
  "summary": "Manual test sidecar",
  "files": [
    {
      "path": "src.txt",
      "notes": [
        { "id": "note:manual",
          "title": "Imported sidecar note",
          "body": "Anchored to line 2 of src.txt.",
          "target": { "side": "new", "startLine": 2, "endLine": 2 }
        }
      ]
    }
  ]
}
EOF

shore notes apply --review-notes review-notes.json | jq .
shore dump --pretty | jq '.stream.rows[] | select(.kind.note) | .kind.note'
shore history --pretty --event-type review-note-imported \
  | jq '.entries | length'
```

**Expect.**

- `notes apply` returns `noteCount: 1`, `notesCreated: 1`, `notesExisting: 0`, and a `statePath`.
- `shore dump` includes one `note` row attached to the right `target_row_id`.
- A second run with the same sidecar increments `notesExisting` and leaves `notesCreated` at 0:
  imported-note events are idempotent on their semantic ID.
- `shore history --event-type review-note-imported` returns one entry.
- `shore revision show --pretty | jq '.adapterNotes'` returns the imported note as one entry in
  the revision's adapter-notes list.

To exercise the TUI by eye, run `shore show` and confirm `j`/`k`, `[`/`]`, and `{`/`}` work, and
that `q` exits cleanly.

## I. Stale and orphan note reload

**Goal.** Confirm that when the working tree drifts away from an imported note's anchor,
`shore dump` surfaces it as a `stale_note` row with a `reload_diagnostics` entry, without losing
durable state.

```bash
# Start from step H (one imported note anchored at src.txt:2).
echo "just one line" > src.txt           # makes line 2 absent

shore dump --pretty | jq '.stream.rows[] | select(.kind.stale_note) | .kind.stale_note'
shore dump --pretty | jq '.reload_diagnostics // {}'
```

**Expect.**

- One `stale_note` row with `resolution_status: "stale"`, plus the original `target_path` and
  `target_line_range`.
- `reload_diagnostics.entries[]` contains a `note_stale` entry naming the note.
- `.shore/data/events/` is unchanged: durable facts are not rewritten by reload.

For the orphan case, do the same in a repo where the sidecar references a path that does not
appear in the captured snapshot at all (for example, a file the working tree has never seen). The
review stream should emit a synthetic `<orphaned notes>` file header followed by one `stale_note`
row per orphan note. The synthetic header is omitted when there are no orphans.

## J. Storage soundness — events, artifacts, and projection rebuildability

**Goal.** Confirm that `.shore/data/events/` and `.shore/data/artifacts/` together are the authoritative
durable store, and that `.shore/data/state.json` is a pure projection that can be deleted and
regenerated.

The authority split (see `docs/storage-model.md`):

- `.shore/data/events/` — append-only immutable per-fact events.
- `.shore/data/artifacts/` — immutable support records that events bind to: captured revision
  snapshots (`artifacts/objects/`), and content-addressed bodies for large observation,
  input request, and assessment payloads (`artifacts/notes/`). `revision show` reads the
  snapshot artifact for the selected revision; the event log alone cannot reconstruct snapshot
  rows or large note bodies.
- `.shore/data/state.json` — rebuildable projection summary. Reads do not depend on its existence;
  writes regenerate it.

```bash
ls .shore/data/events/
ls .shore/data/artifacts/objects/
ls .shore/data/artifacts/notes/        # only populated for large-body events

# Read commands work without state.json
HASH_BEFORE=$(jq -r .eventSetHash .shore/data/state.json)
rm .shore/data/state.json
shore history --pretty | jq -r .eventSetHash    # same hash
shore revision show --pretty >/dev/null
test -f .shore/data/state.json && echo "rebuilt" || echo "still missing (expected for reads)"

# A write command rebuilds the projection
shore observation add --track agent:codex --title "trigger rebuild" >/dev/null
jq '.eventCount, .eventSetHash' .shore/data/state.json
```

**Expect.**

- `shore history` and `shore revision show` both succeed without `state.json` present.
  Their `eventSetHash` matches the value that was in the deleted projection.
- After the next write command, `.shore/data/state.json` exists again and reports a higher
  `eventCount` and a new `eventSetHash`.
- Event files in `.shore/data/events/` are never moved, renamed, or removed during any of this. You can
  list them before and after and confirm the set only grows.

If you want to confirm idempotency directly, re-run the same `observation add` with
`--idempotency-key <same-key>`: the response should show `eventsCreated: 0`, `eventsExisting: 1`,
and the same `observationId` and `eventId` as the first call.

## K. Things to glance at after big changes

When refactoring storage, projections, or CLI surfaces, also look at:

- **JSON document schemas**: every command's top-level `schema` and `version` should still match the
  README's "Current CLI" section.
- **Event file count**: each `add`/`request`/`resolve`/`apply` call should create exactly one new
  event file unless it is a same-key idempotent retry.
- **Artifact dedup**: writing two observations with the same **large** body string should yield
  one file in `.shore/data/artifacts/notes/` (content-addressed) and two events that both reference it
  by content hash. Bodies under roughly 4 KiB stay inline in the event payload and do not produce
  an artifact at all, so use a body well over that threshold to exercise this path —
  `python3 -c "print('x'*5000)" > big-body.txt` and pass `--body-file big-body.txt` to two
  separate `observation add` calls.
- **Exit codes**: piping `shore dump`, `shore revision show`, or `shore history` through
  `jq -e 'has("schema")'` should always exit 0 for successful runs.
- **Tracing**: passing `--log info --log-file /tmp/shore.log` to any command should write to that
  file and not corrupt the JSON on stdout. `shore show` requires `--log-file` when tracing is
  enabled.

## What this playbook does not cover

- Performance benchmarking or stress tests.
- The TUI `shore show` interaction beyond a quick keybinding smoke test.
- Multi-writer coordination — V1 is intentionally single-writer per `.shore/data/`.
- Daemon, notification, or delivery-queue behavior — none of those exist in V1.

If a workflow you exercise during real review reveals a gap that is not covered here, add a short
section above following the same pattern: goal, commands, expected output.
