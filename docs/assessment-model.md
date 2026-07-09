# Assessment Model

Pointbreak records reviewer decisions as `review_assessment_recorded` events. An assessment is the
current review call for a captured revision, file, range, observation, input request, or earlier
assessment.

The assessment values are deliberately narrow. Durable event JSON and command JSON use `snake_case`:

- `accepted`
- `accepted_with_follow_up`
- `needs_changes`
- `needs_clarification`

CLI input and human-facing display use the matching `kebab-case` spelling: `accepted`,
`accepted-with-follow-up`, `needs-changes`, and `needs-clarification`.

State-change outcomes such as deferred, split-out, overridden, and superseded are not assessment
values. They are recorded as review observations with `state-change:*` tags so they stay available
as evidence without changing the current-assessment projection.

## CLI surface

Use `shore assessment add` to record a durable assessment:

```bash
shore assessment add \
  --track human:kevin \
  --assessment accepted \
  --summary "looks good, ship it"
```

The command targets the selected revision by default. It can also target a captured file or range,
or a native observation, input request, or assessment in the same revision:

```bash
shore assessment add --track human:kevin --assessment needs-changes --file src/lib.rs
shore assessment add --track human:kevin --assessment needs-changes \
  --file src/lib.rs --start-line 42 --end-line 58
shore assessment add --track human:kevin --assessment accepted \
  --observation <observation-id>
shore assessment add --track human:kevin --assessment accepted \
  --input-request <input-request-id>
shore assessment add --track human:kevin --assessment accepted-with-follow-up \
  --target-assessment <assessment-id>
```

Summaries may come from `--summary`, `--summary-file`, or `--summary-stdin`. Large summaries use the
same Pointbreak-owned `shore.note-body` artifact path as other note-shaped bodies; command output keeps
artifact paths private.

`--replaces <assessment-id>` is the only relationship that removes an older assessment from the
current set. It points the new assessment forward at the one it supersedes — the same forward-pointer
shape that a capture's `--supersedes` uses to evolve past an earlier revision — so an assessment is
never mutated in place; the replacement is a new fact. `--related-observation` and
`--related-input-request` record evidence links only; they do not mutate observations or close input
requests.

Use `shore assessment show` to read the current assessment projection:

```bash
shore assessment show --format json-pretty
shore assessment show --all --include-summary
shore assessment show --track human:kevin
```

`show` replays `.shore/data/events/`, reports `current.status` as `unassessed`, `resolved`, or
`ambiguous`, and defaults to current assessments only. `--all` includes replaced assessments.
Repeated writes with the same `assessmentId` are preserved but collapsed in read output with a
duplicate semantic diagnostic.

## Payload reference

`review_assessment_recorded` payloads carry:

- `assessmentId`
- `target`
- `assessment`
- optional `summary` or `summaryArtifactPath`
- optional `summaryByteSize`
- optional `summaryContentHash`
- `replacesAssessmentIds`
- `relatedObservationIds`
- `relatedInputRequestIds`

The event envelope owns writer provenance, track, idempotency, and the subject it targets — one
non-optional `target` that addresses the captured revision and, through it, the revision's
content-only object identity. The pre-reshape trio of flat review-unit, revision, and snapshot
identities folds into that single subject plus the distinct object sub-identity.

## Legacy disposition events

Earlier versions of Pointbreak wrote `review_disposition_recorded` events with eight variants. Pointbreak is
pre-V1 and does not preserve those events on disk. Loading a `.shore/data/events/` directory that
contains legacy disposition events fails with a typed error pointing at this section.

**Migration:** delete the local `.shore/data/` directory and re-capture any in-progress reviews. There is
no automatic migration tool.
