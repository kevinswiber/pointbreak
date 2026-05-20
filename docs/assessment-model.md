# Assessment Model

Shore records reviewer decisions as `review_assessment_recorded` events. An assessment is the
current review call for a captured ReviewUnit, file, range, observation, intervention, or earlier
assessment.

The assessment values are deliberately narrow:

- `accepted`
- `accepted_with_follow_up`
- `needs_changes`
- `needs_clarification`

State-change outcomes such as deferred, split-out, overridden, and superseded are not assessment
values. They are recorded as review observations with `state-change:*` tags so they stay available
as evidence without changing the current-assessment projection.

## CLI surface

Use `shore review assessment add` to record a durable assessment:

```bash
shore review assessment add \
  --track human:kevin \
  --assessment accepted \
  --summary "looks good, ship it"
```

The command targets the selected ReviewUnit by default. It can also target a captured file or range,
or a native observation, intervention, or assessment in the same ReviewUnit:

```bash
shore review assessment add --track human:kevin --assessment needs-changes --file src/lib.rs
shore review assessment add --track human:kevin --assessment needs-changes \
  --file src/lib.rs --start-line 42 --end-line 58
shore review assessment add --track human:kevin --assessment accepted \
  --observation <observation-id>
shore review assessment add --track human:kevin --assessment accepted \
  --intervention <intervention-id>
shore review assessment add --track human:kevin --assessment accepted-with-follow-up \
  --target-assessment <assessment-id>
```

Summaries may come from `--summary`, `--summary-file`, or `--summary-stdin`. Large summaries use the
same Shore-owned `shore.note-body` artifact path as other note-shaped bodies; command output keeps
artifact paths private.

`--replaces <assessment-id>` is the only relationship that removes an older assessment from the
current set. `--related-observation` and `--related-intervention` record evidence links only; they
do not mutate observations or close interventions.

Use `shore review assessment show` to read the current assessment projection:

```bash
shore review assessment show --pretty
shore review assessment show --all --include-summary
shore review assessment show --track human:kevin
```

`show` replays `.shore/events/`, reports `current.status` as `unassessed`, `resolved`, or
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
- `relatedInterventionIds`

The event envelope owns writer provenance, track, review-unit identity, revision identity, snapshot
identity, and idempotency.

## Legacy disposition events

Earlier versions of Shore wrote `review_disposition_recorded` events with eight variants. Shore is
pre-V1 and does not preserve those events on disk. Loading a `.shore/events/` directory that
contains legacy disposition events fails with a typed error pointing at this section.

**Migration:** delete the local `.shore/` directory and re-capture any in-progress reviews. There is
no automatic migration tool.
