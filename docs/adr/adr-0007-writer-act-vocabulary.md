# ADR-0007: Writer Act Vocabulary

**Status:** Accepted
**Date:** 2026-06-11
**See also:** [ADR-0003](./adr-0003-agent-resource-claims-advisory-first.md),
[ADR-0004](./adr-0004-event-signatures.md)

## Context

Every event envelope carries `writer.role`. Issue #98 reports that the field is easily misread as
a persona: a capturing agent that annotates its own ReviewUnit appears as `role: reviewer`. The
problem is larger than the issue describes. `WriterRole` has four variants — `Author`, `Reviewer`,
`User`, `Agent` (`src/session/event/writer.rs`) — and the two halves carry opposite kinds of fact:

- The review-domain half (`author` / `reviewer`) names an **act**. It is hardwired by which
  workflow builder produced the event: capture and note import stamp `author`; observations,
  assessments, input requests, and validation evidence stamp `reviewer`. No review-domain logic
  branches on it, and it is fully derivable from `eventType`, which is itself a signed
  `EventToBeSigned` field.
- The task-domain half (`user` / `agent`) names an **actor kind**. The claude_code adapter stamps
  it to record which conversation participant produced the source message
  (`src/session/adapter/claude_code/translate.rs`), and it is load-bearing:
  `src/session/projection/task.rs::response_writer_role_is_binding` treats only `role: user`
  input-request responses as binding for agent resumption.

Two properties make this a federation prerequisite rather than a documentation nit, informed by
cross-project federation research:

- `role` is part of the signed v1 `EventToBeSigned` view (ADR-0004) and appears in the landed
  golden vectors. Once signed events federate, any vocabulary change requires a `sigVersion: 2`
  verifier path maintained alongside v1 forever. Today the change is a mechanical break that the
  pre-adoption hard-break policy explicitly permits, with `sigVersion` staying 1.
- The binding predicate is a trust hole. A signature proves the key holder *claimed* `role: user`,
  not that a human user produced the event; ingest validates only the actor id's shape. A remote
  peer could mint an `input_request_responded` event with `role: user` and a valid signature and
  have its response treated as binding for agent resumption.

`role` never participates in idempotency keys or `eventId` derivation (`eventId` is the SHA-256 of
the idempotency key), so changing the field cannot disturb deduplication or event identity. The
blast radius is the envelope serialization, the signed to-be-signed view, the golden vectors, and
the producer call sites. The vector suite also has a coverage gap:
`tests/fixtures/event_signatures/mutation-cases.json` mutates every other signed envelope fact but
has no negative case for `role`, even though `role` is signed.

## Decision

Split the overloaded vocabulary and remove `role` from the event envelope and from
`EventToBeSigned`.

- **The review act is derived, not stored.** Read surfaces that want an act label
  (captured, annotated, assessed) derive it from the event's `eventType` at projection or display
  time. Because the act is fully determined by `eventType`, which is already signed, dropping the
  field yields a strictly smaller signed surface at the same migration cost as a rename, and the
  misleading `role: reviewer` on a self-annotating capturer disappears entirely.
- **The source speaker moves into the adapter payload.** `user` / `agent` is a fact about the
  source conversation message, not about the durable-event writer; the writer of those events is
  the shore adapter. The claude_code adapter records the fact as a `sourceSpeaker` payload field
  (`"user"` or `"agent"`) on the task-domain payloads it owns. Payload facts are bound by
  `payloadHash`, so the relocated fact remains covered by the v1 signature.
- **Persona is derived at projection time, never stored.** "Is this event's actor the unit's
  capturer?" is computed by comparing the event's verified `actorId` / effective signer against
  the capture event's. This works identically after federation and respects ADR-0004's
  non-aliasing rule, because the comparison happens on whichever concrete identities the events
  carry. A stored persona would duplicate state derivable from the capture event and mint a new
  stored-versus-derived conflict class.
- **Resumption binding re-bases on verified identity.** `response_writer_role_is_binding` is
  replaced by a predicate over verified actor identity — the response event's claimed actor and
  effective signer resolved against trust configuration — never over a writer-asserted vocabulary
  field. The concrete trust source (allowed-signers allowlist, gateway peer binding, or both) is
  settled by the federation-gate implementation plan; this ADR fixes the invariant that no binding
  decision reads a self-asserted field.

## Roles Are Claims, Not Identity

Gateway and federation binding must never key on `role` or on any successor vocabulary field. A
signature proves that the writer claimed a role; it does not prove the claim is true. Authorization
and binding decisions key on `writer.actorId`, the effective signer, and the admitted peer.

## Migration

This executes now, before signed-event adoption, under the hard-break policy:

- Remove `role` from `Writer` and from `EventToBeSigned`. Envelope serialization changes; existing
  stores break, which the pre-adoption policy permits.
- Regenerate the golden vectors in `tests/fixtures/event_signatures/`. `sigVersion` stays 1; the
  to-be-signed view simply no longer contains `role`.
- Close the mutation-coverage gap while regenerating: add a negative vector that mutates the
  relocated `sourceSpeaker` payload fact after signing (expected `invalid`, via `payloadHash`),
  so the formerly uncovered role-shaped fact gains the negative case it never had.
- Update ADR-0004's `EventToBeSigned` description when this ADR is accepted.

Deferring the same change past federation would mean a `sigVersion: 2` payload type, dual verifier
paths, dual golden-vector sets, and the persona-shaped vocabulary permanently embedded in every
v1-signed federated event.

## Consequences

### Accepted

- The envelope no longer carries a persona-shaped field; the #98 misreading cannot recur.
- The signed surface shrinks: the act was a redundant copy of the already-signed `eventType`.
- The source-speaker fact keeps its meaning, its signature coverage, and a home that names what it
  actually describes.
- Persona questions inherit signature verification instead of asserting around it.
- Deduplication, event identity, and idempotency are untouched.
- Raw ledger reads lose the human-readable act label; derived views supply it from `eventType`.

### Rejected

- Documenting the current field as-is: the docs would have to explain one field with two opposite
  semantics, and the persona-shaped signed field would survive into the gateway era.
- Renaming the review half to a stored act field: it keeps a redundant signed copy of information
  `eventType` already carries, at identical migration cost to removal.
- Making `role` actor-typed: review writes would need projection lookups at write time, which is
  undefined across stores and collides with ADR-0004's non-aliased identity claims.
- Storing a persona dimension: duplicates derivable state and creates a stored-versus-derived
  conflict class, against ADR-0003's surface-don't-pick posture.
- Deferring to a future `sigVersion: 2`: dual verifiers forever for a change that is mechanical
  today.

## Revisit Triggers

Reopen this ADR if raw event readability without a stored act label proves insufficient in
practice, if an adapter needs source-speaker vocabulary richer than `user` / `agent`, or if the
verified-identity replacement for the resumption-binding predicate cannot be specified before the
federation gate ships.
