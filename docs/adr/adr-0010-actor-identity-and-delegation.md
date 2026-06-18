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

On-behalf-of is recorded in `.shore/delegates.json`, a JSON file beside the existing
`.shore/allowed-signers.json` convention (`src/session/signing/mod.rs` fixture precedent) — a
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

### A Local Override Layers Over The Committed Map

The committed `.shore/delegates.json` is the **shared default**. It stays load-bearing for
everything the rest of this decision rests on: portability across mirrors, exported bundles, and
non-author readers; the `git log` audit trail of who was enrolled when; and the review-and-commit
authorization model. A second, locally-excluded file — `.shore/delegates.local.json` — may sit
beside it as a **private override** for an operator who needs a different principal mapping than the
team default without committing it.

The two layer **git-config style**: for each agent present in the local file, its records **fully
replace** the committed records for that agent — including replacement with an empty array, which
disavows the agent locally; agents absent from the local file inherit the committed map; and either
file may exist alone. The merge happens at **discovery time** in the CLI helper. The library
`DelegationMap` stays scope-agnostic — a new `with_local_override` method expresses the merge as a
pure operation on the parsed map — and resolution semantics (windows, ambiguity, disavowal) are
unchanged: layering only chooses *which* records feed resolution, never how a record resolves.

The trust posture is deliberate: the committed file remains the shared audit and portability root,
while the local override is a private, non-portable convenience and is never the shared authority.
Shoreline keeps `.shore/delegates.local.json` out of Git via `.git/info/exclude` (the committed
`.shore/delegates.json` and `.shore/allowed-signers.json` are tracked by design); a malformed local
file is advisory and never poisons the committed default.

### Enrollment Is Possession-Based: Agent Proposes, Human Commits

The agent (or the human) runs `shore identity enroll`, which writes the delegates entry into the
working tree and prints the diff. Under ADR-0003 that edit is an advisory act — recorded intent,
never operative on its own. The human's review-and-commit **is** the authorization; there is no
countersignature ceremony in v1, and `git log -p .shore/delegates.json` is the audit trail. This is
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
  `shore keys init` generates a dedicated key under `~/.shore/keys/`.
- **Agents get automatic keygen.** On the first write in declared agent context with no resolvable
  signer, `shore` generates a passphrase-less Ed25519 key (user-level, mode 0600), signs the
  event, and stages enrollment for the human to commit. Passphrase-less is by design: an
  unattended signer either holds a usable secret or does not sign, and an env-var passphrase
  relocates the secret without shrinking the blast radius — the honest mitigations are per-agent
  per-machine scope, cheap rotation, and enrollment-bounded authority.
- **CI prefers ephemeral self-certifying did:keys.** Generate per job, use the `did:key` as
  `writer.actorId` itself (authorized by ADR-0004's actor-equals-signer rule with zero trust-set
  changes), discard after — short-lived keys instead of revocation management.

Keys never live in the repo or the store: the `.shore/data/` store is copyable and linkable by
design, and the committed config siblings (`.shore/delegates.json`, `.shore/allowed-signers.json`)
are checked in; either would eventually ship a private key.

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

## Amendment: Key Custody Landing

This amendment records the as-built key-custody and signing-UX work that landed as a follow-up
key-custody effort. The original decisions stand — **Status:** stays Accepted; this is a landing
record that corrects paths and documents placement and sequencing as built, not a re-decision.

- **Paths (single-`.shore/` layout).** The committed config siblings are `.shore/delegates.json` and
  `.shore/allowed-signers.json`; the store lives at `.shore/data/`, clone-local at `.git/shore`.
  These supersede the older `.shoreline/`-style path references in the body above.
- **User-level key home.** Private keys live in a user-level key home `~/.shore/keys/` (honoring
  `$XDG_DATA_HOME` on Unix, `%APPDATA%\shore` on Windows; a `SHORE_HOME` override exists mainly for
  hermetic tests/CI). Keys **never** live in the repo `.shore/` (copyable/linkable) or the store
  (mirrored/exported) — either would ship a private key. Private-key files are `0600` and the key
  home `0700` on Unix; on Windows mode bits are advisory and the directory inherits the parent ACL.
- **Signing never gates — CLI-layer placement.** Signer resolution lives in the **CLI layer**, not
  the library, because the library signing seam (`sign_event_if_requested`) returns `Result` and
  propagates errors. All fallible resolve/load/validate work happens CLI-side *before* the signing
  call, so the workflow only ever signs with a known-good signer (Ed25519 signing over a loaded key
  is itself infallible). Every resolution failure — no key, an unreadable key home, an unsupported
  algorithm, `SHORE_SIGNING=off`, a malformed configured key — degrades to an unsigned write, exit 0,
  with a named diagnostic. Keeping resolution upstream of the signing call is the reason
  signing never gates a write (the contract this places in the CLI layer). The library seam is
  **unchanged**.
- **Provisioning split.** File-based `shore keys init`/`list`/`show`/`enroll` shipped first. ssh-agent
  reuse (`shore keys use-ssh`, signing Shoreline's DSSE PAE bytes through the agent at the unchanged
  `sign_event_message(&[u8])` seam) is a named fast-follow, deferred to keep this release off the
  ssh-agent-client dependency and the Windows named-pipe portability hazard.
- **Deferred (a separate "validation over time" effort).** Rotation, revocation, and
  transparency/signed-heads are not built — no `rotate`/`revoke` subcommand, no placeholder code.
  `did:key` has no supersession link, the `TrustSet` stays a static allow-list whose `authorizes`
  still ignores `occurred_at` (no validity windows), and `eventSetRoot` / signed inclusion proofs
  stay reserved. These tie to the **Revisit Triggers** above (trust-set validity windows; signed
  delegation records).
- **Allowed-signers format.** `.shore/allowed-signers.json` is custom Shoreline JSON
  (`{"allowedSigners": {"actor:…": ["did:key:…"]}}`) — explicitly **not the OpenSSH**
  `allowed_signers` line format despite the filename. It is reader-supplied trust config, like the
  delegation map: a consumer without it renders a signed event `untrusted_key`, never a wrong
  `valid`.
- **Carry-forwards from the identity-half landing.** Delegation **principals** use
  `is_valid_principal_actor_id` (allows internal whitespace — e.g. a multi-word git name
  `actor:git-name:Kevin Swiber` — forbids control chars, 256-char bound), **distinct** from the
  whitespace-strict `is_valid_actor_id` that gates the `SHORE_ACTOR_ID` override; conflating the two
  would re-break multi-word git-name principals. The `require-resolvable-principal` formula keeps a
  defensive non-agent re-check (`!is_agent_actor_id(principal)`) that is **unreachable via the v1
  parser** (which rejects agent-scheme principals at load) — kept as documented defense-in-depth, not
  a coverage gap. `PrincipalStatus::Disavowed` stays **wire-reserved**: v1 emits only
  `resolved`/`none`/`ambiguous`, and disavowal manifests as `none` plus a diagnostic reason, so no
  schema change is needed when a future positive-disavowal source lands.

## Amendment: ssh-agent Custody Landing

This amendment records the ssh-agent fast-follow the previous amendment named — as built. The original
decisions and the key-custody amendment stand; **Status:** stays Accepted; this is a landing record of
the mechanism and the polymorphic seam, not a re-decision. Throughout, "ssh-agent" is the **key
custodian** (the OpenSSH agent), distinct from a coding or reviewing **agent** (acting software).

- **ssh-agent custody path.** `shore keys use-ssh <pubkey-path | key::literal> [--name]` adopts an
  existing Ed25519 SSH key as an agent-backed default signer. The **ssh-agent holds the private key** —
  Shoreline ships the ADR-0004 DSSE pre-authentication bytes to the agent **raw** (sign flags = 0) and
  **unwraps** the SSH-wire signature (`string "ssh-ed25519", string sig`) to the 64 raw bytes the
  envelope's `sig` field wants. There is **no SSHSIG wrapper**: the `DSSEv1 ` PAE prefix already
  supplies domain separation, so cross-protocol confusion with the same key is structurally excluded.
  Only plain `ssh-ed25519` keys are usable — `ed25519-sk` (FIDO; signs a hash + flags + counter
  construction that can never verify under `verify_ed25519_strict`), RSA, and ECDSA are rejected with a
  diagnostic. The `EventSigner::sign_event_message(&[u8])` seam and ADR-0004's envelope are
  **unchanged**.
- **Polymorphic signer resolution.** `resolve_signer` still returns a `SignerResolution` **by value,
  never a `Result`**, but its `signer` field widened from a concrete file signer to
  `Option<Box<dyn EventSigner + Send + Sync>>` so it can carry either a file-backed signer or the
  agent-backed `SshAgentSigner`. A blanket `impl EventSigner for Box<dyn EventSigner + Send + Sync>`
  (forwarding to the inner value) lets the existing generic `sign_with` accept the boxed value with **no
  change to `sign_with` or the `EventSigner` trait**. (A closed enum over signer *types* was considered
  and rejected — the boxed trait object is more extensible.) Because the boxed signer is type-erased,
  the resolution carries a signing mode beside it so the write builders pick strict vs best-effort
  signing — see the next point.
- **Two-part never-gates for a network-backed signer.** The key-custody amendment recorded that signing
  never gates because a resolved `FileEd25519Signer`'s `sign_event_message` is **infallible** (local
  crypto over a loaded key). `SshAgentSigner`'s `sign_event_message` does a **network round-trip that
  can fail**, so never-gates is kept true by **two cooperating mechanisms**:
  - **(a) An identities-only pre-flight** in the CLI resolve layer: connect to the agent and confirm the
    target key is loaded (identities list) **before** returning the signer. It does **not** probe-sign —
    a probe-sign was considered and **rejected** because it would make a confirmation-constrained or
    hardware agent (`ssh-add -c`, 1Password, YubiKey) prompt the user twice per write. Any pre-flight
    failure → `None` + a named diagnostic (`signing_agent_unavailable`; `signing_agent_key_absent`, which
    also covers a globally-locked agent that lists zero identities; or `signing_key_unsupported_algorithm`)
    → an unsigned write, exit 0.
  - **(b) A tightly-scoped, diagnostic-bearing sign-time degrade** in the library write seam
    `sign_event_if_requested`: because the pre-flight does not sign, the will-it-actually-sign question is
    answered at the real sign — a best-effort (agent) signer's `sign_event_message` failure leaves the
    event **unsigned, exit 0**, with a `signing_agent_sign_failed` diagnostic. This is scoped to
    best-effort signers only; the file-signer path stays **strict** (its errors still propagate, so a real
    bug is never masked).

  Mechanism (b) closes the resolve-to-sign TOCTOU window structurally — the sub-second window where the
  agent dies or locks between a passed pre-flight and the real sign, plus a per-key confirmation deny — so
  there is **no documented residual**; the contained change to `sign_event_if_requested` is part of this
  landing, not a deferred residual.
- **3-OS transport (hand-rolled, zero new dependency).** The agent client is the ~3-message protocol
  subset (request identities, sign request, parse the SSH-wire signature) over length-prefixed string
  framing; it performs **no cryptography** (the agent signs). The transport is platform-abstracted: a
  Unix domain socket via `$SSH_AUTH_SOCK`, and the Windows named pipe `\\.\pipe\openssh-ssh-agent` opened
  via `std::fs` — full 3-OS parity, no new dependency. A Windows `windows-sys` `CreateFileW` escalation
  hatch was reserved behind owner sign-off as a contingency if the `std::fs` pipe proved
  byte/message-mode-flaky in CI; **as built, it was not taken** — the zero-dependency `std::fs` path
  holds.
- **Deferred on-disk private-key parsing — correction.** Both the hand-rolled ssh-agent client and the
  Windows named-pipe transport **shipped here** (zero-dependency), which corrects the previous
  amendment's deferral note. What remains deferred is narrower: only SSH **public**-key parsing ships
  (trivial, no decryption); on-disk **private**-key file parsing stays deferred because the agent holds
  the private key and Shoreline never reads it — an agent-less encrypted-key path is an unsigned-write
  diagnostic, not a prompt. The rotation / revocation / transparency boundary is **unchanged** — still
  the separate "validation over time" effort; the `TrustSet` stays a static allow-list whose
  `authorizes` ignores `occurred_at` (tie to the **Revisit Triggers** above).

*Landed 2026-06-16.*

## Amendment: Delegates-Write CLI (`shore identity enroll`) and Enrollment-Wording Correction

> **The original ADR-0010 decisions stand.** This is a **landing-record correction** (the §"Enrollment Is
> Possession-Based" command was described as if it existed but was never built) plus a concretizing
> decision on the command shape. It changes no actor/principal/delegation model, no resolution semantics,
> and no event bytes.

### The wording correction

ADR-0010 §"Enrollment Is Possession-Based: Agent Proposes, Human Commits" says *"the agent (or the human)
runs `shore identity enroll`, which writes the delegates entry into the working tree and prints the diff."*
That command was **never built** by the read-side landing: it landed only the **read/resolution** side of
delegation — `DelegationMap::resolve`, `with_local_override`, `discover_delegation_map`
(`src/cli/review/common.rs`), and `principal_view_for`. There was **no `DelegationMap` serializer and
no `shore identity enroll`**; the delegates map (and its `.local.json` override) was **hand-edited** until
this amendment landed. Read the body's "runs `shore identity enroll`" as the **intended** command, now
landed — not (until now) an existing one. (This supersedes the body's present-tense phrasing, the same way
the "Key Custody Landing" amendment supersedes the older `.shoreline/` path references.)

### Decision: the delegates-write command

```
shore identity enroll <agent-actor-id> --principal <principal-actor-id>
                      [--from <RFC3339>] [--until <RFC3339>] [--comment <text>]
                      [--local] [--repo <path>] [--pretty]
```

- **Stages a delegation record** into the working-tree `.shore/delegates.json` (the shared default) or,
  with `--local`, `.shore/delegates.local.json` (the private override). It appends to the **chosen file's**
  agent record array (creating it if absent) and **never invokes git** — the human's `git commit` is the
  authorization and `git log -p .shore/delegates.json` is the audit trail (ADR-0003 advisory; exactly the
  §Enrollment posture, now buildable).
- **Validation:** `<agent-actor-id>` must satisfy `is_agent_actor_id` (the map-key contract,
  `actor:agent:<name>`); `<principal-actor-id>` is the **responsible principal — the human/non-agent actor
  that answers for the agent** (syntactically any non-agent actor id: `actor:git-email:…`,
  `actor:git-name:Kevin Swiber`, `actor:local`, or a `did:key`). It must satisfy
  `is_valid_principal_actor_id` **and** be non-agent — the depth-0 rule, enforced exactly as
  `delegates.rs::parse_record` enforces it at load.
- **Defaults:** `--from` = now (UTC RFC 3339); `--until` = null (open window). The common case is a
  one-liner: `shore identity enroll actor:agent:claude-code --principal actor:git-email:kevin@swiber.dev`.
- **Output:** a `DiagnosticDocument` (`shore.identity-enroll`) carrying the staged record/diff, with a
  stderr hint to review and commit (mirroring `shore keys enroll`'s `shore.keys-enroll` document and the
  `use_ssh` enroll-hint pattern).
- **Closes the `--local` *writer* gap — and surfaces the full-replace caveat.** Today only the delegates
  *reader* layering exists (`with_local_override`); there had been **no way to write** the local override.
  `--local` adds it. Critically, the layers merge **git-config style**: at read time the local file's
  agent array **fully replaces** the committed agent array for that actor (ADR-0010 §"A Local Override
  Layers Over The Committed Map") — *not* a merge. So a first local enrollment **shadows**
  any committed records for that agent on this machine. The command output **surfaces this**
  (e.g. "this local entry replaces N committed record(s) for `<agent>` locally"), never presenting
  `--local` as appending onto the committed set.

### Decision: implementation reuses the landed possession-based precedent

The command is a thin CLI over a new **pure `stage_delegation(path, agent, record)`** library writer,
symmetric to the landed `stage_enrollment` (the `.shore/allowed-signers.json` writer): read-or-init,
append, re-serialize sorted + trailing-newline byte-stable, `create_dir_all` the parent, never call git.
This uses a write-oriented `DelegationWriteRecord` that **round-trips** the raw `validFrom`/`validUntil`/
`comment` (the read-path record discards them after parsing into a `ValidityWindow`) — a separate
write-side record struct so the read path stays minimal.

For `--local`, the command ensures `.shore/delegates.local.json` is git-excluded. The existing helper
`ensure_local_delegates_excluded` (`src/session/store/store_init.rs`) was **`pub(crate)`** and so was
**not reachable from the `shore` binary crate** (which calls the library only through `shoreline::…` public
APIs). The landing therefore **exposed a public seam** by promoting that helper (and its
`ensure_local_actor_attributes_excluded` sibling) to `pub`, while keeping `stage_delegation` itself a
pure, git-free writer.

### Decision: command group and v1 scope

- **Group: `shore identity`** — matching the ADR's literal `shore identity enroll` spelling and giving the
  actor/principal model a coherent home. The sibling **`shore identity attest`** (ADR-0012's
  actor-attributes writer) joins the same group with the same stage-and-commit pattern. *Rejected:*
  overloading `shore keys` (key-custody, a different layer) or minting a separate `shore delegates`.
- **Revocation and disavowal stay hand-edited diffs in v1.** No `revoke`/`disavow` mutating subcommands:
  ADR-0010 §"Principals Are Resolved At Projection Time" already routes **revocation** (close the window)
  and **disavowal** (delete the record) through reviewable diffs and `git log`; a mutating subcommand would
  re-introduce the ceremony the ADR deliberately avoids. `--until` on a fresh `enroll` expresses a
  closed-window enrollment; window-closing an *existing* record is a hand edit (or a deferred follow-up).

### Consequences

#### Accepted

- The entire LANDED accountability mechanism finally gets a creation surface: delegation moves from
  hand-edited JSON to `shore identity enroll`, closing the ADR's own described-but-unbuilt divergence, and
  the delegates `--local` writer gap is closed.
- Uniform with the landed `keys enroll` / `stage_enrollment` precedent (stage the working-tree edit, print
  the diff, emit a diagnostic document, hint to commit, never invoke git) and with ADR-0012's
  `shore identity attest` — one possession-based pattern across the config family.

#### Rejected

- **`revoke`/`disavow` mutating subcommands in v1** — the diff/`git log` path is the intended mechanism.
- **A countersignature/broker ceremony** — possession + commit is the trust root (ADR-0003/0009), unchanged.

### Revisit Triggers

- **Window-closing an existing record needs to be ergonomic** (beyond a hand edit) → a deferred
  `shore identity revoke` that stages a `validUntil` edit as a reviewable diff (still no ceremony).
- **The federation-grade signed delegation record** (ADR-0010's deferred `require-signed-delegation`)
  lands → enrollment may gain a signed variant; the possession-based command remains the local default.

*Landed 2026-06-18.*
