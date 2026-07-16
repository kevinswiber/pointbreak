# Manual Testing Playbook

This is a maintainer-facing checklist for spot-checking Pointbreak's current workflows by hand. It is
intentionally small: each section creates a disposable scratch repo, runs a few commands, and
describes what to look for. Use it after big changes to confirm the surfaces still behave the way
the docs claim.

It is not a substitute for `just test` — automated tests still own correctness. The point here is to
exercise the end-to-end ergonomics, the JSON contracts, and the storage layout the way a real
caller would see them.

## Conventions

- Use a release build for representative timings: `cargo build --release` and run
  `./target/release/pointbreak`. A debug build works for behavior checks if you prefer.
- All commands below assume `pointbreak` resolves to that binary. Set
  `POINTBREAK=$(pwd)/target/release/pointbreak` in your shell and substitute `"$POINTBREAK"` if you
  do not want to install it on `PATH`.
- Use a fresh temp directory per test so storage state does not bleed across cases. Default
  `pointbreak capture` captures `HEAD` to the working tree when `HEAD` exists, and Git's empty tree to
  the working tree in a repository with no commits. It excludes untracked files unless
  `--include-untracked` is passed. §B uses `pointbreak capture --root` to review a committed first commit
  against Git's empty tree. A capture with zero changed files fails unless `--allow-empty` is passed.
  For ordinary default worktree capture, include a baseline commit in the setup:

  ```bash
  TMP=$(mktemp -d)
  cd "$TMP"
  git init -q
  git config user.email "manual-test@example.com"
  git config user.name "Manual Test"
  git config commit.gpgsign false

  # Baseline commit — required so default `pointbreak capture` has a HEAD to diff against.
  echo "placeholder" > README
  git add README && git commit -q -m "baseline"
  ```

  Each section below then layers real changes on top of that baseline (modify tracked files, add
  new ones, stage them, leave them unstaged, etc.) so the captured diff is non-empty.

- `pointbreak capture` and the write commands emit **compact JSON only**. Pipe through `jq` or
  `python3 -m json.tool` if you want to read them. Most read commands accept `--format json-pretty`.
- `pointbreak` resolves one of three durable stores, and every command in this playbook reads and writes
  whichever one is resolved:
  - **Default — the clone's common-dir store at `<git-common-dir>/pointbreak/`.** Automatic, no setup, shared by the
    main worktree and every linked worktree of the clone. It lives entirely inside `.git/`, so it
    never appears in `git status` and never adds rows to a captured snapshot, and no `.pointbreak/`
    directory is created. The walkthroughs below use this default unless they say otherwise.
  - **Ephemeral opt-in — a discardable worktree-local store at `.pointbreak/data/`.** Enabled per
    worktree with `pointbreak store mode ephemeral`; Pointbreak also writes a `.pointbreak/store.json` marker
    and a generated `.pointbreak/.gitignore` (ignoring `data/` and `*.local.json`). Remove the worktree
    and the review facts vanish with it. §I opts into this mode to poke at the store's files
    directly.
  - **Family opt-in — a machine-wide store at `<pointbreak-home>/stores/<slug>/`.** Enabled per physical
    clone with `pointbreak store link <slug>` so review facts survive removing any one clone and are
    shared across a repository family, offline. The "Family store" walkthrough exercises the loop.

  See [storage-model.md](./storage-model.md#shared-common-dir-store-selection) for the default and
  ephemeral tiers and [the family-store tier](./storage-model.md#user-level-family-store-tier) for
  depth. After a manual test you can remove the temp directory (and, for the family walkthrough, its
  throwaway `POINTBREAK_HOME`); nothing escapes them.
- **How to run these.** Sections A and D–H share **one** repo: §A does the single `pointbreak capture`
  that the later sections annotate, so keep working in the same temp repo through §H. Bare review
  commands need exactly one captured revision — a second capture triggers
  `multiple captured revisions; pass --revision` — so §B (root capture), §C (untracked files),
  §I (storage soundness), and the family-store walkthrough each start from their **own** fresh temp
  repo and say so.

## A. Basic capture of tracked changes

**Goal.** Confirm that `pointbreak capture` records a `work_object_proposed` event (plus the
`revision_ref_associated` event that binds the revision ref), writes a snapshot artifact, and
rebuilds `<git-common-dir>/pointbreak/state.json`.

```bash
# Add a tracked file on top of the baseline commit, then modify it so the
# working tree has a real diff against HEAD.
echo -e "alpha\nbeta\ngamma" > src.txt
git add src.txt && git commit -q -m "add src"
echo -e "alpha\nbeta-modified\ngamma\ndelta" > src.txt

pointbreak capture | jq .
STORE=$(pointbreak store paths --format json | jq -r .commonStore)
ls -la "$STORE/"
ls "$STORE/events/" "$STORE/artifacts/objects/"
```

**Expect.**

- One JSON document with `schema: "pointbreak.review-capture"`; under `revision` it carries `id`,
  `revisionId`, `objectId`, and `objectArtifactContentHash`. It also reports `eventsCreated: 2` and
  `eventsCreatedByType: { "work_object_proposed": 1, "revision_ref_associated": 1 }`.
- `<git-common-dir>/pointbreak/events/` contains exactly two event files — one `work_object_proposed` and one
  `revision_ref_associated`.
- `<git-common-dir>/pointbreak/artifacts/objects/` contains exactly one snapshot artifact.
- `<git-common-dir>/pointbreak/state.json` exists and reports `revisionCount: 1` with `eventCount: 2`.
- Nothing lands in the working tree: the default store is inside `.git/`, so no `.pointbreak/` directory
  is created, the root `.gitignore` is untouched, and `git status --short` shows only your own
  change (` M src.txt`). (An ephemeral-mode worktree instead materializes `.pointbreak/data/` guarded by
  a generated `.pointbreak/.gitignore`; see §I.)

## B. Root capture of a one-commit repository

**Goal.** Confirm `pointbreak capture --root` records the first commit as files added from Git's empty
tree, without needing an orphan-branch workaround.

Run §B in its **own** fresh temp repo:

```bash
TMP=$(mktemp -d)
cd "$TMP"
git init -q
git config user.email "manual-test@example.com"
git config user.name "Manual Test"
git config commit.gpgsign false

mkdir -p src
echo "hello root" > src/first.txt
git add src/first.txt && git commit -q -m "initial"

pointbreak capture --root \
  | jq '{schema, base: .revision.base.kind, target: .revision.target.kind, diffstat}'
pointbreak revision show --format json-pretty | jq '[.rows[] | select(.kind == "file_header") | .filePath]'
```

**Expect.**

- The capture JSON has `schema: "pointbreak.review-capture"`, `base: "git_tree"`,
  `target: "git_commit"`, and `diffstat.addedFiles: 1`.
- The shown revision has one file header for `src/first.txt`, captured as an added file.
- `pointbreak capture --root --target <rev>` captures an explicit commit the same way, and
  `pointbreak capture --root --path src` scopes the root capture through Git pathspecs.

## C. Capture with untracked files

**Goal.** Confirm that untracked files are excluded by default and appear as `added` only with
`--include-untracked`.

Run §C in its **own** fresh temp repo (re-run the setup baseline) so its capture is the only
revision and the later sections' single-revision commands are unaffected:

```bash
# Fresh temp repo with only the baseline commit (see setup), then add one untracked file:
echo "fresh content" > new-file.txt
pointbreak capture 2>&1 || true
pointbreak capture --include-untracked | jq .diffstat
pointbreak revision show --format json-pretty | jq '[.rows[] | select(.kind == "file_header") | .filePath]'
```

**Expect.**

- The first command fails with `capture produced no changed files`, suggests `--include-untracked`,
  and mentions `--allow-empty`; no empty revision is written.
- The `--include-untracked` capture reports `fileCount: 1`, `addedFiles: 1` (the untracked
  `new-file.txt`), and zero modified, deleted, or renamed files.
- One `file_header` row, for `new-file.txt`, after the `--include-untracked` capture — the untracked
  file is captured as `added`.
- Nothing Pointbreak-owned appears in the snapshot or in `git status`: the default store lives inside
  `.git/`, so there is no `.pointbreak/` directory and no store rows in the captured diff, and Pointbreak
  never edits the root `.gitignore` (`git status --short` shows only `?? new-file.txt`).

## D. Observations — add and list

**Goal.** Confirm observations attach to a revision, support review-wide and range targets, and
can be filtered by track or tag on read.

```bash
pointbreak observation add \
  --track agent:codex \
  --title "Check epsilon handling" \
  --tag correctness

pointbreak observation add \
  --track human:kevin \
  --title "Worth a unit test" \
  --file src.txt --start-line 4 --end-line 4 \
  --body "epsilon line was added in this revision"

pointbreak observation list --format json-pretty
pointbreak observation list --format json-pretty --track agent:codex
pointbreak observation list --format json-pretty --tag correctness
pointbreak observation list --format json-pretty --include-body
```

**Expect.**

- Each `add` returns `pointbreak.review-observation-add` JSON with a new `observationId` and
  `eventId`, plus a `bodyContentHash` for the second observation only.
- `observation list` returns both observations under the same `revisionId`. The range-targeted
  observation has `target.kind: "range"` with `filePath`, `side`, `startLine`, `endLine`.
- The `--track agent:codex` filter returns only the first observation.
- The `--tag correctness` filter returns only observations carrying that exact tag.
- The default `observation list` omits body text; `--include-body` hydrates it.

## E. Input requests — open, list, fetch, respond

**Goal.** Confirm the durable pause/decision lifecycle.

```bash
REQUEST_OUT=$(pointbreak input-request open \
  --track human:kevin \
  --title "Need approval before landing" \
  --reason manual-decision-required)
echo "$REQUEST_OUT" | jq .
INPUT_REQUEST_ID=$(echo "$REQUEST_OUT" | jq -r .inputRequestId)

pointbreak input-request list --format json-pretty
pointbreak input-request list --format json-pretty --status all
pointbreak input-request show "$INPUT_REQUEST_ID" --format json-pretty --include-body

pointbreak input-request respond "$INPUT_REQUEST_ID" \
  --outcome approved \
  --reason "verified plan with on-call DBA"

pointbreak input-request list --format json-pretty --status all
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

## F. Assessments — add and show

**Goal.** Confirm a review assessment lands, and that `--replaces` is the only thing that removes
an older assessment from the current set.

```bash
pointbreak assessment add \
  --track human:kevin \
  --assessment accepted \
  --summary "looks good, ship it"

pointbreak assessment show --format json-pretty
pointbreak assessment show --format json-pretty --include-summary

# Replacing example
ASSESS_OLD=$(pointbreak assessment show | jq -r '.current.assessmentId')
pointbreak assessment add \
  --track human:kevin \
  --assessment accepted-with-follow-up \
  --summary "second pass; follow-up filed" \
  --replaces "$ASSESS_OLD"

pointbreak assessment show --format json-pretty
pointbreak assessment show --format json-pretty --all
```

**Expect.**

- After the first `add`, `assessment show` reports `current.status: "resolved"` and
  `current.assessment: "accepted"`.
- `--include-summary` adds the summary text inline; without it, only the `summaryContentHash`
  appears.
- After the second `add`, the original assessment is no longer in the current list. It still
  appears under `--all` with `status: "replaced"`.

## G. Review history with filters

**Goal.** Confirm `pointbreak history` is chronological, preserves duplicate semantic events,
and applies filters without changing freshness metadata.

```bash
pointbreak history --format json-pretty | jq '.eventCount, .historyCount'
pointbreak history --format json-pretty --event-type review-observation-recorded \
  | jq '.eventCount, .historyCount'
pointbreak history --format json-pretty --track human:kevin \
  | jq '.eventCount, .historyCount'
pointbreak history --format json-pretty --include-body \
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

## H. Review revisions and show with and without `--include-body`

**Goal.** Confirm the discovery surface lists every captured revision, and the composite
revision view returns narrative facts before the snapshot remainder with body text omitted by
default.

### `pointbreak revision list`

`pointbreak revision list` projects `work_object_proposed` events into a flat directory of
revisions. Reach for it whenever `pointbreak revision show` errors with
`multiple captured revisions; pass --revision`.

```bash
pointbreak revision list --format json-pretty | jq '{eventSetHash, revisionCount, ids: [.entries[].revisionId]}'
pointbreak revision list --format json-pretty | jq '.entries[] | {revisionId, capturedAt, objectArtifactContentHash}'
```

**Expect.**

- `revisionCount` matches the number of `work_object_proposed` events on disk; capturing a new
  revision increments it by one.
- Each entry includes `revisionId`, `capturedAt`, `objectId`, `source`, `base`,
  `target`, and `objectArtifactContentHash` and no event paths, artifact paths, or `statePath`.
- Entries are sorted by `capturedAt`, so the newest revision appears last.

### `pointbreak revision show`

`pointbreak revision show` puts each revision fact in two places:

- top-level `observations[]`, `inputRequests[]`, and `assessments[]` carry the
  hydrated facts (including `body` / `summary` / `reason` when `--include-body` is passed).
- `rows[]` carries the projection rendering. Each row has `kind` as a **string**
  (`"observation"`, `"input_request"`, `"assessment"`, `"file_header"`, `"hunk_header"`,
  `"diff"`, `"metadata"`, etc.) and a `projectionPhase` of either `"narrative"`
  or `"snapshot_remainder"`. Body text is **not** carried on rows.

```bash
pointbreak revision show --format json-pretty | jq '.eventSetHash, .summary'
pointbreak revision show --format json-pretty | jq '[.rows[].kind] | unique'
pointbreak revision show --format json-pretty \
  | jq '[.rows[] | {kind, projectionPhase}] | group_by(.projectionPhase) | map({phase: .[0].projectionPhase, count: length})'

# Bodies are omitted by default and live on the top-level fact lists when hydrated.
pointbreak revision show --format json-pretty | jq '.observations[] | {title, body}'
pointbreak revision show --format json-pretty --include-body | jq '.observations[] | {title, body}'
pointbreak revision show --format json-pretty --include-body | jq '.assessments[] | {assessment, summary}'

# Track filter narrows narrative material but leaves the snapshot remainder intact.
pointbreak revision show --format json-pretty --track agent:codex \
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

## I. Storage soundness — events, artifacts, and projection rebuildability

**Goal.** Confirm that `.pointbreak/data/events/` and `.pointbreak/data/artifacts/` together are the authoritative
durable store, and that `.pointbreak/data/state.json` is a pure projection that can be deleted and
regenerated.

This section runs in its **own** fresh temp repo switched to **ephemeral** mode, so the store lands
at a visible, worktree-local `.pointbreak/data/` you can list and delete directly. (The default store
holds the same layout inside `<git-common-dir>/pointbreak/`; ephemeral just surfaces it in the working tree.)

```bash
# Fresh temp repo with the baseline commit (see setup). Add a tracked file, then modify it so the
# working tree has a real diff, and opt into ephemeral BEFORE capturing:
echo -e "alpha\nbeta\ngamma" > src.txt
git add src.txt && git commit -q -m "add src"
echo -e "alpha\nbeta-modified\ngamma\ndelta" > src.txt

pointbreak store mode ephemeral                                # store now resolves to .pointbreak/data/
pointbreak capture >/dev/null
pointbreak observation add --track agent:codex --title "seed one" >/dev/null
pointbreak observation add --track human:kevin --title "seed two" >/dev/null
```

The authority split (see [storage-model.md](./storage-model.md#shared-common-dir-store-selection),
shown here with the ephemeral `.pointbreak/data/` paths):

- `.pointbreak/data/events/` — append-only immutable per-fact events.
- `.pointbreak/data/artifacts/` — immutable support records that events bind to: captured revision
  snapshots (`artifacts/objects/`), and content-addressed bodies for large observation,
  input request, and assessment payloads (`artifacts/notes/`). `revision show` reads the
  snapshot artifact for the selected revision; the event log alone cannot reconstruct snapshot
  rows or large note bodies.
- `.pointbreak/data/state.json` — rebuildable projection summary. Reads do not depend on its existence;
  writes regenerate it.

```bash
ls .pointbreak/data/events/
ls .pointbreak/data/artifacts/objects/
ls .pointbreak/data/artifacts/notes/        # only populated for large-body events

# Read commands work without state.json
HASH_BEFORE=$(jq -r .eventSetHash .pointbreak/data/state.json)
rm .pointbreak/data/state.json
pointbreak history --format json-pretty | jq -r .eventSetHash    # same hash
pointbreak revision show --format json-pretty >/dev/null
test -f .pointbreak/data/state.json && echo "rebuilt" || echo "still missing (expected for reads)"

# A write command rebuilds the projection
pointbreak observation add --track agent:codex --title "trigger rebuild" >/dev/null
jq '.eventCount, .eventSetHash' .pointbreak/data/state.json
```

**Expect.**

- `pointbreak history` and `pointbreak revision show` both succeed without `state.json` present.
  Their `eventSetHash` matches the value that was in the deleted projection.
- After the next write command, `.pointbreak/data/state.json` exists again and reports a higher
  `eventCount` and a new `eventSetHash`.
- Event files in `.pointbreak/data/events/` are never moved, renamed, or removed during any of this. You can
  list them before and after and confirm the set only grows.

If you want to confirm idempotency directly, re-run the same `observation add` with
`--idempotency-key <same-key>`: the response should show `eventsCreated: 0`, `eventsExisting: 1`,
and the same `observationId` and `eventId` as the first call.

## Family store — link, capture, status, unlink

**Goal.** Confirm the opt-in user-level family store: `pointbreak store link` promotes a clone to a
machine-wide store at `<pointbreak-home>/stores/<slug>/`, `pointbreak store status` reports the family
placement, captures write there while linked, and `pointbreak store unlink` detaches without moving data.

Run this in its **own** fresh temp repo, and point `POINTBREAK_HOME` at a throwaway directory so the
family store never touches your real `~/.pointbreak`:

```bash
# Fresh temp repo with the baseline commit (see setup). Set a throwaway family-store home first:
export POINTBREAK_HOME="$(mktemp -d)"
echo -e "alpha\nbeta\ngamma" > src.txt
git add src.txt && git commit -q -m "add src"
echo -e "alpha\nbeta-modified\ngamma\ndelta" > src.txt
pointbreak capture >/dev/null                    # a fact in the clone-local <git-common-dir>/pointbreak store, to fold forward

pointbreak store status | jq '{mode, storeRef}'                     # before link
pointbreak store link demo-family --dry-run | jq '{schema}'         # preview only; writes nothing, exits 0
pointbreak store link demo-family | jq '{schema, familyRef, createdFamily, foldedEventsCreated}'
pointbreak store status | jq '{mode, storeRef, liveCloneCount, orphaned}'   # after link
echo "later change" >> src.txt && pointbreak capture >/dev/null     # now writes into the family store
pointbreak store unlink | jq '{schema, previousFamilyRef, deregistered}'
pointbreak store status | jq '{mode, storeRef}'                     # back to clone-local
```

**Expect.**

- Before link, `store status` reports `mode: "local"` and `storeRef: "local"` (the clone-local
  `<git-common-dir>/pointbreak` default).
- `store link … --dry-run` emits a `pointbreak.store-link-preview` document and exits 0 without writing
  anything; the real `store link` emits `pointbreak.store-link` with `familyRef: "demo-family"`,
  `createdFamily: true`, and `foldedEventsCreated: 2` (the clone-local history folded forward).
- After link, `store status` reports `mode: "user-level"`, `storeRef: "demo-family"`,
  `liveCloneCount: 1`, `orphaned: false`, and the family directory exists at
  `$POINTBREAK_HOME/stores/demo-family/` with `events/` and `artifacts/`.
- Capturing while linked writes into the family store — its `events/` grows to four (the two folded
  events plus the two from the new capture).
- `store unlink` emits `pointbreak.store-unlink` with `previousFamilyRef: "demo-family"` and
  `deregistered: true`; afterward `store status` reports `mode: "local"` again. Unlink moves no
  review data.

See [storage-model.md](./storage-model.md#user-level-family-store-tier) for the link gates
(ephemeral/sensitivity refusals, sync-managed-path warnings, and the destructive `store forget`
verb) that this quick loop does not exercise.

## J. Canonical Review example pack

**Goal.** Verify that the checked checkout-refactor example reconstructs both its synthetic Git
history and its artifact-complete Pointbreak record without copying a raw store.

```bash
just review-example-verify

EXAMPLE_REPO=$(mktemp -d)/checkout-refactor
just review-example-materialize "$EXAMPLE_REPO"

git -C "$EXAMPLE_REPO" log --oneline --reverse
node "$EXAMPLE_REPO/checkout.test.js"
cargo run -- inspect --repo "$EXAMPLE_REPO" --open
```

**Expect.**

- Verification checks the pack manifest, all file-byte digests, the 13 unsigned events, the object
  artifact, the Git bundle, and the checked history/revision documents.
- The Git log contains the base checkout, faulty refactor, and null-user response commits.
- The source test passes, and the inspector shows current `accepted` with the earlier
  `needs_changes` assessment retained as `replaced`.
- The materialized repository owns a newly ingested local store; the pack itself contains no
  `<git-common-dir>/pointbreak`, `.pointbreak/data`, or `state.json` compatibility surface.

Maintainers refresh the pack from an explicit source repository only after the source record has
been reviewed:

```bash
just review-example-export /path/to/source-review-repository
```

The exporter reads committed Git objects plus public Pointbreak events/artifacts/documents, stages
the complete replacement, and validates it before replacing the checked pack.

To refresh the product-owned marketing capture from this exact record, start the local inspector
against the materialized repository, then run the pack-aware capture recipe from another shell:

```bash
cargo run -- inspect --repo "$EXAMPLE_REPO" --port 7878
just capture-marketing-review-screenshots
```

The capture script verifies the pack first, derives the revision, track, selected assessment,
event-set hash, writer set, and unsigned classification from it, captures both themes, and writes
`assets/marketing/review-interface-capture.json` last. The manifest deliberately distinguishes a
publicly reproducible record from a hosted inspector: `reproducibleFromPublicPack` is true while
`publiclyInspectable` remains false. Running `just capture-inspector-screenshots` without the pack
options preserves the generic README screenshot defaults.

## K. Things to glance at after big changes

When refactoring storage, projections, or CLI surfaces, also look at:

- **JSON document schemas**: every command's top-level `schema` and `version` should still match the
  README's "Current CLI" section.
- **Event file count**: each `add`/`request`/`resolve`/`apply` call should create exactly one new
  event file unless it is a same-key idempotent retry.
- **Artifact dedup**: writing two observations with the same **large** body string should yield
  one file in `<git-common-dir>/pointbreak/artifacts/notes/` (content-addressed) and two events that both reference it
  by content hash. Bodies under roughly 4 KiB stay inline in the event payload and do not produce
  an artifact at all, so use a body well over that threshold to exercise this path —
  `python3 -c "print('x'*5000)" > big-body.txt` and pass `--body-file big-body.txt` to two
  separate `observation add` calls.
- **Exit codes**: piping `pointbreak revision show` or `pointbreak history` through
  `jq -e 'has("schema")'` should always exit 0 for successful runs.
- **Tracing**: passing `--log debug --log-file /tmp/pointbreak.log` to any command should write spans to
  that file and not corrupt the JSON on stdout. (`--log info` emits no spans, so the file stays
  empty; use `debug` or `trace` to exercise this path.)

## What this playbook does not cover

- Performance benchmarking or stress tests.
- Multi-writer coordination — V1 is intentionally single-writer per resolved store (the default
  `<git-common-dir>/pointbreak`, an ephemeral `.pointbreak/data`, or a linked family store).
- Daemon, notification, or delivery-queue behavior — none of those exist in V1.

If a workflow you exercise during real review reveals a gap that is not covered here, add a short
section above following the same pattern: goal, commands, expected output.
