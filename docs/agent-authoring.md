# Agent Authoring Handoffs

Shoreline works best when the agent that made a change also leaves the first durable review record.
The agent is not reviewing itself. It is capturing the exact diff it just authored and recording the
context a reviewer would otherwise have to reconstruct from chat, terminal output, and memory.

This loop is for the end of a coherent unit of implementation work. It fits the moment just before an
agent says a task is done, before any commit that would move `HEAD` past part of the task, when a
human says "done" or "hand off", or when the agent is about to switch to unrelated work. Capture once
per meaningful change, not after every file edit.

The authoring skill is reactive: it triggers at the end of a work session, so install it in the
agent environment before that session begins. Capturing before any commit keeps the whole change in
the working tree, the simplest shape to hand off. If the skill is added only afterward and the change
is already committed, capture the landed range with `shore capture --base <rev>` instead of
recreating a worktree diff.

## The Capture Moment

`shore capture` freezes the current Git worktree diff from `HEAD` to the working tree,
including untracked files. That timing matters for the default capture: if the agent commits part of
the task first, a later worktree capture only sees the remaining uncommitted diff. The preferred
order is to finish the implementation, run the relevant checks, capture the revision while the full
change is still in the worktree, record the handoff facts, then stop. If the change is already
committed and the working tree is clean, capture the committed range instead with
`shore capture --base <commit-before-the-change>` (target defaults to `HEAD`) — never rewrite
history to manufacture a worktree diff.

Humans set up the loop by making the expectation explicit: when the agent reaches the end of a task,
it should run Shoreline before declaring the task complete. Agents execute the loop from inside the
repository that contains the change. Reviewers pick up the recorded revision afterward and make the
review call on their own track.

## What The Author Records

The authoring agent records observations and input requests on one author track chosen for that
handoff. Use the form `agent:<agent-name>-<id>`, where `<agent-name>` is the agent's own short name
such as `claude`, `codex`, or `cursor`, and `<id>` is a short run-unique tag. Prefer an issue or PR
number, use the branch's distinctive segment as a fallback, and use a short random tag if neither
exists. Keep the part after `agent:` lowercase, hyphenated, and around 15 characters or fewer.

Tracks are review lanes, not actor identity. The unique tag keeps the lane legible when more than one
agent run writes to the same `.shore/data/` store. Shoreline command output also records local Git and
producer provenance, but the track is the durable lane that names which agent run is writing.

## Agent actor identity

A track is a lane; your **actor id** is your durable identity across sessions and runs. Agents
write under an `actor:agent:<agent-name>` id, set once per run with the environment variable:

```bash
export SHORE_ACTOR_ID="actor:agent:${agent_name}"
```

`SHORE_ACTOR_ID` outranks the local Git identity for every CLI write path, with safe fall-through:
a malformed value is ignored rather than trusted, so it can never silently corrupt provenance. The
actor id carries **no run id** — run entropy stays in the track. Use **one canonical spelling** for
your agent name and always the same one (`claude-code`, never also `claude`): two spellings split
one agent's history across two identities. Keep it lowercase and hyphenated; `/` inside the agent
segment is reserved.

Who an agent acts on behalf of is resolved at read time from the checked-in `.shore/delegates.json`
map, documented in [storage-model.md](./storage-model.md) and decided in
[ADR-0010](./adr/adr-0010-actor-identity-and-delegation.md). A human (or the agent proposing the edit)
creates that binding with `shore identity enroll <agent> --principal <human>` — possession-style like
key enrollment: it stages the working-tree `.shore/delegates.json` edit and the commit is the
authorization (see [cli-reference.md](./cli-reference.md)). Identity is reported, never the basis
of a binding decision. Agent events written before adopting an `actor:agent:` id carry the human's
git-email id and stay exactly that — the `agent:*` track name is a heuristic, never re-attribution.

Signing is automatic for agents. On the first write under an `actor:agent:*` id, Shoreline silently
generates a passphrase-less per-machine key in the user-level key home, signs the event, and prints
a one-line stderr notice with the agent's `did:key` and `shore keys enroll` so a human can add the
agent to the committed `.shore/allowed-signers.json` allow-list. The agent proposes the working-tree
edit; the human's commit is the authorization (possession-style, like delegation). Until enrolled, a
signed event verifies `untrusted_key` — tamper-evident and strictly better than unsigned; once
enrolled it verifies `valid` and binds. Signing never gates: if no key can be made the write still
succeeds, unsigned, at exit 0. `SHORE_SIGNING=off` disables signing. (A **human** can instead reuse an
existing SSH key via `shore keys use-ssh` rather than `shore keys init`; agents still auto-keygen,
unchanged.) See [signing-ux.md](./signing-ux.md).

Observations explain what changed and why. They should call out the design choices, risk areas,
follow-up edges, and files or line ranges a reviewer should inspect first. A useful observation is
specific enough that someone can understand the change without scrolling back through the agent's
transcript.

Validation evidence records concrete check results for the captured revision: tests, lint, builds,
format checks, or equivalent verification commands the agent actually ran. Validation evidence is
advisory review context only. It does not accept, reject, merge, block, or replace the reviewer's
assessment, and it targets the whole captured revision rather than a file or range.

Input requests are for genuine open questions. Use them when the agent could not responsibly decide
something on its own: ambiguous requirements, a risky choice that needs approval, or a manual decision
that should happen before landing. Use `--mode operative` when the answer should block landing, and
`--mode advisory` when the request is durable context for the reviewer but does not need to pause the
workflow.

The authoring agent must not add a `shore assessment`. Assessments are the reviewer's current
call, such as `accepted` or `needs-changes`. If the author records its own assessment, it turns the
handoff into self-grading and pollutes the review surface the reviewer owns. This author/reviewer
split is a review-**policy** convention, not a substrate restriction: the event log is symmetric —
author and reviewer record facts through the same write surface, and nothing mechanically stops an
author from writing an assessment. The discipline is what keeps the handoff honest; the substrate
only records who wrote what.

## Author Loop

An agent-authored handoff looks like this:

```bash
git status --short
capture_file=$(mktemp)
shore capture | tee "$capture_file" | jq .
revision_id=$(jq -r '.revision.id' "$capture_file")
rm "$capture_file"
agent_name="<agent-name>"
run_id="<id>"
track="agent:${agent_name}-${run_id}"

shore observation add \
  --revision "$revision_id" \
  --track "$track" \
  --title "Parser keeps the existing whitespace contract" \
  --file src/parser.rs --start-line 84 --end-line 123 \
  --body "The parser now accepts the new token form while preserving the old whitespace path. The branch stays local to parsing so callers do not need a compatibility shim."

shore observation add \
  --revision "$revision_id" \
  --track "$track" \
  --title "Verification covered the changed parser and full suite" \
  --body "Ran the targeted parser test and the repository test suite after the final edit. No generated artifacts were changed."

shore validation add \
  --revision "$revision_id" \
  --track "$track" \
  --check-name "just check" \
  --status passed \
  --command "just check" \
  --exit-code 0 \
  --summary "Completed after the final edit. This covered commit checks, build, lint, and tests."

shore review input-request open \
  --revision "$revision_id" \
  --track "$track" \
  --title "Confirm whether the relaxed parser should be documented" \
  --reason manual-decision-required \
  --mode advisory \
  --body "The implementation accepts the new form, but I did not update user-facing docs because the prompt did not say whether this behavior should be advertised yet."

shore observation list --revision "$revision_id" --track "$track" --pretty
shore validation list --revision "$revision_id" --track "$track" --include-body --pretty
shore review input-request list --revision "$revision_id" --track "$track" --status open --pretty
```

The commands emit compact JSON by default, so piping capture output through `jq` is only for human
readability. The readback uses bounded list commands so the author can verify the observations and
open input requests without replaying the captured snapshot. `shore review show` remains the
full composite JSON view of a revision; it includes the complete captured snapshot, can be large
for real changes, and is meant for tooling or cases where the full snapshot is genuinely needed. The
write commands above pass the captured revision ID explicitly because write commands can infer a
revision only when exactly one current capture exists. If `jq` is not available, copy
`revision.id` from `shore capture` output and use it in place of `$revision_id`.

## What A Good Handoff Looks Like

A good handoff is short, concrete, and review-oriented. It names the files that matter, the reason
the shape of the change is acceptable, what validation actually ran, and where the author is least
certain. Concrete check results should be recorded with `shore validation add`; observations
should explain the surrounding decision, risk, or interpretation. It does not repeat every diff hunk,
and it does not bury the reviewer in generic status updates.

Prefer anchored observations when the fact belongs to a file or line range. Use review-wide
observations for cross-cutting decisions, verification notes, and risks that do not live in one file.
Open an input request only when someone else needs to answer something; ordinary follow-up ideas are
better as observations unless they require a decision.

After the author stops, a reviewer can read the handoff with:

```bash
shore observation list --revision <revision-id> --track <track> --include-body --pretty
shore validation list --revision <revision-id> --track <track> --include-body --pretty
shore review input-request list --revision <revision-id> --track <track> --status open \
  --include-body --pretty
```

The reviewer then records their own facts on their own track. For example:

```bash
shore assessment add \
  --revision <revision-id> \
  --track human:kevin \
  --assessment needs-clarification \
  --summary "The change is understandable, but the open documentation question needs an answer before landing."
```

That separation keeps the author's explanation and the reviewer's call distinct.

## Reviewer Loop

The `shoreline-reviewer` skill is the reviewer-side pair to the author handoff. It starts from an
existing revision, reads the author's handoff with bounded list commands, reviews the change
independently, records reviewer observations on a separate reviewer track, responds to any open
operative input requests, opens advisory input requests for author decisions, and records exactly one
assessment.

The reviewer uses the author's observations as navigation context, not as proof. It should re-read
the diff and rerun the project's relevant checks rather than trusting the author's verification
claim. It should also read the author's validation evidence as context, then rerun relevant checks
and record reviewer-run checks as validation evidence on the reviewer track. The reviewer compares
the captured revision with the live checkout it reviewed; if the revision snapshot and live
commit diverge, the reviewer records that divergence as an observation.

Reviewer readback uses the same bounded surfaces as the author handoff:

```bash
shore observation list --revision <revision-id> --track <author-track> \
  --include-body --pretty
shore validation list --revision <revision-id> --track <author-track> \
  --include-body --pretty
shore review input-request list --revision <revision-id> --track <author-track> \
  --status open --include-body --pretty
```

When the reviewer runs checks, it records those concrete results separately from the assessment:

```bash
shore validation add \
  --revision <revision-id> \
  --track <reviewer-track> \
  --check-name "just check" \
  --status passed \
  --command "just check" \
  --exit-code 0 \
  --summary "Reproduced the repository check from the reviewed checkout."
```

Reviewer follow-ups that need an author decision should be advisory input requests, not plain
observations:

```bash
shore review input-request open \
  --revision <revision-id> \
  --track <reviewer-track> \
  --title "Decide whether to split the parser cleanup" \
  --reason manual-decision-required \
  --mode advisory \
  --body "The implementation is acceptable as written, but the cleanup decision should be recorded by the author."
```

The reviewer records the review call once:

```bash
shore assessment add \
  --revision <revision-id> \
  --track <reviewer-track> \
  --assessment accepted-with-follow-up \
  --summary "The change is acceptable. I opened an advisory follow-up for the author to decide."
```

The reviewer should not write to the author's track. The author should not record this assessment.

## Author Response Loop

The `shoreline-author-response` skill closes the loop when the original author picks up the
reviewer's pass. It attaches to the existing revision with `--revision`; it does not run
`shore capture` again, and it does not add or replace assessments.

The author reads the reviewer track with bounded commands:

```bash
shore observation list --revision <revision-id> --track <reviewer-track> \
  --include-body --pretty
shore validation list --revision <revision-id> --track <reviewer-track> \
  --include-body --pretty
shore assessment show --revision <revision-id> --track <reviewer-track> \
  --include-summary --pretty
shore review input-request list --revision <revision-id> --track <reviewer-track> \
  --status open --include-body --pretty
```

If the assessment is `needs-changes` or `needs-clarification`, or if an open operative input request
requires an author action, the response is actionable. The author makes the narrow requested change,
runs the relevant checks, responds to any resolved input requests, and records author response
observations on the author track. If an operative request is still a genuine blocker, the author
leaves it open and records what remains unresolved rather than forcing a response.

The original revision snapshot remains frozen. If the author makes response edits and reruns
checks against live code that no longer matches that snapshot, those rerun checks belong in author
response observations unless the author can prove the captured revision still matches the checkout.

If the assessment is `accepted` or `accepted-with-follow-up` and the only open items are advisory or
non-blocking, the author triages them without manufacturing work. Reviewer follow-ups that ask for an
author decision should be answered structurally:

```bash
shore review input-request respond <input-request-id> \
  --outcome approved \
  --reason "tracking this as a separate follow-up because changing it here would widen the reviewed unit"
```

The author then records the response on the author track, referencing the reviewer observation,
input request, and assessment IDs in the body. The reviewer remains responsible for any later
assessment change.

## Landing the change

Capture happens before any commit (see [The Capture Moment](#the-capture-moment)). Landing is the
separate, later step where the reviewed change is actually committed. It happens after the reviewer
reaches an accepting verdict and after any author response, and it belongs to the author, not the
reviewer: the reviewer records its one assessment and stands down.

Record the landed commit as a structural association on the author track — the first-class "the work
landed as commit X" record (a `RevisionCommitAssociated` edge, ADR-0014):

```bash
shore review association associate-commit \
  --revision <revision-id> \
  --track <author-track> \
  --commit <landed-sha>
```

This is git-resolved and machine-readable: the revision then reports `anchored` with merged/live
reachability in `shore review show`, and `shore review revisions --ref <branch>` /
`shore history --ref <branch>` can find the landed work by branch. A worktree-captured revision is
born floating, so this is the event it was waiting for; a commit-range-captured revision is already
anchored at its captured target, so associate that same commit on a rebase or fast-forward — or,
when a squash or merge produced a new commit, expect a `divergent_commit_association` diagnostic and
keep or `withdraw-commit` the edge you do not want. Optionally add a human-readable companion with
`shore observation add --tag state-change:landed --title "landed as <sha>"`. Do not run
`shore capture` again for the landing, and do not add or change the assessment — the resulting
commit is an author fact, not a review call.

When several captures are still current — re-captures stack, and a stale or superseded one is retired
with `shore capture --supersedes <revision>` — pin the landing to the
revision that was actually reviewed and accepted by passing `--revision` explicitly, seeding it on the
accepted revision when it is the current head of a supersession thread. Sibling captures
remain, but routine list/history/exact/thread-scoped reads no longer emit an ambient
`ambiguous_current_revision` diagnostic just because multiple captures exist. Note that one commit
can correspond to more than one accepted revision (for example a sub-task capture nested inside a phase
capture); the landing observation annotates only the revision or thread head you pin, not the
relationship.
