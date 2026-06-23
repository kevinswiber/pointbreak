# Getting Started

This guide walks through one local Shoreline review from an empty scratch repository. It is meant to
show the shape of the workflow, not every option.

## 1. Install Shoreline

Install the `shoreline` crate; it provides the `shore` command:

```bash
cargo install shoreline
shore --help
```

When working from a source checkout instead, build the release binary and use it directly:

```bash
cargo build --release
./target/release/shore --help
```

## 2. Create A Scratch Change

Shoreline reviews a Git worktree diff. The repository needs a baseline commit so Git has a `HEAD`
to compare against.

```bash
rm -rf shoreline-review-scratch
mkdir shoreline-review-scratch
cd shoreline-review-scratch

git init -q
git config user.email "reviewer@example.com"
git config user.name "Reviewer"
git config commit.gpgsign false

mkdir -p src
printf '%s\n' \
  "pub fn greeting() -> &'static str {" \
  '    "hello"' \
  '}' \
  > src/example.rs
git add src/example.rs
git commit -q -m "baseline"

printf '%s\n' \
  'pub fn greeting(name: &str) -> String {' \
  '    format!("hello, {name}")' \
  '}' \
  '' \
  "pub fn fallback_name(input: Option<&str>) -> &str {" \
  '    input.unwrap_or("reviewer")' \
  '}' \
  > src/example.rs
```

Confirm the change is present:

```bash
git status --short
git diff
```

## 3. Capture The Revision

```bash
shore review capture
```

The capture freezes the current diff as a local revision. Shoreline writes immutable event files to
the store's `events/` log, stores captured object artifacts under `artifacts/`, and rebuilds
`state.json` as a projection. By default the store is the shared common-dir store at `.git/shore` —
the same store for every worktree of the clone — and an ephemeral worktree keeps a worktree-local
`.shore/data/` store instead.

Those files are local storage. Use command output as the integration surface instead of depending
on internal file paths.

## 4. Inspect The Review

```bash
shore review show --pretty
```

This shows the composite revision view: captured files and rows, plus any observations, input
requests, assessments, and imported notes already recorded for the same revision.

For a chronological event log, use:

```bash
shore review history --pretty
```

## 5. Record Review Facts

Add an observation for something you noticed while reading the diff:

```bash
shore review observation add \
  --track human:local \
  --title "Fallback name should be intentional" \
  --file src/example.rs \
  --start-line 6 \
  --body "The fallback value is visible user-facing behavior; keep it deliberate."
```

Open an input request when another reviewer, agent, or future you needs to answer a question before
the review can proceed:

```bash
shore review input-request open \
  --track human:local \
  --title "Confirm fallback wording" \
  --reason manual-decision-required \
  --mode advisory \
  --file src/example.rs \
  --start-line 6 \
  --body "Should the fallback be reviewer, user, or something domain-specific?"
```

Record the current assessment:

```bash
shore review assessment add \
  --track human:local \
  --assessment needs-clarification \
  --summary "Implementation is small, but fallback wording needs a decision."
```

Read the updated revision:

```bash
shore review show --pretty --include-body
```

## 6. Where To Go Next

- Run `shore inspect --open` to browse this store in a local web UI: an event timeline,
  per-revision pages, supersession threads, and captured diffs annotated with their review facts.
- [CLI reference](cli-reference.md) lists commands, options, output schemas, and V1 limitations.
- [Review workflow](review-workflow.md) explains when to use capture, observations, input requests,
  assessments, history, and revision reads in a real review.
- [Storage model](storage-model.md) explains durable events, artifacts, and rebuildable
  projections.
- [Input request model](input-request-model.md) explains operative and advisory requests.
- [Assessment model](assessment-model.md) explains review assessment values and replacement
  behavior.
