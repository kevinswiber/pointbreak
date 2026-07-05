---
name: shoreline-author-response
description: Use when the coding agent that authored a change should pick up a Shoreline reviewer pass on its existing revision. Read the reviewer's observations, validation evidence, assessment, and input requests with bounded commands, classify the verdict, respond to advisory requests with shore review input-request respond, make changes only when the review is actionable, record author response observations, never add an assessment, and never recapture.
---

# Shoreline Author Review Response

You are the agent that authored the change. A reviewer has recorded a Shoreline review on the
existing revision, and you are picking that review back up. Your job is to triage the verdict,
respond through structured input-request channels, make required changes when the review asks for
them, and record your response on your author track.

Do not run `shore review assessment add`. The reviewer owns the assessment. Do not run
`shore capture`; this response attaches to the existing revision with `--revision`.

Do not run `shore review show --pretty` as a readback surface. Use bounded list commands for the
reviewer's observations, input requests, and assessment.

## Workflow at a glance

```text
1. Identify the existing revision, reviewer track, and your author track.
2. Read the reviewer's observations, validation evidence, assessment, and input requests.
3. Classify the verdict as actionable or non-blocking triage.
4. Respond to reviewer advisory input requests with input-request respond.
5. Handle open operative input requests only when they are genuinely answerable.
6. For needs-changes, make the requested code change and rerun relevant checks.
7. Record author response observations on your author track.
8. Do not add or change the assessment, and do not capture a new revision.
9. Read back the response with bounded commands, then stop.
```

Do not manufacture work. An accepted review with only advisory or non-blocking follow-ups usually
needs a decision and a durable response, not a new code change in an already-reviewed unit.

## Read the reviewer pass

Set the revision ID, reviewer track, and your existing author track. If the revision ID is not
known, list captured units first:

```bash
shore review revisions --pretty
revision_id="<revision-id>"
reviewer_track="<reviewer-track>"
author_track="<author-track>"
agent_name="<agent-name>"
export SHORE_ACTOR_ID="actor:agent:${agent_name}"
```

Set `agent_name` to the **same canonical spelling** the original author run used (`claude-code`,
never also `claude`): the actor id is inherited per-agent, not per-run, so this response pass writes
under the same durable identity that authored the change. It carries no run id; `/` inside the agent
segment is reserved.

Because the author-response pass writes under the same `actor:agent:*` id as the author, it reuses
the same auto-generated key and enrollment — **signing is automatic** here too: your first write
under this id generates a passphrase-less per-machine key (or reuses the author run's) and signs the
event, printing a one-line notice with your `did:key` and `shore keys enroll` so a human can add you
to the committed allow-list. Signing never blocks a write — if no key can be made the write still
succeeds, unsigned. Set `SHORE_SIGNING=off` to disable signing. A human can instead reuse an existing
SSH key via `shore keys use-ssh` (agents still auto-keygen, unchanged).

Read the reviewer's durable review facts:

```bash
shore review observation list \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --include-body --pretty

shore review validation list \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --include-body --pretty

shore review assessment show \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --include-summary --pretty

shore review input-request list \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --status open \
  --include-body --pretty
```

Use the assessment, validation evidence, and open requests to decide what kind of response is needed.
Validation evidence is advisory context only; it does not replace the reviewer's assessment.

## Classify the verdict

Treat the review as actionable when either condition is true:

- The current assessment is `needs-changes` or `needs-clarification`.
- Any open operative input request requires an author action or decision.

Treat the review as non-blocking triage when the assessment is `accepted` or
`accepted-with-follow-up` and the only open items are advisory or clearly non-blocking. In that case,
respond to decision-seeking requests and record the response, but do not widen the reviewed change
unless the user asks you to.

Use a focused operative-request read when the classification is unclear:

```bash
shore review input-request list \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --mode operative \
  --status open \
  --include-body --pretty
```

## Respond to advisory requests

Reviewer follow-ups that need your decision should arrive as advisory input requests. Respond to
them with `shore review input-request respond`; do not answer only in an observation body.

```bash
shore review input-request list \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --mode advisory \
  --status open \
  --include-body --pretty

shore review input-request respond <input-request-id> \
  --outcome approved \
  --reason "agreed; tracking the parser cleanup as a separate follow-up because changing it here would widen the reviewed change"
```

Use `approved`, `rejected`, `dismissed`, `superseded`, or `abandoned` for the outcome. The response
should state the author decision and why it is appropriate for this revision.

## Handle open operative requests

An open operative input request is actionable, but it is not automatically yours to close. If the
reviewer opened it and your response now answers it, do the required work or make the required
decision, then respond with `shore review input-request respond` and a reason that names what
changed or what decision was made.

```bash
shore review input-request respond <input-request-id> \
  --outcome approved \
  --reason "answered by the parser cleanup change and verified with the targeted parser test"
```

If the operative request is still a genuine blocker, leave it open and record an author response
observation explaining what remains unresolved. If the operative request is one you originally
opened and the review made it obsolete, respond with the accurate outcome, usually `superseded` or
`abandoned`, and explain why it no longer needs a reviewer answer.

## Make changes only when actionable

For `needs-changes`, make the requested change in the working tree and rerun the relevant checks
before recording the response. Keep the edit scoped to the review finding. If the reviewer asked
for clarification, answer the question first; change code only when the answer requires it.

The revision snapshot remains the original captured snapshot. Do not run a fresh capture as part
of this response. When your live code has moved beyond the snapshot, say so in the author response
observation and reference the reviewer IDs you are addressing.

If you rerun checks after making response edits, be precise about what those checks validated. Checks
against live code that no longer matches the frozen revision should be recorded as author response
observations, not misleading validation evidence for the old snapshot.

## Record author response observations

Record responses on your author track. Reference the reviewer observation IDs, input request IDs,
and assessment ID in the body so a reader can connect the response to the review.

```bash
shore review observation add \
  --revision "$revision_id" \
  --track "$author_track" \
  --title "Response to reviewer parser follow-up" \
  --body "Responded to reviewer advisory request <input-request-id> from assessment <assessment-id>: accepted the follow-up but kept it out of this revision because the current assessment is accepted-with-follow-up and the cleanup would widen the reviewed change."

shore review observation add \
  --revision "$revision_id" \
  --track "$author_track" \
  --title "Addressed reviewer observation <observation-id>" \
  --file src/parser.rs --start-line 84 --end-line 123 \
  --body "Addressed reviewer observation <observation-id> from assessment <assessment-id> by tightening the parser branch and rerunning the targeted parser test plus the full suite."
```

Do not add, replace, or update an assessment. If the reviewer needs to revise the review call after
your response, the reviewer records that later on the reviewer track.

**Body content type — prefer Markdown for review content.** Author response `--body` and input-request
`--reason` default to plain text, but the inspector renders Markdown and the human reviews there, so
**author review content as Markdown** whenever it names code. Pass `--body-content-type text/markdown`
(or `--reason-content-type text/markdown`) and: backtick file paths, code/identifier/type references,
and function signatures; prefer short bullet lists over long paragraphs. Keep responses concise —
Markdown is for scannability, not license to write a document.

## Record the landing commit

After the review reaches an accepting verdict and you commit the reviewed change, record the landed
commit as a structural association on your author track — the first-class "the work landed as commit
X" record (a `RevisionCommitAssociated` edge, ADR-0014):

```bash
shore review association associate-commit \
  --revision "$revision_id" \
  --track "$author_track" \
  --commit <landed-sha>
```

Unlike a prose note, this association is git-resolved and machine-readable: the unit then reports
`anchored` with merged/live reachability in `shore review show`, and `shore review revisions
--ref <branch>` / `shore review history --ref <branch>` can find the landed work by branch. It is an
author fact — it never goes on the reviewer track, never becomes an assessment, and is never a
recapture (`shore capture` is not re-run for the landing).

Pick the landed commit deliberately:

- A **worktree-captured** unit is born floating (no commit OID); `associate-commit` is its canonical
  landing record — the event the capture left it waiting for.
- A **commit-range-captured** unit is already anchored at its captured target commit. If the work
  landed as that same commit (rebase or fast-forward), associate it — same OID, nothing diverges. If
  landing produced a **new** commit (a squash or merge commit), associating it adds a second current
  OID and the projection surfaces a `divergent_commit_association` diagnostic. That is correct — the
  reviewed content is at the captured target and the work also landed as the squash commit — so leave
  both, or `withdraw-commit` whichever edge you do not want to keep current.

Optionally also record a human-readable companion for readers scanning observations:

```bash
shore review observation add \
  --revision "$revision_id" \
  --track "$author_track" \
  --tag state-change:landed \
  --title "landed as <sha>" \
  --body "revision $revision_id (accepted by $reviewer_track) landed as <full-sha> on <branch>."
```

If more than one revision is current, pin the landing to the one that was actually reviewed and
accepted with `--revision`. `--revision <id>` is a head seed: passing a current head resolves it
exactly, while passing a superseded revision resolves its thread's current head (and a fork with
competing heads errors, listing them). Sibling captures stay current, but routine
list/history/exact reads no longer emit an ambient `ambiguous_current_revision` diagnostic just
because multiple captures exist. A stale or nested capture is retired by a later capture that
supersedes it (`shore capture --supersedes <revision>`); competing heads stay visible, so there
is no single "canonical" scalar to set.

## Read back and stand down

Verify the author response with bounded read commands:

```bash
shore review observation list \
  --revision "$revision_id" \
  --track "$author_track" \
  --include-body --pretty

shore review validation list \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --include-body --pretty

shore review input-request list \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --status all \
  --include-body --pretty

shore review assessment show \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --include-summary --pretty
```

Then stop. Report the revision ID, author track, reviewer track, what you changed or deliberately
did not change, and which input requests you responded to. Leave the assessment untouched.

## Common errors

- **Adding an assessment as the author.** The author never assesses. Only the reviewer records the
  review call.
- **Putting the run id in `SHORE_ACTOR_ID`.** The run/issue id belongs in `--track`
  (`agent:claude-code-234`), never the actor id. `SHORE_ACTOR_ID=actor:agent:claude-code-234` mints a
  new per-run identity whose auto-generated key is **not enrolled**, so events land
  signed-but-**untrusted** with no diagnostic while the durable, already-enrolled
  `actor:agent:claude-code` key goes unused. Always
  `export SHORE_ACTOR_ID="actor:agent:<agent-name>"` (the same canonical, run-free id the author run
  used); if the run id arrives as a skill arg, take it as the track only.
- **Recapturing the revision.** Attach to the existing revision with `--revision`; do not run
  `shore capture` for the response leg.
- **Using full revision show for readback.** Use bounded observation, input-request, and
  assessment read commands. Do not use `shore review show --pretty` for this response loop.
- **Ignoring reviewer validation evidence.** Read `shore review validation list` on the reviewer
  track before deciding what checks to rerun.
- **Attaching live-code checks to an old snapshot.** If response edits moved the checkout beyond the
  captured revision, record rerun checks as observations unless you can prove the snapshot matches.
- **Manufacturing work after an accepted review.** Accepted follow-ups often need triage, not a new
  code change.
- **Answering advisory requests only in prose.** Use `shore review input-request respond` so the
  request has a structured response.
- **Closing operative requests mechanically.** Respond only when the request is genuinely answered;
  otherwise leave it open and record what is still blocked.
- **Writing to the reviewer track.** The response observations belong on the author's track.
- **Recording the landing commit on the reviewer track or as an assessment.** The landed-commit
  fact is an author association (with an optional companion observation); the reviewer owns the
  assessment.
- **Pinning the landing to the wrong unit when captures are ambiguous.** With multiple current
  revisions, pass the accepted revision with `--revision` (a head resolves exactly; a superseded
  revision resolves its thread's current head).
