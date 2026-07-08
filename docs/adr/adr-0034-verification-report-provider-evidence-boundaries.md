# ADR-0034: VerificationReport And Provider Evidence Boundaries

**Status:** Accepted (owner-approved 2026-07-08); landed in-repo via plan 0124.
**Date:** 2026-07-08
**See also:** [ADR-0003](./adr-0003-agent-resource-claims-advisory-first.md),
[ADR-0004](./adr-0004-event-signatures.md),
[ADR-0008](./adr-0008-cross-peer-conflict-policy.md),
[ADR-0009](./adr-0009-resumption-binding-trust-source.md),
[ADR-0010](./adr-0010-actor-identity-and-delegation.md)

## Context

ADR-0004 gives every event an optional v1 producer signature: raw Ed25519 over DSSE
pre-authentication encoding for the explicit `EventToBeSigned` view. The effective signer is a
`did:key`, and the public verifier collapses to four values:

```text
valid / invalid / untrusted_key / unsigned
```

That shape is intentionally small. It lets old unsigned stores remain readable, keeps verification
reader-relative, and avoids storing verification status as event authority. It is also too small for
the next layer of real-world signing infrastructure. OpenSSH and Git can discover useful signing
keys and allowed-signers conventions. SSH certificates can bind a leaf key to principals and
validity windows. Sigstore bundles can attest CI or hosted-agent identity. OpenPubkey, OIDC, and
KMS/HSM systems can explain identity, custody, or workload context. None of those systems alone
answers the Pointbreak question: is this signer authorized, under this reader's policy, to speak for
`writer.actorId` in this repo context?

The current implementation already hints at the needed split. `verify_event_signature` checks
signature integrity and then asks `TrustSet::authorizes(actor, signer, occurred_at)`, while
`TrustSet` currently recognizes only self-certifying `did:key` actors or a direct actor-to-signer
map and ignores the timestamp. `EventVerificationPolicy` consumes only the four-value
`EventVerificationStatus`. `PrincipalPolicy` separately answers whether an agent actor resolves to a
sufficient responsible principal. Provider integrations need a richer internal explanation without
turning any provider into the core trust root.

## Decision

### 1. Keep the v1 event-signature contract stable

The v1 producer signature remains the ADR-0004 shape:

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

`EventToBeSigned`, DSSE pre-authentication encoding, `EventSignature`, `sigVersion = 1`, and the
public `EventVerificationStatus` ladder are unchanged. Provider certificates, OIDC claims,
transparency-log bundles, hosting-account lookups, KMS/HSM custody statements, and other external
evidence are not added to the v1 `signature` object and are not folded into `EventToBeSigned`.

Unsupported or future signature suites must use an explicit suite marker and verifier path. They
must not reinterpret `alg: "ed25519", sigVersion: 1`.

### 2. Add `VerificationReport` as the internal verifier result

Pointbreak introduces an internal, derived `VerificationReport` for reads, ingest diagnostics, and
future export/publish gates. It is a read-model result, not stored event authority. The first report
shape carries these logical fields:

```text
VerificationReport {
  event_status: EventVerificationStatus,
  signature_integrity: SignatureIntegrity,
  key_trust: KeyTrust,
  actor_binding: ActorBinding,
  authorization: Authorization,
  principal_sufficiency: PrincipalSufficiency,
  evidence: Vec<EvidenceFinding>,
  reasons: Vec<VerificationReason>
}
```

The field meanings are:

| Field | Meaning |
| --- | --- |
| `event_status` | The ADR-0004 public ladder: `valid`, `invalid`, `untrusted_key`, or `unsigned`. |
| `signature_integrity` | Whether the signed event bytes verify under the claimed key and suite. |
| `key_trust` | Whether reader policy trusts the key, CA, issuer, bundle, custody source, or enrollment evidence. |
| `actor_binding` | Whether trusted evidence binds the signer or identity to the claimed `writer.actorId`. |
| `authorization` | Whether the signer or bound identity is allowed to speak for that actor in the reader's repo/policy context. |
| `principal_sufficiency` | Whether any required responsible-human principal policy passes for agent actors. |
| `evidence` | Normalized findings about provider evidence, including provenance and verification result. |
| `reasons` | Stable diagnostic reason codes plus human-oriented messages. |

This ADR fixes the dimensions, not every Rust enum variant. The implementing plan may tune variant
names, but it must preserve the separation between signature integrity, key trust, actor binding,
authorization, evidence provenance, and principal sufficiency.

### 3. Collapse the report back to the public ladder

`event_status` is derived from the event-signature dimensions only:

- `unsigned`: no producer signature or accepted explicit future signature suite is present.
- `invalid`: the present signature or suite is malformed, unsupported for the selected verifier,
  non-canonical, mismatched, or fails byte-level verification.
- `untrusted_key`: signature integrity passes, but key trust, actor binding, or authorization does
  not pass under the selected reader policy.
- `valid`: signature integrity passes and key trust, actor binding, and authorization all pass.

`principal_sufficiency` is deliberately not part of this collapse. It may cause a higher-level
binding predicate, review policy, ingest gate, export gate, or publish gate to reject or warn, but it
does not change an event's `EventVerificationStatus`. A valid signature by an enrolled agent can
still be principal-insufficient; an unsigned event can still have a resolvable responsible principal.

Malformed provider evidence follows the same boundary. If the provider evidence is merely optional,
its failure is an evidence diagnostic and does not make an otherwise valid v1 Ed25519 event invalid.
If reader policy requires that evidence for trust, the event becomes `untrusted_key` when the
producer signature itself still verifies. It becomes `invalid` only when the failing material is the
selected signature suite itself.

### 4. Normalize provider evidence outside signed event bytes

Provider integrations feed a resolver through normalized evidence findings. The resolver accepts two
kinds of provider input, and the difference is load-bearing:

- **Verification evidence** is evaluated against a concrete event signature or detached attestation
  while building `VerificationReport`.
- **Enrollment evidence** helps a reader stage or explain trust policy, but it is not authorization
  by itself.

The logical target of verification evidence is one of these shapes:

```text
VerificationEvidenceTarget::EventSignature {
  event_id,
  event_record_hash,
  signer,
  signature_suite,
  signature_digest?
}

VerificationEvidenceTarget::DetachedAttestationMember {
  target_event_id,
  target_event_record_hash,
  attesting_signer,
  signature_suite,
  attestation_signature_digest
}
```

`event_record_hash` is the current `EventRecordView` hash: the stored event record minus exactly the
hash-excluded envelope metadata (`signer`, `signature`, `sourceRef`, `ingest`, `contentEncoding`,
and `payloadVersion`). Binding verification evidence to both `event_id` and `event_record_hash`
prevents a sidecar or fetched result from drifting across same-id divergent content while still
allowing signed and unsigned copies of the same fact to converge. `signer` and `signature_suite`
prevent evidence for one credential or suite from being silently reused for another.

`signature_digest` is optional for evidence that explains a credential's suitability for the event
signer and suite, such as an SSH certificate whose leaf key matches the event signer. It is required
when evidence applies to one concrete signature artifact. Detached co-signature member evidence must
use `DetachedAttestationMember`, because co-signature member identity is the full attestation triple:
target record hash, attesting signer, and attestation signature bytes.

The first implementation does not need to settle the physical carrier. Provider evidence may arrive
from an adjacent event family, a content-addressed artifact, an export sidecar, a local evidence
cache, or an explicit live fetch. Any carrier must present the same logical boundary to the
resolver: versioned evidence type, target, raw or content-addressed payload, source, obtained time,
optional freshness/expiry metadata, and verification result. The verifier reports whether evidence
was embedded, adjacent, fetched live, cached, stale, missing, malformed, or rejected.

Enrollment evidence has its own shape:

```text
EnrollmentEvidence {
  subject,
  key_or_signer,
  provider,
  provider_account_or_principal?,
  observed_at,
  source,
  expires_at?
}
```

Enrollment evidence can propose, refresh, or explain entries in the committed trust policy. It does
not produce `valid` unless that policy authorizes the key or identity for the actor. Authoritative
trust remains the committed `.shore/allowed-signers.json` decision from ADR-0004; there is no
silent `allowed-signers.local.json` layer. A local evidence cache may avoid refetching provider
data or stage an enrollment change, but it cannot silently turn `untrusted_key` into `valid`. If a
future command deliberately selects a non-portable local trust policy, the report must identify that
local-only policy mode explicitly, and that mode needs its own decision record before it can feed
binding-sensitive behavior.

Network fetch is never implicit in the ordinary local read path. A read or ingest can use cached or
local evidence by default; live provider lookup requires an explicit command or policy mode and must
be visible in `VerificationReport.evidence`.

### 5. Providers are evidence sources, not automatic authorization

Provider-specific rules follow from the same dimensions:

- OpenSSH/Git config discovery is enrollment help. `gpg.format=ssh`, `user.signingKey`,
  `key::ssh-ed25519`, allowed-signers files, and local SSH public keys can propose reviewed
  `.shore/allowed-signers.json` updates, but discovery alone does not make an event `valid`.
- GitHub and GitLab signing keys are enrollment evidence. A hosting provider can say that an account
  advertises a key; Pointbreak policy still decides whether that account or key may speak for
  `writer.actorId`.
- SSH certificates are optional identity evidence over the existing event signer. The resolver must
  verify the certificate signature, ensure the certificate leaf key matches the event signer, map
  principals to actor policy, enforce namespace/scope, respect critical options, and apply
  revocation and validity windows.
- Sigstore bundles are CI/export/publish evidence first. They may attest a stable Pointbreak digest
  as adjacent evidence. A first-class Sigstore signature suite can be added later only with an
  explicit suite marker and verifier path.
- OpenPubkey, OIDC workload identity, KMS/HSM custody, and similar systems explain identity or key
  custody. They do not replace repo/team authorization or agent principal policy.

### 6. Time policy is explicit

Short-lived provider evidence needs an explicit time basis. The event's signed `occurredAt` is useful
for advisory diagnostics, but it is producer time and can be backdated by a compromised signer. A
policy may use `occurredAt` for low-friction local trust windows, but strict acceptance of expired
short-lived credentials requires a trusted insertion, witness, timestamp, transparency-log bundle,
or equivalent configured proof.

The first implementation should extend trust entries with validity windows and reason codes before
accepting SSH certificate, OpenPubkey, or Sigstore expiry semantics. `TrustSet::authorizes` already
receives `occurred_at`; this ADR makes that parameter semantically load-bearing once windowed trust
records exist.

### 7. Surfaces expose explanation, not new authority

`VerificationReport` is a substrate for diagnostics and policy composition, not a new default
surface. Existing CLI, inspector, and read-API surfaces that already render `verificationStatus`
keep rendering the four-value ladder as the compact default. Any richer exposure is additive,
explicit, and explanatory:

- CLI surfaces may add an opt-in explanation path, such as a `--verification-report` flag or a
  dedicated verification-explain command, that prints the report dimensions, reason codes, and
  provider-evidence provenance for selected events.
- CLI enrollment helpers may discover OpenSSH/Git/GitHub/GitLab evidence and stage reviewed
  `.shore/allowed-signers.json` edits. Discovery commands do not run implicitly during ordinary
  reads and do not authorize keys by themselves.
- Library and inspector JSON may add optional report fields beside `verificationStatus` once the
  report model exists. They must preserve the existing status field and avoid making
  provider-specific states part of the public ladder.
- The inspector may show report detail from an existing verification chip or event detail pane,
  especially for `invalid` and `untrusted_key`, but the presentation remains advisory. It must not
  make positive signature states look like a merge/acceptance gate, and it must not become a trust
  policy editor in this decision.
- Raw provider payloads are not exposed by default. Public/read surfaces should prefer normalized
  reason codes, provenance labels, and actionable enrollment hints until the adjacent evidence
  carrier and privacy policy are decided.

GitHub issues for this surface work should be cut only after owner approval of this ADR or when a
named implementation plan needs public tracking. Public issue wording should describe behavior
(`explain why signature is untrusted`, `discover Git SSH signing key for enrollment`) rather than
private planning context.

## Consequences

### Accepted

- Pointbreak keeps the small ADR-0004 public status ladder while gaining enough internal structure
  to explain provider-backed verification.
- Provider integrations compose through reader policy instead of becoming a global Pointbreak CA.
- The v1 event signature remains stable and forwardable; provider evidence can be added, omitted,
  refreshed, or rejected without rewriting event bytes.
- Principal sufficiency remains a sibling policy result, so agent responsibility checks can become
  stricter without redefining event-signature validity.
- Optional provider evidence can fail closed only when the selected policy says it is required.
- Live network lookup is explicit and auditable rather than hidden inside normal local reads.
- Short-lived credentials can start advisory and later become strict when Pointbreak has a trusted
  time or insertion proof.
- User-facing surfaces stay stable by default: the four-value verification ladder remains the
  scannable summary, and richer detail appears only through explicit explanation affordances.

### Rejected

- Making "Ed25519 plus a CA" the core abstraction.
- Treating GitHub, GitLab, OpenSSH, Sigstore, OpenPubkey, OIDC, or KMS/HSM as automatic
  authorization for `writer.actorId`.
- Adding provider bundles, certificates, account claims, or custody metadata to `EventToBeSigned`.
- Reinterpreting `alg: "ed25519", sigVersion: 1` as SSHSIG, Sigstore, OpenPubkey, or another suite.
- Expanding `EventVerificationStatus` to include principal sufficiency or provider-specific states.
- Persisting `VerificationReport` as event authority.
- Silently fetching provider evidence during ordinary local reads.
- Turning the inspector into a trust-policy editor or treating positive signature readback as an
  acceptance/merge authority.
- Filing broad provider-integration issues before the ADR is accepted and the first implementation
  slices are named.

## Revisit Triggers

- A public API needs richer diagnostics than the four-value ladder. Revisit the optional
  `VerificationReport` exposure shape without letting provider-specific states become the
  event-signature status enum.
- Users need to answer "why is this event untrusted or invalid?" from CLI or inspector workflows.
  Create a focused explanation-surface issue that consumes `VerificationReport` but does not change
  event validity.
- Users need first-class enrollment help for Git/OpenSSH/GitHub/GitLab signing keys. Create a
  focused enrollment-helper issue that stages reviewed trust-policy edits, not automatic trust.
- Inspector users need to compare report dimensions across many events. Revisit filter/facet design
  without making local display preferences or advisory trust details part of shareable query state
  unless that sharing behavior is explicitly decided.
- The adjacent evidence carrier is selected. Write a follow-up ADR or implementation plan that fixes
  the physical storage/export shape while preserving this resolver boundary.
- SSHSIG, Sigstore, OpenPubkey, or another provider becomes a first-class signature suite. Add an
  explicit suite marker, payload type, verifier, and collapse rules.
- Strict expired-certificate replay matters in practice. Choose the trusted insertion or timestamp
  primitive before enforcing expired short-lived credentials as `valid`.
- Provider evidence creates privacy or retention issues. Revisit which evidence is cached, embedded,
  redacted, or fetch-only.
- Principal sufficiency proves inseparable from event-signature status. Reopen ADR-0010 and this ADR
  together rather than silently folding principal policy into `EventVerificationStatus`.
