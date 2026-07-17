---
name: pointbreak-author
description: Use when a coding agent has finished a coherent implementation change, is about to declare work done, is about to commit the current task, or the user says done, hand off, ready for review, or ship it and wants to leave a durable Pointbreak review record. Capture with pointbreak capture, record what changed and why as observations, record validation evidence for checks actually run, open input requests for genuine unresolved questions, and then stand down.
---

# Pointbreak Author Handoff

You are the coding agent that just authored the change. At the end of a coherent unit of work, leave a
durable Pointbreak handoff record before you tell the user the task is done. Your job is to make your
change legible for review, not to review your own work.

Do not run `pointbreak assessment add`. Assessments are the reviewer's call. If you assess your own
work, you turn the handoff into self-grading and pollute the review surface the reviewer owns.

## Workflow at a glance

This skill targets the `0.7.0`-and-later flat `pointbreak` CLI. Use only the canonical Pointbreak
environment and storage names; existing installations must complete the offline move described in
`docs/installation.md` before review work resumes. `pointbreak store paths --repo . --format json`
reports the canonical paths, including the Git-common-dir store shared by linked worktrees.

```text
1. Confirm the full task change you intend to hand off — uncommitted in the worktree, or landed in commits.
2. Capture the revision with a concise `--summary`: `pointbreak capture --summary <text>`, or add
   `--base <rev>` for a landed range.
3. Choose one author track for this handoff.
4. Add observations on that track for what changed, why, and review risks.
5. Record validation evidence on that track for checks you actually ran.
6. Open input requests on that track only for genuine unanswered decisions.
7. Read back the authored observations, validation evidence, and open input requests.
8. Stop and tell the user the Pointbreak handoff record exists.
```

Run this loop when you are about to say the task is complete, when the user says "done" or
"hand off", or before switching to unrelated work. It may run before or after committing; choose the
capture source that covers the whole coherent change. Capture once per coherent change, not once per
edit.

## Capture The Right Revision

If the task is still uncommitted, plain `pointbreak capture` records the Git worktree diff from `HEAD`
to the working tree. Default worktree capture excludes untracked files to match `git diff HEAD`, so
use `pointbreak capture --include-untracked` when the intended change includes new files that are not
staged.

Always pass a concise `--summary` that names the coherent change in human terms. This is the primary
discovery label in revision lists, the Inspector, VS Code, and review loops. It does not change the
revision ID, but it is immutable capture metadata, so do not omit it expecting to add it on a later
recapture. Prefer an imperative phrase specific enough to distinguish this handoff from neighboring
work, such as `Add capture summaries to revision discovery`.

If the task is committed, capture the landed range with `pointbreak capture --base <rev>`. That captures
the tree diff from `<rev>` to `--target` (default `HEAD`) without reading the working tree or
untracked files. `--base` resolves any rev — a branch, tag, `HEAD~N`, or commit OID — so point it at
the commit the task started from.

```bash
git status --short

# Uncommitted task in the worktree.
capture_file=$(mktemp)
pointbreak capture --summary "<concise change summary>" | tee "$capture_file" | jq .
revision_id=$(jq -r '.revision.id' "$capture_file")
rm "$capture_file"

# Committed task: capture the landed range from the task's starting commit.
capture_file=$(mktemp)
pointbreak capture --base <commit-before-task> --summary "<concise change summary>" | tee "$capture_file" | jq .
revision_id=$(jq -r '.revision.id' "$capture_file")
rm "$capture_file"
```

Find the starting commit from the task's own history — for example `git log` to locate the commit
before your first task commit, or the branch point. Never rewrite history to manufacture a
capturable diff: do not `git reset --soft` back to the base just to fake a worktree change. Use
`--base` instead.

If `git status --short` is empty and no commits belong to this task, there is nothing to hand off:
tell the user there is no change for Pointbreak to capture. If you committed only part of the task
and left the rest uncommitted, plain `pointbreak capture` sees only the uncommitted remainder; capture
the whole change with `--base` from the task's starting commit, or finish the remaining work before
capturing.

Use the captured revision ID for every write. If `jq` is unavailable, copy `revision.id` from the
compact JSON output and use it in place of `$revision_id`.

## Choose your track

Choose one track for the whole handoff and reuse it for every `--track`. Use the form
`agent:<agent-name>-<id>`.

`<agent-name>` is your own short lowercase agent name. `<id>` is a short run-unique tag: prefer the
issue or PR number, use the branch's distinctive segment as a fallback, and use a short random tag if
neither exists. Keep the part after `agent:` lowercase, hyphenated, and around 15 characters or fewer.

Tracks are review lanes, not actor identity. The unique tag keeps lanes legible when more than one
agent run writes to the same resolved store; Pointbreak still records writer provenance separately
in the event envelope.

```bash
agent_name="<agent-name>"
run_id="<id>"
track="agent:${agent_name}-${run_id}"
export POINTBREAK_ACTOR_ID="actor:agent:${agent_name}"
```

The actor id is your durable identity across sessions and runs — it carries no run id. Use **one
canonical spelling** for your agent name and always the same one (`claude-code`, never also
`claude`): two spellings split one agent's history across two identities. Keep it lowercase and
hyphenated, like the track rule; `/` inside the agent segment is reserved.

**Signing is automatic.** On your first write under this `actor:agent:*` id, Pointbreak generates a
passphrase-less per-machine key and signs the event; it prints a one-line notice with your `did:key`
and `pointbreak key enroll` so a human can add you to the committed allow-list (once enrolled, your
signed events verify and bind). Signing never blocks a write — if no key can be made the write still
succeeds, unsigned. Set `POINTBREAK_SIGNING=off` to disable signing. A human can instead reuse an existing
SSH key via `pointbreak key use-ssh` (agents still auto-keygen, unchanged).

## Record observations

Use observations for durable author context, including decisions, trade-offs, risk areas, and files
the reviewer should inspect first. Prefer file and line anchors when the observation belongs to a
specific part of the diff.

```bash
pointbreak observation add \
  --revision "$revision_id" \
  --track "$track" \
  --title "Parser keeps the existing whitespace contract" \
  --file src/parser.rs --start-line 84 --end-line 123 \
  --body "The parser now accepts the new token form while preserving the old whitespace path. The branch stays local to parsing so callers do not need a compatibility shim."

pointbreak observation add \
  --revision "$revision_id" \
  --track "$track" \
  --title "Verification covered the changed parser and full suite" \
  --body "Ran the targeted parser test and the repository test suite after the final edit. No generated artifacts were changed."

pointbreak observation add \
  --revision "$revision_id" \
  --track "$track" \
  --title "Targeted parser test was red first" \
  --body "The targeted parser test failed before the implementation change, confirming it covered the old behavior. That pre-change failure did not run against the captured revision, so it is recorded as context rather than validation evidence."
```

Good observation titles are short and specific. The body should explain why the fact matters for the
reviewer. Do not paste a transcript, summarize every hunk, or claim verification that you did not
actually run.

To acknowledge or dispose of a reviewer's observation non-destructively — "noted — tracking as issue
#N" — add an observation with `--responds-to <observation-id>` (see
[ADR-0026](../../docs/adr/adr-0026-fact-to-fact-response-relationship.md)). This leaves the reviewer's
observation `Active` and records who responded. Do not use `--supersedes` to acknowledge; that retires
the target from the current set. Answer a reviewer's *decision* request with
`pointbreak input-request respond --outcome {dismissed|superseded|abandoned} --reason …`, not a plain
observation.

**Body content type — prefer Markdown for review content.** Observation and input-request `--body`,
and validation `--summary`, default to plain text, but the inspector renders Markdown and the human
reviews there, so **author review content as Markdown** whenever it names code. Pass
`--body-content-type text/markdown` (or `--summary-content-type text/markdown`) and:

- backtick file paths, code/identifier/type references, and function signatures
  (e.g. `` `src/parser.rs` ``, `` `parse_tokens` ``, `` `Option<&str>` ``);
- prefer short bullet lists over long paragraphs.

Keep bodies concise — Markdown is for scannability, not license to write a document.

## Record validation evidence

Use validation evidence for concrete check results: tests, lint, builds, format checks, or equivalent
verification commands that ran against the captured revision. Validation evidence is advisory
review context only. It never accepts, rejects, merges, blocks, or replaces the reviewer's
assessment.

```bash
pointbreak validation add \
  --revision "$revision_id" \
  --track "$track" \
  --check-name "targeted parser test" \
  --status passed \
  --command "cargo +stable nextest run -p pointbreak --test parser" \
  --exit-code 0 \
  --summary "Passed after the final edit against the captured revision."

pointbreak validation add \
  --revision "$revision_id" \
  --track "$track" \
  --check-name "just check" \
  --status passed \
  --command "just check" \
  --exit-code 0 \
  --summary "Completed after the final edit. This covered commit checks, build, lint, and tests."
```

Validation checks target the whole captured revision. Do not add file, range, or path targets; if
the reviewer needs to know where a check matters, add an anchored observation separately. Do not
record checks you did not run. If a planned check was intentionally skipped, record it as `skipped`
only when the summary says why.

## Open input requests

Open an input request only when someone else needs to answer something. Use `--mode operative` when
the answer should block landing. Use `--mode advisory` for durable context that does not need to
pause the workflow.

```bash
pointbreak input-request open \
  --revision "$revision_id" \
  --track "$track" \
  --title "Confirm whether the relaxed parser should be documented" \
  --reason manual-decision-required \
  --mode advisory \
  --body "The implementation accepts the new form, but I did not update user-facing docs because the prompt did not say whether this behavior should be advertised yet."

pointbreak input-request open \
  --revision "$revision_id" \
  --track "$track" \
  --title "Choose the default for conflicting config values" \
  --reason ambiguous-state \
  --mode operative \
  --body "Both existing call sites are plausible defaults. I left the behavior unchanged and need a reviewer to choose before landing the new option."
```

Use the current command name `pointbreak input-request`. Use `pointbreak assessment` only when you
are acting as the reviewer, not while authoring the handoff.

## Read back and hand off

Verify that the handoff is visible before you stop:

```bash
pointbreak observation list --revision "$revision_id" --track "$track" --format json-pretty
pointbreak validation list --revision "$revision_id" --track "$track" --include-body --format json-pretty
pointbreak input-request list --revision "$revision_id" --track "$track" --status open --format json-pretty
```

These commands verify the author's writes without replaying the captured snapshot. The
`pointbreak revision show --format json-pretty` command emits the full integration-JSON document: it includes the
complete captured snapshot, is large for any real change, and is meant for tooling or the rare case
where the full snapshot is genuinely needed. It is not the human readback surface.

Then stand down with a concise message:

```text
Created the Pointbreak handoff record on `<track>`. Read it with
`pointbreak observation list --revision <id> --track <track> --include-body --format json-pretty`
and
`pointbreak validation list --revision <id> --track <track> --include-body --format json-pretty`
and
`pointbreak input-request list --revision <id> --track <track> --status open --include-body --format json-pretty`.
I did not add an assessment; that is for the reviewer.
```

## Standing down

After the capture, observations, validation evidence, any input requests, and readback are complete,
do not keep editing as part of the same handoff. Continue with commit, push, PR, or review-loop
steps only when the user has asked for them. Do not add an assessment from this authoring role.

If the user immediately asks for another implementation task, treat that as a new unit of work and
capture a separate handoff when that task reaches its own end.

## Common errors

- **Faking a worktree diff after a commit.** Committing first is not an error: capture the landed
  change with `pointbreak capture --base <commit-before-task>`. The error is rewriting history —
  for example `git reset --soft` — to manufacture a worktree diff. Never do that; use `--base`.
- **Recapturing because a commit landed.** A commit that lands the already-captured change — during
  review or after an assessment — is recorded with
  `pointbreak association record --commit <sha> --revision <id> --track <track>` on the same revision,
  never with a new capture. Successive landed commits accreting on one revision are the expected
  multi-pass shape. `pointbreak capture --supersedes` is only for a genuinely new content state
  replacing the reviewed one.
- **Claiming verification you did not run.** Record only checks you actually performed, including
  failures or skipped checks when they matter.
- **Putting check results only in observations.** Record concrete command results with
  `pointbreak validation add`; use observations for the surrounding decision or risk context.
- **Treating validation as acceptance.** Validation evidence is advisory and never replaces the
  reviewer's assessment.
- **Forgetting `--revision`.** If more than one revision is current, write commands fail until
  you pass the captured revision ID.
- **Self-assessing.** The authoring agent records observations and input requests only. A reviewer
  records assessments.
- **Recording vague observations.** "Implemented the feature" is not useful. Say what changed, why
  the shape is reasonable, what was verified, and where the reviewer should look first.
- **Opening input requests for ordinary notes.** If no answer is needed, write an observation.
- **Capturing every small edit.** Wait for a coherent unit of implementation work.
- **Using inconsistent tracks.** Set one `track` value for the handoff and reuse it for every author
  observation, input request, and readback command.
- **Putting the run id in `POINTBREAK_ACTOR_ID`.** The run/issue id belongs in the `--track`
  (`agent:claude-code-234`), NEVER in the actor id. `POINTBREAK_ACTOR_ID=actor:agent:claude-code-234` mints
  a brand-new per-run identity whose auto-generated key is **not enrolled**, so every event lands
  signed-but-**untrusted** with no diagnostic — while the durable, already-enrolled
  `actor:agent:claude-code` key goes unused. Always
  `export POINTBREAK_ACTOR_ID="actor:agent:<agent-name>"` with no run suffix (`pointbreak key list` shows the
  canonical name as `enrolled:true`). Do not `pointbreak key enroll` a throwaway per-run key to paper
  over it. When these instructions arrive as skill args, take the run id as the track only.
