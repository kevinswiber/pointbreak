---
name: shoreline-author-response
description: Use when the coding agent that authored a change should pick up a Shoreline reviewer pass on its existing ReviewUnit. Read the reviewer's observations, assessment, and input requests with bounded commands, classify the verdict, respond to advisory requests with shore review input-request respond, make changes only when the review is actionable, record author response observations, never add an assessment, and never recapture.
---

# Shoreline Author Review Response

You are the agent that authored the change. A reviewer has recorded a Shoreline review on the
existing ReviewUnit, and you are picking that review back up. Your job is to triage the verdict,
respond through structured input-request channels, make required changes when the review asks for
them, and record your response on your author track.

Do not run `shore review assessment add`. The reviewer owns the assessment. Do not run
`shore review capture`; this response attaches to the existing ReviewUnit with `--review-unit`.

Do not run `shore review unit show --pretty` as a readback surface. Use bounded list commands for the
reviewer's observations, input requests, and assessment.

## Workflow at a glance

```text
1. Identify the existing ReviewUnit, reviewer track, and your author track.
2. Read the reviewer's observations, assessment, and input requests with bounded commands.
3. Classify the verdict as actionable or non-blocking triage.
4. Respond to reviewer advisory input requests with input-request respond.
5. Handle open operative input requests only when they are genuinely answerable.
6. For needs-changes, make the requested code change and rerun relevant checks.
7. Record author response observations on your author track.
8. Do not add or change the assessment, and do not capture a new ReviewUnit.
9. Read back the response with bounded commands, then stop.
```

Do not manufacture work. An accepted review with only advisory or non-blocking follow-ups usually
needs a decision and a durable response, not a new code change in an already-reviewed unit.

## Read the reviewer pass

Set the ReviewUnit ID, reviewer track, and your existing author track. If the ReviewUnit ID is not
known, list captured units first:

```bash
shore review unit list --pretty
review_unit_id="<review-unit-id>"
reviewer_track="<reviewer-track>"
author_track="<author-track>"
```

Read the reviewer's durable review facts:

```bash
shore review observation list \
  --review-unit "$review_unit_id" \
  --track "$reviewer_track" \
  --include-body --pretty

shore review assessment show \
  --review-unit "$review_unit_id" \
  --track "$reviewer_track" \
  --include-summary --pretty

shore review input-request list \
  --review-unit "$review_unit_id" \
  --track "$reviewer_track" \
  --status open \
  --include-body --pretty
```

Use the assessment and open requests to decide what kind of response is needed.

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
  --review-unit "$review_unit_id" \
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
  --review-unit "$review_unit_id" \
  --track "$reviewer_track" \
  --mode advisory \
  --status open \
  --include-body --pretty

shore review input-request respond <input-request-id> \
  --outcome approved \
  --reason "agreed; tracking the parser cleanup as a separate follow-up because changing it here would widen the reviewed change"
```

Use `approved`, `rejected`, `dismissed`, `superseded`, or `abandoned` for the outcome. The response
should state the author decision and why it is appropriate for this ReviewUnit.

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

The ReviewUnit snapshot remains the original captured snapshot. Do not run a fresh capture as part
of this response. When your live code has moved beyond the snapshot, say so in the author response
observation and reference the reviewer IDs you are addressing.

## Record author response observations

Record responses on your author track. Reference the reviewer observation IDs, input request IDs,
and assessment ID in the body so a reader can connect the response to the review.

```bash
shore review observation add \
  --review-unit "$review_unit_id" \
  --track "$author_track" \
  --title "Response to reviewer parser follow-up" \
  --body "Responded to reviewer advisory request <input-request-id> from assessment <assessment-id>: accepted the follow-up but kept it out of this ReviewUnit because the current assessment is accepted-with-follow-up and the cleanup would widen the reviewed change."

shore review observation add \
  --review-unit "$review_unit_id" \
  --track "$author_track" \
  --title "Addressed reviewer observation <observation-id>" \
  --file src/parser.rs --start-line 84 --end-line 123 \
  --body "Addressed reviewer observation <observation-id> from assessment <assessment-id> by tightening the parser branch and rerunning the targeted parser test plus the full suite."
```

Do not add, replace, or update an assessment. If the reviewer needs to revise the review call after
your response, the reviewer records that later on the reviewer track.

## Record the landing commit (optional)

If you commit the reviewed change after the review reaches an accepting verdict, record the
resulting commit as an observation on your author track. The commit is an author fact, not a review
call: it never goes on the reviewer track and never becomes an assessment.

```bash
shore review observation add \
  --review-unit "$review_unit_id" \
  --track "$author_track" \
  --tag state-change:landed \
  --title "landed as <sha>" \
  --body "ReviewUnit $review_unit_id (accepted by $reviewer_track) committed as <full-sha> on <branch>."
```

This is an interim convention pending a first-class landed-commit record
(kevinswiber/shoreline#103; docs in kevinswiber/shoreline#104). Do not run `shore review capture`
for the landing, and do not add or change the assessment.

If more than one ReviewUnit is current, pin the landing to the one that was actually reviewed and
accepted with `--review-unit`, or use `--lineage` when the accepted ReviewUnit is the current head of
a recorded lineage. Sibling captures stay current, but routine list/history/exact/lineage-scoped
reads no longer emit an ambient `ambiguous_current_review_unit` diagnostic just because multiple
captures exist. Shoreline still has no way to retire a stale capture (kevinswiber/shoreline#106).

## Read back and stand down

Verify the author response with bounded read commands:

```bash
shore review observation list \
  --review-unit "$review_unit_id" \
  --track "$author_track" \
  --include-body --pretty

shore review input-request list \
  --review-unit "$review_unit_id" \
  --track "$reviewer_track" \
  --status all \
  --include-body --pretty

shore review assessment show \
  --review-unit "$review_unit_id" \
  --track "$reviewer_track" \
  --include-summary --pretty
```

Then stop. Report the ReviewUnit ID, author track, reviewer track, what you changed or deliberately
did not change, and which input requests you responded to. Leave the assessment untouched.

## Common errors

- **Adding an assessment as the author.** The author never assesses. Only the reviewer records the
  review call.
- **Recapturing the ReviewUnit.** Attach to the existing ReviewUnit with `--review-unit`; do not run
  `shore review capture` for the response leg.
- **Using full ReviewUnit show for readback.** Use bounded observation, input-request, and
  assessment read commands. Do not use `shore review unit show --pretty` for this response loop.
- **Manufacturing work after an accepted review.** Accepted follow-ups often need triage, not a new
  code change.
- **Answering advisory requests only in prose.** Use `shore review input-request respond` so the
  request has a structured response.
- **Closing operative requests mechanically.** Respond only when the request is genuinely answered;
  otherwise leave it open and record what is still blocked.
- **Writing to the reviewer track.** The response observations belong on the author's track.
- **Recording the landing commit on the reviewer track or as an assessment.** The landed-commit
  fact is an author observation; the reviewer owns the assessment.
- **Pinning the landing to the wrong unit when captures are ambiguous.** With multiple current
  ReviewUnits, pass the exact accepted unit with `--review-unit`, or pass `--lineage` only when that
  lineage's current head is the accepted ReviewUnit.
