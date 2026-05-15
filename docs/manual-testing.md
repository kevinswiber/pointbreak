# Manual Testing Playbook

This is a maintainer-facing checklist for spot-checking Shore's current workflows by hand. It is
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
- Use a fresh temp directory per test so storage state does not bleed across cases:

  ```bash
  TMP=$(mktemp -d)
  cd "$TMP"
  git init -q
  git config user.email "manual-test@example.com"
  git config user.name "Manual Test"
  git config commit.gpgsign false
  ```

- `shore review capture` and the write commands emit **compact JSON only**. Pipe through `jq` or
  `python3 -m json.tool` if you want to read them. Most read commands accept `--pretty`.
- `shore` writes durable state into `.shore/` inside the worktree. After a manual test, you can
  remove the temp directory; nothing escapes it.

## A. Basic capture of tracked changes

**Goal.** Confirm that `shore review capture` records a `review_unit_captured` event, writes a
snapshot artifact, and rebuilds `.shore/state.json`.

```bash
echo -e "alpha\nbeta\ngamma" > src.txt
git add src.txt && git commit -q -m "initial"
echo -e "alpha\nbeta-modified\ngamma\ndelta" > src.txt

shore review capture | jq .
ls -la .shore/
ls .shore/events/ .shore/artifacts/snapshots/
```

**Expect.**

- One JSON document with `schema: "shore.review-capture"`, a `reviewUnit.id`, a `revisionId`, a
  `snapshotId`, and a `snapshotArtifactContentHash`.
- `.shore/events/` contains exactly one event file.
- `.shore/artifacts/snapshots/` contains exactly one snapshot artifact.
- `.shore/state.json` exists and reports `reviewUnitCount: 1`.
- `.gitignore` has been updated to include `.shore/`.

## B. Capture with untracked files

**Goal.** Confirm that untracked files appear as `added` in the captured snapshot.

```bash
echo "fresh content" > new-file.txt
shore dump | jq '.stream.rows[] | select(.kind.file_header) | .kind.file_header'
```

**Expect.**

- One `file_header` row per modified, added, or deleted path.
- The untracked `new-file.txt` appears with `status: "added"`.
- The auto-added `.gitignore` (from step A) also appears with `status: "added"` because it is part
  of the working tree relative to `HEAD`.

If you want a fresh capture that *only* sees the untracked file, run this in a different temp repo
that has no other diff.

## C. Observations — add and list

**Goal.** Confirm observations attach to a ReviewUnit, support review-wide and range targets, and
can be filtered by track on read.

```bash
shore review observation add \
  --track agent:codex \
  --title "Check epsilon handling"

shore review observation add \
  --track human:kevin \
  --title "Worth a unit test" \
  --file src.txt --start-line 4 --end-line 4 \
  --body "epsilon line was added in this revision"

shore review observation list --pretty
shore review observation list --pretty --track agent:codex
shore review observation list --pretty --include-body
```

**Expect.**

- Each `add` returns `shore.review-observation-add` JSON with a new `observationId` and
  `eventId`, plus a `bodyContentHash` for the second observation only.
- `observation list` returns both observations under the same `reviewUnitId`. The range-targeted
  observation has `target.kind: "range"` with `filePath`, `side`, `startLine`, `endLine`.
- The `--track agent:codex` filter returns only the first observation.
- The default `observation list` omits body text; `--include-body` hydrates it.

## D. Interventions — request, list, fetch, resolve

**Goal.** Confirm the durable pause/decision lifecycle.

```bash
INT_OUT=$(shore review intervention request \
  --track human:kevin \
  --title "Need approval before landing" \
  --reason manual-decision-required)
echo "$INT_OUT" | jq .
INT_ID=$(echo "$INT_OUT" | jq -r .interventionId)

shore review intervention list --pretty
shore review intervention list --pretty --status all
shore review intervention fetch "$INT_ID" --pretty --include-body

shore review intervention resolve "$INT_ID" \
  --outcome approved \
  --reason "verified plan with on-call DBA"

shore review intervention list --pretty --status all
```

**Expect.**

- `intervention request` returns an `interventionId` and `reasonCode: "manual_decision_required"`
  (snake_case in the output).
- `intervention list` defaults to status `open` and includes the new request.
- `intervention fetch` returns one intervention plus an empty `resolutions` list before resolve.
- `intervention resolve` returns an `interventionResolutionId` and `outcome: "approved"`.
- After resolve, `intervention list --status all` shows the intervention with `status: "resolved"`
  and one entry under `resolutions`. `intervention list` with the default `--status open` returns
  zero entries.

## E. Dispositions — add and show

**Goal.** Confirm a final disposition lands, and that `--replaces` is the only thing that removes
an older disposition from the current set.

```bash
shore review disposition add \
  --track human:kevin \
  --disposition accepted \
  --summary "looks good, ship it"

shore review disposition show --pretty
shore review disposition show --pretty --include-summary

# Overriding/replacing example
DISP_OLD=$(shore review disposition show | jq -r '.current.dispositionId')
shore review disposition add \
  --track human:kevin \
  --disposition overridden \
  --summary "second pass; deferring" \
  --overrides-disposition "$DISP_OLD" \
  --replaces "$DISP_OLD"

shore review disposition show --pretty
shore review disposition show --pretty --all
```

**Expect.**

- After the first `add`, `disposition show` reports `current.status: "resolved"` and
  `current.disposition: "accepted"`.
- `--include-summary` adds the summary text inline; without it, only the `summaryContentHash`
  appears.
- After the second `add`, the original disposition is no longer in the current list. It still
  appears under `--all` with `status: "replaced"`.

## F. Review history with filters

**Goal.** Confirm `shore review history` is chronological, preserves duplicate semantic events,
and applies filters without changing freshness metadata.

```bash
shore review history --pretty | jq '.eventCount, .historyCount'
shore review history --pretty --event-type review-observation-recorded \
  | jq '.eventCount, .historyCount'
shore review history --pretty --track human:kevin \
  | jq '.eventCount, .historyCount'
shore review history --pretty --include-body \
  | jq '.entries[] | select(.eventType=="review_observation_recorded") | .body'
```

**Expect.**

- The two count fields differ when a filter applies: `eventCount` reflects the full validated
  scan; `historyCount` reflects the returned entries. The `eventSetHash` is identical across
  filtered and unfiltered runs of the same event set.
- `--include-body` hydrates observation bodies, intervention bodies and resolution reasons, and
  disposition summaries inline.

## G. Review unit show with and without `--include-body`

**Goal.** Confirm the composite ReviewUnit view returns narrative facts before the snapshot
remainder, and that body text is omitted by default.

```bash
shore review unit show --pretty | jq '.eventSetHash, .summary // {}'
shore review unit show --pretty | jq '.rows[].kind | keys'
shore review unit show --pretty --include-body \
  | jq '.rows[] | select(.kind.observation) | .kind.observation.body // "(no body)"'
shore review unit show --pretty --track agent:codex \
  | jq '.summary // {}, [.rows[] | select(.kind.observation) | .kind.observation.trackId] | unique'
```

**Expect.**

- `rows[]` begins with narrative material (observations, interventions, dispositions, imported
  notes) and continues with the snapshot remainder (file headers, hunk headers, metadata rows,
  diff rows).
- Default output has no body strings on observation/intervention/disposition rows. With
  `--include-body`, the body text appears inline.
- The `--track` filter narrows narrative rows only; the snapshot remainder still includes every
  captured file.

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
shore review history --pretty --event-type review-note-imported \
  | jq '.entries | length'
```

**Expect.**

- `notes apply` returns `noteCount: 1`, `notesCreated: 1`, `notesExisting: 0`, and a `statePath`.
- `shore dump` includes one `note` row attached to the right `target_row_id`.
- A second run with the same sidecar increments `notesExisting` and leaves `notesCreated` at 0:
  imported-note events are idempotent on their semantic ID.
- `shore review history --event-type review-note-imported` returns one entry.
- `shore review unit show --pretty | jq '.adapterNotes'` returns the imported note as one entry in
  the ReviewUnit's adapter-notes list.

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
- `.shore/events/` is unchanged: durable facts are not rewritten by reload.

For the orphan case, do the same in a repo where the sidecar references a path that does not
appear in the captured snapshot at all (for example, a file the working tree has never seen). The
review stream should emit a synthetic `<orphaned notes>` file header followed by one `stale_note`
row per orphan note. The synthetic header is omitted when there are no orphans.

## J. Storage soundness — events, artifacts, and projection rebuildability

**Goal.** Confirm `.shore/state.json` is a pure projection and that the event log alone is
sufficient authority.

```bash
ls .shore/events/
ls .shore/artifacts/snapshots/
ls .shore/artifacts/notes/        # only populated for large-body events

# Read commands work without state.json
HASH_BEFORE=$(jq -r .eventSetHash .shore/state.json)
rm .shore/state.json
shore review history --pretty | jq -r .eventSetHash    # same hash
shore review unit show --pretty >/dev/null
test -f .shore/state.json && echo "rebuilt" || echo "still missing (expected for reads)"

# A write command rebuilds the projection
shore review observation add --track agent:codex --title "trigger rebuild" >/dev/null
jq '.eventCount, .eventSetHash' .shore/state.json
```

**Expect.**

- `shore review history` and `shore review unit show` both succeed without `state.json` present.
  Their `eventSetHash` matches the value that was in the deleted projection.
- After the next write command, `.shore/state.json` exists again and reports a higher
  `eventCount` and a new `eventSetHash`.
- Event files in `.shore/events/` are never moved, renamed, or removed during any of this. You can
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
- **Artifact dedup**: writing two observations with the same body string should yield one file in
  `.shore/artifacts/notes/` (content-addressed), but two events that both reference it.
- **Exit codes**: piping `shore dump`, `shore review unit show`, or `shore review history` through
  `jq -e 'has("schema")'` should always exit 0 for successful runs.
- **Tracing**: passing `--log info --log-file /tmp/shore.log` to any command should write to that
  file and not corrupt the JSON on stdout. `shore show` requires `--log-file` when tracing is
  enabled.

## What this playbook does not cover

- Performance benchmarking or stress tests.
- The TUI `shore show` interaction beyond a quick keybinding smoke test.
- Multi-writer coordination — V1 is intentionally single-writer per `.shore/`.
- Daemon, notification, or delivery-queue behavior — none of those exist in V1.

If a workflow you exercise during real review reveals a gap that is not covered here, add a short
section above following the same pattern: goal, commands, expected output.
