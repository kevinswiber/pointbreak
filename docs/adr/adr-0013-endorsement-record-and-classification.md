# ADR-0013: Endorsement Record and Classification

**Status:** Accepted
**Date:** 2026-06-18
**See also:** [ADR-0012](./adr-0012-actor-attributes-and-roles.md) (consumed),
[ADR-0004](./adr-0004-event-signatures.md) (the co-signature substrate this classifies),
[ADR-0009](./adr-0009-resumption-binding-trust-source.md) (any-of binding, kept separate),
[ADR-0010](./adr-0010-actor-identity-and-delegation.md), ADR-0011 (the `authorize_at` seam),
[ADR-0003](./adr-0003-agent-resource-claims-advisory-first.md), [ADR-0007](./adr-0007-writer-act-vocabulary.md)

> **Post-approval refinement (2026-06-18, owner-approved):** `reverse_resolve` no longer manufactures a
> self actor from a bare `did:key`. An unenrolled `did:key` endorser now classifies `unknown_endorser`
> (not `endorsement-trusted`) — endorsement trust must be explicitly granted (enrolled / resolved
> principal), not inferred from key syntax (consistent with the trust-set Option-1 posture). Self-cert is
> unchanged in the authoring scope. Also applied: ownership wording (the `classification` field and
> `has_trusted_endorsement()` are added by the ADR-0004 amendment; this ADR defines their meaning) and a
> bidirectional cross-reference to that amendment.

## Context

Plan 0068 / the ADR-0004 detached co-signature amendment already let Shoreline **store** a bare
co-signature: an `EventSignatureRecorded` carrier records "signer X attests target event E's exact bytes"
(an attestation over the target's `event-tbs.v1` view; member identity = the full attestation triple). The
co-signature set is a convergent G-Set. What is **missing is recognition**: today the projection reads
every member through one relation — a member is `Valid` iff its signer is authorized for the *target
event's* actor (`verify_cosignature` Step 5, `src/session/signing/cosignature.rs:97-112`) — so a genuine
endorsement, where an actor signs E **in its own identity** (its key is *not* in E's actor's
allowed-signers), lands `UntrustedKey`, visually indistinguishable from a stray key. There is no notion
that an `UntrustedKey` member might be *a different actor vouching for this change*.

This ADR adds that recognition as a **read-side classification**. It is a **precision ADR, not a
feature-expansion ADR**: it changes **no event bytes**, mints no new event type, adds no envelope or
carrier field, and freezes ADR-0004's `EventVerificationStatus`. The convergence constraint forbids a
carrier-payload marker anyway — `eventSetHash` is computed over `{event_id, payload_hash}`
(`src/session/projection/freshness.rs:18-36,68`), so a payload field excluded from identity would diverge
across mirrors — which is *why* classification is **derived** from the bytes we already store plus
human-committed config. It recognizes the bare endorsement we can already store; richer review records are
out of scope (below).

## Decision

### Endorsement is actor-neutral, defined by relationship — not by party

A co-signature member is an **endorsement** when its attesting signer resolves to an actor **distinct from
the event's actor, signing in its own identity** — as opposed to an **authoring** attestation, whose
signer is authorized for the event's *own* actor (the agent's key, or a second key enrolled under the same
actor). The definition names no "human" and no "principal": *human-ness* and *the principal relationship*
are attributes/relationships the classification **surfaces** (read from ADR-0012 + the delegates map), not
conditions of being an endorsement. A reviewer-model agent vouching is as much an endorsement as a human.

### A derived member classification — no carrier change

The `classification` field on the co-signature member is **added by the ADR-0004 amendment** (the
substrate change); this ADR defines what it *means*. It is a **derived** value computed at projection time
from `(member.source, member verify-status, attesting_signer → reverse-resolved actor(s),
target.writer.actorId, trust set, delegates, ADR-0012 attributes)`. **Only detached `EventSignatureRecorded`
members are endorsement candidates** — the inline member (#1) is the event's own author attestation and is
always `authoring`. Top-level buckets are kept small:

- **`authoring`** — the inline member (#1, any status), **or** a detached member whose signer is authorized
  for the event's own actor (today's detached `Valid`; binding-relevant).
- **`endorsement-trusted`** — a **detached** member that verifies and reverse-resolves to exactly one known
  actor distinct from the event's actor (an actor vouching in its own identity). Carries the **resolved
  endorser actor id** so relationship + attributes can be read downstream.
- **`endorsement-untrusted`** — a **detached** member that verifies, but whose signer cannot be placed as a
  single distinct known actor.

`EventVerificationStatus` (ADR-0004) is unchanged and the verify-before-store gate is unchanged (it still
stores `Valid` + `UntrustedKey`, drops `Invalid`). Classification is a pure read-side projection layered
over a member that already verifies cryptographically.

### The classifier — precedence and required reason detail

Buckets stay small; every non-clean outcome carries a **reason** so a UI or policy never guesses.
Classification first branches on the member's **source** (`CosignatureSource`,
`src/session/projection/cosignature.rs:32-39`): the **inline** member (#1) is the event's *own* author
attestation and is **never** an endorsement, whatever its verify status; only **detached**
`EventSignatureRecorded` carriers can be endorsements. Precedence (first match wins, mirroring ADR-0009's
bounded reason list):

```text
classify(member, target, trustSet, delegates, attributes):
  # Invalid detached members never reach here (dropped at the verify-before-store gate).
  if member.source == Inline:
      # member #1 = the event's own author attestation; authoring regardless of Valid/UntrustedKey/Invalid.
      return authoring          # an UntrustedKey/Invalid inline is a failed AUTHORING attempt, not an endorsement.

  # member.source == Detached (an EventSignatureRecorded co-signature carrier):
  if signer authorized for target.writer.actorId:           # detached `Valid` → authoring authority
      if signer ALSO reverse-resolves to a distinct actor:   # overlap / laundering guard
          return authoring { reason: authoring_not_endorsement }
      return authoring
  else:                                                      # detached `UntrustedKey` → endorsement candidate
      actors = reverse_resolve(signer, trustSet)             # see resolution order below
      match actors.count():
        exactly one  => endorsement-trusted { endorser: that actor }   # (always distinct: not authorized for target)
        zero         => endorsement-untrusted { reason: unknown_endorser }
        many         => endorsement-untrusted { reason: ambiguous_endorser }
```

**Reverse resolution order (`reverse_resolve`):** resolve **only** via the **explicit `allowed-signers`
mappings** — the set of actors `{X : signer ∈ allowed-signers[X]}`. A signer with no such mapping resolves
to **zero actors** (→ `unknown_endorser`), whether or not it is a syntactic `did:key`. Endorsement trust is
**not manufactured from key syntax**: an unenrolled `did:key` is *not* treated as its own singleton self
actor for endorsement classification. (The `trust.rs:53-59` self-certifying shortcut still applies in the
**authoring** scope #1 — a `did:key` *actor* signing its own event — but it does **not** mint a trusted
endorser from a bare signer.) To be a trusted endorser, a key must be explicitly resolvable: enrolled in
`allowed-signers`, or the resolved principal. This keeps `endorsement-trusted` meaning "a known,
explicitly-granted actor vouched," consistent with the Option-1 trust posture (trust is committed/explicit,
never manufactured).

Reason codes (small, closed for v1):

- **`authoring_not_endorsement`** — a *detached* signer is authorized for the event's actor, so the member
  is authoring authority, *deliberately not* counted as an endorsement even though the key also maps to a
  distinct actor. The laundering guard surfaced: a key enrolled under the agent's actor cannot be laundered
  into an independent endorsement. **This sharpens the earlier dual-enrollment wording**,
  which described the case as "flagged ambiguous": this ADR decides target-actor
  authorization has **precedence**, and the distinct-actor mapping is surfaced as `authoring_not_endorsement`,
  not as an endorsement ambiguity.
- **`unknown_endorser`** — the signer verifies but resolves to no known actor (not enrolled in
  `allowed-signers` and not the resolved principal) — **including a bare, unenrolled `did:key`**. Stored,
  shown as an untrusted endorsement; never satisfies `has_trusted_endorsement()`.
- **`ambiguous_endorser`** — the signer resolves to more than one *explicitly enrolled* actor (a key
  enrolled under multiple actor ids — legal today). **Surfaced, never guessed**, mirroring
  `PrincipalStatus::Ambiguous` (ADR-0010); a future disambiguation hint may narrow it (see triggers).

### `endorsement-trusted` is trusted *as an endorsement*, never as authoring authority

This is the spine, and it is written to be unmissable:

- **`has_valid_member()` remains THE binding surface.** Any-of binding (ADR-0009 amendment) counts **only
  `authoring` members with `Valid` status**. An endorsement — of any classification — **never** satisfies
  binding. Stewardship vouches for a change; it does **not** confer the agent's signing authority, so it
  cannot make an event bind.
- **`has_trusted_endorsement()` is a sibling reader surface** (added to ADR-0004's co-signature set by the
  ADR-0004 amendment; its meaning defined here), for the stewardship/classification/policy plane. It
  reports whether the set has an `endorsement-trusted` member (optionally narrowed by
  relationship/attributes downstream). It feeds *no* binding decision.

Keeping these two surfaces rigorously separate is the whole point: accountability/binding
(authoring, ADR-0009/0010) and stewardship (endorsement, this ADR) stay on different axes.

### The two authorization scopes and the ADR-0011 seam

The classifier uses the **same** actor-parametric authorization primitive at **two distinct scopes**, in
this order (this is why `TrustSet::authorizes` being actor-parametric, `trust.rs:53`, matters):

1. **Authoring precedence + binding — `authorize_at(target.writer.actorId, signer, occurredAt)`.** The
   *first* check (and the only one binding ever reads) authorizes the signer against the **target event's
   actor**, exactly as today (`cosignature.rs:97-112`). A pass → `authoring`; this is what
   `has_valid_member()` / the any-of binding fold consume.
2. **Endorsement trust — `authorize_at(endorserActor, signer, occurredAt)`.** *Only* for a **detached,
   non-authoring** member (it failed scope #1), after `reverse_resolve` yields a single endorser actor, is
   the signer authorized against the **endorser's own** actor to confirm `endorsement-trusted`. This scope
   never feeds binding.

Base classification works with **today's** `TrustSet::authorizes` at both scopes. When ADR-0011 lands the
windowed `authorize_at(actor, signer, occurredAt) -> Verdict`, both call-sites pass through unchanged —
scope #1 keeps `target.writer.actorId`, scope #2 keeps the endorser's actor; the temporal `Verdict` simply
refines scope #2 for the deferred preset below. Two contract points for ADR-0011's landing plan (no
ADR-0011 change required, only awareness):

1. Keep `authorize_at`'s `actor` **caller-supplied**; do not bake "authorize against `event.actorId`" as
   an invariant — endorsement is a second caller with a different scope.
2. Endorsement members stay **excluded from the binding any-of fold** (that fold authorizes against
   `event.actorId` and is correct for authoring members only).

The temporal upgrade — requiring an endorsement to be **live-`Valid` at read time** rather than merely
trusted-at-projection — is the deferred `require-verified-endorsement` policy preset and rides ADR-0011;
it is **not** in v1.

### Attributes feed relationship + kind/role, not the trusted bucket

`endorsement-trusted` requires only "resolves to a distinct known actor." Whether the endorser is the
event's **resolved principal** (a *relationship*, via the delegates map) or carries `kind=human` /
`role=reviewer` (*attributes*, via ADR-0012's `discover_actor_attributes`) is **surfaced alongside** the
classification for the policy plane to read — it is **not** a condition of trust. Per ADR-0012's hard
split, an **absent attribute is unattributed** and does not satisfy any `kind=`/`role=` predicate; the
scheme (`is_agent_actor_id`) is a display hint only, never a predicate.

### Policy is consumer-side and advisory

The endorsement classification is the substrate a **consumer** (CI, a PR gate, another agent, the relay)
reads to gate an action — Shoreline **computes the classification always and does not itself block**
(ADR-0003 advisory-first; ADR-0010's reader-side-policy posture). The preset *family* —
`require-endorsement` (any trusted actor), `require-principal-endorsement` (the resolved principal),
`require-endorsement[kind=human]`, `require-endorsement[role=reviewer]`, deferred `require-k-endorsers`
and `require-verified-endorsement` — is a reusable predicate library over `has_trusted_endorsement` +
relationship + attributes. The presets are not Shoreline gates and are specified by
the policy work, not minted here; this ADR only provides the classification they read.

### Out of scope (deferred — would change the primitive)

This ADR recognizes the **bare** endorsement (a content-free vouch for exact bytes). The following are
real but each would drag this ADR into a *different* primitive and are deferred:

- **A value/verdict (approve/reject/neutral), a comment, or a confidence.** A bare co-signature is
  affirmative-only and carries no signer-authored content; a verdict/comment/confidence must be **signed
  by the endorser and identity-bearing**, i.e. a distinct "review decision" event — not a field on this
  carrier (unsigned → forgeable; excluded-from-identity payload → breaks `eventSetHash` convergence).
- **Targeting an event set / review round** rather than a single event. There is no addressable set-root
  object today (`eventSetRoot` is reserved); per-revision scoping over a round is future work.

## Consequences

### Accepted

- Endorsements become **recognizable** — "an actor vouched for this change, in its own identity" — with
  **zero event-byte change**, no new event type, and `EventVerificationStatus` frozen. A precision layer,
  not new storage.
- **Convergence-safe by construction:** no carrier-payload field, so endorsement carriers stay
  byte-identical across mirrors (`payload_hash`/`eventSetHash` unchanged). Classification is reader-relative
  config, degrading to `endorsement-untrusted`/unattributed at a config-less mirror — never a wrong answer.
- **Binding is untouched:** `has_valid_member()` keeps its exact authoring-only meaning; the ADR-0009
  any-of predicate does not change. No risk of laundering stewardship into signing authority.
- **No guessing:** small buckets + required reason detail (`unknown_endorser` / `ambiguous_endorser` /
  `authoring_not_endorsement`) give UI and policy a precise, first-match-wins signal.
- **Actor-neutral and forward-compatible:** a reviewer-model endorser is in-model; the `authorize_at`
  seam composes with ADR-0011 when it lands (two scopes — target actor for binding, the endorser's own
  for endorsement trust; deferred temporal tier) without a redesign.

### Rejected

- **A carrier-payload `relation`/endorsement marker** (the original research draft). Withdrawn: a
  payload field excluded from identity breaks `eventSetHash` convergence (`freshness.rs`). If an explicit
  stored marker is ever required it must be **identity-bearing** (in `eventId`) — deferred.
- **Reusing `Valid` / `has_valid_member()` for endorsement.** Would (i) require enrolling the endorser
  under the *agent's* actor (delegation-shaped, wrong axis) and (ii) make a per-event voucher silently
  confer standing signing authority and flip binding. The two surfaces stay separate.
- **Guessing on ambiguous/unknown endorsers.** A key under multiple actors, or under none, must surface a
  reason and classify untrusted — never be auto-picked into an endorsement.
- **Requiring a human / non-agent endorser as the trust condition.** Actor-neutral by decision; kind is a
  policy filter (ADR-0012), not a gate on `endorsement-trusted`.
- **Deriving kind from the actor-id scheme.** Per ADR-0012, the scheme is a display hint, not authority.

## Cross-References

- **ADR-0012 (Actor attributes & roles):** consumed for the endorser's kind/roles; the hard split
  (absent = unattributed for all branching) is honored here.
- **ADR-0004 (Event Signatures + co-signature amendment):** the substrate this classifies; its member
  identity, G-Set, and `EventVerificationStatus` are unchanged. The **ADR-0004 amendment of this pass**
  ("Co-Signature Member Classification and Trust-Set Locality") is the home of the substrate additions this
  ADR gives meaning to — the derived `classification` field, the `has_trusted_endorsement()` reader, and
  the convergence invariant — and of the trust-set committed-only decision.
- **ADR-0009 (Resumption Binding):** the any-of binding surface (`has_valid_member`) is kept authoring-only
  and untouched; endorsement is a separate sibling reader.
- **ADR-0010 (Actor Identity and Delegation):** the actor/principal model and the delegates map (the
  *relationship* read); accountability is unchanged.
- **ADR-0011 (Key validity / trust lifecycle):** the `authorize_at` primitive, called at two scopes
  (target actor for authoring/binding; the endorser's own actor for endorsement trust); the live-`Valid`
  `require-verified-endorsement` tier defers to it. ADR-0011's landing plan carries the two contract points
  above.
- **ADR-0003 (advisory-first):** Shoreline computes the classification and does not gate.
- Research 0010: `synthesis.md`; `q3-endorsement-semantics-and-identity.md` (the classifier);
  `q4-stewardship-policy-tiers.md` (the consumer preset family); `q6-federation-and-trust-over-time.md`
  (convergence + temporal); `addendum-endorsement-shape-open-questions.md` (the deferred review-decision
  layer).

## Revisit Triggers

- **A verdict/comment/confidence is wanted** → design a distinct, signed, identity-bearing "review
  decision" event (addendum), with `endorsement` as its affirmative value; do not bolt fields onto this
  carrier.
- **Endorsing an event set / review round is wanted** → define an addressable set-root/round target
  (`eventSetRoot` is the reserved slot) and per-revision scoping.
- **`ambiguous_endorser` is common** → revisit the `did:key`→actor reverse resolution (a reverse index, a
  carrier-`writer` disambiguation hint, or an enrollment-uniqueness rule).
- **`require-verified-endorsement` (live-`Valid` at read time) is wanted** → lands with ADR-0011's windowed
  `authorize_at` + trust-set validity windows.
