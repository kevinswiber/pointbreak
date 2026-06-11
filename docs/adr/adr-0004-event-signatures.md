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
- `sigVersion` is not inside the to-be-signed view; it selects the verifier path and payload type.

The to-be-signed view excludes `payload`, `sourceRef`, `signature`, `sigVersion`, and future
hop-added metadata.

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
