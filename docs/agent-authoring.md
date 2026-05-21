# Agent Authoring Handoffs

Shoreline works best when the agent that made a change also leaves the first durable review record.
The agent is not reviewing itself. It is capturing the exact diff it just authored and recording the
context a reviewer would otherwise have to reconstruct from chat, terminal output, and memory.

This loop is for the end of a coherent unit of implementation work. It fits the moment just before an
agent says a task is done, before any commit that would move `HEAD` past part of the task, when a
human says "done" or "hand off", or when the agent is about to switch to unrelated work. Capture once
per meaningful change, not after every file edit.

The authoring skill is reactive: it triggers at the end of a work session, so install it in the
agent environment before that session begins. If the skill is added only afterward, the change may
already be committed, which leaves only the degraded post-commit path: v0.1.0 `shore review capture`
reads `HEAD` to the working tree, so a clean post-commit working tree has nothing useful left to
capture.

## The Capture Moment

`shore review capture` freezes the current Git worktree diff from `HEAD` to the working tree,
including untracked files. That timing matters. If the agent commits part of the task first, a later
capture only sees the remaining uncommitted diff; if it commits everything and leaves a clean working
tree, there is no task diff left for Shoreline to capture. For an agent-authored change, the expected
order is: finish the implementation, run the relevant checks, capture the ReviewUnit while the full
change is still in the worktree, record the handoff facts, then stop.

Humans set up the loop by making the expectation explicit: when the agent reaches the end of a task,
it should run Shoreline before declaring the task complete. Agents execute the loop from inside the
repository that contains the change. Reviewers pick up the recorded ReviewUnit afterward and make the
review call on their own track.

## What The Author Records

The authoring agent records observations and input requests on one author track chosen for that
handoff. Use the form `agent:<agent-name>-<id>`, where `<agent-name>` is the agent's own short name
such as `claude`, `codex`, or `cursor`, and `<id>` is a short run-unique tag. Prefer an issue or PR
number, use the branch's distinctive segment as a fallback, and use a short random tag if neither
exists. Keep the part after `agent:` lowercase, hyphenated, and around 15 characters or fewer.

Tracks are review lanes, not actor identity. The unique tag keeps the lane legible when more than one
agent run writes to the same `.shore/` store, while Shoreline records writer provenance separately in
the event envelope.

Observations explain what changed and why. They should call out the design choices, tests run, risk
areas, follow-up edges, and files or line ranges a reviewer should inspect first. A useful observation
is specific enough that someone can understand the change without scrolling back through the agent's
transcript.

Input requests are for genuine open questions. Use them when the agent could not responsibly decide
something on its own: ambiguous requirements, a risky choice that needs approval, or a manual decision
that should happen before landing. Use `--mode operative` when the answer should block landing, and
`--mode advisory` when the request is durable context for the reviewer but does not need to pause the
workflow.

The authoring agent must not add a `shore review assessment`. Assessments are the reviewer's current
call, such as `accepted` or `needs-changes`. If the author records its own assessment, it turns the
handoff into self-grading and pollutes the review surface the reviewer owns.

## Author Loop

An agent-authored handoff looks like this:

```bash
git status --short
capture_file=$(mktemp)
shore review capture | tee "$capture_file" | jq .
review_unit_id=$(jq -r '.reviewUnit.id' "$capture_file")
rm "$capture_file"
agent_name="<agent-name>"
run_id="<id>"
track="agent:${agent_name}-${run_id}"

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

shore review input-request open \
  --review-unit "$review_unit_id" \
  --track "$track" \
  --title "Confirm whether the relaxed parser should be documented" \
  --reason manual-decision-required \
  --mode advisory \
  --body "The implementation accepts the new form, but I did not update user-facing docs because the prompt did not say whether this behavior should be advertised yet."

shore review observation list --review-unit "$review_unit_id" --track "$track" --pretty
shore review input-request list --review-unit "$review_unit_id" --track "$track" --status open --pretty
```

The commands emit compact JSON by default, so piping capture output through `jq` is only for human
readability. The readback uses bounded list commands so the author can verify the observations and
open input requests without replaying the captured snapshot. `shore review unit show` remains the
full composite JSON view of a ReviewUnit; it includes the complete captured snapshot, can be large
for real changes, and is meant for tooling or cases where the full snapshot is genuinely needed. The
write commands above pass the captured ReviewUnit ID explicitly because write commands can infer a
ReviewUnit only when exactly one current capture exists. If `jq` is not available, copy
`reviewUnit.id` from `shore review capture` output and use it in place of `$review_unit_id`.

## What A Good Handoff Looks Like

A good handoff is short, concrete, and review-oriented. It names the files that matter, the reason
the shape of the change is acceptable, what validation actually ran, and where the author is least
certain. Verification observations should report only checks the author actually performed. It does
not repeat every diff hunk, and it does not bury the reviewer in generic status updates.

Prefer anchored observations when the fact belongs to a file or line range. Use review-wide
observations for cross-cutting decisions, verification notes, and risks that do not live in one file.
Open an input request only when someone else needs to answer something; ordinary follow-up ideas are
better as observations unless they require a decision.

After the author stops, a reviewer can read the handoff with:

```bash
shore review observation list --review-unit <review-unit-id> --track <track> --include-body --pretty
shore review input-request list --review-unit <review-unit-id> --track <track> --status open \
  --include-body --pretty
```

The reviewer then records their own facts on their own track. For example:

```bash
shore review assessment add \
  --review-unit <review-unit-id> \
  --track human:kevin \
  --assessment needs-clarification \
  --summary "The change is understandable, but the open documentation question needs an answer before landing."
```

That separation keeps the author's explanation and the reviewer's call distinct.
