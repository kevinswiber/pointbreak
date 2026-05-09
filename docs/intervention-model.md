# Intervention Model

## Status

This is architecture guidance, not current implementation scope.

Plan 0005 should not add intervention commands, prompts, queues, or UI flows. Its durable event
envelope, target references, state projection, and polling shape should leave room for intervention
events later.

## Goal

Shore needs a durable way to represent moments where normal review flow should pause, ask for a
decision, surface an escalation, or record that outside input changed the path forward.

Do not call this "human-in-the-loop" in the core model. The actor may be a human, reviewer, monitor
process, automated tool, cloud worker, or another Shore client. The model should describe the
workflow fact, not assume who resolves it.

## Core Terms

- **Intervention:** a durable request for attention, decision, override, or acknowledgement.
- **Blocking intervention:** an intervention that should stop a cooperative client before it
  continues a workflow step such as publishing, applying notes, acknowledging review, pushing, or
  mutating state.
- **Advisory intervention:** an intervention that should be visible but does not block progress.
- **Resolution:** the durable answer to an intervention, such as approved, rejected, dismissed,
  superseded, or resolved by a later event.
- **Cancellation:** withdrawal of an intervention request before a decision is made, usually because
  the request was mistaken, superseded, or no longer relevant.
- **Escalation:** a higher-priority intervention, usually created because the original workflow
  cannot decide safely from local state.

## Architectural Principles

Interventions are durable events, not UI prompts. A TUI may render an intervention as a modal, a CLI
may print it to stderr, and a monitor may react in real time, but the durable model should be the
source of truth.

Interruption should be cooperative. Shore does not need to preempt another process mid-operation.
Clients should check durable state at safe boundaries and decide whether unresolved blocking
interventions require them to pause.

The model must support both real-time and polling clients:

- A monitor-style client can subscribe to stdout/stderr, filesystem notifications, or a future event
  stream and respond quickly.
- A turn-boundary client can poll at start, stop, or before risky operations.
- A cloud client can poll or receive backend-specific notifications without changing the event
  vocabulary.

The same durable event model should work for all three.

## Event Sketch

Future event types should be able to fit the same event envelope used for review/session state:

```text
intervention_requested
intervention_resolved
intervention_cancelled
intervention_escalated
```

`intervention_escalated` should target an existing intervention and change its routing or urgency in
the derived projection. It should not create a second intervention. If a separate decision is needed,
create another `intervention_requested` event with an explicit relationship to the first.

`intervention_cancelled` means the request was withdrawn without a decision. `intervention_resolved`
means the request was decided, with an outcome such as approved, rejected, dismissed, or resolved by a
later event.

Each event should carry:

- a stable intervention ID
- target reference: `ReviewId`, `WorkUnitId`, `RevisionId`, `ReviewArtifactId`, or `EventId`
- optional urgency label for display and triage
- blocking/advisory flag
- reason code
- short title
- body or structured details
- requesting actor or writer provenance
- resolving actor or writer provenance, for resolution events
- timestamps using the same UTC timestamp policy as other Shore events
- idempotency key

Reason codes should stay workflow-oriented, not actor-oriented. Useful starting categories:

- `ambiguous_state`
- `unsafe_action`
- `stale_revision`
- `failed_gate`
- `external_side_effect`
- `conflicting_event`
- `missing_permission`
- `manual_decision_required`

The `blocking` flag is the control-flow signal. Urgency is advisory; it should not decide whether a
client may continue.

Interventions should not expire automatically. Clearing an unresolved intervention requires an
explicit `intervention_resolved` or `intervention_cancelled` event. A future `expiresAt` field can be
added if a concrete workflow needs advisory expiry, but it should not silently unblock a client.

Every blocking intervention must have a defined exit event or escalation policy. That does not mean
blocking states should clear themselves on a timer. For review workflows, the external decision is
often the point. The requirement is that Shore can represent how the state ends: resolved,
cancelled, superseded, escalated, or explicitly abandoned.

Resolution events should preserve the audit trail even when the target is no longer live. For
example, an `intervention_resolved` event targeting a closed work unit should still be recorded, but
any resume or apply action derived from it should be a no-op. Distinguish "the event happened" from
"the action still applies."

## Derived State

Derived state should expose unresolved interventions in a way that every frontend can consume.

Minimum future projection:

```text
unresolved_interventions
unresolved_blocking_interventions
latest_intervention_event_id
```

A client should be able to ask:

- Are there unresolved blocking interventions for this work unit?
- Are there unresolved blocking interventions targeting the current revision?
- Has anything changed since my last event cursor?
- Which event or artifact caused the intervention?

That implies Shore should eventually expose an `events_since(cursor)` style API or equivalent
cursor-based projection. Plan 0005 does not need to implement that API, but it should not choose a
storage shape that makes it awkward.

## Design Constraints For Plan 0005

Plan 0005 should preserve these future requirements:

- Use generic target references in event payloads rather than hard-coded single-target fields.
- Keep event IDs and idempotency keys stable enough for polling clients.
- Keep derived `state` rebuildable from durable events.
- Do not make terminal UI state the only place an interruption can live.
- Do not assume intervention actors are humans.
- Do not assume intervention delivery is real-time.
- Do not assume local filesystem notification is available.
- Do not require async storage yet, but avoid event semantics that depend on POSIX-only behavior such
  as atomic rename; remote backends may need conditional create, versioned writes, or transactions.
- Re-read target state before applying a resolution or resume action; stale targets should preserve
  the event but suppress the action.

Intervention transport is independent of review-exchange transport. An intervention is not a review
artifact, verdict, or review note. A future adapter may export or import intervention facts, but the
core model should keep them separate.

## Non-Goals

This document does not require:

- a prompt system
- a daemon
- a notification service
- a lock or lease protocol
- a cloud backend
- a TUI modal
- note mutation
- an acknowledgement command

Those may become useful later, but the architectural requirement is narrower: Shore's durable model
should be interruptible at safe workflow boundaries.
