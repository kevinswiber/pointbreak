---
name: shoreline-author
description: Use when a coding agent has finished a coherent implementation change, is about to declare work done, is about to commit the current task, or the user says done, hand off, ready for review, or ship it and wants to leave a durable Shoreline review record. Capture with shore review capture, record what changed and why as observations, open input requests for genuine unresolved questions, and then stand down.
---

# Shoreline Author Handoff

You are the coding agent that just authored the change. At the end of a coherent unit of work, leave a
durable Shoreline handoff record before you tell the user the task is done. Your job is to make your
change legible for review, not to review your own work.

Do not run `shore review assessment add`. Assessments are the reviewer's call. If you assess your own
work, you turn the handoff into self-grading and pollute the review surface the reviewer owns.

## Workflow at a glance

```text
1. Confirm the full task diff you intend to hand off is still in the worktree.
2. Capture the current ReviewUnit with `shore review capture`.
3. Choose one author track for this handoff.
4. Add observations on that track for what changed, why, tests run, and review risks.
5. Open input requests on that track only for genuine unanswered decisions.
6. Read back the authored observations and open input requests with bounded list commands.
7. Stop and tell the user the Shoreline handoff record exists.
```

Run this loop when you are about to say the task is complete, before committing any part of the
current task, when the user says "done" or "hand off", or before switching to unrelated work. Capture
once per coherent change, not once per edit.

## Capture first

Shoreline v0.1.0 captures the Git worktree diff from `HEAD` to the working tree, including untracked
files. If you commit part of the task first, a later capture only sees the remaining uncommitted
diff. If you commit everything and leave a clean working tree, there may be nothing useful left to
capture.

```bash
git status --short
capture_file=$(mktemp)
shore review capture | tee "$capture_file" | jq .
review_unit_id=$(jq -r '.reviewUnit.id' "$capture_file")
rm "$capture_file"
```

If `git status --short` is empty, do not invent a handoff. Tell the user there is no working-tree
diff for Shoreline to capture. If you already committed part of the current task, do not present a
later capture as the full task handoff; tell the user Shoreline can only capture the remaining
worktree diff unless they choose another review shape.

Use the captured ReviewUnit ID for every write. If `jq` is unavailable, copy `reviewUnit.id` from the
compact JSON output and use it in place of `$review_unit_id`.

## Choose your track

Choose one track for the whole handoff and reuse it for every `--track`. Use the form
`agent:<agent-name>-<id>`.

`<agent-name>` is your own short lowercase agent name. `<id>` is a short run-unique tag: prefer the
issue or PR number, use the branch's distinctive segment as a fallback, and use a short random tag if
neither exists. Keep the part after `agent:` lowercase, hyphenated, and around 15 characters or fewer.

Tracks are review lanes, not actor identity. The unique tag keeps lanes legible when more than one
agent run writes to the same `.shore/` store; Shoreline still records writer provenance separately in
the event envelope.

```bash
agent_name="<agent-name>"
run_id="<id>"
track="agent:${agent_name}-${run_id}"
```

## Record observations

Use observations for durable author context, including decisions, trade-offs, validation, risk areas,
and files the reviewer should inspect first. Prefer file and line anchors when the observation belongs
to a specific part of the diff.

```bash
shore review observation add \
  --review-unit "$review_unit_id" \
  --track "$track" \
  --title "Parser keeps the existing whitespace contract" \
  --file src/parser.rs --start-line 84 --end-line 123 \
  --body "The parser now accepts the new token form while preserving the old whitespace path. The branch stays local to parsing so callers do not need a compatibility shim."

shore review observation add \
  --review-unit "$review_unit_id" \
  --track "$track" \
  --title "Verification covered the changed parser and full suite" \
  --body "Ran the targeted parser test and the repository test suite after the final edit. No generated artifacts were changed."
```

Good observation titles are short and specific. The body should explain why the fact matters for the
reviewer. Do not paste a transcript, summarize every hunk, or claim verification that you did not
actually run.

## Open input requests

Open an input request only when someone else needs to answer something. Use `--mode operative` when
the answer should block landing. Use `--mode advisory` for durable context that does not need to
pause the workflow.

```bash
shore review input-request open \
  --review-unit "$review_unit_id" \
  --track "$track" \
  --title "Confirm whether the relaxed parser should be documented" \
  --reason manual-decision-required \
  --mode advisory \
  --body "The implementation accepts the new form, but I did not update user-facing docs because the prompt did not say whether this behavior should be advertised yet."

shore review input-request open \
  --review-unit "$review_unit_id" \
  --track "$track" \
  --title "Choose the default for conflicting config values" \
  --reason ambiguous-state \
  --mode operative \
  --body "Both existing call sites are plausible defaults. I left the behavior unchanged and need a reviewer to choose before landing the new option."
```

Use the current command name `shore review input-request`. Use `shore review assessment` only when you
are acting as the reviewer, not while authoring the handoff.

## Read back and hand off

Verify that the handoff is visible before you stop:

```bash
shore review observation list --review-unit "$review_unit_id" --track "$track" --pretty
shore review input-request list --review-unit "$review_unit_id" --track "$track" --status open --pretty
```

These commands verify the author's writes without replaying the captured snapshot. The
`shore review unit show --pretty` command emits the full integration-JSON document: it includes the
complete captured snapshot, is large for any real change, and is meant for tooling or the rare case
where the full snapshot is genuinely needed. It is not the human readback surface.

Then stand down with a concise message:

```text
Created the Shoreline handoff record on `<track>`. Read it with
`shore review observation list --review-unit <id> --track <track> --include-body --pretty`
and
`shore review input-request list --review-unit <id> --track <track> --status open --include-body --pretty`.
I did not add an assessment; that is for the reviewer.
```

## Standing down

After the capture, observations, any input requests, and readback are complete, stop. Do not keep
editing or make a commit as part of this handoff; wait for the user's next instruction. Do not add an
assessment from this authoring role.

If the user immediately asks for another implementation task, treat that as a new unit of work and
capture a separate handoff when that task reaches its own end.

## Common errors

- **Capturing after a commit.** `shore review capture` records the working-tree diff. Capture while
  the full current-task change is still uncommitted in the worktree.
- **Claiming verification you did not run.** Record only checks you actually performed, including
  failures or skipped checks when they matter.
- **Forgetting `--review-unit`.** If more than one ReviewUnit is current, write commands fail until
  you pass the captured ReviewUnit ID.
- **Self-assessing.** The authoring agent records observations and input requests only. A reviewer
  records assessments.
- **Recording vague observations.** "Implemented the feature" is not useful. Say what changed, why
  the shape is reasonable, what was verified, and where the reviewer should look first.
- **Opening input requests for ordinary notes.** If no answer is needed, write an observation.
- **Capturing every small edit.** Wait for a coherent unit of implementation work.
- **Using inconsistent tracks.** Set one `track` value for the handoff and reuse it for every author
  observation, input request, and readback command.
