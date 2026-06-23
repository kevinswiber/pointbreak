# ADR-0016: Content-Targeted Artifact Removal and Local Compaction for the Append-Only Store

**Status:** Accepted (owner-approved 2026-06-19); landed via the store-topology collapse implementation
work. Approved across two `adr-review` cycles: a first approval, then an owner-directed revision (event
renamed `ArtifactPruned` → **`ArtifactRemoved`**, since the `gc`/`compact` sweep is the git-sense
"prune"; a session-anchored, content-addressed **event-target model** made explicit, §2; and
**remove-only** stated outright, §5), re-approved.
**Date:** 2026-06-19
**See also:** [ADR-0015](./adr-0015-single-common-dir-store.md) (the single-common-dir collapse that
makes this day-one). In-repo (`docs/adr/`): [ADR-0008](./adr-0008-cross-peer-conflict-policy.md)
(cross-peer conflict policy — projection-vs-sync-plane allocation; no timestamp arbitration);
[ADR-0014](./adr-0014-reviewunit-commit-range-lifecycle.md) (ReviewUnit commit-range lifecycle — the
monotonic withdrawal family + `retraction_target_missing` precedent; this removal is **distinct**: it
retracts *content*, not a commit/ref association);
[ADR-0003](./adr-0003-agent-resource-claims-advisory-first.md) (advisory contract — recovery via later
events); [ADR-0002](./adr-0002-large-snapshot-artifact-policy.md) (snapshot-artifact policy). Also GitHub
#146 (snapshot-scoped, deduped artifact body — the fact that forces content-targeting).

## Context

- **Sensitive bytes live in artifacts, not events.** Shoreline's durable facts are content-addressed
  events; projections are pure functions of the event set (ADR-0008). The captured *content* — diff
  rows (file contents) and large note bodies — lives in content-addressed **artifacts**
  (`artifacts/snapshots/<sha256(snapshotId)>.json`, `artifacts/notes/<sha256>.json`; note bodies are
  hashed at `src/session/store/body_artifact.rs:76`). The `ReviewUnitCaptured` event
  (`ReviewUnitCapturedPayload`, `src/session/event/review.rs:32`) carries only `snapshot_id`
  (`review.rs:38`) + `snapshot_artifact_content_hash` (`review.rs:39`) — pointers, not content.
  `referenced_artifacts()` (`src/session/workflow/artifact_transfer.rs:124`) already enumerates both
  blob kinds by a normalized `sha256:` content hash.
- **The collapse removes today's accidental privacy guarantee.** Per-worktree isolation gives two
  *accidental* guarantees: `git worktree remove` discards that worktree's `.shore/data` (the de-facto
  GC) and physically deletes its snapshot artifacts, so sensitive captured contents die with the
  worktree (`docs/storage-model.md`). The store-topology collapse (ADR-0015: shared `.git/shore`
  + write-through) breaks both — bytes persist in the shared store, and read-time scoping **hides** but
  does not **delete** them. The day-one privacy finding named the concrete regression: an
  *ephemeral worktree reviewing sensitive untracked/customer files*. An explicit, non-automatic removal
  is therefore a **day-one** constraint of the collapse, not deferrable polish.
- **#146 forces the granularity.** The snapshot-scoped artifact change (#146) made snapshot artifacts
  snapshot-scoped (`src/session/store/snapshot_artifact.rs:15`) and **deduped** on snapshot-content match regardless of
  the referencing unit (`snapshot_artifact.rs:63`): one blob is shared by **many** ReviewUnits
  (`review_unit → snapshot_id` is many-to-one; the two-worktree shared-blob repro is
  `two_linked_worktrees_capture_same_range_into_shared_store`, `src/session/workflow/capture.rs:950`).
  This is decisive for *what* a removal can coherently target.
- **Greenfield, but the pattern exists.** There is no `remove`/`gc`/`tombstone` today. The substrate
  already defines **retraction** as "a new event that withdraws an earlier event without mutating it"
  (`docs/substrate-language.md`), and ADR-0014's withdrawal family is the working precedent.

## Decision

### 1. Removal targets content (a `content_hash`), never a ReviewUnit

The removal fact is an `ArtifactRemoved { content_hash }` event, idempotency key
`artifact_removed:<content_hash>`. The payload carries **only** the `content_hash` — no free-text
`reason` field — so two peers removing the same content emit a byte-identical payload (the convergence
property §6 depends on; a justification/`reason` is deliberately deferred, see Deferred). This mirrors
the reason-free, identity-only ADR-0014 withdrawal payloads (`src/session/event/association.rs:80,103`),
the cited convergent-retraction precedent. It applies uniformly to snapshot **and** note-body artifacts
(both are sha256-addressed blobs). Content-targeting is **forced** by the #146 dedup: identical content
is one shared blob, so "remove unit U's content but keep unit V's identical content" is incoherent —
there is one set of bytes. It also dissolves the unit-targeting circular dependency ("delete X to delete
Y" vs "delete Y to delete X"): retracting *content* marks every referencing unit removed at once, with
no cross-unit ordering.

### 2. The removal event is session-anchored and content-addressed (not review-unit-scoped)

An `ArtifactRemoved` event is **not** a review-unit event, and that is deliberate: #146 dedup makes one
blob shared by **many** ReviewUnits (often across different sessions), so content has no single
review-unit home. Its `EventTarget` (`src/session/event/target.rs:10`) therefore carries **only
`session_id`** (the removing actor's session), and the `content_hash` rides in the **payload**,
addressing the target by content identity. Note that `target.session_id` is itself
**first-stored-local provenance, not a convergent value** — like `writer`/`occurredAt` it lives in
`eventRecordHash` but **not** `payloadHash` (`record_hash.rs:33`, `mod.rs:135`), so two peers removing
the same content from different sessions converge on the same `{ content_hash }` fact while keeping
different first-stored session envelopes (§3/§6). This mirrors the detached co-signature carrier
(`EventTarget::for_event_signature`, `target.rs:98`), which already addresses its target by payload
content-identity with a session-only envelope. The model already admits non-review-unit events —
initialization and imported notes use `EventTarget::new(session, work_unit)` (`target.rs:36`),
task-attempt events use `for_work_object` (`target.rs:75`) — so a content-addressed removal is a new
*shape*, not a new *category*. The store is **flat, keyed only by idempotency key**
(`src/session/store/event_store.rs:27`), not partitioned by unit or session, so the single
content-keyed removal is **globally visible** to every projection that reads the store — which is what
lets §3 render the removal for *every* referencing unit, in any session. (Whether `content`/`artifact`
should be promoted to a first-class substrate **work object** — `WorkObjectType` is today only
`{ReviewUnit, TaskAttempt}`, `src/model/work_object.rs` — is a deferred future investigation; see
Deferred.)

### 3. The event log stays immutable; the projection joins capture + removal

Appending the removal event never rewrites or tombstones the capture event. The ReviewUnit projection
joins the capture event with any `ArtifactRemoved` over its `content_hash` and renders **"content
removed"** for every referencing unit. That *the content is removed* is the **convergent shared fact** —
every peer holds an `ArtifactRemoved` for that `content_hash`, so every peer agrees. Any **actor/
`occurredAt`** shown alongside is **first-stored-local provenance, not a convergent value**: those
fields live in the `payloadHash`-exclusive envelope, so two peers that independently remove identical
content each keep their own first-stored record (`src/session/store/event_store.rs:62-77`) and their
`eventRecordHash`es differ (`writer`/`occurredAt` are part of `eventRecordHash`,
`src/session/event/record_hash.rs:34-35`) — the *who/when* can disagree across peers while the
*that-it-is-removed* cannot. (In the common single-remover workflow there is one removal event, so
attribution is unambiguous; the divergence is specific to concurrent independent removals of
byte-identical content.) This upgrades today's hard `missing artifact for snapshot …; import referenced
artifacts before reading` error (`src/session/store/snapshot_artifact.rs:170`) into a graceful,
*explained* absence, and disambiguates *removed* from *not-yet-synced/corrupt*. No per-ReviewUnit
removal event (redundant and racy — new units can reference the content later); no silent
dangling-pointer (indistinguishable from a missing import). Structural metadata
(supersedes/superseded_by, lineage, endpoints) survives for free, and signatures / idempotency /
convergence are untouched (the breakage class the snapshot-scoped artifact work spent four review rounds
avoiding).

### 4. A multi-selector command resolves to a content set

`shore store remove` accepts ergonomic selectors that **all resolve to a set of `content_hash`es**:
`--snapshot <id>`, `--review-unit <id>`, `--ref <branch>` / `--range A..B` (via the
reachability/context projection), `--orphans`. `--review-unit` reports co-referencing units before
acting ("snapshot S is also referenced by V, W; removing it removes the content for all of them") —
consent/awareness, not a block.

### 5. Two-phase: `remove` (event) then `compact` (local sweep); remove-only

`remove` appends the event (cheap, convergent, auditable); the bytes it marks survive on disk until
`compact`, so **compaction — not removal — is the point of no return** for the data. Removal is
**one-way (remove-only)**: Tier-1 defines no append-only un-remove event. This is deliberate and matches
the established retraction precedent — ADR-0014's withdrawal family is **monotonic**: deterministic
content ids (`src/session/event/association.rs:132-167`), an `associated − withdrawn` set-subtraction
fold (`src/session/projection/commit_range.rs:5,229`), and **terminal** withdrawal — a later identical
`Associated` returns `Existing` and does **not** revive a withdrawn edge
(`docs/adr/adr-0014-reviewunit-commit-range-lifecycle.md:87-91`). And ADR-0008's no-timestamp-arbitration
rule means a reversible remove/restore *toggle* would have no convergent way to order
"removed-then-restored" against the reverse. Moreover, content-keying makes a removed `content_hash` permanently absent for any future
identical capture — a privacy *feature*, not a defect. Recovering content before `compact` is therefore
**re-capture / re-import** (the bytes are content-addressed; any uncompacted peer can re-materialize
them), **not** an un-remove; a true `ArtifactRestored` is a separate deferred decision (see Deferred).
`shore store gc` / `shore store compact` is a **local, non-event** maintenance sweep that physically
deletes the removed blobs. GC is deliberately **not** an event — "I deleted my local bytes" is a
sync-process/local fact, not a shared review fact (ADR-0008's allocation rule). Because removal is
content-targeted, a removed `content_hash` has no live referrer by construction, so GC needs **no
reference-count wait**: removed content is immediately collectable, and the sweep is re-derivable from
the log. Operator advice: **to remove sensitive data, `remove` then `compact`.**

### 6. Convergence and safety

The content-addressed idempotency key **plus a `content_hash`-only payload** means two peers removing the
same content **converge to one fact**. The mechanism is exact: the `eventId` derives from the
idempotency key alone (`src/session/event/mod.rs:136`) and `payloadHash` covers the whole payload
(`mod.rs:135`), so a byte-identical `{ content_hash }` payload yields an identical `payloadHash`; the
second writer then dedups rather than conflicting (`src/session/store/event_store.rs:58`). Differing
`writer`/`occurredAt`/signature **and `target` (including `session_id`)** live in the
**payload-hash-exclusive** envelope and fall to the keep-first-stored-record path
(`event_store.rs:62-77`), not a conflict — so there is **no timestamp arbitration** (ADR-0008). This is precisely why §1 keeps `reason` out of the payload: a free-text
`reason` would mint a divergent `payloadHash` for the same content and `record_event_once` would
**hard-conflict** (`event_store.rs:78-83`) — the breakage class the snapshot-scoped artifact work spent
four rounds avoiding.
A removal whose target content has not backfilled is a `retraction_target_missing`-style projection
diagnostic that self-heals on backfill (the ADR-0014 precedent). The surviving removal event retains
**no content bytes** — `content_hash` is a sha256, not the content — but the hash is still a stable
fingerprint: it discloses content **equality/existence** and is **dictionary-testable** for low-entropy
or guessable content (so it is not zero-leak, only byte-free). The removal event is actor-attributed and
signable (accountability), with actor identity carried in the envelope, not the payload — but precisely
because attribution is envelope-borne (and thus `payloadHash`-exclusive), the rendered actor/`occurredAt`
is **first-stored-local provenance, not a convergent shared value** (§3): convergence holds on *that the
content is removed*, not on *who/when* removed it. Making attribution itself convergent would need a
separate unionable mechanism, out of Tier-1 scope (see Deferred).

### 7. Mirror reality (the scope boundary)

The removal **event** converges to peers (they learn the content is removed and *may* GC their own copy),
but **bytes cannot be un-sent.** So remove+compact is *complete* only **before** the artifact is
pushed/mirrored — aligning with "privacy = don't push." The ephemeral / never-pushed case is fully
solvable locally; the already-mirrored case is a **documented limitation**, and cross-store/federation
redaction is out of scope here.

## Consequences

### Accepted

- The collapse's day-one privacy regression is answerable with a small, on-model mechanism: one
  content-addressed retraction event + a local blob sweep, with **no event mutation**.
- Removed content renders as an *explained* absence everywhere it is referenced, derived from a single
  event.
- The event slots into the existing model as a session-anchored, content-addressed event (§2) — no new
  work-object type and no new target category are required for Tier-1.
- GC is trivially safe (no refcount race) and re-derivable from the log.
- **Cost:** a pre-push window is required for a *complete* privacy guarantee (documented; ties to "don't
  push").
- **Cost:** an `ArtifactRemoved` event type, a `shore store remove` + `shore store gc`/`compact` CLI
  surface, a projection join, and the graceful-render path are net-new.

### Rejected

- **Unit-targeted removal (a `ReviewUnitRemoved` that deletes a unit's blob).** With #146 dedup a blob is
  shared, so unit-targeting creates the circular-dependency/refcount deadlock the owner flagged and
  cannot coherently express "delete these bytes." (*Forgetting a unit's existence/metadata* while
  keeping shared content is a different, deferred operation — see below.)
- **A review-unit-targeted removal event (envelope `review_unit_id`).** Content is shared across units
  and sessions (#146), so no single unit is the right envelope target; the event is session-anchored +
  content-addressed in the payload instead (§2).
- **A reversible remove/restore toggle in Tier-1.** A symmetric un-remove fights ADR-0008's
  no-timestamp-arbitration rule (a toggle needs ordering the model refuses to provide) and the
  monotonic ADR-0014 precedent; deferred (see below).
- **A snapshot-id selector with silent graceful failure on dangling pointers.** A missing blob is then
  indistinguishable from not-yet-synced/corrupt; removal must be a *recorded fact*, not an absence.
- **A per-referencing-unit removal event.** Redundant and racy; the projection join is the single source.
- **Rewriting / tombstoning the capture event on compaction.** Breaks `eventRecordHash` / signatures /
  idempotency / convergence — the exact failure mode #146 / the snapshot-scoped artifact work avoided.
  Events are immutable; only blobs are collected.
- **GC as an event.** Local byte-removal is not a shared review fact (ADR-0008); converging it would
  falsely assert that peers' bytes are gone.

### Deferred (explicitly NOT decided here)

- **Tier-2 metadata redaction** — forgetting data that lives *in events*: small **inline** note bodies
  (not externalized below the inline limit), file paths / OIDs / `worktreeRoot` on the capture event.
  Removing those means altering signed / content-addressed / possibly mirrored events — the genuinely
  hard append-only problem; warrants its own ADR.
- **Reversal / un-remove (`ArtifactRestored`).** Tier-1 is remove-only (§5). A symmetric restore would
  have to solve convergent toggle ordering against ADR-0008's no-arbitration rule **and** the fact that
  post-`compact` bytes cannot be recovered by an event at all (only re-imported); it is its own decision
  if a genuine un-remove (as distinct from re-capture/re-import) is ever needed.
- **Artifact as a first-class substrate work object.** `WorkObjectType` is today `{ReviewUnit,
  TaskAttempt}` (`src/model/work_object.rs`); the removal event is session-anchored + content-addressed
  (§2) rather than targeting an `Artifact` work object. Promoting `content`/`artifact` to its own work
  object — with its own lifecycle — is the more substrate-principled long-term shape and is flagged here
  as a **future investigation**, explicitly not decided now.
- **Full mirror-aware retention policy** — retention windows, tombstone-vs-hard-delete across mirrors,
  federation redaction.
- **Integration-ref definition and gc'd-commit (object-gone) degradation** — these belong to the
  merge-status / orphan projection, not the removal mechanism.
- **The ephemeral worktree-local opt-out** — the sibling day-one privacy constraint, decided in ADR-0015;
  referenced here, decided there.
- **A removal `reason`/justification.** Deliberately **not** carried in the Tier-1 `ArtifactRemoved`
  payload — a free-text payload field defeats content-addressed convergence (§1/§6). If a justification
  is wanted later it must be carried convergence-safely: externalized as its own (removable)
  content-addressed note-body artifact referenced by hash, or otherwise kept out of the identity-bearing
  payload (Tier-2-adjacent).
- **Convergent (unionable) removal attribution.** Tier-1 renders *who/when* removed as
  first-stored-local provenance, not a convergent shared value (§3/§6); the convergent fact is only
  *that the content is removed*. Making attribution itself converge across concurrent independent
  removals would need a separate unionable attribution mechanism (not the removal-event envelope) —
  deferred.

## Revisit Triggers

- Cross-store / federation redaction becomes a requirement (the "can't un-send" boundary must move).
- Tier-2 metadata redaction is needed in practice (inline-body or capture-metadata sensitivity
  surfaces).
- A genuine un-remove (not re-capture/re-import) is needed — revisit the remove-only decision (§5) and
  its toggle-ordering / post-compaction byte-recovery constraints.
- The Artifact-as-work-object investigation (Deferred) concludes that content should be promoted to a
  first-class substrate work object.
- A retention policy beyond manual removal is required (auto-removal windows, quota-driven GC).
- The dedup assumption changes — if snapshot artifacts ever become unit-scoped again (reversing #146),
  the content-vs-unit granularity must be reconsidered.

## Amendment: ArtifactRemoved Files Into the Journal Carrier (`session_id` → `journal_id`) (2026-06-22)

**The original decision stands; this is an envelope reconciliation only.** This ADR landed before the
`EventTarget` identity reshape (ADR-0017) and described the `ArtifactRemoved` carrier in pre-reshape terms.
ADR-0017 then reshaped `EventTarget` from the flat optional bag into the non-optional triple
`EventTarget { journal_id, subject: TargetRef, track_id }` and renamed the top scope `session_id` →
`journal_id`. This amendment records how §2's session-anchored, content-addressed removal carrier maps onto
that reshaped envelope. **No model rule, convergence property, or `sigVersion` changes** — only the carrier's
spelling.

**The mapping.** §2's "its `EventTarget` therefore carries **only `session_id`**, and the `content_hash`
rides in the **payload**" becomes, under the reshape: the removal event carries the envelope's `journal_id`
and a subject-less, fieldless **`TargetRef::Journal`** carrier (`subject: TargetRef::Journal`, `track_id:
None`), with the `content_hash` still riding in the payload. ADR-0017 introduces `TargetRef::Journal` as the
single carrier shape for the genuinely subject-less events — the detached co-signature carrier **and** this
`ArtifactRemoved` — each filing into its journal by the envelope's `journal_id` while its target stays
addressed by payload content (`content_hash` here, `target_event_id`/`target_event_record_hash` for the
co-signature carrier), never duplicated onto the envelope. So §2's note that the carrier "mirrors the
detached co-signature carrier" is now literal: post-reshape they are the **same** `TargetRef::Journal`
variant. §2's references to the pre-reshape constructors (`EventTarget::new(session, work_unit)`,
`for_work_object`, `for_event_signature`) are superseded by that collapsed carrier; read `session_id` as
`journal_id` throughout §2/§3/§6.

**What does not change.** `journal_id` (formerly `session_id`) remains **first-stored-local provenance, not
a convergent value** — like `writer`/`occurredAt` it lives in `eventRecordHash` but **not** `payloadHash`
(§6), so two peers removing the same content from different journals still converge on the same
`{ content_hash }` fact while keeping different first-stored envelopes. Remove-only (§5), the immutable
event log + capture-join projection (§3), the multi-selector content set (§4), and the mirror-reality scope
boundary (§7) all stand verbatim.

**Three distinct retraction verbs (confirming cross-reference).** Removal stays the third of three distinct
verbs, each kept distinct: **supersede** — evolve a revision (ADR-0018; the superseded revision and its
facts remain inspectable); **withdraw** — retract a structural association edge (ADR-0014; `associated −
withdrawn`); **remove** — delete content bytes (this ADR). They operate on different targets (a revision
position, a structural edge, content bytes respectively) and never substitute for one another.

**Status:** Accepted; lands with the substrate-reshape implementation work that reconciles this ADR to
ADR-0017's reshaped `EventTarget`. The original ADR-0016 text above and its top-level **Status: Accepted**
are unchanged.

## Amendment: Removal Authorization — `ArtifactRemoved` Is Advisory Until a Named `RemovalPolicy` Makes It Operative (2026-06-23)

**The original decision stands.** ADR-0016 decided *what* a removal targets (content, never a unit), its
shape (an immutable, content-addressed `ArtifactRemoved { content_hash }` fact), and its two phases
(`remove` appends; `compact`/`gc` sweeps — the point of no return, §5). This amendment decides *when a
removal claim becomes operative* — a question the original ADR left unanswered and therefore answered
*unconditionally*. It reconciles removal with ADR-0003 (advisory-first) and upgrades §6's
accountability claim from *attributed-but-ungated* to *attributed-and-policy-gated*.

### The gap (grounded)

As built before this amendment, a removal claim was operative the instant it was appended, with **no
authority, signature, or trust check anywhere on the path** — the gap this amendment closes:

- `ArtifactRemovalProjection::from_events` folded *every* `ArtifactRemoved` into a flat
  `removed: BTreeSet<String>` and `is_removed()` was plain set membership — reading no signature, actor,
  or trust (`src/session/projection/artifact_removal.rs`).
- `resolve_snapshot_content` returned a single removed state for any removed content hash **before the
  byte read**, suppressing the artifact in *every* referencing revision's view
  (`src/session/workflow/revision_projection/snapshot.rs`).
- `compact_store` then built the same unconditional projection and **physically deleted** the bytes
  (`src/session/workflow/artifact_removal/mod.rs`), the irreversible step.

In the common single-writer local store this was benign: the remover is the store owner, and ADR-0009's
possession arm already makes a locally-authored fact legitimate by construction. The gap bit on
**ingested/relayed** removals, where possession ≠ authorship: the ingest path verifies signatures
(`verify_events_for_ingest`, `src/session/workflow/ingest.rs:142-146`) but **nothing downstream
conditioned `is_removed` on that verdict**, so a relayed or mistaken actor could suppress evidence — and
`compact` could make the suppression irreversible. That contradicts ADR-0003 (agent claims are advisory
unless a named projection policy treats them as operative) and ADR-0016 §6's own "actor-attributed and
signable (accountability)" claim, which is true of the attribution but gates nothing.

### Decision

#### 1. `ArtifactRemoved` is an advisory claim; operativeness is a named, reader-side policy

A removal **event** remains an immutable, convergent, content-addressed fact (unchanged). Whether that
claim is **operative** — whether a reader *suppresses* the artifact, and whether `compact` *erases* its
bytes — is decided by a named **`RemovalPolicy`**, a sibling of `EventVerificationPolicy`
(`src/session/signing/policy.rs:32-76`) and `PrincipalPolicy`. Like every operative notion in the
substrate (ADR-0003), the policy is **named, locatable, testable, and diagnostic-rich**; it is never an
intrinsic property of the event and never a write-side gate — a removal write is never rejected, and a
non-operative removal is **surfaced as a diagnostic, never silently dropped**.

#### 2. Two thresholds, not one: render-time *suppression* vs `compact` *erasure*

The decisive refinement of the finding: a removal drives two actions with opposite risk profiles, and
they must be gated **separately and along independent axes**.

- **Suppression** (hide the artifact in a read view) is **per-reader** and **non-destructive** (the bytes
  stay on disk). It is reversible only in the sense that it is a *reader-side interpretive decision over
  still-present bytes*: change the reader's `TrustSet` or `RemovalPolicy` — or trust/endorse the removal —
  and the same bytes render again. It is **not** reversed by an append-only un-remove: ADR-0016 is
  remove-only (§5), and a drop-and-rebuild of the projection re-derives the *same* suppression because the
  immutable `ArtifactRemoved` tombstone persists. "Reversible" here means *re-renderable under a policy/
  trust change while bytes remain*, never *un-removable*.
- **Erasure** (`compact`/`gc` deletes the bytes) is **irreversible** and **global** (once any actor with
  store access sweeps, the bytes are gone for everyone sharing that `.git/shore`). It is executive policy
  over irreversible state.

These are independent axes, not a single ladder: the render preset governs *what a reader sees*; the
erasure gate (§4) is a *separate fixed rule* governing *what bytes may be destroyed*, never a function of
the active render preset. The danger the separation prevents is a **render preference lowering the bar
for permanent destruction**: because erasure does not read the render preset, a laxer render policy can
never expand the erasure-eligible set.

#### 3. Render-time suppression presets (per-reader, non-destructive)

The operative-suppress decision composes two orthogonal inputs: **origin** (is the removal *locally
authored* — no ingest-provenance marker — or *ingested/relayed*?) and **verification** (its
`EventVerificationStatus` under the reader's `TrustSet`, where `valid` already means *cryptographically
valid and trusted* and `untrusted_key` means *cryptographically valid but untrusted* — `verify.rs:36-44`
— plus any trusted endorsement of it). The default preset **reuses ADR-0009's possession arm (a) and
valid-signer arm (b)**, then **adds a removal-specific trusted-endorsement extension to arm b** that
ADR-0009 binding does not have (the named elevation in §3a):

| Preset | Operative-suppress when… | Otherwise |
| ------ | ------------------------ | --------- |
| `advisory` | never | every removal claim renders as a diagnostic; nothing is hidden |
| `possession-or-trusted` **(default)** | **not the `invalid` floor (below), and then** — **arm a (possession):** locally authored; **OR arm b (trusted):** the removal verifies `valid` for a **trusted** signer, **or** carries a **trusted endorsement** (§3a) | advisory diagnostic, no suppression |
| `trusted-strict` | not the `invalid` floor, and then **arm b only** (trusted signer or trusted endorsement); the possession arm is dropped | advisory diagnostic, no suppression |

**The `invalid` integrity floor (overrides everything).** A removal whose own inline signature verifies
`invalid` is **never** operative-suppress and **never** erasure-eligible — under any preset, regardless of
possession *or* endorsement. A broken inline signature means the event's integrity is suspect; for an
adversarial, irreversible operation it must be re-issued cleanly, not vouched back to life. (`invalid`
defeating possession is also exactly ADR-0009 arm a's qualification.)

The `{origin × verification}` truth table for the default `possession-or-trusted` preset, after the
`invalid` floor is applied:

| inline `EventVerificationStatus` of the removal ↓ \ origin → | locally authored (possessed) | ingested / relayed |
| ----------------------------------------------------------- | ---------------------------- | ------------------ |
| `valid` (cryptographically valid **and** trusted signer) | **suppress** (arm a + b) | **suppress** (arm b) |
| `untrusted_key` (cryptographically valid, **untrusted** signer) | **suppress** (arm a) | **suppress** *iff* trusted endorsement (arm b), else advisory diagnostic |
| `unsigned` | **suppress** (arm a) | **suppress** *iff* trusted endorsement (arm b), else advisory diagnostic |
| `invalid` | advisory diagnostic (**`invalid` floor**) | advisory diagnostic (**`invalid` floor**) |

A **trusted endorsement** promotes an `untrusted_key`/`unsigned` removal to operative on **either** origin
via arm b (§3a) — it is the one signal that lifts an ingested low-trust removal — but it never overrides
the `invalid` floor. This same rule governs render, compact (§4), and diagnostics (§5).

This default is what **preserves the zero-setup local floor**: the store owner's own keyless (`unsigned`)
removals suppress by possession (arm a), so single-user `remove` still hides the owner's content with no
keys — while a **relayed** removal (no local possession) must verify valid-and-trusted, or carry a trusted
endorsement, to suppress. `trusted-strict` is for a reader who does not extend trust to local possession
(e.g. inspecting a copied/mirrored store).

**Deliberate divergence from ADR-0004, scoped to the trust axis.** For ordinary events, `integrity-strict`
accepts `untrusted_key` as operative (tamper-evident is enough). For removal that is unsafe on the
*trust* axis: a self-consistent signature from an *unknown* key must not be able to hide evidence. So for
a **non-possessed** (ingested) removal there is **no `integrity-strict` analog** — an ingested
`untrusted_key`/`unsigned` removal is never operative-suppress unless a trusted endorsement lifts it (§3a),
and the bar is never a merely-valid unknown key. (A *possessed* `unsigned` removal still suppresses — that
is the orthogonal possession axis, not an `untrusted_key` exception.)

##### 3a. The trusted arm reads endorsements (and that is the ratification path)

A removal's arm-b "trusted" qualification is satisfied by **either** the removal event verifying `valid`
for a trusted signer **or** a **trusted endorsement** of it. An endorsement is the existing ADR-0013
construct: a co-signature member whose attesting signer resolves to a *distinct, trusted* actor,
classified `endorsement-trusted` and surfaced by `has_trusted_endorsement()`
(`src/session/projection/cosignature.rs:116-130`). The `CosignatureIndex` + `endorsement_readbacks` the
revision projection already builds (`src/session/workflow/revision_projection/mod.rs:307-341`) are the
same inputs `RemovalPolicy` reads.

**This is a deliberate, named elevation.** `has_trusted_endorsement()` is today a *non-binding*,
stewardship-plane signal — it feeds no existing binding decision (ADR-0009 resumption binding reads
`has_valid_member` only; ADR-0013 is a precision/recognition ADR, not a binding one). `RemovalPolicy` is a
**new** named reader-side policy that *elects to treat a trusted endorsement as operative for removal* —
precisely the ADR-0003 pattern ("advisory unless a specific projection policy treats it as operative").
This elevation is **scoped to removal**; it does not change ADR-0009 binding or any other consumer.

This makes the **ratification path concrete and on-model.** A relayed removal that is not yet operative
(`untrusted_key`/`unsigned`, not locally possessed, not under the `invalid` floor) is ratified by one of:

1. the reader **extending trust** to the removal's original signer (adding it to the `TrustSet`) — the
   simplest path, pure ADR-0004/0009, no new mechanism; or
2. a trusted local actor **endorsing** the removal with `shore review endorse <removal-event-id>`, which
   records an `EventSignatureRecorded` co-signature carrier over that event via `record_event_signature`
   (`src/cli/review/endorse.rs:76-79`) — the dedicated local endorsement path. ADR-0013 then classifies
   that co-signature `endorsement-trusted`, satisfying arm b.

Note two things that are **not** ratification paths. (a) Re-authoring "your own" `ArtifactRemoved` does
**not** create a new local-possession fact and does **not** endorse: the idempotency key is
`artifact_removed:<content_hash>`, so a re-`remove` of a relayed hash returns `ExistingDivergentSignature`
which `remove_content` simply counts as existing and records **no** co-signature
(`src/session/workflow/artifact_removal/mod.rs:159-168`); the first-stored (relayed) record stays
authoritative (`src/session/store/event_store.rs:56-80`). (b) The `ExistingDivergentSignature` →
co-signature *transcription* happens only on the **ingest** path (`transcribe_divergent_signature`,
`src/session/workflow/ingest.rs:200-212`) — when a relayed store already carries a divergently-signed
copy — not as a local ratification act. Local ratification is `shore review endorse`, full stop.

#### 4. The `compact`/`gc` erasure gate (irreversible, global) — a separate fixed rule

Physical byte deletion is gated on two orthogonal checks, **independent of the render preset**:

- **Eligibility (a fixed rule, same as the default-policy operative test):** `compact` erases the bytes of
  a removal **only** when that removal is **not** under the `invalid` floor **and** qualifies under **arm a
  (possession) OR arm b (trusted signer or trusted endorsement)** — the same arms and the same `invalid`
  floor as §3, but applied as `compact`'s *own* fixed rule rather than read from the active render preset.
  This means: (a) a laxer render policy (`advisory`, or `trusted-strict` which omits possession) can never
  make a byte-delete reachable on weaker trust — render preset is simply not an input to erasure; (b) an
  ingested `untrusted_key`/`unsigned` removal is erasure-eligible **only** once a trusted endorsement (or
  trust extension) lifts it via arm b; and (c) an `invalid` removal is **never** erasure-eligible, even
  possessed or endorsed. Whoever runs `compact` already holds filesystem possession, so their own
  possession-authored removals are eligible — keeping single-user keyless compaction working.
- **Consent (the act):** even a fully-eligible erasure is irreversible, so `compact`/`gc` surfaces the
  exact set it will permanently delete and requires explicit confirmation (a `--dry-run`/preview plus a
  consent gate), independent of the trust tier — mirroring the consent-gated `shore store migrate`. The
  eligibility rule decides *which claims may be erased*; the consent gate confirms *the
  act*. (Because erasure is independent of the render preset, an owner reading under `advisory` — which
  suppresses nothing — can still `compact` their own possessed removals; the consent gate, not the render
  view, is the safety control on the destructive act.)

A removal that is **not** erasure-eligible does not vanish: it remains an advisory render state plus a
diagnostic until ratified by §3a (extend trust, or trusted-endorse) — the right friction before a point
of no return, not a dead end.

**Optional opt-in knob (deferred default):** a stricter compact tier may additionally require the
removing actor (or endorser) to resolve to a **non-agent principal** at `occurredAt` (mirroring ADR-0010
`require-resolvable-principal`) — "a human, not an agent, authorizes irreversible erasure." This belongs
to the compact tier if anywhere; it stays **off by default** (coupling all removal to the delegation map
is heavier than the floor needs and breaks when the map is unpopulated).

#### 5. Diagnostics (advisory surfacing, never silent)

Non-operative and lower-trust removals surface as projection diagnostics so ambiguity is preserved
(never a silent no-op, never silent suppression):

- `removal_claim_unsigned` — an ingested `ArtifactRemoved` with no signature and no local possession (so
  it did not suppress under the active render policy). **Ratifiable** (§3a).
- `removal_claim_untrusted` — a signed ingested removal whose signer is `untrusted_key` under the reader's
  `TrustSet` and which carries no trusted endorsement (so it did not suppress). **Ratifiable** (§3a).
- `removal_claim_invalid` — a removal whose own inline signature verifies `invalid` (the integrity floor).
  Distinct because it is **not ratifiable** by trust or endorsement — the event must be re-issued cleanly.
- A pending-claims summary (e.g. "N removal claims are pending ratification") so an operator can see what
  *would* be removed and ratify the ratifiable ones (§3a).

These compose with — and are distinct from — the multi-state render vocabulary
(`suppressed_present` / `physically_removed` / …) that the removal implementation introduces to stop the
read surface overstating erasure; that vocabulary describes *what happened to an operative removal's
bytes*, while these diagnostics describe *why a claim is or is not operative*.

#### 6. Identity reuse after removal — a diagnostic, not a new event

Because removal is content-addressed and the log is immutable, the `ArtifactRemoved { content_hash }`
fact **already tombstones the content identity** (the immutable, content-addressed removal fact is the
content-identity tombstone, as the removal-divergences amendment records): re-capturing identical content
re-mints the same content-derived `object_id` (and, at the same git provenance, the same `revision_id` —
both are content/provenance-derived, `src/session/store/fingerprint.rs:20-35`), and
`ArtifactRemovalProjection` **re-suppresses it automatically** because the operative removal over that
content hash still holds. **This is the desired privacy property — erasure is durable against re-capture —
not a gap.** No identity-level tombstone event is added; that would be redundant with the content
tombstone, would grow the signed wire surface (the exact schema-break exposure that growing the signed
wire incurs), and a *blocking* tombstone would be the write-side gate the substrate rejects.

**Durable erasure (re-suppress + re-sweep), stated as the property users can rely on.** The render
state `suppressed_present` (an operative removal whose content blob is present on disk) is exactly the
observable condition after a re-capture re-materializes content — and the response is a **re-sweep**: the
next `compact` re-erases any `suppressed_present` blob whose removal is still operative. This needs **no
sweep ledger and no new event**, derived from the event set + the on-disk inventory `compact` already
computes (`on_disk_blobs`, `artifact_removal/mod.rs:550-580`). Note the model deliberately does **not**
distinguish *never-compacted* from *recaptured-after-compact*: `compact` records no durable sweep fact
(it deletes files and emits no event — `compact_emits_no_event`, `artifact_removal/mod.rs:1038-1051`), so
both present as `suppressed_present`, and that is sufficient because the response (re-sweep) is identical.

**The diagnostic, scoped to the event-observable condition.** `identity_reused_after_removal` fires when a
**distinct** capture (`WorkObjectProposed`) event binds a content hash that carries an *operative*
`ArtifactRemoved` — i.e. a work-object identity is (re)used over removed content — a condition derivable
purely from `from_events`. It distinguishes *content/object reuse* (a second capture references the same
removed bytes — expected, auto-suppressed) from *revision reuse* (a more anomalous identity collision,
which the idempotent capture path does not normally produce and so is worth a human's eye). It does **not**
claim to detect "after compact," which is not observable. The diagnostic is a small projection addition;
the **decision** is recorded here, and its implementation lands with this removal-authorization work.

### Consequences

#### Accepted

- The one genuine ADR-0016 trust gap closes: a relayed/mistaken removal can no longer unconditionally
  suppress evidence, and can never irreversibly delete bytes, without meeting a named trust/possession/
  endorsement bar — while the zero-key single-user local floor is preserved exactly (possession arm a).
- The reversible-by-policy / per-reader render axis and the irreversible / global erasure axis are made
  independent: a render preference can never lower the bar for permanent destruction.
- Erasure gains a consent/dry-run safeguard orthogonal to trust — two independent checks before the point
  of no return.
- Erasure is durable against re-capture (re-suppress + re-sweep), and identity reuse is surfaced rather
  than silently re-materialized — with no new event type and no write-side gate.
- Ratification of a relayed removal is on-model and uses existing machinery: extend trust to the signer,
  or record a trusted endorsement via `shore review endorse` (an ADR-0013 co-signature carrier).
- The mechanism is on-model: reader-side named policy over verified identity + possession + endorsement,
  the same advisory→operative discipline the substrate applies everywhere, finally wired to removal.
  KurrentDB buys the equivalent safety with a privileged controller + chunk lock + authz; Shoreline buys
  it with content-addressed convergence + reader-side policy, having no controller.

#### Costs

- A `RemovalPolicy` type, its threading through the read options builders (alongside `with_trust_set` /
  `with_verification_policy`, `src/session/workflow/revision_projection/identity.rs:60-89`), a
  trust-aware removal projection (the projection must retain per-hash claim provenance — origin marker,
  signer, and endorsement set — to evaluate the arms under a `TrustSet`, replacing the flat `BTreeSet`),
  the `compact` eligibility + consent gate, and the new diagnostics are net-new work.
- A reader who does not configure trust and is reading a non-possessed store will see relayed removals as
  *advisory diagnostics* rather than as suppressions until trust or an endorsement is established — a
  deliberate, safer default.

#### Rejected

- **Keeping removal unconditionally operative** (status quo) — the removal trust gap; contradicts ADR-0003 and
  ADR-0016 §6.
- **One threshold for suppress and erase** — conflates the per-reader render axis with the irreversible/
  global erasure axis and lets a display preference authorize permanent destruction.
- **`untrusted_key` operative-suppress for an ingested removal** (an `integrity-strict` removal analog on
  the trust axis) — lets any self-consistent signature from an unknown key hide evidence; the removal
  trust hole with a thin lid.
- **Re-authorship as the ratification path** — broken by content-keyed idempotency: a local re-`remove`
  dedups to the first-stored relayed event (`event_store.rs:56-80`); ratification is trust or endorsement
  instead.
- **Local-user-only operative** — too restrictive as a blanket rule; it kills the legitimate trusted-relay
  case (a trusted reviewer on another machine flags and removes a leaked secret). Its instinct is honored
  only at the irreversible step, via the possession arm of the erasure gate.
- **Requiring a non-agent principal by default** — coupling all removal to the delegation map is heavier
  than the floor needs and breaks when the map is unpopulated; offered only as an opt-in compact-tier knob.
- **An identity-level tombstone event** — redundant with the content tombstone, grows the signed
  wire surface, and a blocking variant is a write-side gate.
- **A trusted endorsement overriding an `invalid` inline signature** — considered and rejected: for an
  adversarial, irreversible operation, a broken inline signature is a hard integrity floor, so the event
  is re-issued cleanly rather than vouched back to life by an endorsement over suspect bytes.

### Revisit Triggers

- A real multi-agent workflow needs a removal proposal treated as operative without verification/possession/
  endorsement (reopen via ADR-0003's executive-policy exception, naming the executive behavior directly).
- Cross-store / federation redaction requires propagating the *operative* decision, not just the event.
- The deferred non-agent-principal compact knob is wanted on by default (orgs requiring human-authorized
  erasure) — promote it from opt-in to a named preset.
- Tier-2 inline-body removal lands — the authorization model must extend to event-payload bytes, not just
  artifact blobs.

### What does not change

`sigVersion` stays 1; the `ArtifactRemoved` payload stays `{ content_hash }`-only and convergent; the
idempotency key, the immutable event log + capture-join projection (§3), remove-only (§5), the
content-hash convergence unit, and the mirror-reality scope boundary (§7) all stand verbatim. This
amendment adds only **reader-side policy and gating** over the existing, unchanged events.

**Status:** Accepted; lands with the removal-authorization implementation work. The original ADR-0016
text above and its top-level **Status: Accepted** are unchanged.

## Amendment: Removal Convergence, Reference Stability, and Erasure Inventory (2026-06-23)

**The original decision stands; this amendment records conscious divergences and forward-binding
invariants only, with no model-rule, wire, or convergence-property change.**

The KurrentDB comparison sorted this ADR's removal mechanism against KurrentDB's scavenge + redaction
subsystems. Four properties resolve **in Shoreline's favor by construction** — KurrentDB buys them with
a global log position + a privileged controller, which the substrate rejects; Shoreline buys the same
safety with content-addressing. They are recorded here as conscious divergences and forward-binding
invariants.

### A. The removal-convergence unit is the **content hash**, not a position+clock

KurrentDB makes two replicas converge to the same removed set with a replicated **ScavengePoint**
(frozen global position + frozen `EffectiveNow` clock + weight threshold). Shoreline has no global
order to freeze and needs none: the `ArtifactRemoved { content_hash }` event is itself the deterministic
convergence point. Two peers removing the same content emit a byte-identical payload → identical
`payloadHash` → one converged fact (§6; `same_content_hash_converges_across_sessions`). The **logical**
removed set converges (event-set); the **physical** swept set is intentionally *local and divergent*
(`compact`/`gc` is a non-event maintenance action, §5 / ADR-0008 allocation rule) — each peer collects
its own bytes when it chooses. There is, by design, no "same physical removed state across peers"
requirement.

**Forward-binding note for future declarative retention (ADR-0016 Revisit Triggers):** if a
declarative retention policy is ever added ("drop snapshots older than N / keep last K"), it MUST
**freeze the policy inputs into a resolved `content_hash` set at decision time** and emit ordinary
`ArtifactRemoved` facts — the direct analog of ScavengePoint freezing `EffectiveNow`. Peers must
converge on the resolved hashes, never independently re-evaluate a clock-relative rule (which would
diverge, since `occurredAt` is non-convergent first-stored provenance). Today's manual `remove` already
does this implicitly (the selector resolves to hashes before any event is emitted).

### B. Durable references are **offset-independent**; `compact` is delete-only — no relocation map

KurrentDB needs a per-chunk **PosMap** to keep logical addresses stable after a scavenge repacks
survivors into a new chunk. Shoreline takes the opposite fork: every durable reference is content- or
identity-addressed (`events/<sha256(idempotencyKey)>.json`, `artifacts/<…>/<sha256>.json`), and the
storage model **bans** projections from depending on filenames, filesystem order, or any positional
signal. `compact` does **not** rewrite or repack — it only deletes whole content-addressed blob files,
so nothing relocates and there is no address to stabilize. After compaction a removed blob's reference
still resolves *logically* (the content hash persists in the immutable capture event; the projection
renders the explained "content removed").

**Invariant (pre-commit now):** this holds **only as long as `compact` stays delete-only**. If a future
`compact` ever *repacks* (e.g. coalescing many small note artifacts into a pack file for inode
pressure — a deferred storage option), it MUST keep the `content_hash → location` lookup as a
**rebuildable read-side index** (the PosMap analog) and MUST NOT let any durable reference point at a
pack offset.

### C. The erasure sweep MUST enumerate the **physical** artifact directories, never a projection

KurrentDB's redaction deliberately uses a *keep-duplicates* index read to locate every physical copy of
an event. Shoreline's sweep already does the strongest version of this: `compact_store` walks the raw
filesystem (`on_disk_blobs` over `artifacts/objects` + `artifacts/notes`,
`src/session/workflow/artifact_removal/mod.rs:550-580`) and matches each *physical file* against the
removed-hash set — it never consults a deduplicated projection to decide what to delete. Content-addressing also makes physical duplicates impossible within a store (one shared blob),
so "enumerate every copy" reduces to "delete the one shared file."

**Invariant (pre-commit now):** the erasure sweep MUST enumerate the physical artifact directories,
never a read-side/dedup projection. If a future read-index ever caches artifact presence, the sweep must
keep walking disk so a stale/deduped index can never hide a physical copy from erasure. And if Tier-2
inline-body removal lands (deferred), the sweep must **additionally scan event payloads** (those bytes
live in `events/`, not the two artifact directories).

### D. Compaction is **byte-only**; no event is ever discardable → all lifecycle facts are anchors

KurrentDB scavenge can discard non-structural events and keeps a `$streamDeleted` tombstone specially.
Shoreline is *stronger*: removal targets content bytes only, and the immutable event log is never
rewritten or tombstoned — so **every** lifecycle fact (capture/`WorkObjectProposed`, association +
withdrawal, supersession, assessments, and the `ArtifactRemoved` fact itself) survives any compaction
automatically. The immutable `ArtifactRemoved` event **is** Shoreline's tombstone-equivalent: "this
content existed and was removed" is permanently explainable and irreversible by design (remove-only).
Record explicitly: *which lifecycle facts are non-discardable anchors? — all of them, by construction.*

**The identity-reuse half is decided elsewhere (now resolved):** whether to add an
`identity_reused_after_removal` **diagnostic** was the open question this record deferred. It is now
**decided** in the companion removal-authorization amendment (§6), and this record adopts that predicate
verbatim: the diagnostic fires when a **distinct** capture (`WorkObjectProposed`) binds a content hash that
carries an **operative** `ArtifactRemoved` — a **diagnostic only, no new event type**. (The `operative`
scoping matters: an advisory/non-operative removal claim must NOT read as an identity tombstone.) The
content-addressed `ArtifactRemoved` already tombstones the content identity, and durable erasure
(re-suppress + re-sweep) needs no new mechanism. This anchors record and the removal-authorization
amendment's §6 are the two halves of the same removal-identity question; they agree (the `ArtifactRemoved`
event is the content tombstone) and do not overlap in what they assert.

---

**Status:** Accepted; lands with the removal implementation work, after the removal-authorization
amendment above. The original ADR-0016 text and its top-level **Status: Accepted** are unchanged.
