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

## First useful Review walkthrough — fixed protocol

This section freezes the end-to-end journey behind [getting-started.md](./getting-started.md) as a
repeatable evidence protocol: one operator, a disposable repository with a real tracked change, a
source-built binary, and the complete paired author/reviewer loop, with every visible state and
intervention recorded. Run it after changes to the onboarding surfaces to prove the journey still
holds — and to record exactly what the run can and cannot claim.

Requirements: `jq`, Git, a SHA-256 tool (`shasum -a 256` or the platform equivalent — record which),
a headed local browser, and screen capture. VS Code is neither required nor credited.

### Boundaries and clocks

The protocol keeps two boundaries deliberately separate and never adds their timings together:

- **Clock A — supported acquisition context.** Optionally record the current public release and the
  supported installer route from [installation.md](./installation.md). If rerun, it installs the
  latest published release and proves only that release's installer and checksum behavior. It does
  not exercise the source-built walkthrough below; its timing is version-labelled and kept apart.
- **Source boundary.** Pin the exact source commit under test, verify the checkout is clean and in
  sync with its remote, build with `cargo +stable build --locked --bin pointbreak`, and record the
  commit, `git describe`, the binary SHA-256, and `version --format json`. Every walkthrough
  command uses that absolute binary. Build time and dependency setup stay outside the walkthrough
  clocks. Never describe the source-built binary as the supported installer route.
- **Clock B-short** starts immediately before the capture command and stops at the first Review
  from which the operator can answer all five stage questions (listed below).
- **Clock B-paired** starts immediately before the first author fact and stops when the
  replacement call, the open follow-up, and the same-revision landing are all visible in Review.

Acquisition, PATH repair, browser launch rehearsal, empty-state inspection, operator explanation,
and artifact post-processing stay outside the clocks only when they are explicitly timestamped in
the journal. Assistance is never subtracted from a claimed time.

### Disposable setup (outside the clocks)

From the pinned, clean source checkout:

```bash
cargo +stable build --locked --bin pointbreak
POINTBREAK_BINARY="$PWD/target/debug/pointbreak"

WALK_ROOT=$(mktemp -d)
WALK_REPO="$WALK_ROOT/repo"
WALK_HOME="$WALK_ROOT/home"
WALK_EVIDENCE="$WALK_ROOT/evidence"
mkdir -p "$WALK_REPO" "$WALK_HOME" "$WALK_EVIDENCE"
export POINTBREAK_HOME="$WALK_HOME"

git -C "$WALK_REPO" init
git -C "$WALK_REPO" config user.name "Pointbreak Walkthrough"
git -C "$WALK_REPO" config user.email "pointbreak-walkthrough@example.invalid"
git -C "$WALK_REPO" config commit.gpgsign false
printf '%s\n' 'First useful Review' > "$WALK_REPO/onboarding.txt"
git -C "$WALK_REPO" add onboarding.txt
git -C "$WALK_REPO" commit -m "chore: add onboarding baseline"
printf '%s\n' 'First useful Review' 'Checks are evidence, not a verdict.' > "$WALK_REPO/onboarding.txt"

"$POINTBREAK_BINARY" version --format json > "$WALK_EVIDENCE/00-version.json"
shasum -a 256 "$POINTBREAK_BINARY" > "$WALK_EVIDENCE/00-binary-sha256.txt"
git -C "$WALK_REPO" status --short > "$WALK_EVIDENCE/00-repo-status.txt"
"$POINTBREAK_BINARY" store paths --repo "$WALK_REPO" --format json \
  > "$WALK_EVIDENCE/00-store-paths.json"
```

Do not disable signing, enroll a key, or add `.pointbreak` configuration before the first useful
Review: trust is introduced after value, exactly as the guide teaches.

### Empty first-open check (outside Clock B)

Launch Review once against the empty store before capturing:

```bash
"$POINTBREAK_BINARY" inspect --repo "$WALK_REPO" --open
```

Record screenshot `01-empty-first-open.png`. The empty state must identify the repository, say
there are no captured revisions, and point at capture with a summary — without demanding schema or
trust setup first. Copy any offered command and verify the copied text equals the visible text with
placeholders intact; do not run commands from Review, which stays read-only. A filtered-empty state
must explain filter recovery rather than suggesting a new capture. Any mismatch is a product
defect: stop the run, fix it at the surface that owns it, and restart the protocol from a fresh
disposable setup — do not patch mid-run.

End the check by stopping this Inspector (`Ctrl+C`). The Inspector is a foreground server holding
the default port, so the rehearsal instance must exit before the short path starts the
walkthrough's own server; a second `inspect` against the same port fails with
`Address already in use`.

### Short path (Clock B-short)

The `inspect --open` below starts the walkthrough's only Review server and opens the browser; keep
it running for the rest of the walkthrough and run every later command in a second terminal. Start
a screen recording or timestamped journal, then start Clock B-short immediately before this
capture:

```bash
"$POINTBREAK_BINARY" capture \
  --repo "$WALK_REPO" \
  --summary "Explain evidence in first-use guidance" \
  --format json \
  > "$WALK_EVIDENCE/02-capture.json" \
  2> "$WALK_EVIDENCE/02-capture.stderr.txt"
REVISION_ID=$(jq -r '.revision.id' "$WALK_EVIDENCE/02-capture.json")
"$POINTBREAK_BINARY" inspect --repo "$WALK_REPO" --open
```

Stop Clock B-short only when the operator can answer, from rendered Review without opening raw
JSON or event files:

- **Work:** which tracked file and lines changed, and the immutable summary labelling the revision;
- **Claims:** none recorded yet, plus the observation command that would add one;
- **Evidence:** none recorded yet, plus the validation command that would add one;
- **Questions:** none open yet, plus the request command that would open one;
- **Call:** unassessed, plus the assessment command that would record one.

Record the duration, UTC start/end, screenshot `03-first-useful-review.png`, the copied commands,
viewport, browser console state, and any intervention.

### Paired loop (Clock B-paired) — author handoff

Start Clock B-paired immediately before the first author fact. The first authored fact introduces
the explicit actor ("who wrote this?") and track ("which review lane owns it?"):

```bash
export POINTBREAK_ACTOR_ID="actor:agent:first-review-author"
AUTHOR_TRACK="agent:first-review-author"

"$POINTBREAK_BINARY" observation add \
  --repo "$WALK_REPO" \
  --exact-revision "$REVISION_ID" \
  --track "$AUTHOR_TRACK" \
  --title "First-use guidance distinguishes evidence" \
  --body "The tracked change explains that checks are evidence rather than a verdict." \
  --format json > "$WALK_EVIDENCE/04-author-observation.json"

git -C "$WALK_REPO" diff --check
"$POINTBREAK_BINARY" validation add \
  --repo "$WALK_REPO" \
  --exact-revision "$REVISION_ID" \
  --track "$AUTHOR_TRACK" \
  --check-name "git diff --check" \
  --status passed \
  --command "git diff --check" \
  --exit-code 0 \
  --summary "The captured tracked change has no whitespace errors." \
  --format json > "$WALK_EVIDENCE/05-author-validation.json"
```

Journal the first automatic-signing diagnostic (from capture or the first authored write) and the
trust state Review shows for each writer. Signing is automatic and was never a setup prerequisite.
If signed-but-untrusted appears, continue: it is advisory. Show — but do not run — the optional
`pointbreak key enroll <name>` recovery and note that it stages `.pointbreak/allowed-signers.json`
for human review. Enrollment happens after value, never inside the captured change.

### Paired loop — reviewer pass

```bash
export POINTBREAK_ACTOR_ID="actor:agent:first-review-reviewer"
REVIEWER_TRACK="agent:first-review-reviewer"

"$POINTBREAK_BINARY" observation add \
  --repo "$WALK_REPO" \
  --exact-revision "$REVISION_ID" \
  --track "$REVIEWER_TRACK" \
  --title "Release proof remains separate" \
  --body "The walkthrough is reviewable locally, but the released artifact has not repeated it." \
  --format json > "$WALK_EVIDENCE/06-reviewer-observation.json"
REVIEWER_OBSERVATION_ID=$(jq -r '.observationId' "$WALK_EVIDENCE/06-reviewer-observation.json")

git -C "$WALK_REPO" diff --check
"$POINTBREAK_BINARY" validation add \
  --repo "$WALK_REPO" \
  --exact-revision "$REVISION_ID" \
  --track "$REVIEWER_TRACK" \
  --check-name "reviewer git diff --check" \
  --status passed \
  --command "git diff --check" \
  --exit-code 0 \
  --summary "The reviewer independently reran the whitespace check." \
  --format json > "$WALK_EVIDENCE/07-reviewer-validation.json"

"$POINTBREAK_BINARY" input-request open \
  --repo "$WALK_REPO" \
  --revision "$REVISION_ID" \
  --track "$REVIEWER_TRACK" \
  --title "Confirm the recovery boundary" \
  --reason manual-decision-required \
  --mode advisory \
  --body "Should PATH recovery stay separate from the first useful Review clock?" \
  --format json > "$WALK_EVIDENCE/08-reviewer-question.json"
INPUT_REQUEST_ID=$(jq -r '.inputRequestId' "$WALK_EVIDENCE/08-reviewer-question.json")

"$POINTBREAK_BINARY" assessment add \
  --repo "$WALK_REPO" \
  --exact-revision "$REVISION_ID" \
  --track "$REVIEWER_TRACK" \
  --assessment needs-clarification \
  --summary "The Review is useful, but the clock boundary needs an explicit answer." \
  --related-observation "$REVIEWER_OBSERVATION_ID" \
  --related-input-request "$INPUT_REQUEST_ID" \
  --format json > "$WALK_EVIDENCE/09-provisional-assessment.json"
PROVISIONAL_ASSESSMENT_ID=$(jq -r '.assessmentId' "$WALK_EVIDENCE/09-provisional-assessment.json")
```

### Paired loop — author response

The durable answer lives on the request; the follow-up observation adds context on the author
track. (`observation add --responds-to` links observation to observation only — it does not accept
an input-request id.)

```bash
export POINTBREAK_ACTOR_ID="actor:agent:first-review-author"

"$POINTBREAK_BINARY" input-request respond "$INPUT_REQUEST_ID" \
  --repo "$WALK_REPO" \
  --outcome approved \
  --reason "PATH recovery is setup assistance outside the first useful Review clock." \
  --format json > "$WALK_EVIDENCE/10-author-response.json"

"$POINTBREAK_BINARY" observation add \
  --repo "$WALK_REPO" \
  --exact-revision "$REVISION_ID" \
  --track "$AUTHOR_TRACK" \
  --title "Clock boundary is explicit" \
  --body "The journal separates acquisition and PATH recovery from the walkthrough clocks." \
  --format json > "$WALK_EVIDENCE/11-author-follow-up.json"
```

Both writes change the durable review record, not the captured content; nothing is recaptured and
the revision id does not change.

### Paired loop — reviewer replacement and open follow-up

```bash
export POINTBREAK_ACTOR_ID="actor:agent:first-review-reviewer"

"$POINTBREAK_BINARY" input-request open \
  --repo "$WALK_REPO" \
  --revision "$REVISION_ID" \
  --track "$REVIEWER_TRACK" \
  --title "Verify the release-candidate rerun" \
  --reason insufficient-evidence \
  --mode advisory \
  --body "The released artifact must repeat this path before a release claim." \
  --format json > "$WALK_EVIDENCE/12-release-follow-up.json"
FOLLOW_UP_REQUEST_ID=$(jq -r '.inputRequestId' "$WALK_EVIDENCE/12-release-follow-up.json")

"$POINTBREAK_BINARY" assessment add \
  --repo "$WALK_REPO" \
  --exact-revision "$REVISION_ID" \
  --track "$REVIEWER_TRACK" \
  --assessment accepted-with-follow-up \
  --summary "The walkthrough is accepted; released-artifact proof remains open." \
  --replaces "$PROVISIONAL_ASSESSMENT_ID" \
  --related-input-request "$FOLLOW_UP_REQUEST_ID" \
  --format json > "$WALK_EVIDENCE/13-current-assessment.json"
```

### Paired loop — land the commit on the same revision

```bash
git -C "$WALK_REPO" add onboarding.txt
git -C "$WALK_REPO" commit -m "docs: clarify first Review evidence"
LANDED_COMMIT=$(git -C "$WALK_REPO" rev-parse HEAD)

export POINTBREAK_ACTOR_ID="actor:agent:first-review-author"
"$POINTBREAK_BINARY" association record \
  --repo "$WALK_REPO" \
  --revision "$REVISION_ID" \
  --track "$AUTHOR_TRACK" \
  --commit "$LANDED_COMMIT" \
  --format json > "$WALK_EVIDENCE/14-commit-association.json"

"$POINTBREAK_BINARY" revision list --repo "$WALK_REPO" --format json \
  > "$WALK_EVIDENCE/15-revision-list.json"
"$POINTBREAK_BINARY" attention list --repo "$WALK_REPO" --format json \
  > "$WALK_EVIDENCE/16-attention.json"
"$POINTBREAK_BINARY" revision show "$REVISION_ID" --repo "$WALK_REPO" --format json \
  > "$WALK_EVIDENCE/17-revision.json"
```

Stop Clock B-paired only when Review visibly shows the final state below.

### Required visible final state

Without reading raw event files or reconstructing the JSON evidence documents, the operator must
answer from Review:

| Question | Required Review answer |
| --- | --- |
| Work | The exact tracked file/diff and the immutable capture summary |
| Claims | Author claim, reviewer release-boundary observation, and author follow-up, each with attribution/track |
| Evidence | Author and reviewer `git diff --check` passes with command/exit context, worded as evidence, not a verdict |
| Questions | The first advisory request answered; the release-verification follow-up still open |
| Call | `accepted-with-follow-up` current; `needs-clarification` visibly replaced |
| Follow-up | The release rerun visible in attention and detail |
| Roles | Author and reviewer facts distinguishable by actor and track |
| Trust | Each writer's verification state visible; untrusted distinguished from invalid |
| Landing | The exact commit associated with the original `REVISION_ID` |
| Identity | Exactly one revision in the list; no successor was created for the commit |

Record wide and narrow screenshots, copied contextual commands, keyboard navigation, the help
overlay, browser console/network state, and the final URL and repository identity.

### Recovery points

Every recovery is journaled. Setup-only recoveries stay outside the clocks when timestamped:

| Point | Expected recovery |
| --- | --- |
| Installed binary not found | Apply the installer-emitted PATH command; on Windows restart the terminal; re-run `pointbreak --version` |
| Wrong repository/store | Run `pointbreak store paths --repo <repo> --format text`; restart Review with an exact `--repo` |
| No tracked change | Confirm `git status --short`; modify an already tracked file; do not substitute a sample |
| Untracked change omitted | Track the file before modifying it, or deliberately pass `--include-untracked`; journal the choice |
| More than one revision | Keep `.revision.id` from the capture output and pass it explicitly; do not rely on single-revision defaults |
| Port/browser open failure | Read the startup URL, choose another `--port`, open the printed URL manually; never expose a non-loopback host |
| Signed but untrusted writer | Continue; explain the advisory state; offer `pointbreak key enroll` only after value is visible |
| Ambiguous assessment | Use `assessment add --replaces <id>` and verify one current call |
| Follow-up missing from attention | Open a related advisory request and link it with `--related-input-request` |
| Commit created after capture | Record the association on the same revision; do not recapture unchanged content |
| Raw JSON temptation | Use Review detail/diff/attention first; JSON stays a captured evidence artifact, not the comprehension path |

### Evidence, interventions, and nonclaims

- Preserve stdout JSON untouched as evidence; build the journal from command results and Review
  screenshots, never from edited transcripts.
- Write an artifact manifest: relative path, SHA-256, producing command, UTC timestamp, and whether
  the artifact belongs to setup, Clock B-short, or Clock B-paired.
- Write an intervention ledger: trigger, exact assistance, clock inclusion/exclusion, and recovery
  result. Zero interventions are written as zero, not omitted.
- Reconfirm the disposable repository holds exactly one revision with the associated commit. Do not
  enroll trust, recapture, or copy the disposable store into real state. Remove the disposable
  directories only after evidence retention is confirmed.
- State the nonclaims explicitly: one experienced operator, a disposable repository, a source-built
  binary, and a local loopback Review. The supported installer route was not exercised for this
  walkthrough, so the run proves nothing about a released artifact. The result is not a novice
  claim, not a five-minute-activation claim, and not a population or confidence claim.

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
