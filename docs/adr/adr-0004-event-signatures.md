# ADR-0004: Per-Event Ed25519 Signatures

**Status:** Accepted
**Date:** 2026-06-03
**See also:** [ADR-0003](./adr-0003-agent-resource-claims-advisory-first.md)

## Context

Shoreline events are durable review facts that can be forwarded between clones, bridges, and
library consumers. Existing event validation proves internal consistency: an event's `eventId`
matches its idempotency key and its `payloadHash` matches the payload. That integrity layer does
not prove that the named writer controlled the claimed actor identity.

Federated review workflows need an authenticity layer that survives at-rest storage and later
forwarding. Transport authentication alone is insufficient because the durable event may be
replayed after the connection that carried it is gone.

## Decision

Add optional per-event Ed25519 signatures to the `shore.event` envelope. Unsigned historical events
remain valid and continue to serialize without new fields. Signed events add two top-level envelope
siblings:

```json
{
  "signer": "did:key:z6Mk...",
  "signature": {
    "alg": "ed25519",
    "sigVersion": 1,
    "sig": "base64-ed25519-signature"
  }
}
```

`signature = { alg, sigVersion, sig }` is the complete v1 signature object. It does not carry
`publicKey` or `keyId`; the signing identity is the top-level `signer`, or, for self-certifying
events, the `writer.actorId` when that actor id is itself a `did:key`.

For `sigVersion = 1`, the payload type is:

```text
application/vnd.shore.event-tbs.v1+json
```

The signed bytes are literal Dead Simple Signing Envelope (DSSE) pre-authentication encoding over
the canonical `EventToBeSigned` JSON bytes. The media type is versioned and keeps `event-tbs.v1`
as the protocol media type label for "event to be signed"; public Rust names spell out
`EventToBeSigned`:

```text
payloadType = "application/vnd.shore.event-tbs.v1+json"
toBeSignedBytes = canonical_json(EventToBeSigned)
message         = preAuthenticationEncoding(payloadType, toBeSignedBytes)
signature       = Ed25519.sign(message)
```

The Dead Simple Signing Envelope pre-authentication encoding byte format is:

```text
preAuthenticationEncoding(type, body) = "DSSEv1" SP len(type) SP type SP len(body) SP body
```

`len` is the ASCII decimal byte length and `SP` is byte `0x20`.

## EventToBeSigned

`EventToBeSigned` is an explicit producer-fact view, not "the whole event minus signature." It
contains:

```text
{
  schema,
  version,
  eventType,
  eventId,
  payloadHash,
  target,
  actorId,
  signer,
  occurredAt,
  assertionMode
}
```

- `actorId` is `writer.actorId`, the claimed actor identity.
- `signer` is the resolved effective signer and is always a `did:key`.
- `payloadHash` binds the payload without signing raw payload bytes.
- `sourceRef` is excluded because it is hop metadata.
- `ingest` (the import-seam provenance stamp,
  [ADR-0009](./adr-0009-resumption-binding-trust-source.md)) is a realized instance of the
  hop-added metadata this exclusion anticipated; stamping a signed event cannot invalidate its
  signature.
- `sigVersion` is not inside the to-be-signed view; it selects the verifier path and payload type.

The to-be-signed view excludes `payload`, `sourceRef`, `ingest`, `signature`, `sigVersion`, and
future hop-added metadata.

## Identity And Trust

V1 uses `did:key:z6Mk...` for Ed25519 signer identity. A `did:key` identity may also be the
claimed `writer.actorId`.

`did:key` actor attribution and friendly `actor:*` attribution signed by the same key are distinct,
non-aliased identity claims. For example, `writer.actorId = did:key:P` and
`writer.actorId = actor:git-email:alice@example.com` with `signer = did:key:P` are different
events and remain distinct by design.

Verification resolves the effective signer as follows:

- If `signature` is present and `signer` is present, `signer` is the effective signer.
- If `signature` is present, `signer` is omitted, and `writer.actorId` is a `did:key`, that actor id
  is the effective signer.
- If `signature` is present and no effective signer can be resolved, verification is `invalid`.

Friendly `actor:*` ids are authorized by an allowed-signers trust set that maps actors to one or
more `did:key` signers. A self-certifying `did:key` actor is authorized only when the effective
signer is the same key.

## Verification Status And Policy

Verification returns one of these status values:

```text
valid / invalid / untrusted_key / unsigned
```

- `valid`: the signature verifies and the signer is trusted for the claimed actor.
- `invalid`: the key, algorithm, signature, version, or signed bytes are malformed or mismatched.
- `untrusted_key`: the signature verifies, but the signer is not authorized for the claimed actor.
- `unsigned`: the event has no signature.

Ed25519 verification uses strict semantics, such as `ed25519-dalek`'s `verify_strict` or an
equivalent normative ruleset. Unsupported algorithms, unsupported `sigVersion` values, malformed
`did:key` values, non-Ed25519 keys, truncated or over-long signatures, non-canonical public keys,
and signature mismatches are `invalid`.

Verification is advisory by default, matching ADR-0003. The policy presets are:

| Preset | `invalid` | `untrusted_key` | `unsigned` |
| ------ | --------- | --------------- | ---------- |
| `advisory` | accept with diagnostic | accept with diagnostic | accept with diagnostic |
| `integrity-strict` | reject | accept with diagnostic | accept with diagnostic |
| `trusted-strict` | reject | reject | reject unless `allowUnsigned` |

These presets separate corruption checks from trust-root enforcement and unsigned-event migration.
Verification status is separate from artifact availability: a valid signed event can still
reference an unavailable artifact.

## Idempotent Existing Events

Signatures do not select a conflict winner. When a write or ingest sees an already-stored event with
the same idempotency key and payload hash, the first stored event remains authoritative. If the later
copy has a different `signer` or `signature`, Shoreline keeps the first event and reports
`divergent_signature_existing_event` on ingest. Other metadata differences with the same payload hash
remain an idempotent existing event; a different payload under the same idempotency key remains a
conflict.

## Consequences

### Accepted

- Signatures authenticate durable events, not transport connections.
- The signed to-be-signed view binds event identity, payload hash, target, actor id, signer,
  timestamp, and assertion mode.
- Unsigned events remain valid so existing stores can be read and forwarded during migration.
- Advisory mode surfaces authenticity information without making trust a default write-side gate.
- Strict policies are explicit reader or ingest choices.
- `sourceRef` remains unsigned hop metadata and is not part of the producer signature.

### Rejected For V1

- A `.sig` sidecar, because it would split the event's forwarding unit.
- `publicKey` or `keyId` fields inside the v1 signature object.
- Persisting verification status in the event bytes.
- Using signatures to pick an automatic conflict winner.

## Deferred Vocabulary

These names are reserved for future work and are not implemented by this v1 signature contract.

### Signed Heads And Event-Set Roots

Signed track heads are deferred. When they exist, they will sign an `eventSetRoot` computed from a
versioned event-set algorithm. The reserved root algorithm name is:

```text
shore.event-set.canonical-map.v1
```

The reserved entry shape is:

```text
entry = eventId SP payloadHash SP eventRecordHash LF
eventSetRoot = sha256(concat(entries sorted by eventId, then eventRecordHash))
```

Reserved signed-head payload types:

```text
shore.trackHead.store-state.v1
shore.trackHead.producer-fact.v1
```

Reserved feature levels:

```text
none
trackRoot
parentAnchored
```

### Relay Attestation

`relay_attestation` is reserved as a future signed event family for durable relay provenance.
Per-event producer signatures do not authenticate who forwarded an event. `sourceRef` remains
unsigned hop metadata.

### Multi-Signature Envelopes

This v1 signature contract supports a single producer `signature`. Multi-signature event envelopes
are deferred. If a future design adopts `signatures: []`, signer identity belongs per signature
entry rather than as a single top-level `signer`.

## What Signatures Do Not Prove

Per-event signatures do not prove:

- global completeness;
- absence of selectively withheld events;
- confidentiality under selective replication;
- uncompromised human intent when a key holder or signing agent is compromised;
- relay provenance without a future `relay_attestation`;
- availability of referenced snapshot or note-body artifacts;
- an automatic winner for conflicting events.

## Future Work

Future review lineage and event-sync ADRs should cross-reference this ADR. New event families should
remain signable under the generic `EventToBeSigned` contract unless they intentionally introduce a
new `sigVersion` and payload type.

## Amendment: Detached Co-Signature Event Family

This amendment extends ADR-0004's deferred "Multi-Signature Envelopes" section and activates the
reserved `eventRecordHash` name into a concrete, **back-compatible** contract: signatures over a
Shoreline event form a **set of attestations** keyed to the event's signature-exclusive identity, and
multiple signatures over one fact are **co-signers, not a conflict**. The original decisions stand —
**Status:** stays Accepted; this is a landing record, not a re-decision. It introduces **no new
`sigVersion`** and **migrates no stored bytes**. It landed with the co-signature event family
(owner-approved 2026-06-17), using the same `## Amendment` mechanism ADR-0010 used for "Key Custody
Landing".
The governing definition of `eventRecordHash` lives in
[ADR-0008](./adr-0008-cross-peer-conflict-policy.md); the binding generalization it enables is the
amendment to [ADR-0009](./adr-0009-resumption-binding-trust-source.md); it composes under
[ADR-0010](./adr-0010-actor-identity-and-delegation.md) unchanged.

### Context

ADR-0004 v1 ships a **single** inline producer `signature` and explicitly defers multi-signature
envelopes ("If a future design adopts `signatures: []`, signer identity belongs per signature entry
rather than as a single top-level `signer`"). The field's settled cross-industry answer — DSSE
`signatures[]`, JWS, CMS SignerInfos, PGP, cosign + Rekor, Certificate Transparency — is uniform:
**identity is the content; signatures are a set of attestations attached to it.** The cautionary
tales (Bitcoin `txid` malleability; git's signature-in-the-SHA) are exactly why this amendment keeps
signatures *out* of the identity hash.

Shoreline's store is **append-only and content-addressed**: an event's stored bytes are immutable and
`eventId = sha256(idempotencyKey)` is already signature-exclusive, so you **cannot** grow an inline
`signatures: []` array on a stored event without rewriting its bytes. Co-signatures are therefore
forced into the only shape the substrate allows — **detached, append-only attestation records keyed
by content identity** — which is also the cosign/Rekor/PGP/git-notes pattern. And it is on-brand: *a
co-signature is itself an event.*

### Decision

#### D1 — The inline `signer`/`signature` is attestation #1

The v1 envelope `signer`/`signature` pair is reinterpreted, with **no byte change**, as the **first
member** of the event's co-signature set. An unsigned event has an empty set; a v1 single-signed event
has a one-member set. Nothing about already-stored events changes — a reinterpretation of existing
bytes, not a migration.

#### D2 — Additional attestations are a detached co-signature event family

Every attestation beyond the inline author signature is recorded as a member of a new **append-only
co-signature event family** (`event_signature`). A co-signature event is an ordinary `shore.event`: it
has its own `eventId`, `writer`, `occurredAt`, and replicates over the same event-sync plane as every
other event; it **references the target by its signature-exclusive content identity** —
`targetEventId` **and `targetEventRecordHash`** (the ADR-0008 signature-exclusive hash), **not**
`targetPayloadHash`; and its own `eventId`/idempotency key **derives from the full attestation
`(targetEventRecordHash, attestingSigner, signature)`**, so the member identity is the *whole triple*
(see D3), re-submitting the identical attestation is idempotent, and two distinct signatures by one
signer are two distinct members — never two claimants to one slot. Signer identity belongs per
attestation, never as a single top-level field.

#### D3 — Signatures do not enter event identity; the set converges by union

The target event's `eventId` and signature-exclusive `eventRecordHash` (ADR-0008) remain
**signature-exclusive**. The co-signature set is a **grow-only set (G-Set / join-semilattice)** whose
**member identity is the full attestation triple `(targetEventRecordHash, attestingSigner,
signature)`** — *not* `(targetEventRecordHash, signer)`. Keying on the full triple closes a
**signer-slot-poisoning** hazard: if member identity were `(target, signer)`, a malformed or
adversarial attestation occupying that slot first would, under first-wins idempotency, block the
signer's later valid attestation. With the full attestation in the identity, a valid attestation is a
*distinct* member from a bad one by the same signer; merge is set-union; identical triples dedup;
union is commutative, associative, and idempotent, so two stores holding different subsets of one
event's attestations **converge to the union with no winner-selection and no conflict**.

Because each co-signature is itself an *event*, **signature-set convergence is subsumed by event-set
convergence**: a store missing an attestation is missing that event and backfills it on the next sync.
Co-signature events carry their own `eventId`/`payloadHash` and are covered by the shipped
signature-blind `eventSetHash` and the reserved `eventSetRoot` like any event, while the *target's*
`eventRecordHash` stays signature-exclusive so a divergent inline author-signature never breaks root
convergence. There is **no separate signature-reconciliation channel** to build.

#### D4 — A co-signature attests the target's `EventToBeSigned` view (no new `sigVersion`)

The attestation in a co-signature event is an Ed25519 signature over the **target event's
`EventToBeSigned` view with `signer` set to the attesting signer** — the existing v1 message,
`application/vnd.shore.event-tbs.v1+json`, with the same DSSE pre-authentication encoding. **No new
`sigVersion`, no new payload type.** This is load-bearing twice: the inline author signature is
co-signature #1 **with no transformation** (D1), and a co-signature is verifiable with the unchanged
ADR-0004 verifier (strict Ed25519, allowed-signers authorization, the `valid / invalid /
untrusted_key / unsigned` status vocabulary, per attestation).

Two digests of the target are in play and **must never be confused**. The attestation signs the
**signer-inclusive** `EventToBeSigned` view (so each signer signs a view naming themselves and neither
attestation is replayable as the other), while the carrier binds the **signer-exclusive**
`targetEventRecordHash` — the convergent content-identity. These are **different digests over
different field sets** (the TBS view includes `signer`/`actorId` but not `payload`/`idempotencyKey`;
`eventRecordHash` includes `payload`/`idempotencyKey` but excludes `signer`/`signature`); they are
*not* interchangeable. A verifier reconstructs `EventToBeSigned` for the target with `signer` set to
the attestation's signer (all other fields from the target the carrier's `targetEventRecordHash`
resolves to) and checks the Ed25519 signature, so the co-signature is tied to exactly the
content-identity that converges across mirrors. The carrier event's own envelope provenance (who
*recorded* it, its ingest stamp) is **orthogonal** to the attestation's trust: a co-signature's trust
rests entirely on its embedded signature verifying against the trust set.

#### D5 — Verification is per-member; detached attestations verify before they store

The set's verification is the **multiset of per-attestation statuses**, and no member's status changes
another's — a `valid` attestation stands whatever else is in the set, which is what makes a fact's
trust robust to a single bad or revoked co-signer. A detached co-signature event **verifies
cryptographically before it is stored**: a structurally `invalid` one (the ADR-0004 `invalid` set) is
**rejected, not stored** (reader-independent noise), while `untrusted_key` is **kept** (reader-relative;
may become `valid` on a trust-set update). So the stored set contains only `valid` and `untrusted_key`
members. The **one** attestation that may be `invalid` in a stored event is the **inline** one — part
of the event's own bytes, kept per ADR-0004's "keep the event, surface `invalid`" rule and read only
by ADR-0009 arm (a).

#### D6 — Class-(b) divergence is reconciled by transcription, not reported as a conflict

When ingest offers an event whose `eventId`, `payloadHash`, **and signature-exclusive
`eventRecordHash`** match a stored event but whose inline attestation differs, the store keeps its
first-stored copy **and records the incoming inline attestation as a co-signature event** (D2),
converging the set to both signatures. The matching `eventRecordHash` is the precise predicate for
"this is the *same fact*, differently signed"; were `eventRecordHash` to differ, the copies are not the
same record and it is not a co-signature case. Because the incoming attestation is a real signature the
importer *received and can verify* over the target's TBS view, this is **transcription, not minting** —
the importer never needs the co-signer's private key and never forges anything (the relay never signs
as the reviewer); per D5 it transcribes only `valid`/`untrusted_key`, never `invalid`. The legacy
`divergent_signature_existing_event` signal is retired as a *divergence* report; a diagnostic now fires
only when the newly merged co-signer is **untrusted for the claimed actor**, not for divergence per se.

### Resolved design questions

| # | Question | Resolution |
| - | -------- | ---------- |
| 1 | Binding over a set: any-of vs threshold vs "responder's own signature present" | **Any-of a `valid` attestation.** ADR-0004 `valid` already means "verifies *and* signer authorized for the claimed `writer.actorId`," so any-of is intrinsically actor-scoped. Threshold-of-N (`require-k-cosigners`) is a named **deferred** policy tier. Detailed in the ADR-0009 amendment. |
| 2 | Storage shape; merge key; dedup | **New event family** (D2), not a sidecar. Merge is G-Set union with **member identity = the full attestation triple `(targetEventRecordHash, attestingSigner, signature)`** (D3); full-attestation keying + verify-before-store (D5) closes signer-slot poisoning. |
| 3 | Backward compatibility | **Inline `signer`/`signature` = attestation #1; no historical byte migration** (D1). Signature-exclusive identity is what makes this free. |
| 4 | Interaction with the trust lifecycle | Revoking one co-signer's key distrusts one *attestation*, never the fact's identity; a fact co-signed by A and B survives A's revocation on B's attestation (D5). Revocation/rotation/transparency over set members is designed separately. |
| 5 | `eventSetHash` / `eventSetRoot` | **Co-signature events are ordinary records in the set**, so `eventSetHash` (shipped, signature-blind) and the reserved `eventSetRoot` converge them as events; the *target's* `eventRecordHash` stays **signature-exclusive**. Signature-set reconciliation is therefore **not** a separate sync channel — it is event-set convergence. |

### Backward Compatibility

- **Already-stored single-signature events** are valid as written: their inline attestation is member
  #1 of a now-explicit set. No re-signing, no `eventId` change, no `sigVersion` change, golden vectors
  untouched.
- **Unsigned events** have an empty co-signature set and behave exactly as ADR-0004 specifies.
- **Mixed stores** are internally consistent; a reader without the co-signature events sees a smaller
  set and converges on backfill.
- **The v1 single-signer verifier** is a strict special case of the per-member verifier (a one-member
  set).

### Consequences

#### Accepted

- Multiple signatures over one fact are **co-signers, not a conflict**.
- Signatures are decoupled from identity: rotation is "co-sign with the new key," and a fact's trust is
  robust to single-key revocation.
- Conflict class (b) dissolves (ADR-0008); the relay's divergent-signature *report* becomes expected
  *reconciliation*.
- Binding generalizes to any-of a bound signer over the set (ADR-0009 amendment) without reopening
  either arm's trust basis.
- No `sigVersion` bump, no payload-type change, no historical byte migration.

#### Rejected

- **An inline `signatures: []` array on the event envelope** — impossible on a content-addressed,
  append-only store without rewriting stored bytes and breaking `eventId`.
- **A `.sig` sidecar** — splits the event's forwarding unit; detached *events* keep one forwarding unit
  and converge over the event plane.
- **Folding signatures into `eventId` / `eventRecordHash`** — re-affirmed rejected; it is what makes the
  divergent-signature conflict class exist in the first place.
- **The importer minting a co-signature on a reviewer's behalf** — transcription re-homes a received,
  verifiable signature; it never synthesizes one.
- **A dedicated co-signature payload type / new `sigVersion`** — breaks lossless transcription of a
  divergent inline attestation (D6) and adds a payload type for no convergence benefit.

> **The original ADR-0004 decision stands.** This is a back-compatible extension to the co-signature member
> model plus a deliberate trust-set-locality decision. It changes **no event bytes**, no `sigVersion`, no
> member-identity triple, and leaves `EventVerificationStatus` frozen. The text below is appended verbatim
> as a `## Amendment` section to the landed ADR-0004 (the append-only ADR discipline).

---

## Amendment: Co-Signature Member Classification and Trust-Set Locality

### Context

ADR-0013 introduces **endorsement** (an actor co-signing an event in its own identity) and a **derived
classification** over co-signature members. That classification is read over *this* ADR's co-signature
substrate — the member model, `EventVerificationStatus`, and the `allowed-signers.json` trust set
("Identity And Trust") — so three substrate deltas need recording here, and one trust-set housekeeping
decision the local-override pattern (ADR-0010 `delegates.json`, ADR-0012 `actor-attributes.json`) now
makes conspicuous needs settling deliberately. The classification *semantics* live in ADR-0013 and are
**not** restated here; this amendment records only what changes for ADR-0004's substrate.

### Decision

#### Co-signature members carry a derived classification (semantics: ADR-0013)

A co-signature member gains a **derived, read-side** `classification` ∈ {`authoring`,
`endorsement-trusted`, `endorsement-untrusted`}, computed at projection time over the bytes already
stored plus **reader-supplied config** (the committed trust set, plus the delegates and actor-attributes
maps — which may carry `.local.json` overlays). `EventVerificationStatus` is **unchanged and frozen**; the
full-attestation-triple member identity, the G-Set union/dedup, and the verify-before-store gate are
**unchanged**. The classification's definition, precedence, reason codes
(`unknown_endorser`/`ambiguous_endorser`/`authoring_not_endorsement`), inline-vs-detached scoping, and the
two `authorize_at` scopes are **ADR-0013's** decision. Net effect on ADR-0004: a member is no longer read
through the single authority relation (`valid` ⟺ authorized for the target's actor) — that relation is
preserved exactly as the `authoring` path, and a *second*, derived, **non-binding** reading recognizes an
endorsement.

#### A sibling reader surface: `has_trusted_endorsement()`

ADR-0004's co-signature set gains `has_trusted_endorsement()` beside `has_valid_member()`.
`has_valid_member()` keeps its **exact** authoring-only, binding-relevant meaning (it is what ADR-0009's
any-of binding reads); `has_trusted_endorsement()` reports an `endorsement-trusted` member for the
stewardship/policy plane and **never** feeds binding. The two surfaces are kept rigorously separate.

#### Convergence invariant: member meaning is *derived* or *identity-bearing*, never an excluded payload field

**Any meaning attached to a co-signature member must be either derived at projection or identity-bearing**
(folded into the member's `idempotencyKey`, hence `eventId`). A co-signature *payload* field that is
excluded from member identity but included in `payloadHash` is **forbidden as a carrier of member
meaning**, because `eventSetHash` is computed over `{event_id, payload_hash}`
(`src/session/projection/freshness.rs:18-36`, with the payload-hash-sensitivity test at `:68`): such a
field would let two independently-minted carriers for one triple share an `eventId` yet **diverge on
`eventSetHash`**, breaking cross-mirror convergence. The reserved `inclusion_proof` slot is **not** a model
to follow: populating it provably **changes `payloadHash`** while leaving identity unchanged
(`src/session/event/event_signature.rs:166`), so it is tolerated only as an **unproduced, unconsumed v1
reserved field** — any future activation must explicitly handle its `payloadHash`/`eventSetHash` effect and
must **not** be used to carry co-signature member meaning. This is the substrate reason ADR-0013's
classification is derived (not a stored `relation` marker); a future explicit marker, if ever needed, must
be **identity-bearing**. The landing implementation **must add a cross-mirror convergence test**: for
one attestation triple, independently-minted carriers carry **no payload meaning/relation field** and keep
**identical `idempotencyKey`, `eventId`, `payloadHash`, and `eventSetHash`**, while envelope-only fields
(e.g. the carrier `writer`) may differ without affecting convergence (`eventSetHash` ignores them,
`freshness.rs:23`).

#### Trust-set locality: `allowed-signers.json` stays committed-only (no `.local.json`)

The trust set (`allowed-signers.json`, this ADR's "Identity And Trust") **remains committed-only**. There
is intentionally **no `allowed-signers.local.json`** layer, even though ADR-0010's `delegates.json` and
ADR-0012's `actor-attributes.json` carry git-excluded `.local.json` overrides. This asymmetry is now a
**deliberate decision**, not the accidental gap it has read as:

- The trust set decides `valid` vs `untrusted_key`, which feeds `has_valid_member()` → **binding** →
  operative evaluation. A local, git-excluded trust override would make `valid`/binding **diverge silently
  and un-auditably per machine** — and operative actions taken on a locally-bound view (commits, handoffs,
  "this is authoritative") propagate even though their trust *basis* does not. Trust is already
  reader-relative across clones; a `.local.json` would add the dangerous kind of divergence: silent,
  non-portable, and un-auditable, on the one config that gates authenticity.
- Trust is the high-stakes, should-be-shared decision. The right way to say "I trust this key" is the
  **committed** file, where it is a reviewable `git log -p` diff that grows the team's shared trust set.
  (Contrast: a `delegates.local.json` / `actor-attributes.local.json` override changes only the local
  reader's own accountability/descriptive view — legitimately per-operator and low blast-radius.)
- The dev/onboarding cost is acknowledged and accepted: a human writing under `actor:git-email:…` signed
  by a `did:key` must **commit their enrollment** to render their own events `valid` (the self-certifying
  shortcut, `trust.rs:53-59`, only helps `did:key` *actors*, not a git-email actor signed by a `did:key`).
  That one-time, auditable commit is treated as a property of a trust root, not friction to remove.

### Backward Compatibility

No event bytes, `payloadHash`, `eventRecordHash`, member identity, or `sigVersion` change. `has_valid_member()`
and ADR-0009 binding are behaviorally identical (every `Valid` member is `authoring`; endorsement members
are detached `UntrustedKey` and were never counted). The classification and `has_trusted_endorsement()` are
purely additive read surfaces. The trust-set-locality decision changes nothing in code — it records the
existing committed-only behavior (`discover_trust_set`, `src/cli/review/common.rs:81-86`) as intentional.

### Consequences

#### Accepted

- ADR-0004's co-signature member model gains an endorsement reading without any byte, identity, or
  `EventVerificationStatus` change — recorded here as a substrate extension, with semantics owned by
  ADR-0013.
- The **convergence invariant** is generalized beyond endorsement: it now constrains *any* future
  co-signature member meaning (derive or be identity-bearing; never an excluded-from-identity payload
  field), with a mandated cross-mirror test.
- The `allowed-signers.json` committed-only posture is now a **deliberate, written** decision, ending the
  "accidental asymmetry" reading and keeping the trust root shared and auditable.

#### Rejected

- **A carrier-payload classification marker** (a stored `relation`/endorsement field excluded from member
  identity) — breaks `eventSetHash` convergence; see the invariant. An identity-bearing marker is the only
  stored alternative and is deferred (ADR-0013).
- **`allowed-signers.local.json` for v1** — per the blast-radius rationale above; the committed file is the
  correct, auditable place to extend trust.

### Revisit Triggers

- **A real dev-local-trust-tier demand materializes** → revisit `allowed-signers.local.json` **only** with
  hard guardrails: a **loud, per-member "locally-trusted-only" marker in rendered output** (never silent),
  and a hard boundary that locally-trusted verdicts **never cross egress/federation** (the relay and other
  readers verify against their own shared trust config, so a local override may affect only local CLI
  evaluation — and that effect must be visible, not silent). Absent those guardrails, committed-only stands.
- Endorsement-classification revisit triggers live in **ADR-0013**.

## Amendment: Opaque-Coded Signed Identity, the View-Upcast Seam, and the Storage Descriptor

The original ADR-0004 decision stands and this file's top-level **Status remains Accepted**: the signing
mechanism — DSSE pre-authentication encoding, strict Ed25519, `sigVersion = 1`, the
`valid / invalid / untrusted_key / unsigned` status vocabulary, and the co-signature member model — is
unchanged. Unlike the two prior co-signature amendments, this one is **not** byte-neutral: it introduces
**one deliberate, owner-migrated signed-store break** confined to the *content* of the signed view (the
identity tokens it binds), landed as a single clean migration with no dual-read of signed bytes.

### Context

ADR-0004 signs a human-readable producer view: `EventToBeSigned` binds `eventType` as the snake_case
wire string (`tbs.rs:22`, via `EventType::as_str`, `kind.rs:24-46`) and binds the whole `EventTarget`
(`tbs.rs:25`), whose `subject` carries renamable variant/kind tags and id strings. Worse, the event's
content identity folds those same human names: `eventId = sha256(idempotencyKey)` (`mod.rs:137-139`),
and every idempotency-key builder prefixes the human event-type string and the renamable ids — e.g.
`"review_observation_recorded:{revisionId}:{trackId}:{sourceKey}"` (`observation.rs:35-41`). So a pure
*rename* — of an enum tag, a scope prefix, a digest-key name, a target variant — changes the signed
bytes **and** the `eventId` → `eventSetHash` (`freshness.rs:16-43`), forcing a full signed-store
migration even though the *meaning* of the recorded fact never changed.

An audit of the breaks spent across the three most recent signed-store migrations makes the cost
concrete: **of the seven breaks that touched the signed event view, six were naming or reshaping** of an
unchanged referent (the `review_unit`→`revision` family rename, the `EventTarget` reshape, the
`Ledger`→`Journal` scope-prefix migration, the `reviewUnitId`→`revisionId` digest-key rename, the
generative-move type collapse, the `intervention_*`→`input_request_*` rename). The lone *whole* genuine
payload-semantics break in the window was the `review_disposition_recorded` → assessment split — a
concept that genuinely became a differently-shaped fact, which *must* be identity-bearing. The treadmill
is overwhelmingly self-inflicted by signing the human-readable token rather than a stable identity.

This amendment removes the renamable material from the signed identity and from the content-id derivation
so future renames/reshapes become *projection-only* changes presented by a read-time view upcast, with no
migrator. It does three mutually-compatible things, and is deliberate about the **two break classes it
does not address** (see *Honest scope*).

### Decision

#### D1 — Opaque-code the signed `eventType`; reduce the signed `target` to content-derived identity

Sign **stable opaque identity tokens** instead of renamable human strings:

1. **`eventType` → a frozen `TypeCode`.** Each logical event family is assigned an opaque type code once,
   from an **append-only, never-reassigned registry**: the code is fixed when the family is first
   introduced; renaming the Rust variant or its display string never changes the code; a retired family
   keeps its code reserved forever so old signed events stay decodable. The signed view binds the
   `TypeCode`, not the snake_case name. The strict decoder decodes by code; `EventType::as_str`
   (`kind.rs:24-46`) becomes a **display lookup** the projection reads, not a signed/identity value.

   *Type-code form:* the code is a **short opaque counter token** in the frozen registry (e.g. `"t:07"`),
   human-traceable through the registry, which is the single source of truth. The alternative — a sha256
   of a frozen canonical descriptor (no registry, but opaque to debugging and itself a re-derivation
   surface) — is recorded under *Rejected*.

2. **`target` → `{ journalId, opaque subjectId?, trackId? }`.** `journalId` (content-derived scope key)
   and `trackId` (identity-bearing attribution lane) stay signed unchanged. For a subject-bearing carrier
   the signed `subject` collapses to an opaque **`subjectId` = sha256 over the subject's identity-bearing
   fields only** — the `revisionId` and any sub-ids (observation id, assessment id, line range, file
   path) — and **not** the renamable variant/kind tag strings. The structural interpretation (work-object
   kind, sub-anchor kind, the human-readable revision/sub-id spellings) is reconstructed by a projection
   from the subject structure the events already carry in their **payload**.

   **`subjectId` is absent for the fieldless `TargetRef::Journal` carrier** — the genuinely subject-less
   events (the detached co-signature carrier, content removal, and pre-revision journal events such as
   `ReviewInitialized` and `EventSignatureRecorded`; `work_object.rs:46-56`, `target.rs:44-49`). These
   already address their real target by payload content, never by a duplicated envelope subject, so the
   opaque target for a journal carrier is just `{ journalId, trackId? }` with no `subjectId`. This mirrors
   the existing fieldless `TargetRef::Journal` exactly — opaque-coding does not invent a subject where the
   current model has none.

3. **The domain axis is structurally derived, never a separately-signed field.** Per ADR-0017 §A4 (one
   domain axis, structurally derived, never an independently-asserted wire value), the opaque `subjectId`
   **must not** smuggle the `Review`/`Task` domain back as a signed tag. Domain is derived at the write
   boundary by `EventTarget::for_generative_move`'s engagement-type check (`target.rs:57-71`) and at read
   time over the projected subject. This generalizes the existing `for_generative_move` discipline (which
   already derives domain rather than storing it) one notch: the signed envelope stores neither the domain
   nor the renamable subject structure — only the opaque content-id.

4. **Re-base the idempotency keys onto the opaque tokens (the load-bearing step).** Opaque-coding the
   signed *fields alone is insufficient*: because `eventId = sha256(idempotencyKey)` and every key builder
   bakes the human type string and renamable ids into the key, the `eventId` would still fold the old
   names. The builders must prefix the **stable `TypeCode`** and fold the **opaque subject/revision
   content-id**, not the human strings, so that after the re-base a future rename touches neither the
   signed `TypeCode`/`subjectId` nor the `eventId`. Subject-less journal carriers have no subject id to
   fold: `ReviewInitialized` keys on `journalId` (`review.rs:12`), and the co-signature carrier keys on the
   target's signer-exclusive `eventRecordHash` plus the attesting signer (`event_signature.rs:66`) — itself
   re-derived during the migration when the target re-keys (the co-signature re-home of `store-migration.md`
   §3). For these the re-base is the `TypeCode` prefix only; the renamable human type string is what leaves
   the key.

5. **This is one final signed-store break, gated by the content-id-convergence test.** Re-basing the
   idempotency keys is a content-id re-derivation — exactly the frozen-content-id hazard
   `docs/store-migration.md` §8 warns about. It MUST re-derive every affected content id (`eventId`, and
   any content id that folds the renamed inputs) via the **live builders**, in dependency order, remapping
   references — never carry an old id string forward — and be gated by a **convergence test**
   (`events_created == 0` on a fresh re-record of a fact the migrated store already holds), not the
   self-check alone. It lands as one owner-run migration over the real stores, the same discipline as the
   prior signed-store breaks.

6. **`sigVersion` stays `1`.** The signing mechanism is unchanged and the break is clean: the old shape is
   fully migrated and rejected by the strict reader, so no two signature schemes coexist and no verifier
   dispatch is needed. The `EventToBeSigned` field set is adjusted (the `eventType` field binds a
   `TypeCode`; a subject-bearing `target`'s subject becomes the opaque `subjectId`, while a
   `TargetRef::Journal` carrier omits it per D1.2) and the golden signing vectors are re-minted as part of
   the one break — the same way prior breaks re-minted ids and hashes without bumping `sigVersion`. Bumping
   to a `sigVersion = 2` / `event-tbs.v2` payload type is recorded under *Rejected*.

#### D2 — Re-admit a read-time **view** upcast seam (a bounded, layered reversal of "no silent dual-read")

`docs/store-migration.md` §1 ("no silent dual-read") governs the **signed identity** of an event. This
amendment re-admits a read-time **view** upcast for a *different* class of change — re-*interpreting* an
event whose signed identity is unchanged — and reconciles the two as **two rules split by layer**:

- **Clean break + migrator for signed-identity changes** (`eventType`/`target`/`payloadHash` and anything
  those digests bind). Unchanged; this is D1's and §1's domain.
- **Read-time view upcast for interpretation changes.** A pure `upcast(old_value) -> current_model` runs
  **in the projection layer only**, on the value the projection was going to deserialize anyway (e.g. the
  `WorkObjectProposedPayload` decode in `revision_projection/resolving.rs`). It re-presents a payload or
  target in the current in-memory model. It never writes, never re-serializes back to stored bytes, and
  never re-derives a digest.

This is **signature-safe by construction**: every digest (`payloadHash` at `mod.rs:135`, the to-be-signed
bytes at `tbs.rs`, `eventRecordHash` at `record_hash.rs`, `eventSetHash` at `freshness.rs`) is computed by
a `from_event`/`from_events` builder **over the stored `ShoreEvent`**, never over a projection view, and
verification rebuilds the to-be-signed bytes from the stored event. The standing existence proof is the
`ingest`/`sourceRef` exclusion: a hop already stamps those fields on a signed event at rest and the
signature still verifies. An upcast that reads `event.payload` into a richer in-memory model leaves every
digest input byte-for-byte unchanged.

**The seam's key is a payload-level, hash-excluded *view version* — not the envelope `version`.** The
envelope `version` (`ShoreEvent.version`, currently `EVENT_VERSION = 1`) is **reject-only on read**:
`read_event` → `validate_event` → `validate_schema_version` rejects any non-current envelope version
*before* any projection code runs, so an older-envelope event never reaches the upcast and a bumped
envelope version is rejected by the strict reader. The seam therefore keys on the **`payloadVersion`**
field reserved in D3 — a per-payload view version that rides *inside* an accepted envelope and is
**hash-excluded** (so bumping it is not itself a payload break), sharing the `ingest` stamp's exclusion
discipline. (The fallback — relaxing `validate_schema_version` to an accept-range of view versions —
loosens the strict envelope contract and is the less-preferred option, recorded under *Rejected*.)

**The boundary is the whole point.** The view upcast **MAY** re-present a payload or target in the
projected view; it **MAY NEVER** change `payloadHash`/`eventType`/`target` as signed, nor any of
`{ signed bytes, eventRecordHash, eventSetHash }`, nor **re-serialize the upcast view to re-derive a
digest**. That re-serialize-and-re-hash move is the §8 "re-key the payload, carry the old id forward"
anti-pattern in another guise and would silently fork the store; it is forbidden. When the signed
identity itself must change, you are back in §1's clean-break discipline.

This amendment **drives a `docs/store-migration.md` §1a edit** stating this layer split; the landing plan
applies it. The seam is *preventive* — no interpretation change is waiting for it today — so the landing
plan lands the doctrine text, the signature-safety invariant, the verify-next signature-safety test, the
hash-excluded view-version field, and a single targeted first upcast, and **defers the per-family hook
generalization until a second family needs it** rather than building all hooks speculatively.

#### D3 — Model the storage descriptor: identity-bearing content type, a hash-excluded encoding pipeline, and the view version

Storage carries three distinct concerns, modeled on HTTP's `Content-Type` / `Content-Encoding` split.
**Identity is computed over the fully-decoded canonical content, never over the stored encoded bytes** —
the encoding is reversed *before* the content is hashed, signed, or verified (this is the rule that makes
the hash-excluded encoding safe; it is the D4 invariant applied to the storage boundary).

1. **`contentType` — what the decoded object *is* (identity-bearing; modeled here, not reserved now).**
   The media type of the fully-decoded object. Changing it changes *what the object is* (the same bytes as
   `application/json` vs `text/plain` are different objects), so it belongs **inside** the content hash —
   in the hashed artifact body alongside the existing `schema` tag (`object_artifact.rs`: `schema` is
   already an identity-bearing type tag in the artifact's `contentHash`), **never** in the hash-excluded
   set. To keep content-addressing convergent it enters the hash as a **normalized canonical type tag** (a
   closed kind enum, or a canonicalized media-type string — lowercased type/subtype, parameters dropped or
   canonically ordered), **never a raw MIME string** (raw MIME's case-insensitivity, `charset=`/`boundary=`
   parameters, and `+json` structured suffixes would let two producers of the same content diverge — a raw
   MIME string is a poor hash input).
   **This amendment does not reserve `contentType`.** For events it is the constant `application/json`; for
   the current artifact it is *derivable from `schema`* (a `shore.object` is JSON by construction), so it is
   **not yet an independent identity axis** — it is implied by the already-hashed `schema`. It becomes a
   real field only when content under one schema/kind can legitimately differ in media type (the first
   note-body or binary blob). Adding it then re-hashes artifacts, which is the **artifact-digest break
   class this amendment scopes out** (see *Honest scope*): modeled now, added there.

2. **`contentEncoding` — how the object is stored (hash-excluded; reserved now).** An **ordered list** of
   content-coding tokens (compression and/or encryption) applied in list order at write and **reversed on
   read** — exactly HTTP `Content-Encoding`. Default `[]` (identity: the stored bytes are the canonical
   content). Example: `["zstd", "aes128gcm"]` means compress then encrypt; a reader decrypts then
   decompresses. Each token names a registered codec whose own parameters ride **in-band** in its
   self-describing frame (e.g. an RFC 8188 `aes128gcm` body carries its salt/keyid), so the list stays
   simple tokens. Policy: **encryption codings are outermost** (applied last), preserving the
   compress-then-encrypt order. The encoding applies to the **payload/body content** (what `payloadHash` /
   the artifact `contentHash` cover); the **envelope carrying `contentEncoding` stays plaintext**, so a
   reader reads the list before decoding the body. This field **replaces the single `codec` discriminator
   the prior draft proposed** and rides on the event record **and** the artifact (a blob's encoding is
   independent of an event's). Granularity is per-record / per-blob (one file per event/blob; Shoreline has
   no chunk concept), finer than a file-level transform and strictly better for content-targeted erasure.

3. **`payloadVersion` — the decoded payload's view version (hash-excluded; reserved now).** D2's
   view-upcast seam key, a `u32` **defaulting to `1`** (absent reads as `1`), bumped when a payload
   family's view interpretation changes. Orthogonal to `contentType`: `contentType` says "this is JSON,"
   `payloadVersion` says "this JSON is at view version N."

**The exclusion is exact and mandatory for the two reserved fields.** `EventRecordView` is the whole record
minus exactly `signer`/`signature`/`sourceRef`/`ingest` (`record_hash.rs:25-39`), and `EventToBeSigned` is
the 10-field view (`tbs.rs:19-30`). Both `contentEncoding` and `payloadVersion` are added to the
**excluded** set: they appear in **neither** `EventToBeSigned` **nor** `EventRecordView`, and they are
**not** inputs to `payloadHash` (`mod.rs:135`, over the payload value only) **nor** to `eventSetHash`
(`freshness.rs:18-20`, over `{eventId, payloadHash}`). They ride envelope-adjacent exactly like the
`ingest` stamp. Adding or bumping either is **signature-neutral** — default `[]`/`1` mean zero behavior
change today, no `sigVersion` bump, no re-hash, no convergence effect.

**Why the encoding pipeline is fork-free.** Because identity is over the decoded content and
`contentEncoding` is hash-excluded: two stores that compress/encrypt the same object differently still
**converge** (same content hash); a signed blob still **verifies** after re-encoding; and recompression or
encryption-key rotation / per-content-hash crypto-shred is **fork-free**. A missing codec token is a typed
"unsupported encoding" error, not a guess. `contentEncoding` needs **no break to add on its own** (the
default is identity); both reserved fields can ride D1's break for free if landed together. The downstream
encryption-boundary decisions (whole-blob delete + crypto-shred, sign-then-encrypt over plaintext) are out
of scope here and tracked separately.

#### D4 — The signature-safety invariant (the load-bearing premise of D2)

**Every event digest is computed over the stored `ShoreEvent`, never over a projection or upcast view.**
This is the invariant that makes the view-upcast seam safe and the boundary enforceable: a projection may
read and re-present stored bytes arbitrarily, but the moment any code re-serializes a view and feeds it
back into a digest, the store can silently fork. Stated as a rule for future work: a read-time transform
that re-presents an event is permitted; a read-time transform that re-derives `payloadHash`, the
to-be-signed bytes, `eventRecordHash`, or `eventSetHash` from anything other than the stored event is
forbidden. The landing plan ships a test that computes all four digests, runs the upcast against the
projected view, asserts byte-identical digests and a still-`valid` signature, and asserts the upcast did
something observable (so a no-op cannot pass vacuously).

#### D5 — Convergence-invariant compliance for the moved fields

This amendment's moves honor ADR-0004's existing convergence invariant (a signed-view field's meaning must
be **derived at projection or identity-bearing, never an excluded-from-identity payload field**):

- **`TypeCode`** is identity-bearing — folded into the idempotency key (D1.4), hence into `eventId` and
  `eventSetHash`, signed directly, and in `eventRecordHash`. Two independently-minted carriers of one
  logical event share the same code → same key → same `eventId` → same `eventSetHash`. ✔
- **`journalId` / `trackId`** are unchanged content-derived/identity-bearing fields. ✔
- **`subjectId`** is *derived* (sha256 over the subject's identity-bearing fields) and *identity-bearing*
  once folded into the idempotency key. The renamable subject *structure* it replaces is carried in the
  **payload**, where it is bound by `payloadHash` exactly like all other payload content — so it is
  **convergent-as-stored, not "outside convergence":** `payloadHash` is itself signed (`tbs.rs:24`), is
  half of `eventSetHash` (`freshness.rs:18-20`), and a same-idempotency-key/different-`payloadHash` write
  is a hard conflict (`event_store.rs:78-82`). The stored payload bytes are therefore frozen and canonical,
  and two carriers of the same fact must agree on them to converge. What opaque-coding buys is **not** that
  the payload structure escapes convergence; it is that the subject *identity* the dedup turns on is the
  opaque `subjectId` in the signed view (unaffected by a display rename), so a future rename of that
  structure can be handled two ways: a **display** rename is a read-time view upcast (D2) that re-presents
  the frozen stored bytes without re-hashing; a change to the **stored payload shape** is a genuine payload
  break on the clean-break + migrator path, never a view upcast. ✔

The one hazard is D1.5's re-base: re-deriving the opaque ids is itself a content-id re-derivation and must
be gated by the §8 convergence test, or the store silently non-converges.

### Honest scope — what opaque-coding does NOT address

Opaque-coding `eventType` + `target` addresses **only the signed-view break class**. Two enumerated break
classes are explicitly **out of reach** and must not be counted toward this amendment's leverage:

- **Artifact-digest breaks** (the snapshot v1→v2 body shrink; the `shore.snapshot`→`shore.object` schema
  rename). These re-hash `ObjectArtifact.contentHash`, which is the artifact's **own** content hash and
  out of band from any event digest. They are governed by content-addressing and are
  convergence-motivated, a different class entirely. **Adding the identity-bearing `contentType` (D3) to a
  hashed artifact body is in this class** — modeled now, taken when the first non-JSON blob lands.
- **`eventRecordHash`-only envelope breaks** (the retired writer role-field removal;
  `writer.tool`→`producer`). The signed
  view carries only `actorId`, not the full `writer`; `writer` is in `eventRecordHash` (`record_hash.rs`)
  but **never reached the Ed25519 signature**. These are provenance-field renames, not signed-view
  changes.

And the lone genuine payload-semantics split (`review_disposition_recorded` → assessment) is **correctly
identity-bearing**; opaque-coding cannot and should not save it. The policy is "stop paying for renames,"
not "stop ever breaking."

### Resolved design questions

| # | Question | Resolution |
| - | -------- | ---------- |
| 1 | Does opaque-coding the subject as one content-id absorb the non-optional-`subject` invariant and the generative-move discriminator, or must the domain axis stay a signed field? | The domain axis is **structurally derived, never a separately-signed field** (ADR-0017 §A4). The `subjectId` digests identity-bearing fields only; the non-optional-`subject` requirement is a **structural-validation rule** at the type layer, not a signed-byte break; the generative-move discriminator is identity-bearing inside the `subjectId`. |
| 2 | Type-code form: counter token vs descriptor hash? | **Frozen append-only counter token** in a registry that is the single source of truth (human-traceable). Descriptor-hash recorded under *Rejected*. |
| 3 | The seam's durable key. | The hash-excluded **`payloadVersion`** field (D2/D3). The envelope `version` is reject-only on read and cannot be the key; a bump must not enter `payloadHash`. The accept-range relaxation is the less-preferred fallback (*Rejected*). |
| 4 | `sigVersion` bump? | **No — stays `1`** (D1.6). Clean break, full re-sign, no coexistence, mechanism unchanged. |
| 5 | Sequencing of D1 and D2. | Land the **ADR + §1a doctrine + signature-safety invariant + verify-next test + the `payloadVersion` field + a single first upcast** together; land D1's opaque-coding break and idempotency re-base as the one owner-migrated signed-store break (D3's `contentEncoding`/`payloadVersion` fields can ride it); **defer per-family upcast hooks** until a second family needs one. |
| 6 | Storage descriptor: one `codec` token, or richer? | **Three concerns, modeled on HTTP (D3):** `contentType` (what it is — **identity-bearing**, modeled now / added with the first non-JSON blob, a **normalized** type tag never raw MIME), `contentEncoding` (how it's stored — a **hash-excluded ordered token list**, compression + encryption, reversed on read, encryption outermost; **reserved now**, replaces the single `codec`), and `payloadVersion` (the view version — hash-excluded, reserved now). |
| 7 | Is `contentType` inside the identity hash? | **Inside** (it is *what the object is*), in the hashed artifact body alongside `schema`, as a **normalized canonical type tag, never raw MIME** (raw MIME would break convergence). Hash-excluding it would let one `objectId` carry contradictory type claims. Not reserved now — today derivable from `schema`; added at the first non-JSON blob (an artifact-digest break). |

### Backward Compatibility

- This amendment is **not** byte-neutral (unlike the two prior co-signature amendments): D1 is a deliberate
  signed-store break that re-keys the signed view and re-derives content ids, landed as **one clean,
  owner-run migration** over the real stores. There is **no dual-read of signed bytes** — the strict reader
  rejects the old shape; the one-shot migrator bridges it. `sigVersion` stays `1`.
- D2's view upcast and D3's `contentEncoding`/`payloadVersion` fields are **additive and
  signature-neutral**: they touch no stored-byte digest and need no `sigVersion` change.
- The co-signature member model, the `EventVerificationStatus` vocabulary, the DSSE PAE, and strict Ed25519
  verification are **unchanged**. Co-signature attestation still signs the target's `EventToBeSigned` view;
  after D1 that view carries the opaque `TypeCode` (and the opaque `subjectId` for a subject-bearing target;
  a journal carrier omits it per D1.2), but the attestation contract is otherwise intact (the verifier
  reconstructs the opaque-coded view for the target).

### Consequences

#### Accepted

- Future renames and structural reshapes of the signed view become **projection-only** changes the view
  upcast presents with **no migrator** — retiring the rename-per-break treadmill.
- The signed identity stops folding human-readable names; identity is the opaque `TypeCode` + content-ids,
  which is what `eventId`/`payloadHash` already are.
- The cost is **human-readable signed bytes** (you can no longer eyeball `eventType: "..."` in the signed
  material). This is accepted; the substrate inspector (ADR-0017 §A4) is the natural home for a code→name
  decode, moving readability off the signed bytes.
- "No silent dual-read" is preserved exactly for signed identity; a *bounded, layered* exception is added
  for interpretation, with a sharp, testable boundary (D4).
- The `contentEncoding` pipeline makes future compressed/encrypted storage self-describing at zero cost
  today, and fork-free thereafter (hash over decoded content, encoding excluded) — recompression and
  encryption-key rotation / crypto-shred never re-key the store.

#### Rejected

- **Opaque-coding the signed fields without re-basing the idempotency keys** — insufficient; the `eventId`
  would still fold the human names and a rename would still re-key the store (D1.4).
- **A descriptor-hash type code** with no registry — opaque to debugging and itself a re-derivation surface;
  the frozen counter-token registry is the single source of truth.
- **Smuggling the domain axis back as a signed tag inside `subjectId`** — violates ADR-0017 §A4's
  one-domain-axis, derived-not-asserted rule.
- **Bumping `sigVersion` to 2 / a `event-tbs.v2` payload type** — implies a verifier dispatch that does not
  exist after a clean full-resign break; the break is confined to the signed view's content.
- **Keying the view upcast on the envelope `version`** — reject-only on read; the event never reaches the
  projection. **Relaxing `validate_schema_version` to an accept-range** — loosens the strict envelope
  contract; the hash-excluded per-payload view version is preferred.
- **Re-serializing an upcast view to re-derive a digest** — the §8 fork-the-store anti-pattern; forbidden
  (D4).
- **Using the view upcast for a genuine payload-semantics change** — that is a signed-identity change; it
  takes D1's opaque-coding or a clean break + migrator, not the seam.
- **Letting `contentEncoding`/`payloadVersion` enter any identity hash** — would make them a hard break to
  add or bump; they stay envelope-adjacent and hash-excluded like the `ingest` stamp.
- **A single flat `codec` token, or two typed `compression`/`encryption` slots, instead of an ordered
  `contentEncoding` list** — a single token cannot express a multi-transform stack; typed slots cannot
  express arbitrary order (e.g. a future signing frame in the chain). One ordered token list, reversed on
  read with encryption outermost, is the HTTP-proven shape.
- **Hash-excluding `contentType`** — it is *what the object is*, so excluding it would let one `objectId`
  carry contradictory type claims with nothing authoritative to resolve them, and would attest bytes
  without attesting what they are. It belongs inside the identity hash.
- **Hashing a raw MIME string for `contentType`** — raw MIME's case-insensitivity, `charset=`/`boundary=`
  parameters, and `+json` suffixes are a poor hash input that breaks convergence; the hashed form is a
  normalized canonical type tag (or closed kind enum).

### Revisit Triggers

- **A genuine payload-semantics break that opaque-coding cannot demote to projection-only** arrives → it is
  identity-bearing by definition and takes the clean-break + migrator path; revisit only whether the new
  shape belongs in the opaque subject or a new signed field.
- **A second payload family needs a view upcast** → generalize the per-family `upcast(version, value)` hook
  then (deferred until the second need; see D2 and resolved question 5).
- **A real demand for compressed or encrypted storage** → populate the reserved `contentEncoding` pipeline
  with concrete codec tokens (encryption outermost) + register their codecs; the encryption-boundary
  decisions are tracked separately.
- **The first non-JSON / varying-media-type blob** (a note body, a binary artifact) → add the
  identity-bearing `contentType` to the hashed artifact body alongside `schema`, as a normalized canonical
  type tag — an artifact-digest break, out of this amendment's scope but modeled in D3.
- **The frozen type-code registry is ever tempted to reassign a retired code** → forbidden; a reassignment
  would un-decode historical signed events. Reopen only to add new codes, never to reuse.
- **An artifact-digest or `writer.*` envelope break recurs** → out of this amendment's scope; addressed by
  content-addressing and the storage-model record respectively, not by opaque-coding.

**Status:** Accepted; the one signed-store break this amendment authorizes (the opaque-coded signed
identity migration) lands with its owner-gated implementation work. The signature-neutral pieces —
the reserved storage-descriptor fields and the view-upcast doctrine — land ahead of the break.
