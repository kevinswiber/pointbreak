# Input Request Model

## Status

V1 has a local durable input-request ledger. Shore can record `input_request_opened` events,
append `input_request_responded` events, and expose polling read surfaces through
`shore review input-request list` and `shore review input-request fetch`.

This document describes the model around that V1 surface. Prompt delivery, watch mode, daemon
behavior, notification transport, UI prompts, and automatic cancellation are deferred.

## Goal

Shore needs a durable way to represent moments where normal review flow needs input from another
actor: a decision, an answer, an approval, a clarification, or an explicit response that changes how
the work proceeds.

Do not call this "human-in-the-loop" in the core model. The actor may be a human, reviewer, monitor
process, automated tool, cloud worker, or another Shore client. The model describes the workflow
fact, not who resolves it.

## Core Terms

- **Input request:** a durable request for another actor's input.
- **Operative request:** a request whose envelope assertion mode is `operative`. Cooperative
  clients may treat it as binding under their explicit workflow policy.
- **Advisory request:** a request that should be visible but does not imply that a cooperative
  client must pause.
- **Response:** the durable answer to an input request, such as approved, rejected, dismissed,
  superseded, or abandoned.

## Event Model

Input request events use the same event envelope as other review/session state:

```text
input_request_opened
input_request_responded
```

`input_request_opened` records the durable request. The request has a stable `inputRequestId`, a
target reference, a required track, a public request mode derived from the event envelope's
`assertionMode` (`operative` or `advisory`), a short title, an optional body, and a structured
`reasonCode`.

`input_request_responded` records a durable answer. The response has a stable
`inputRequestResponseId`, targets the input request, and carries an `outcome` such as `approved`,
`rejected`, `dismissed`, `superseded`, or `abandoned`. Response `outcome` is intentionally separate
from request `reasonCode`: one describes why the input was requested, the other describes how the
request ended.

Future event types may represent explicit cancellation or escalation. V1 expresses
cancellation-like closures through response outcomes such as `dismissed`, `superseded`, or
`abandoned`, and does not model escalation as a separate lifecycle event.

Response events keep the request event's review unit, revision, snapshot, and track context. That
anchors the decision to the captured material that caused the input request, not to whatever worktree
state happens to exist when the input request is answered.

Multiple different response events are preserved as append-only facts. Current V1 read surfaces
report that state as `ambiguous` rather than choosing a timestamp winner.

Duplicate events with the same semantic ID are different from multiple responses. If a request is
written more than once with the same `inputRequestId`, `list` and `fetch` return one input request
and include a duplicate semantic diagnostic. If a response is written more than once with the same
`inputRequestResponseId`, `fetch` returns one response and keeps the input request `responded`.
Only distinct response IDs make an input request `ambiguous`.

Input requests do not expire automatically. Clearing an open input request requires an explicit
`input_request_responded` event. A future expiry field can be added if a concrete workflow needs
advisory expiry, but it should not silently unblock a client.

## Commands And Derived State

The command surface is:

```bash
shore review input-request open --track human:kevin --title "Need approval" \
  --reason manual-decision-required [--mode operative|advisory]
shore review input-request list [--status open|responded|ambiguous|all]
shore review input-request fetch <input-request-id> [--include-body]
shore review input-request respond <input-request-id> --outcome approved [--reason "approved"]
```

The V1 read surface is polling-oriented. `list` and `fetch` replay `.shore/events/`; they do not
depend on `state.json` as authority. Bodies and response reasons may use internal
`shore.note-body` artifacts, but command output does not expose artifact paths.

`list` and `fetch` project semantic IDs, not raw event count. `idempotencyKey` decides whether a
write is the same event-file retry; `inputRequestId` and `inputRequestResponseId` decide whether
read output represents one logical request or response. Duplicate semantic IDs are preserved in
storage and reported through diagnostics rather than silently hidden.

Bounded `state.json` exposes only summary counters:

```text
inputRequestCount
openInputRequestCount
openOperativeInputRequestCount
```

The authoritative store is the `.shore/events/` event log plus any body or snapshot artifacts under
`.shore/artifacts/`. `state.json`, command-output views, and future read indexes are rebuildable
projections derived from that durable storage.

## Design Constraints For Local Durable State

The local durable-state model should preserve these requirements:

- Use generic target references in event payloads rather than hard-coded single-target fields.
- Keep event IDs and idempotency keys stable enough for polling clients.
- Keep derived state rebuildable from durable events.
- Do not make terminal UI state the only place an input request can live.
- Do not assume input-request actors are humans.
- Do not assume input-request delivery is real-time.
- Do not assume local filesystem notification is available.
- Re-read target state before applying a response-derived action; stale targets should preserve the
  event but suppress the action.

Input-request transport is independent of review-exchange transport. An input request is not a
review artifact, verdict, or review note. A future adapter may export or import input-request facts,
but the core model should keep them separate.

Native assessments may relate to input requests through `--related-input-request`, but that
relationship is evidence, not lifecycle. An assessment does not close an input request. Use
`shore review input-request respond` to append the explicit closure event.

## Non-Goals

This document does not require:

- a prompt system
- a daemon
- a notification service
- a lock or lease protocol
- a cloud backend
- a TUI modal
- note mutation
- automatic expiry
- explicit cancellation/escalation event types

Those may become useful later, but the architectural requirement is narrower: Shore's durable model
should request input at safe workflow boundaries and record the response that closes the request.

## Legacy Intervention Events

Earlier development versions of Shore wrote intervention events and exposed a
`shore review intervention` command family. Current Shore uses input request events and
`shore review input-request` instead. Because Shore has not released this storage contract, the
supported migration is to discard the old local `.shore/` directory and recapture the review.
