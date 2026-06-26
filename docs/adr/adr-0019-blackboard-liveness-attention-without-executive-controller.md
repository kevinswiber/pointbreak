# ADR-0019: Blackboard Liveness — Attention/Notification Without an Executive Controller

**Status:** Accepted (owner-approved 2026-06-19); landed via the substrate-reshape implementation work.
**Independent** of ADR-0017/ADR-0018 (touches no identity); landed in parallel.
**Date:** 2026-06-19
**See also:** [substrate language](../substrate-language.md) and
[substrate thesis summary](../substrate-thesis-summary.md). Relates to: ADR-0017 §A5 / the ADR-0003 amendment (advisory-generative — a
notification carries attention, never authority), ADR-0018 (a liveness signal over a fork must carry
competing heads, not "the head moved"). In-repo `docs/adr/`: **ADR-0003** advisory-first / no-write-gate
(the executive guardrail this ADR reads precisely, not relaxes), **ADR-0008** cross-peer conflict policy
(the convergence invariants a distributed blackboard leaves untouched). External: the `shoreline-relay`
bridge (push lives there, outside the core).

## Context

The re-architecture frames Shoreline as a **blackboard**: multiple actors (agents and humans) coordinate
indirectly through recorded, attributed facts. A blackboard with no control component is **inert** unless
an actor either polls or is pushed to. Classic blackboard systems answer this with a *control component*
that conflates two jobs — *direct attention* and *authorize execution*. Shoreline forbids the second
(no scheduler, lease, write-gate, or global current-state — ADR-0003; `docs/substrate-language.md:80-82,
134-137`), which raises the question this ADR settles: can the blackboard be *live* without acquiring an
executive controller?

The substrate vocabulary already carves the distinction the answer needs. `docs/substrate-language.md`
defines `attention policy` and `derived attention state` ("Projection output used to guide attention, **not
to authorize or block writes**", `:68`) as first-class and *non-authorizing*, sitting beside `executive
policy` (`:73-77`), which is the one held to the "explicit, locatable, never a write-gate" bar. So the
attention layer is a *different category* from the executive controller the guardrails forbid.

The current code is strictly pull-only. `shore inspect` is a GET-only std HTTP server with no in-memory
state (`src/cli/inspect/server.rs`); the browser UI fakes liveness by client-side polling of
`/api/freshness` every 3 s (`src/cli/inspect/assets/app.js:1399`). The change signal it polls is
`event_set_hash` — an order-independent SHA-256 over sorted `(event_id, payload_hash)` that ignores
envelope-only changes (`src/session/projection/freshness.rs:23-43`). All *push* already lives outside the
core, in the `shoreline-relay` bridge (a Boardwalk `publish` after a durable write, with subscribers
attaching via `stream.subscribe`); the relay's README states Shoreline V1 "is not a daemon, notification
system, or multi-session broker." No `notify`/`tokio`/websocket/subscriber-registry exists in the core.

This ADR is written abstraction-down: this is liveness for the agent-work code-review journal, delivered as
transport over an unchanged journal — not a general pub/sub platform.

## Decision

### D1. Re-read "no controller" as "no *executive* controller"

The guardrail "no controller" is read precisely as **"no *executive* controller; attention and
notification are allowed and load-bearing."** This is a clarification, **not** a relaxation: the substrate
vocabulary already separates `attention policy` / `derived attention state` (non-authorizing) from
`executive policy` (`docs/substrate-language.md:68,73-77`). ADR-0003 and `substrate-language.md` are
updated to **cite** this split explicitly. A controller-free blackboard may *direct attention*; it may not
*authorize execution* (no scheduler, lease, write-gate, or global head).

### D2. `event_set_hash` is the canonical liveness token

Promote `event_set_hash` from an inspector-private freshness detail to **the documented liveness contract**.
It is a read-time fingerprint — cheap (no body hydration), order-independent, and envelope-stable
(`freshness.rs:23-43`). It is **optionally scoped**: whole-`Journal`, per-`Engagement`, or per-work-object
(the hash function already takes an arbitrary event iterator, so scope is a projection-selection choice,
not a new primitive). The token carries `{ scope, eventSetHash, eventCount, diagnosticCount }` and **carries
no instruction, no "head," and no gate** — it is `derived attention state`, full stop.

### D3. The core emits the change signal but never delivers it

The Shoreline core stays **strictly pull-only**: every read is a pure function of the store, with no
callbacks and no in-memory subscriber set. Delivery is always someone else's job:

- **Pull/poll transport lives in the client** — the inspector's 3 s poll (`app.js:1399`); a CLI
  `shore … --watch` is a client-side poll of the same token (no daemon, no fs-watch in the core, honoring
  ADR-0003's explicit "no filesystem notifications").
- **All push lives in relay/Boardwalk** — `publish` onto a Boardwalk stream, subscribers via
  `stream.subscribe` (an anonymous *advisory* read by policy). The relay should publish the generic signal
  ("store moved to `event_set_hash` X; N new facts") for *any* move, **replacing or accompanying** its
  existing per-transition announcements (today `review.input-request-responded` and
  `review.events-ingested`, `crates/shoreline-relay/src/actor.rs:315,476`), so subscribers get blackboard
  liveness for every move kind rather than a hand-picked few.
- **No push primitive enters the core** — no `notify`/`inotify`/`tokio`/subscriber-registry. ADR-0003
  already names "filesystem notifications" and "daemon broker" as the rejected architecture, and the relay
  proves push composes from outside over an unmodified core.

The seam's defining property: **the core emits a content-addressed change signal but never delivers it.**

### D4. The notification-independence invariant

A notification must **never be a precondition for a valid write.** Formally: the journal actor B produces is
byte-identical whether or not B received a notification — the only thing a notification changes is *when* B
reads, never *what* B may write. This single invariant is what keeps the attention layer from sliding into
executive control, and it is testable. (The relay already honors it: a dropped `publish` does not un-write
or fail the transition; the freshness probe is read-only and gates nothing.)

### D5. A distributed blackboard changes nothing in identity or convergence

Liveness is **pure transport over an unchanged journal.** The two digests the substrate already computes do
the distributed work: `event_set_hash` is the order-independent **set-convergence** primitive ("are two
mirrors in sync?" reduces to a 32-byte compare), and `event_record_hash` is the **per-event cross-mirror
agreement** primitive (a fact minted locally and the same fact ingested through a relay hash identically —
ADR-0008's convergence). A distributed blackboard therefore layers push *delivery* over a journal whose
identity and convergence are untouched; this ADR adds no new identity, no `sigVersion`, no merge rule.

### D6. Notification carries attention, never authority

A pushed *generative move* ("actor X proposed revision R") is the most tempting thing to escalate and the
most dangerous to let become operative. The rule: a notification may carry the **fact** of a move and its
`derived attention state` — including, over a forked supersession DAG (ADR-0018), "competing current heads
exist; ambiguity diagnostic" — but **never** an instruction ("implement it") and **never** a presumption of
a single head. This is the liveness face of the advisory-generative rule (ADR-0017 §A5 / the ADR-0003
amendment): generative moves stay Advisory; attention surfaces them, policy never promotes them via a
notification.

## Consequences

### Accepted

- **The blackboard is live without an executive controller**, and no guardrail is weakened — only cited
  precisely. The attention/executive split was already in the vocabulary; this ADR makes it load-bearing.
- **A single, cheap, scoped liveness token** (`event_set_hash`) serves local poll, CLI `--watch`, and
  distributed push alike; the signal lives in the core, delivery never does.
- **The core↔relay boundary is ratified** (truth + change-signal + per-event identity + on-command batch
  copy in the core; pull in the client; push/subscribe/networking/async in relay), so the relay keeps
  composing an unmodified core.
- **Costs accepted:** whole-`Journal` scope is coarse (any write wakes every watcher) until per-`Engagement`/
  per-work-object scoping is wired; a *live* distributed sync of the underlying facts (vs. liveness signaling)
  still needs a public ingest surface, which is an identity/store decision (ADR-0017 / store-migration
  territory), not this ADR's — liveness *signaling* is buildable today, fact *sync* is gated on exposing ingest.

### Rejected

- **A push primitive in the core** (`notify`/`tokio` watch/a subscriber registry). It would pull an async
  runtime into a crate that deliberately has none and re-introduce in-memory subscriber state into a core
  whose every read is a pure function of the store — the "daemon broker / filesystem notifications" ADR-0003
  rejects. Push composes from outside (relay), proven.
- **A control component / scheduler / lease / write-gate.** None is required to make B notice A's write; all
  are the executive functions the guardrails forbid.
- **A single-head "the head moved" notification.** Under fork-tolerant supersession (ADR-0018) "current
  revision" can be ambiguous; the signal must carry the set of heads / an ambiguity diagnostic, never presume
  one winner.
- **Relaxing the executive guardrail.** The liveness story needs the guardrail *read precisely*, not
  weakened; an attention signal that ever becomes a write precondition is the failure this ADR forecloses.

## Revisit Triggers

- Whole-`Journal` signal scope becomes a real noise floor (many work objects, many watchers) — then wire
  per-`Engagement` / per-work-object scoping (a projection-selection change, not a new primitive).
- A genuine need for *live fact sync* (not just liveness signaling) across mirrors emerges — that requires a
  public ingest surface and is an ADR-0017 / store-migration (identity/store) decision, reopened there, not here.
- Any proposal to let a notification gate or precede a write — reopen via ADR-0003's executive-policy
  exception, naming the executive behavior directly; never introduce it as ordinary attention metadata.

## Related Docs

- In-repo `docs/`: `substrate-language.md` (the attention/executive split this ADR cites — `:68,73-82`),
  `docs/adr/adr-0003-*` (advisory-first / no-write-gate), `docs/adr/adr-0008-*` (convergence invariants).
- Relates to ADR-0017 §A5 (advisory-generative), ADR-0018 (competing-heads attention).
- External: the `shoreline-relay` bridge (`crates/shoreline-relay/src/{actor.rs,reviewer.rs}`) — where push
  lives, composed over an unmodified core.
