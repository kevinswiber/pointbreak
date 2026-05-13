# Disposition Model

## Status

V1 has a local durable disposition ledger. Shore can append `review_disposition_recorded` events and
read them through `shore review disposition show`.

Native disposition projection into `shore dump`, `shore show`, history/export commands, and TUI
surfaces are deferred.

## Goal

A disposition records the review outcome for a captured ReviewUnit without collapsing the rest of
the ledger. It is an append-only fact with a bounded vocabulary, explicit relationships, and a
replay-derived current view.

## Values

V1 disposition values are:

- `accepted`
- `accepted_with_follow_up`
- `needs_changes`
- `needs_clarification`
- `overridden`
- `deferred`
- `split_out`
- `superseded`

CLI input uses kebab-case values such as `accepted-with-follow-up`; JSON output uses snake_case
values such as `accepted_with_follow_up`.

`overridden` requires a non-empty summary and at least one override reference. `superseded` is a
recorded value only; it does not imply a replacement unless the caller also passes `--replaces`.

## Targets

Dispositions target the captured ReviewUnit by default. They may also target:

- a file in the captured snapshot
- a range in a captured file
- a native observation in the same ReviewUnit
- a native intervention in the same ReviewUnit
- a native disposition in the same ReviewUnit

Relationship targets must exist in the same ReviewUnit. Shore rejects unknown replacement,
observation, intervention, and override references.

## Relationships

`--replaces <disposition-id>` is the only V1 relationship that removes an older disposition from the
current set.

`--overrides-observation`, `--overrides-intervention`, and `--overrides-disposition` record
authority or invalidation metadata. They do not replace a disposition by themselves. If a disposition
both overrides and replaces an earlier disposition, name the earlier disposition in both
`--overrides-disposition` and `--replaces`.

`--related-observation` and `--related-intervention` record evidence links. They do not mutate
observations and do not close interventions. Intervention lifecycle remains explicit through
`shore review intervention resolve`.

## Projection

`shore review disposition show` replays `.shore/events/`; it does not treat `state.json` as
authority.

The current view is:

- `none` when there are no unreplaced dispositions
- `resolved` when exactly one unreplaced disposition remains
- `ambiguous` when multiple unreplaced dispositions remain

Readers do not choose a timestamp winner. Use a new disposition with `--replaces` to resolve
ambiguity.

Repeated writes with the same logical `dispositionId` create multiple durable events only when the
caller varies the event idempotency key. The read projection collapses those duplicate semantic
events to one row and emits `duplicate_semantic_disposition_event`.

Summaries are omitted by default. `--include-summary` hydrates inline summaries or internal
`shore.note-body` artifacts. Artifact paths remain storage details and are not part of command
output.

## Non-Goals

V1 does not add:

- PR hosting or GitHub review comments
- automatic intervention closure
- daemon/watch transport
- cloud sync
- TUI disposition rendering
- native disposition projection into `shore dump` or `shore show`
- migration shims or aliases for removed pre-release decision commands
