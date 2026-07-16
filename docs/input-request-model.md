# Input Request Model

## Status

V1 has a local durable input-request ledger. Pointbreak can record `input_request_opened` events,
append `input_request_responded` events, and expose polling read surfaces through
`pointbreak input-request list` and `pointbreak input-request show`.

This document describes the model around that V1 surface. Prompt delivery, watch mode, daemon
behavior, notification transport, UI prompts, and automatic cancellation are deferred.

## Goal

Pointbreak needs a durable way to represent moments where normal review flow needs input from another
actor: a decision, an answer, an approval, a clarification, or an explicit response that changes how
the work proceeds.

Do not call this "human-in-the-loop" in the core model. The actor may be a human, reviewer, monitor
process, automated tool, cloud worker, or another Pointbreak client. The model describes the workflow
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

A `reasonCode` of `insufficient_evidence` is one reason for a judgment request, not a separate
object: it types an ask for more evidence about the change. A debugger or CI run can satisfy it by
recording validation evidence on the same revision, and the request clears through the ordinary
`input_request_responded` path once the evidence is in hand.

`input_request_responded` records a durable answer. The response has a stable
`inputRequestResponseId`, targets the input request, and carries an `outcome` such as `approved`,
`rejected`, `dismissed`, `superseded`, or `abandoned`. Response `outcome` is intentionally separate
from request `reasonCode`: one describes why the input was requested, the other describes how the
request ended.

Future event types may represent explicit cancellation or escalation. V1 expresses
cancellation-like closures through response outcomes such as `dismissed`, `superseded`, or
`abandoned`, and does not model escalation as a separate lifecycle event.

Response events keep the request event's subject — the captured revision and its content-only object —
and its track context. That anchors the decision to the captured material that caused the input
request, not to whatever worktree state happens to exist when the input request is answered.

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
pointbreak input-request open --track human:kevin --title "Need approval" \
  --reason manual-decision-required [--mode operative|advisory]
pointbreak input-request list [--status open|responded|ambiguous|all]
pointbreak input-request show <input-request-id> [--include-body]
pointbreak input-request respond <input-request-id> --outcome approved [--reason "approved"]
```

The V1 read surface is polling-oriented. `list` and `show` replay events from the resolved store;
they do not depend on `state.json` as authority. Bodies and response reasons may use internal
`shore.note-body` artifacts, but command output does not expose artifact paths.

Open input requests also surface in `pointbreak attention list` — operative requests as primary
attention items, advisory requests as secondary — alongside the other review state that needs an
actor's judgment. That surface guides, never gates (ADR-0019): it never blocks a write.

`list` and `show` project semantic IDs, not raw event count. `idempotencyKey` decides whether a
write is the same event-file retry; `inputRequestId` and `inputRequestResponseId` decide whether
read output represents one logical request or response. Duplicate semantic IDs are preserved in
storage and reported through diagnostics rather than silently hidden.

Bounded `state.json` exposes only summary counters:

```text
inputRequestCount
openInputRequestCount
openOperativeInputRequestCount
```

The authoritative store is the event log plus any body or object artifacts in the resolved store.
`state.json`, command-output views, and future read indexes are rebuildable projections derived from
that durable storage. Use `pointbreak store paths` to discover the active common or ephemeral
location.

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
`pointbreak input-request respond` to append the explicit closure event.

A review follow-up that expects a *decision or disposition* ("fix now or track separately?") is an
**advisory input request**, not a plain observation: it can target the observation or range and carries
the `open → responded` lifecycle. Reserve plain observations for facts that need no response. To
acknowledge or dispose of another observation non-destructively — without opening a request and without
removing the target from the current set — use `responds_to` (`pointbreak observation add --responds-to
<observation-id>`; see [ADR-0026](adr/adr-0026-fact-to-fact-response-relationship.md)). Use `--supersedes`
only for a destructive correction that retires the target from the current set; do not use it to
acknowledge.

## Legacy Intervention Events

Earlier development versions of Pointbreak wrote intervention events and exposed a
a nested `review intervention` command family beneath `pointbreak`. Current Pointbreak uses input request events and
`pointbreak input-request` instead. Because Pointbreak has not released this storage contract, the
supported migration is to discard the old local `.pointbreak/data/` directory and recapture the review.
