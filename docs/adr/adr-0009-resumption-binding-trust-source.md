# ADR-0009: Resumption Binding Trust Source

**Status:** Accepted
**Date:** 2026-06-11
**See also:** [ADR-0003](./adr-0003-agent-resource-claims-advisory-first.md),
[ADR-0004](./adr-0004-event-signatures.md),
[ADR-0007](./adr-0007-writer-act-vocabulary.md),
relay ADR-0001 (peer-to-actor binding, shoreline-relay repo)

## Context

ADR-0007 removed `writer.role` from the envelope and fixed the invariant that no binding decision
reads a self-asserted field. It deliberately reserved the replacement: "the concrete trust source
(allowed-signers allowlist, gateway peer binding, or both) is settled by the federation-gate
implementation." The interim that landed with ADR-0007 is deny-all:
`response_identity_is_binding` (`src/session/projection/task.rs`) is a constant `false`, every
responded operative input request projects as `Blocked`, and the diagnostic
`agent_resumption_response_identity_not_binding` reports that no binding trust source is
configured. Agent resumption through responded input requests does not work at all today. This
ADR supplies the trust source.

Two kinds of evidence exist that are not the event's own claims:

- **Possession of the store.** The writer contract (ADR-0003) assumes one active Shoreline writer
  per `.shore/data/` store at a time. Everything in your store was either written by you or
  deliberately imported by you. For the local-first product — a human answering
  `shore review input-request respond` in their own worktree — possession is the trust root, and
  demanding key ceremony for it would break the product's floor.
- **Per-event signatures.** ADR-0004's Ed25519 signatures are the only verified identity that
  survives at-rest storage and re-forwarding. Verification resolves an effective signer and
  returns `valid` only when the signature verifies *and* the signer is authorized for the claimed
  `writer.actorId` under the allowed-signers trust set (`src/session/signing/trust.rs`).

Possession, however, is only evidence about events the store's own writer produced — and today
the store cannot tell those apart from imports. Both foreign-event entry points write the
received envelope verbatim through `record_event_once`: the ingest workflow
(`src/session/workflow/ingest.rs::ingest_events`, with `import_event` as a thin wrapper) and
bundle apply (`src/session/store/bundle.rs::commit_events`). Neither stamps anything. `sourceRef`
does not help: it is adapter vocabulary naming a location in a source conversation, set only by
the claude_code adapter (`src/session/adapter/claude_code/translate.rs`, `write.rs`), and the
local respond workflow (`src/session/workflow/input_request/respond.rs`) never sets it — so an
unsigned `input_request_responded` event ingested from a remote peer is envelope-indistinguishable
from one the local human just wrote. A possession arm built on today's store would silently extend
the local trust root to every imported event, which is exactly the forgery ADR-0007 closed.

So this decision has two parts: the predicate, and the small core change that makes its local arm
decidable.

## Decision

`response_identity_is_binding(event, policy)` is true iff **either** arm holds:

```text
binding(event, policy) :=
     verification(event, trustSet) == valid                      # arm (b): verified signer
  or (    policy permits local-possession binding
      and event carries no ingest provenance
      and verification(event, trustSet) != invalid )             # arm (a): local possession
```

Neither arm reads a field the writer asserted. Arm (b) reads the signature and the store's trust
configuration. Arm (a) reads the local importer's own bookkeeping and the store's policy. The
claimed `writer.actorId` is reported in the projection but is never the basis of the decision.

### Arm (a): local possession

An event with no ingest provenance was produced by this store's own writer under the
single-writer contract — written by you, on your machine, through your workflows. For such events
possession of the store is the trust root, exactly as it is for every other fact in `.shore/data/`.
This arm preserves the local-first product: a human responding in their own worktree binds with
zero keys, zero configuration.

One qualification: a signature that is present but **`invalid`** defeats arm (a). An invalid
signature is affirmative evidence of tampering or corruption (ADR-0004), and binding is the one
decision that must not shrug at it. Arm (a) accepts `unsigned` and any status better; it never
accepts `invalid`.

Arm (a)'s trust basis must be stated honestly. The ingest-provenance marker (below) is local
bookkeeping written by the store owner's own importer. It is trustworthy *to this store* under
the single-writer contract; it is not a signed fact, and it is never trustworthy to a third party
reading a mirrored or copied store. A store owner who hand-copies event files into
`.shore/data/events/` bypasses the seams and manufactures "local" events — but that is an act of the
owner against their own store, which possession already trusts. What the marker rules out is a
*remote* actor minting an unsigned binding response: a remote event can only enter through an
import seam, and the seam stamps it.

### The ingest provenance marker (required by this decision)

Today no marker exists, so this ADR requires one as part of the decision — a small core change,
not an optional nicety:

- Every path that writes a pre-formed foreign event — `ingest_events` / `import_event`
  (`src/session/workflow/ingest.rs`) and bundle apply
  (`src/session/store/bundle.rs::commit_events`) — stamps ingest provenance on the stored
  envelope, unconditionally. A reserved optional top-level envelope sibling:

  ```json
  "ingest": { "via": "ingest-events", "receivedAt": "2026-06-11T00:00:00Z" }
  ```

  with `via` drawn from a bounded vocabulary (`ingest-events`, `bundle-apply`). The predicate
  reads presence only; `via` and `receivedAt` are operator-facing detail.
- Import seams always **overwrite** any inbound `ingest` value with their own stamp. A stamp in
  arriving bytes is some other store's bookkeeping; only the local importer's stamp means
  anything here. (This is the same honesty rule ADR-0004 applies to `sourceRef`: hop metadata
  from elsewhere is not a fact.)
- The marker is excluded from `EventToBeSigned` — ADR-0004 already excludes `sourceRef` and
  "future hop-added metadata" from the to-be-signed view, so stamping a signed event cannot
  invalidate its signature — and it participates in neither idempotency keys nor `eventId`.
  `sigVersion` stays 1; the golden vectors are untouched. Re-ingesting an event the local writer
  already authored remains an idempotent existing event with first-stored-wins, so a locally
  authored event can never acquire a stamp after the fact, and an ingested event can never lose
  one.
- `sourceRef` is **not** reused as the marker. It means "where in the source conversation this
  fact came from," it is producer-supplied, and a remote adapter-translated event legitimately
  arrives already carrying one. Overloading it would destroy both meanings.

Existing stores predate the marker, so their imported events are unstamped. The migration honesty
is the same as everywhere else pre-adoption: a store owner who imported events before this lands
possesses a store whose history they chose; the marker discriminates from its landing forward.

### Arm (b): verified signer

The event's verification — effective signer resolved per ADR-0004 (explicit `signer`, else a
did:key `writer.actorId`), strict Ed25519 over the DSSE-encoded `EventToBeSigned`, authorization
checked against the store's allowed-signers trust set — returns `valid`. ADR-0004's `valid`
already folds in trust: the signature verifies *and* the signer is authorized for the claimed
actor, including the self-certifying actor-equals-signer rule
(`src/session/signing/trust.rs::authorizes`).

To be precise about which ADR-0004 policy machinery applies: **none of the presets**. The
`advisory` / `integrity-strict` / `trusted-strict` presets map verification status to
accept-or-reject decisions at ingest and read seams. Binding is a different decision with its own
mapping: only `valid` binds; `unsigned`, `untrusted_key`, and `invalid` never bind. That is
behaviorally the `trusted-strict` row with `allowUnsigned = false`, applied at the binding
decision point regardless of the store's general verification preset — an advisory store still
accepts and surfaces an unsigned ingested response as a fact, and still refuses to treat it as
binding. Acceptance and bindingness are separate questions; conflating them was the trust hole.

One limitation stated plainly: `TrustSet::authorizes` ignores `occurred_at` — there are no
validity windows, so a revoked-then-removed key stops authorizing new evaluations but nothing is
time-scoped. Binding inherits this; see Revisit Triggers.

### Policy presets

The binding policy is an explicit, named, reader-side projection policy — ADR-0003's "a specific
projection policy treats them as operative" given a name, living where ADR-0003 says policy
belongs: with the reader, not the substrate.

| Preset | Arm (a) local possession | Arm (b) verified signer |
| ------ | ------------------------ | ----------------------- |
| `local-and-verified` (default) | yes | yes |
| `verified-only` | no | yes |

Zero configuration means `local-and-verified`: the local-first product works out of the box, and
ingested events still bind only via signatures (every import seam stamps them, so arm (a) never
fires for them — even under the default). `verified-only` is for stores where possession does not
imply authorship: a checkout shared by several humans, or a store created by wholesale filesystem
copy (`cp -r` carries unstamped events; bundle apply does not). Choosing it is choosing that
nothing binds without a key — including your own unsigned responses.

### Diagnostics

The diagnostic code is unchanged — `agent_resumption_response_identity_not_binding` — and gains a
bounded `reason` detail naming the cheapest fix, first match wins:

- `signature_invalid` — verification is `invalid`; tampering or corruption; never binds via
  either arm.
- `signer_not_authorized` — verification is `untrusted_key` and arm (a) is unavailable; the
  signature is fine, the allowed-signers trust set does not authorize this signer for this actor.
- `ingested_unsigned` — the event carries ingest provenance and no signature; the responder must
  sign for this response to ever bind.
- `policy_excludes_local` — a local unsigned response under `verified-only`; the store's own
  policy demands a key.

The vocabulary is ambiguity-honest: each reason states what the projection knows, never a guess
about who the responder "really" was. Duplicate-response collapse and freshness evaluation are
untouched; this predicate replaces only the constant-false interim.

## The Federated Consequence

This is normative, not advisory: **an ingested response binds only via arm (b).** Therefore:

- Remote reviewers' responses must be **signed by the reviewer, end-to-end**. The signature is
  produced where the reviewer's key lives, not minted at a hub on their behalf.
- The relay must forward signatures intact. The relay's verify-on-ingest seam is the natural
  enforcement point for "ingested events keep their signatures" — a relay that strips or loses a
  signature converts a bindable response into `ingested_unsigned`.
- Restating relay ADR-0001: **the relay never signs as the reviewer.** A relay holding reviewer
  keys would collapse every reviewer identity into the relay's blast radius.
- Gateway peer binding (relay ADR-0001's peer→signer map) is a transport gate that *complements*
  this predicate and can never substitute for it. It is enforcement at the transition seam; it
  leaves no durable trace the projection can read. A projection evaluating a store six months
  later — or a mirror of it — sees only the events. The only admission evidence that survives
  into the store is the signature.

## Interaction Map

- **ADR-0003** — this is a named projection policy, the form ADR-0003 reserves for operative
  treatment of facts. The substrate still records every response; bindingness is the reader's
  explicit policy. The single-writer contract is what makes arm (a)'s possession root meaningful.
- **ADR-0004** — supplies the entire verification machinery for arm (b): the to-be-signed view,
  effective-signer resolution, the status vocabulary, the allowed-signers trust set, and the
  hop-metadata exclusion that makes the ingest marker signature-compatible.
- **ADR-0007** — the invariant is honored: neither arm reads a self-asserted field. Possession
  (the local importer's own stamp, or its absence) and signatures are both evidence outside the
  event's claims. The reserved decision this ADR completes is ADR-0007's last open revisit
  trigger for the predicate.
- **Relay ADR-0001** — the transport complement. Peer binding contains which signer keys an
  admitted link may introduce; this ADR decides what a store, reading its own durable events,
  treats as binding. Disjoint coverage, neither substitutes.

## Provisional: Key Custody And Enrollment Are Deferred

The predicate is decidable now; the UX of satisfying arm (b) cheaply is the follow-up. Explicitly
deferred to a dedicated signing-UX research, and listed as revisit triggers rather than solved
here:

- SSH-key reuse for event signing (developers already hold Ed25519 keys).
- Agent key provisioning: how a reviewer harness or coding agent gets a keypair.
- Enrollment: how an actor's key enters a store's allowed-signers set, and who reviews that
  change.
- Revocation and rotation flows, including validity windows in `TrustSet::authorizes`.

Until that work lands, arm (b) is exercised by manually maintained allowed-signers files — honest
but expensive — and arm (a) carries the local product.

## Consequences

### Accepted

- Agent resumption works again locally with zero configuration, and the deny-all interim is
  replaced by a predicate with a stated trust basis for each arm.
- A small core change rides along: every foreign-event seam stamps ingest provenance. The
  envelope gains an optional field; signatures, idempotency, event identity, and golden vectors
  are untouched.
- Remote reviewers cannot bind without keys. This is the point, and it makes the relay-side work
  (reviewer-side signing, signature-preserving forwarding) a hard prerequisite for federated
  resumption rather than a polish item.
- The marker is local bookkeeping, not a signed fact. Third parties reading mirrored stores must
  not trust it; `verified-only` exists for exactly that posture.
- Stores that imported events before the marker lands cannot retroactively discriminate them.
- Binding evaluation runs signature verification at projection time; the projection needs the
  store's trust set, which is threaded to it.

### Rejected

- **Role-based binding.** Removed by ADR-0007; a signature proves the key holder claimed a role,
  not that the claim is true. No successor vocabulary field may be a binding input.
- **Relay-signs-as-reviewer.** Rejected by relay ADR-0001 and restated here: it forges the very
  attribution the signature exists to carry.
- **Relay attestation as the v1 trust root.** `relay_attestation` is ADR-0004 reserved
  vocabulary, unimplemented; building binding on it would implement reserved vocabulary casually
  and root trust in the forwarder rather than the producer. Revisit when a durable hop-provenance
  requirement activates the family.
- **Trusting `sourceRef` — or any inbound provenance content — from remote stores.** It is
  unsigned hop metadata. Only the local stamp written by the local importer is meaningful, which
  is why import seams overwrite inbound stamps.
- **Inferring locality from the actor id** (e.g. "matches the store owner's git identity"). That
  reads a self-asserted field, violating the ADR-0007 invariant — an ingested event can claim any
  actor id.
- **Reusing the store's verification preset as the binding policy.** An advisory store would bind
  unsigned ingested responses; the trust hole reopened by configuration default.

## Cross-Project Impact

Two streams of shoreline-relay work are affected by this ADR:

- **The relay's peer-to-actor binding work** builds its respond seam around an unsigned
  claimed id (`verified_signer` is `None` at today's seam). Under this ADR such a response is
  durable but never binding (`ingested_unsigned`). The reviewer harness must sign responses
  with a reviewer-held key for them to bind — a deliberate revision to that work, not a silent
  scope change.
- **The relay's signature-verification-on-ingest work** is the natural enforcement point for
  "ingested events keep their signatures": the ingest seam already verifies before the durable
  write, so it is also where signature-stripping would be caught and where the ingest provenance
  stamp lands on the relay path.

## Revisit Triggers

Reopen this decision if one of these occurs:

- The signing-UX research lands key custody, enrollment, or rotation flows that make arm (b)
  cheap enough to question whether arm (a) should remain the default — or proves the enrollment
  cost so high that `verified-only` is unadoptable in practice.
- Trust-set validity windows land (`TrustSet::authorizes` currently ignores `occurred_at`) and
  binding needs time-scoped semantics this design did not anticipate.
- A real deployment needs the local arm in a store whose possession is shared (multiple humans,
  one checkout) and `verified-only` proves too coarse a refusal.
- An audit requirement activates `relay_attestation`, giving the projection durable transport
  evidence this predicate could legitimately read.
- The presence-only ingest marker proves insufficient — e.g. a projection needs to distinguish
  import seams or import times for binding, not just for operator display.

## Related Docs

- [ADR-0003](./adr-0003-agent-resource-claims-advisory-first.md) — advisory-first posture; the
  projection-policy form this decision instantiates.
- [ADR-0004](./adr-0004-event-signatures.md) — signature contract, verification statuses,
  allowed-signers trust set, hop-metadata exclusions.
- [ADR-0007](./adr-0007-writer-act-vocabulary.md) — the invariant and the reserved decision this
  ADR completes.
- Relay ADR-0001 (shoreline-relay repo) — peer-to-actor binding at the gateway; the transport
  complement that never reaches the durable store.

## Amendment: Binding Over a Signature Set

This amendment generalizes the binding predicate's verified-signer arm from a single signer to
**any-of a `valid` attestation over the co-signature set** introduced by the
[ADR-0004 detached co-signature amendment](./adr-0004-event-signatures.md) (its dependency). The
original decisions stand — **Status:** stays Accepted; this is a landing record that generalizes one
clause, not a re-decision. **Neither arm's trust basis, the ingest provenance marker, the policy
presets, nor the diagnostic vocabulary change** except for the set generalization stated here, and it
**composes under** [ADR-0010](./adr-0010-actor-identity-and-delegation.md)'s principal policy
unchanged. It landed with the co-signature event family (owner-approved 2026-06-17), bundled with the family
the predicate reads, using the same `## Amendment` mechanism ADR-0010 used.

### Context recap

ADR-0009's predicate was written when an event had at most one signature; its arm (b) reads *the*
signer:

```text
binding(event, policy) :=
     verification(event, trustSet) == valid                      # arm (b): verified signer
  or (    policy permits local-possession binding
      and event carries no ingest provenance
      and verification(event, trustSet) != invalid )             # arm (a): local possession
```

Under the co-signature amendment an event carries a **set** of attestations (the inline author
signature plus zero or more detached co-signature events), so "the event's verification" is no longer
a single status — it is a per-member multiset. Binding must ask its question over the set, and the
natural generalization is **any-of**.

### The generalized predicate

```text
cosignatures(event) := { event's inline attestation, if present }
                     ∪ { detached co-signature events targeting event.eventId }

binding(event, policy) :=
     (∃ a ∈ cosignatures(event) : verification(a, trustSet) == valid)     # arm (b'): any verified co-signer
  or (    policy permits local-possession binding
      and event carries no ingest provenance
      and inlineAttestationStatus(event) != invalid )                     # arm (a): local possession
```

### Arm (b') — any verified co-signer (the generalization)

Binds iff **at least one** member of the set verifies `valid`. ADR-0004 `valid` already means "the
signature verifies **and** the signer is authorized for the claimed `writer.actorId` under the store's
allowed-signers trust set" — so any-of is **intrinsically scoped to the event's own actor**. No
separate "the responder's own signature must be present" rule is needed: a co-signature only counts
toward binding if it is authorized for *this* event's actor. Threshold-of-N bound co-signers
(`require-k-cosigners`) is a named **deferred** policy tier — the same posture ADR-0010 used for its
deferred presets — not the v1 floor; v1 is any-of, matching ADR-0009's original "only `valid` binds."

### Arm (a) — local possession (refined, not changed)

Arm (a) is unchanged in basis: an event with **no ingest provenance** was produced by this store's own
writer; possession is the trust root. The one refinement is *which* attestation's `invalid` status
defeats it: `inlineAttestationStatus(event)` is the status of the event's **own inline attestation**. A
present-but-`invalid` inline signature still defeats arm (a) (tampering is not shrugged). Detached
co-signatures **cannot defeat arm (a) at all** — closed twice over: per ADR-0004 amendment D5 a
cryptographically `invalid` detached attestation is rejected before storage and never becomes a member,
and even if one were present, arm (a) reads only the inline attestation. Arm (a) therefore reads
exactly one status — the inline one — exactly as it did before the set existed.

### Diagnostics (generalized, same vocabulary)

The diagnostic code stays `agent_resumption_response_identity_not_binding`; its bounded `reason` detail
generalizes over the set, **first match wins**:

- `signature_invalid` — the event's **inline** attestation is `invalid`; tampering or corruption,
  defeats arm (a) and never binds. (Detached attestations cannot produce this reason — an invalid one
  is never a member.)
- `signer_not_authorized` — **no** member is `valid` and arm (a) is unavailable; at least one
  attestation verifies but every member is `untrusted_key`. Fix: authorize a present signer, or add a
  co-signature from an already-trusted signer.
- `ingested_unsigned` — the event carries ingest provenance and the set is empty; a bound signer must
  co-sign for this response to ever bind.
- `policy_excludes_local` — a local event whose set has no `valid` member under `verified-only`.

### Presets (unchanged)

| Preset | Arm (a) local possession | Arm (b') any verified co-signer |
| ------ | ------------------------ | ------------------------------- |
| `local-and-verified` (default) | yes | yes |
| `verified-only` | no | yes |

Zero configuration is still `local-and-verified`. Ingested events still bind only via arm (b') — every
import seam stamps ingest provenance, so arm (a) never fires for them — but now an ingested event binds
if **any** member of its set is `valid`. That is the federated improvement: a reviewer's response that
arrives with a divergent inline signature still binds as long as a bound signer's attestation is
somewhere in the set.

### Composition with ADR-0010 principal policy

Unchanged and still conjunctive — ADR-0010's `principal_sufficient` refinement composes *over* this
generalized `binding`:

```text
binding'(event, bindingPolicy, principalPolicy) :=
      binding(event, bindingPolicy)                 # this amendment's any-of generalization
  and principal_sufficient(event, principalPolicy)  # ADR-0010, verbatim
```

The any-of generalization can only widen what `binding` admits; `principal_sufficient` can only narrow
it. ADR-0010 is **not** reopened: principal resolution keys on `writer.actorId` and the human-committed
delegates map, neither of which the co-signature set touches.

### Consequences

#### Accepted

- A fact's bindingness is **robust to a single bad or revoked co-signer**: as long as one bound
  signer's attestation is `valid`, the response binds.
- The federated loop improves: a reviewer response that diverged on inline signature across stores
  still binds wherever the set contains one authorized attestation, instead of failing closed.
- No arm's trust basis changes; the ingest marker, presets, and diagnostic vocabulary are preserved.
- The v1 single-signer predicate is the one-member-set special case, so single-signed and unsigned
  events are unchanged.

#### Rejected

- **Threshold-of-N as the v1 default** — a deferred policy tier (`require-k-cosigners`), not the floor.
- **Defeating arm (a) on any invalid set member** — would hand a peer a denial-of-binding primitive;
  closed twice over (D5 pre-store rejection + arm (a) reads only the inline attestation).
- **Requiring the responder's own signature specifically** — redundant; ADR-0004 `valid` already binds
  every attestation to the claimed actor, so any-of is actor-scoped without a position rule.
