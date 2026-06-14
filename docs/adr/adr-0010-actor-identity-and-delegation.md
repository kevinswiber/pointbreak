# ADR-0010: Actor Identity and Delegation

**Status:** Accepted
**Date:** 2026-06-11
**See also:** [ADR-0003](./adr-0003-agent-resource-claims-advisory-first.md),
[ADR-0004](./adr-0004-event-signatures.md),
[ADR-0007](./adr-0007-writer-act-vocabulary.md),
[ADR-0009](./adr-0009-resumption-binding-trust-source.md)

## Context

Every Shoreline event names its writer once: `writer.actorId`, resolved per write as per-call
override > `SHORE_ACTOR_ID` env var > git `user.email` > git `user.name` > `actor:local`
(`src/session/identity/writer.rs`). In shipped reality that resolution makes every agent an
impersonator: across the dogfood and fixture stores, all 315+ events carry the human's git-email
actor id (`actor:git-email:kswiber@gmail.com`) regardless of whether a human or a coding agent
wrote them. The only surviving agent signal is the `agent:*` track-name convention — a field the
docs explicitly define as a review lane, not identity. An agent writing as the human is
indistinguishable from the human.

Three distinct concepts are squeezed into two fields today: the **lane** (track — correct, stays),
the **actor** (inherits the human's git identity — impersonation in practice), and the
**principal** — the responsible human behind an agent's work — which has no home at all. The
owner's requirement is that a responsible human be resolvable somewhere in any review chain,
without violating two landed constraints: ADR-0007's invariant that no binding decision reads a
self-asserted field, and ADR-0009 arm (a)'s zero-setup floor — the local product must work with no
keys and no configuration.

The fix is plumbing and policy, not model surgery. Nothing in the codebase branches on the
actor-id scheme: the only scheme-sensitive paths are shape validation
(`src/session/identity/writer.rs::is_valid_actor_id`), the did:key self-certifying check
(`src/session/event/signature.rs`, `src/session/signing/trust.rs`), and the constant-false binding
stub ADR-0009 replaces (`src/session/projection/task.rs::response_identity_is_binding`).
`actor:agent:claude-code` is grammar-legal and accepted by validation, ingest, signing, every view
document, history, and the inspector today, via an env var that already outranks git config with
safe fallthrough. The trust map already supports multiple signers per actor
(`TrustSet: BTreeMap<ActorId, BTreeSet<SignerId>>`, `src/session/signing/trust.rs`), and ADR-0004
already separates the friendly actor id from the rotatable `did:key` credential.

A terminology rule this ADR adopts and its successors should keep: **"agent"** names acting
software (a coding or reviewing agent driving `shore`); **"producer"** names any software that
writes events, including the `shore` CLI itself when a human drives it. The word "tool" is avoided
as a category noun for acting software because it collides with the model-API and MCP sense of
"tool" — a callable function an agent invokes — which is unrelated to event authorship. The
existing `writer.tool` envelope field predates this rule; its disposition is an explicit decision
below.

## Decision

Agents get their own actor identity; a checked-in delegation map makes the responsible human
resolvable behind it; keys arrive by default without ever gating a write; and principal
requiredness is reader-side policy composed under ADR-0009's binding predicate. Nine decisions:

### Agents Write Under Their Own Actor Ids

`writer.actorId = actor:agent:<agent-name>` (e.g. `actor:agent:claude-code`). The id is per-agent
and constant: it survives sessions, rebases, and reinstalls. This is convention atop shipped
validation — zero core change for acceptance — and each shipped skill adopts it with one exported
line (`export SHORE_ACTOR_ID="actor:agent:${agent_name}"`), reusing the same `<agent-name>` token
the skills already mint for tracks; the author-response skill inherits the author's id, which
per-agent granularity makes automatic.

The other identity dimensions land where they belong instead of in the actor id: run entropy stays
in the **track** (`agent:<agent-name>-<run_id>`, unchanged); machine and install differentiation
lives in the **key** (one key per machine, many keys per actor id via the existing multi-signer
trust map); model and version detail belongs in the **producer field and payloads**, never in the
identity. Per-session ids would duplicate the track and churn the delegation map unboundedly — the
dogfood stores show 30+ distinct `agent:*` track tags from one human in a few weeks, exactly the
entropy identity must not copy.

Agent names follow the track rule's charset (short, lowercase, hyphenated), skill text pins one
canonical spelling per agent (`claude-code`, not also `claude`) so one agent's history never
splits across two ids, and `/` within the agent segment is reserved for future install-scoping.

### Identity And Credential Stay Two Layers

The friendly actor id is the durable identity; the per-machine Ed25519 `did:key` is rotatable
evidence carried in `signer` and bound to the actor id through the existing multi-signer
allowed-signers entries. This applies the human identity model ADR-0004 already landed to agents,
unchanged. Rotation is a trust-map edit (old key window closes, new key added) with no identity or
event change — the property a `did:key`-only identity cannot offer, because the non-aliasing rule
(ADR-0004) makes a new key a new, unrelated actor. `did:key`-as-actor remains what it already is:
the self-certifying form for keyless edge actors (ephemeral CI identities), never what the skills
set.

### Delegation Lives In A Sibling Checked-In File

On-behalf-of is recorded in `.shoreline/delegates`, a JSON file beside the existing
`.shoreline/allowed-signers` convention (`src/session/signing/mod.rs` fixture precedent) — a
sibling, not an extension. Top-level key `delegates`; each agent actor id maps to an **array** of
delegation records:

```json
{
  "delegates": {
    "actor:agent:claude-code": [
      {
        "principal": "actor:git-email:kevin@swiber.dev",
        "validFrom": "2026-06-10T00:00:00Z",
        "validUntil": null,
        "comment": "claude-code, enrolled by Kevin"
      }
    ]
  }
}
```

`principal` is a fully qualified actor id and **must not itself be an agent-scheme id in v1**:
every agent, including a spawned subagent, maps directly to the responsible human (chain depth 0,
RFC 3820's explicit no-re-delegation constraint stated as a rule rather than inherited as an
accident). `validFrom` is required; `validUntil: null` means open; the interval is half-open
`[validFrom, validUntil)`. The array makes revocation-as-window-close, principal handoff, and the
multi-human edge representable without a format change. Unknown top-level keys are ignored — the
same forward-compatibility posture as the trust-set parser.

The split is load-bearing: allowed-signers binds *keys* to actors (operational, rotation-driven,
ADR-0004's contract — untouched by this ADR); delegates binds *agent actors* to a responsible
*human* (organizational, enrollment-driven). A delegates diff is a pure responsibility change a
human can review without parsing key material; rotating a key never touches delegates; and
verification and principal resolution stay independent — an unsigned local agent event can resolve
a principal, and a validly signed agent event can resolve `principal: none`.

### Enrollment Is Possession-Based: Agent Proposes, Human Commits

The agent (or the human) runs `shore identity enroll`, which writes the delegates entry into the
working tree and prints the diff. Under ADR-0003 that edit is an advisory act — recorded intent,
never operative on its own. The human's review-and-commit **is** the authorization; there is no
countersignature ceremony in v1, and `git log -p .shoreline/delegates` is the audit trail. This is
the same trust root the system already stands on twice: whoever can edit the checked-in
allowed-signers file already controls which keys verify as `valid`, and ADR-0009 arm (a) already
makes possession of the store the local trust root. An agent with worktree access could commit the
file itself — but that is an act against the owner's own repo, which possession already trusts,
and the same answer applies as in ADR-0009: stores where possession does not imply one human use
verified policy tiers, and the federation-grade fix is the deferred signed delegation record.

### Principals Are Resolved At Projection Time, Never Stored

Resolution selects the delegation record whose `[validFrom, validUntil)` window contains the
event's `occurredAt` — the exact parameter `TrustSet::authorizes` already receives (and today
ignores). Projections and command JSON carry a structured principal object
(`{actorId, status, source}`, `status ∈ resolved | none | ambiguous | disavowed`) beside the
existing optional verification status; human surfaces render it as
`claude-code (for kevin@swiber.dev)`. No principal is ever stored in events: no envelope change,
no `EventToBeSigned` change, no idempotency change.

The two ways a delegation ends are two distinct human acts with distinct semantics. **Revocation
closes the window**: future events resolve `principal: none`, every past event still falls inside
the closed window and resolves exactly as it did when the work happened — replay-stable, no git
archaeology in the resolution path. **Deleting a record is disavowal**: past events deliberately
resolve `principal: none` with a disavowal diagnostic, the only honest way to say "I do not answer
for that work," and visible as a reviewable diff. Resolution reads the file as checked out, so it
works in uncommitted working trees, exported bundles, and any non-git context; the file's git
history remains the audit log without being the mechanism.

### Key Provisioning Is Default-On, Split By Consent Model

One flow cannot serve both consent models: ceremony kills agent adoption (the sigstore record),
while silently borrowing a human's identity key violates consent (git makes even SSH-key reuse an
explicit opt-in). So:

- **Humans opt in explicitly.** `shore keys use-ssh` adopts an existing Ed25519 SSH key via
  ssh-agent — the agent signs Shoreline's DSSE pre-authentication bytes directly, so the private
  key file is never read and encrypted or hardware-custodied keys work for free — or
  `shore keys init` generates a dedicated key under `~/.shoreline/keys/`.
- **Agents get automatic keygen.** On the first write in declared agent context with no resolvable
  signer, `shore` generates a passphrase-less Ed25519 key (user-level, mode 0600), signs the
  event, and stages enrollment for the human to commit. Passphrase-less is by design: an
  unattended signer either holds a usable secret or does not sign, and an env-var passphrase
  relocates the secret without shrinking the blast radius — the honest mitigations are per-agent
  per-machine scope, cheap rotation, and enrollment-bounded authority.
- **CI prefers ephemeral self-certifying did:keys.** Generate per job, use the `did:key` as
  `writer.actorId` itself (authorized by ADR-0004's actor-equals-signer rule with zero trust-set
  changes), discard after — short-lived keys instead of revocation management.

Keys never live in the repo or the store: `.shore/` is copyable and linkable by design and
`.shoreline/` is checked in; either would eventually ship a private key.

### Signing Never Gates A Write

Every key failure mode — no agent socket, encrypted key without an agent, unsupported algorithm
(`ed25519-sk` cannot produce plain Ed25519 signatures), keygen failure on a read-only HOME —
degrades to an **unsigned write with a named diagnostic, exit 0**. A key that exists but is not
yet enrolled still signs: the event verifies as `untrusted_key`, which is tamper-evident and
strictly better than unsigned. Signing is an enhancement ladder (`unsigned` → `untrusted_key` →
`valid`), never a gate, so ADR-0009 arm (a)'s zero-setup floor is structurally incapable of being
broken by this design.

### Principal Requiredness Is Reader-Side Policy, Never Schema

Whether an agent event needs a resolvable human is a named projection policy in ADR-0003's sense,
living beside `EventVerificationPolicy` (`src/session/signing/policy.rs`) and threaded through the
same options builders as `with_trust_set`. It composes **conjunctively under ADR-0009's binding
predicate** — ADR-0009 is not reopened; neither arm, the ingest marker, nor its presets change:

```text
binding'(event, bindingPolicy, principalPolicy) :=
      binding(event, bindingPolicy)                      # ADR-0009, verbatim
  and principal_sufficient(event, principalPolicy)       # this ADR's refinement

principal_sufficient(event, policy) :=
  policy in {none, prefer}
  or (policy == require-resolvable-principal
      and (writer.actorId is not an agent-scheme id      # humans are their own principal
           or resolve(writer.actorId, event.occurredAt) == Resolved(<non-agent principal>)))
```

ADR-0009 answers "is this identity verified (or possession-rooted local)?"; principal policy
answers the separate question "is that identity sufficient — does a responsible human resolve
behind it?" The refinement is conjunctive, so it can only narrow what binds, never widen it, and
it reads no self-asserted field: the delegates map is human-committed checked-in config, the same
possession root as allowed-signers.

| Preset | What it requires of an agent event |
| ------ | ---------------------------------- |
| `none` (default) | Nothing. Resolution is still computed and rendered when a map exists. |
| `prefer` | Nothing operative; unresolved, ambiguous, or disavowed principals surface as diagnostics. |
| `require-resolvable-principal` | Operative decisions additionally require the agent actor id to resolve to a non-agent principal at `occurredAt`. Unsigned local agent events still qualify via arm (a) plus the map. |

`require-verified-principal` (principal resolves **and** the event verifies `valid` — arm (b)
only) and `require-signed-delegation` (the delegation record itself is signed by the principal's
key) are named deferred tiers, not v1 presets.

This closes shoreline #98 as policy over verified identity. The "user responses bind" instinct was
a writer-asserted role field, which ADR-0007 banned; it returns here intact but relocated — "only
a response attributable to a responsible human binds" is `require-resolvable-principal` evaluated
over ADR-0009's verified identity plus delegation config the agent does not control.

### `writer.tool` Becomes `writer.producer`

The envelope field `writer.tool` (`src/session/event/writer.rs`) names the producing software —
hardcoded `{ "name": "shore", "version": <crate version> }` on every write today. Under this ADR's
vocabulary the field name is wrong twice over: the value it carries is a producer, and "tool" now
reads as a model-API callable. This ADR renames the field to `writer.producer` as part of its
implementation.

The consequence check is honest and favorable: the field is **not** in the signed
`EventToBeSigned` view (`src/session/event/tbs.rs` — the view carries `actorId` and `signer` but
no producer fact; `tests/fixtures/event_signatures/canonical-tbs-v1.json` confirms it) and
participates in neither idempotency keys nor `eventId`. The rename is therefore an envelope
serialization change only: existing stores break, which the pre-adoption hard-break policy
(ADR-0007) explicitly permits; the golden TBS bytes, the embedded signatures, and `sigVersion: 1`
are all untouched, and the event fixtures under `tests/fixtures/event_signatures/` get a
mechanical field rename with no re-signing. Doing it now, while the field is hardcoded `"shore"`
and no external producer populates it, is the last cheap moment. Enriching the field beyond the
hardcoded value — recording the driving agent's name and version on agent-driven writes — is
deliberately not part of this ADR; see Revisit Triggers.

## Migration

Prospective-only, under the pre-adoption hard-break policy (ADR-0007): no rewrite tooling, no
shim.

- **Old events cannot be re-attributed.** `writer.actorId` is inside the to-be-signed view
  (`src/session/event/tbs.rs`) and participates in derived-id material when supplied as a per-call
  override; rewriting history would invalidate signatures and identities, so it is off the table
  by construction. One operational note for the cutover: a mid-stream actor-id switch means
  re-runs of previously idempotent override-keyed commands append rather than no-op — harmless,
  but real.
- **Old agent writes stay human-attributed — say so.** Every pre-cutover agent event carries the
  human's git-email id and remains exactly what it claimed at write time. The only retroactive
  agent signal is the `agent:*` track-name heuristic, and any surface that shows it must render it
  as a heuristic ("written on an agent track"), never silently promote it to re-attribution. The
  delegation map does not pretend to resolve principals for the pre-cutover era.
- **Mixed stores are internally consistent.** New `actor:agent:*` events coexist with old
  git-email events with no identity conflicts and no re-ingest; recapture remains the hard-break
  escape hatch for stores that want clean attribution.
- **The producer rename rides the same policy.** Envelope serialization changes; signatures,
  idempotency, event identity, golden TBS bytes, and `sigVersion` do not.
- **Surfaces to update:** the three shipped skills (one export line each plus the naming rule),
  the docs set (agent-authoring, cli-reference, review-workflow, storage-model, library-api), and
  tests that assert git-email attribution for agent-driven flows.

## Consequences

### Accepted

- An agent writing an event is no longer indistinguishable from the human; the responsible human
  is resolvable in any review chain through human-committed config, with no self-asserted field
  anywhere in the path.
- The local-first floor survives intact: a zero-setup machine still captures, still attributes
  (by string claim), and still binds locally under ADR-0009 arm (a); keys and principals upgrade
  evidence without ever being required.
- Verification and principal resolution stay orthogonal; each can succeed or fail independently
  and each failure has its own diagnostic vocabulary.
- Revocation preserves history and disavowal rewrites it — deliberately, visibly, and as two
  distinct reviewable acts.
- Mirrors and bundle consumers without the repo's delegates file degrade to `principal: none`
  plus an `unresolvable: no delegation map` diagnostic — never to a wrong answer.
- The delegates parser's `occurredAt`-scoped window lookup is built once and reused when
  trust-set validity windows land (the two files share one validity vocabulary).
- Stores that adopt `require-resolvable-principal` accept that un-enrolled agents' operative
  events stop binding until a human commits an enrollment. That is the point.
- The envelope field rename breaks existing stores under the permitted pre-adoption policy.

### Rejected

- **Ambient git-config human binding.** git-ai is the cautionary precedent: its `human_author` is
  whatever git identity was active at commit time — the same inheritance that produced Shoreline's
  315-event impersonation record — and everything in it is unsigned and self-asserted, which
  ADR-0007 rules out for any authority use. git-ai proves the field is cheap; the delegation map
  is what makes it meaningful.
- **Per-session actor ids.** Run entropy is the track's job; per-session identity churns the
  delegation map unboundedly and destroys cross-session continuity in every projection.
- **Principals stored in the envelope.** A self-asserted authority field (the agent asserting "on
  behalf of Kevin"), frozen into immutable history, wrong forever if the delegation was wrong, and
  a forced `EventToBeSigned` migration ADR-0004 does not owe — three independent disqualifiers.
- **`did:key`-only agent identity.** Welds identity to a credential: rotation becomes identity
  rupture under the non-aliasing rule, zero-setup attribution becomes impossible (no key, no
  identity), and every map and rendering surface keys on opaque `z6Mk…` strings.
- **Unsigned self-asserted attribution as authority.** The claimed `actor:agent:*` id is honest
  labeling, locally trustworthy only via possession (arm (a)); treating the bare claim as
  sufficient for operative decisions on ingested events would reopen the hole ADR-0007 closed.

## Cross-References

- **ADR-0009** — composition target. The principal policy is a conjunctive refinement *under* its
  binding predicate: `binding'(event) := binding(event) and principal_sufficient(event)`. Both
  arms, the ingest provenance marker, the preset table, and the diagnostics are untouched;
  principal sufficiency can only narrow what binds.
- **ADR-0007** — the invariant holds everywhere here: enrollment, resolution, and policy read only
  human-committed config and verified identity, never a writer-asserted field. The producer rename
  also inherits its hard-break authority from ADR-0007's migration policy.
- **ADR-0004** — untouched. The trust-set contract, the signature envelope, the status vocabulary,
  and `EventToBeSigned` are all unchanged; this ADR adds a sibling file and a sibling policy.
- **ADR-0003** — enrollment proposals and the surfaced-not-blocked diagnostics (`ambiguous`,
  `disavowed`, `unresolvable`) instantiate the advisory-claim posture.
- **Relay** — impact is nil for v1. The delegates map is store-local, reader-supplied config like
  the trust set; bundle import, `store link`, and relay mirroring move events and artifacts, never
  config, so mirrors degrade to `principal: none` plus a diagnostic. The relay's existing
  obligations (forward signatures intact, never sign as the reviewer) are ADR-0009's, unchanged.
- **Issue #98** — closed by this ADR as policy-over-verified-identity (see the policy section).

## Revisit Triggers

Reopen this decision if one of these occurs:

- **Signed delegation records** become necessary — federation wants delegation that travels with
  events rather than sitting in repo config. The proven vocabulary to adopt is the SSH-certificate
  field set (`key_id`, `valid_principals`, validity window, KRL-style revocation), signed by the
  human principal's key; `require-signed-delegation` activates as a preset then.
- **Re-delegation chains** are needed — an agent must answer for another agent. v1's non-agent
  principal constraint (chain depth 0) is a format-compatible extension point: agent-valued
  principals plus a bounded-depth transitive resolve.
- **Trust-set validity windows** land (`TrustSet::authorizes` currently ignores `occurred_at`,
  an ADR-0009 revisit trigger) — the delegates windows land first and must supply the shared
  `occurredAt`-scoped lookup and one validity vocabulary, so the trust-set upgrade is a reuse,
  not a second invention.
- **Adapter-id harmonization** — the claude_code adapter's synthetic
  `actor:claude_code:user`/`assistant` ids (`src/session/adapter/claude_code/translate.rs`)
  attribute ingested session-log events, a different act than durable review writes; folding them
  into `actor:agent:*` plus the `sourceSpeaker` payload fact is a separable follow-up.
- **Producer-field enrichment** — agent-driven writes recording the driving agent's name and
  version in `writer.producer` (today hardcoded `"shore"`). The field stays outside the signed
  view either way; enriching it is a producer-call-site change this ADR deliberately did not
  bundle with the rename.
- The possession-based enrollment root proves too weak in practice — a real multi-human store
  where `verified-only`-style policy plus `ambiguous` diagnostics are too coarse — before signed
  delegation records land.
