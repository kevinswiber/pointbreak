# ADR-0012: Actor Attributes and Roles

**Status:** Accepted
**Date:** 2026-06-18
**See also:** [ADR-0010](./adr-0010-actor-identity-and-delegation.md),
[ADR-0007](./adr-0007-writer-act-vocabulary.md),
[ADR-0003](./adr-0003-agent-resource-claims-advisory-first.md),
[ADR-0013](./adr-0013-endorsement-record-and-classification.md)

## Context

ADR-0010 gave every event a `writer.actorId` with a **scheme**: `actor:agent:<name>`,
`actor:git-email:<addr>`, `actor:git-name:<name>`, `actor:local`, or a bare `did:key`. The scheme is a
**derivation namespace** — *how the id was minted* — not a statement about *what kind of party* the actor
is. The only kind-ish predicate in the code is `is_agent_actor_id` (`src/session/identity/writer.rs:104-108`),
a pure scheme-prefix check, and it is an unreliable proxy for "is this a human":

- `actor:git-email:automaton@example.com` is a valid id and may be a **bot**, not a human.
- `actor:agent:review-bot` is an agent id and may be a **trusted, code-review-tuned model** you want to
  treat as a reviewer.

Research 0010 surfaced this sharply. Once endorsement (per-event review/vouching) was made
**actor-neutral** — an endorser is *any actor signing in its own identity*, not necessarily a human — the
question "is this endorser a human / a reviewer / a service?" stopped being answerable from the actor id.
It is a property of the *actor*, declared by a human, that consumers' policies read. Both review axes need
it: accountability (ADR-0010) may want to note a principal's kind, and the endorsement classification
plane (ADR-0013) must surface the endorser's kind/roles so a policy preset can require, say, a human
endorsement or a `role=reviewer` endorsement.

Shoreline has the right precedent already: human-committed, reader-relative, never-self-asserted sibling
config files — `.shore/delegates.json` (ADR-0010), which adds a locally-excluded `.local.json` override
layered git-config-style, and `.shore/allowed-signers.json` (ADR-0004), the sibling **tracked-config**
precedent (`discover_trust_set` loads only the committed file today, `src/cli/review/common.rs:81-86`) —
all with `git log -p` as the audit trail. There is simply no file today that records an actor's
**kind/roles**. This ADR adds one.

## Decision

### A new sibling config: `.shore/actor-attributes.json`

Add a human-committed config file, a sibling of `.shore/delegates.json` and
`.shore/allowed-signers.json`, mapping an **actor id** to its declared attributes:

```json
{
  "schema": "shore.actor-attributes.v1",
  "actors": {
    "actor:agent:claude-code":            { "kind": "agent",            "roles": ["author"] },
    "actor:agent:review-bot":             { "kind": "reviewer-model",   "roles": ["reviewer"] },
    "actor:git-email:kevin@swiber.dev":   { "kind": "human",            "roles": ["author", "reviewer"] },
    "actor:git-email:ci@example.com":     { "kind": "service",          "roles": ["ci"], "comment": "release bot" }
  }
}
```

- **Keys** are any well-formed *persisted* actor id — every shape `writer.actorId` can take, including
  agent ids, whitespace-bearing `actor:git-name:Kevin Swiber` ids (Shoreline mints these from
  `git config user.name`), and `did:key`. Validate with the **whitespace-permitting** rule of
  `is_valid_principal_actor_id` (`writer.rs:89-96`) — which accepts any `actor:*` id (agent or not) and
  `did:key` — **not** the env/override-strict `is_valid_actor_id` (`:74-80`), which forbids whitespace and
  would reject git-name ids. A dedicated `is_valid_attribute_actor_id` alias may clarify intent while
  reusing that rule. (Attributes apply to *both* agent and non-agent actors — unlike `delegates.json`,
  whose keys must be agent-scheme.)
- **`kind`** is a **reserved-but-open** token (grammar below). Reserved well-known values: `human`,
  `agent` (a coding agent), `service` (non-interactive automation, e.g. CI), `reviewer-model` (a model
  whose role is review). Exactly one `kind` per actor. Readers branch only on known values; an unrecognized
  `kind` round-trips and **does not satisfy any `kind=` predicate** (forward-compatible). **`kind`
  deliberately excludes `supervised-agent`/`autonomous-agent`:** supervised vs. autonomous is a *per-event*
  state derived from endorsement presence (ADR-0013), not a static actor property; encoding it as a kind
  would be conflatable with the per-event truth (see Rejected).
- **`roles`** is an **open set** of tokens (e.g. `author`, `reviewer`, `ci`). No reserved closure;
  consumers match exactly on the roles they care about. Absent = empty.
- **Token grammar (both `kind` and `roles`):** lowercase ASCII kebab-case (`[a-z0-9-]+`); matching is
  exact and case-sensitive after normalization, so there is no `Reviewer` vs `reviewer` drift. The writer
  lowercase-normalizes, and **deduplicates and sorts `roles`**, for byte-stable config (mirroring the
  byte-stable `stage_enrollment` writes).
- **`comment`** is optional free text for the human maintaining the file (not interpreted).

### A local override layers over the committed map

`.shore/actor-attributes.local.json` layers over the committed file with the same git-config-style
**per-actor full-replace** semantics ADR-0010 defined for `delegates.local.json` (the local entry for an
actor id replaces the committed entry for that id; other ids are untouched). The override is kept out of
git via the store-init exclude seam (`ensure_local_*_excluded`, mirroring
`src/session/store/store_init.rs:175-180`); a new `.local.json` exclusion is added alongside the delegates
one.

### Attributes are advisory, reader-relative, and never self-asserted

- **Advisory:** like delegates and allowed-signers, attributes never gate a write and never change an
  event's bytes. They are consumed at projection/read time by classification and policy.
- **Reader-relative:** an actor's attributes are resolved against the **reader's current config**, not
  pinned to an event's `occurredAt`. Attributes resemble `allowed-signers` only in being **reader-supplied
  config** — *not* in their time semantics: unlike **ADR-0011's trust windows** (whose accepted
  `authorize_at(actor, signer, occurredAt)` *does* consult `occurredAt`) and **ADR-0010's delegates**
  (windowed over `occurredAt`), v1 attributes carry **no `[validFrom, validUntil)`** and **do not consult
  `occurredAt`** at all. `kind`/`roles` describe an actor identity, not a time-scoped relationship.
- **Never self-asserted (ADR-0007 invariant):** the file is human-committed config; an actor cannot
  declare its own kind in any event it writes. `git log -p .shore/actor-attributes.json` is the audit
  trail. This is the same posture ADR-0007 enforced by removing the self-asserted `writer.role` from the
  envelope, and ADR-0010 enforced by resolving principals from committed config rather than storing them.

### `is_agent_actor_id` is demoted to a display-only fallback hint (hard split)

Where code needs an actor's kind, it reads the attributes map. When no attribute is declared, the actor is
**unattributed** — and the split is hard:

- **Policy and classification predicates require a *declared* attribute.** An absent or unrecognized
  attribute **does not** satisfy `kind=human`, `kind=reviewer-model`, `role=reviewer`, or any other
  `kind=`/`role=` predicate. Unattributed is unattributed for *all* branching. This is what keeps
  ADR-0013's policy predicates unambiguous — they never silently accept a scheme presumption.
- **The scheme (`is_agent_actor_id`) may only feed a *display/UI hint*** ("looks like an agent") for a
  human reading a surface — **never** a gate or a predicate. It is a presumption, not an authority.

`is_agent_actor_id` also remains valid for genuine *scheme* decisions (e.g. ADR-0010's depth-0 principal
validation, which is about the id's namespace, not the party's kind). Code that branches on
`is_agent_actor_id` for a *kind/policy* decision migrates to the attributes seam.

### Read seam: `discover_actor_attributes`

A `discover_actor_attributes` resolution seam mirrors `discover_delegation_map`
(`src/cli/review/common.rs:38-67`): load the committed `.shore/actor-attributes.json`, layer the
`.local.json` override, expose an `ActorAttributesMap` with a `resolve(actor) -> ActorAttributes`
(returning an explicit *unattributed* result when absent — never an error). Both axes consume it:
accountability rendering (ADR-0010) and the endorsement classification plane (ADR-0013).

### Attributes describe actors; they do not assign responsibility

This config is purely **descriptive**. It does **not** change ADR-0010's accountability model: delegation
still resolves *who answers for an agent's work* via the windowed delegates map, and the **depth-0
rule** (a principal must be a non-agent actor) is unchanged. A `reviewer-model` agent having
`kind=reviewer-model` does not make it an accountable principal — attributes never confer authority or
responsibility, they only let a consumer *read what an actor is*.

### Out of scope here

- The **write CLI** (`shore identity attest` + a `stage_actor_attributes` writer mirroring
  `stage_enrollment`) lands via the *identity + endorse creation CLIs*, not this ADR. v1 can be hand-edited
  like any other `.shore/` config.
- **How endorsement classification consumes attributes** (the relationship + attribute reads, the policy
  presets) is ADR-0013's decision; this ADR only provides the substrate.

## Consequences

### Accepted

- A single, reviewable place to declare *what an actor is*, decoupled from how its id was minted. The
  `human`/`agent`/`service`/`reviewer-model` distinctions become first-class and policy-readable without
  any envelope or schema change.
- Uniform with the landed config family: same possession-based, human-committed, advisory posture and
  `git log -p` audit trail as `delegates.json` (whose `.local.json` override this also adopts) and
  `allowed-signers.json` (the tracked-config sibling).
- Forward-compatible: reserved-but-open `kind`, open `roles`, and graceful "unattributed" degradation mean
  new kinds/roles need no schema migration, and a config-less reader (or a federation mirror that lacks
  the file, since config does not travel) simply sees unattributed actors — never a wrong answer.
- **Reader-relative is a deliberate, stated cost:** because attributes resolve against *current* config
  (not event-time), changing the config **intentionally reinterprets historical projections** — relabeling
  an actor's `kind` re-colors past events' classification. Event-time-stable actor classification (windowed
  attributes) is a **future** feature (see Revisit Triggers), not v1 behavior.

### Rejected

- **A self-asserted `kind` in the envelope / `writer`.** This is exactly the `writer.role` anti-pattern
  ADR-0007 removed: an actor declaring its own kind in immutable history is unfalsifiable and forgeable.
  Kind is human-committed config, not a signed claim.
- **Treating `is_agent_actor_id` (the scheme) as the authoritative kind.** The scheme is a derivation
  namespace; the two failure directions above (`automaton@…` the bot, `review-bot` the trusted reviewer)
  make it unreliable. It survives only as a fallback presumption.
- **A closed `kind` enum.** Would force a schema migration for every new actor kind (e.g. a future
  `quorum-service`); a reserved-but-open string avoids that while keeping known values meaningful.
- **`supervised-agent` / `autonomous-agent` as actor kinds.** Supervised vs. autonomous is a *per-event*
  state (ADR-0013: endorsement presence/absence), not a static actor property — a `kind=agent` actor can
  produce a stewarded event and an unstewarded one in the same session. A static kind with those names
  would be conflatable with the per-event truth, so only coarse, time-stable descriptors
  (`human`/`agent`/`service`/`reviewer-model`) are kinds.
- **Modeling attributes as events.** Attributes are slowly-changing description, not an append-only fact
  stream; config with a git history is the right model (as with delegates). It also keeps attributes
  *non-replicating* — a deliberate property: like delegation, an actor's kind is the reader's judgment,
  not something an origin store dictates to a mirror.
- **Time-windowing attributes in v1.** Deferred; `kind`/`roles` are treated as current-config facts.
  Revisit if an actor's kind must vary by event time (see triggers).

## Cross-References

- **ADR-0010 (Actor Identity and Delegation):** this ADR *extends* the actor model with a descriptive
  attribute layer; it does **not** touch the actor/principal/delegation decision or the depth-0 rule.
- **ADR-0007 (Writer Act Vocabulary):** the "no self-asserted authority field in the envelope" invariant
  this config honors.
- **ADR-0013 (Endorsement record & classification):** the primary consumer — endorsement classification
  reads `discover_actor_attributes` to surface the endorser's kind/roles for policy presets.
- **ADR-0011 (Key validity / trust lifecycle):** *no entanglement.* Attributes are descriptive config;
  they are independent of temporal key trust.
- Research 0010: `synthesis.md` Key Finding 4 + artifact allocation; `q5-config-and-enrollment-ux.md` §(c)
  (the write UX); `q2`/`q4` (how classification + policy consume kind/roles).

## Revisit Triggers

- **An actor's kind must vary over time** (e.g. a key reused across a kind change) → add `[validFrom,
  validUntil)` windows mirroring `delegates.json`, and resolve attributes against the event's `occurredAt`
  rather than reader-current.
- **A reserved `kind` proves load-bearing for a *gate*** (not just a label) → it may need to graduate from
  reader-relative config to something with stronger provenance; reassess against ADR-0011's trust model.
- **Roles need structure** (scoping, hierarchy, expiry) beyond a flat open set → revisit the `roles`
  shape.
- **Cross-store attribute portability is wanted** (a mirror wanting the origin's attributes) → revisit the
  non-replicating decision; today, like delegation, attributes degrade to "unattributed" at a mirror.
