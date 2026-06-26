# ADR-0017: EventTarget Identity Layering, Object/Revision Separation, and Engagement/Journal Naming

**Status:** Accepted. §A1–A3 / §A5–A6 owner-approved 2026-06-19; the §A4 Engagement/Revision layering
refinement and its subsequent additions — all detailed in §A4 — re-approved through 2026-06-22:
engagement-derivation and the derived engagement lifecycle (no `EngagementOpened`/`Closed` events); the
`WorkObjectProposed` generative-move name (ranging over `WorkObjectType { Revision, TaskAttempt }`; surface
`shore review capture` unchanged); the wire-naming convention; the store-scope-layer rename
`Ledger`/`LedgerId`/`ledger_id`/`TargetRef::Ledger` → `Journal`/`JournalId`/`journal_id`/`TargetRef::Journal`
with the journal-cardinality clarification (a store holds one-to-many journals) and the store-as-log /
journal-as-scope rationale; the internal-naming-follows-the-layer rule (with `object` = content identity +
its stored artifact while `DiffSnapshot` keeps `snapshot`); and the `shore inspect` substrate-inspector
surface exception. Landed via the substrate-reshape implementation work, which also reconciles the
already-landed ADR-0016 to this ADR's target shape via an **ADR-0016 amendment** (ADR-0016's `session_id`
anchor predates this ADR's `session_id`→`journal_id` rename).
**Date:** 2026-06-19
**See also:** [substrate thesis summary](../substrate-thesis-summary.md) and
[substrate language](../substrate-language.md), plus ADR-0018 for supersession. This ADR **combines** the
identity-and-naming analysis into one record: identity layering is §A1–A3, and the Engagement/Journal
naming + the advisory-generative rule are §A4–A5. **Amends** in-repo ADR-0003 (advisory-first → extends to
generative moves; its Revisit Trigger 4 is the named hook) and ADR-0007 (writer-act vocabulary → capture is
the generative move any actor performs), each via its own appended amendment. **Coupled ADRs:** ADR-0018
(event-borne supersession replaces lineage — depends on this ADR's `Revision`), ADR-0019 (blackboard
attention/liveness — independent, parallel), and the one-time owner-run store migrations (depend on this
envelope).

## Context

*(Implementation status: this Context describes the **pre-reshape baseline** that motivated the decision.
The substrate-reshape implementation has since landed the reshaped `EventTarget` triple **and** the §A1/§A4
`Ledger`→`Journal` rename — current source is `EventTarget { journal_id, subject, track_id }`
(`src/session/event/target.rs`), with tests asserting the legacy `ledgerId` is absent — together with the
**deferred wire-key renames** (the third break, §A6) and the non-wire internal-identifier renames. The
"today …" descriptions below are the historical baseline, not current source.)*

Shoreline's event envelope addresses every fact through `EventTarget` (`src/session/event/target.rs:8-33`).
Today it is a flat bag of one required `session_id` plus eight optional fields — `work_unit_id`,
`work_object_id`, `work_object_type`, `review_unit_id`, `revision_id`, `snapshot_id`, `track_id`,
`subject` — with five constructors that each populate a different subset. *Which* optionals are set is
what silently encodes which domain a fact belongs to. This is three identity regimes accreted over time:
a pre-substrate `work_unit_id` vestige (read only by two sentinel guards,
`src/session/projection/state.rs:169,216`); the review triple `review_unit_id`/`revision_id`/`snapshot_id`;
and the substrate pair `work_object_id`/`work_object_type` that **only** task-domain events populate.

The substrate thesis claims `ReviewUnit` and `TaskAttempt` *share* `WorkObjectId` identity
(`docs/substrate-thesis-summary.md:32`). In the code this is false: the one place the two domains were
meant to converge is the one place they diverged — review kept its bespoke triple, tasks alone adopted
`work_object_id`. Two further facts ground the reshape:

- `revision_id` is literally `snapshot_id` re-prefixed — both are `…:sha256:<same descriptor hash>`
  (`src/session/store/fingerprint.rs:147,198`), a frozen 1:1 redundancy. And `review_unit_id`
  additionally folds a local-only `source_repo_namespace` (`fingerprint.rs:185-195`), so two clones of
  identical content get the same snapshot but **different** review-unit ids — identity does not converge.
- `target` is hashed **in full** inside both the TBS signing view (`src/session/event/tbs.rs:25`) and
  `event_record_hash` (`src/session/event/record_hash.rs:33`). Any field added, removed, or reordered in
  `EventTarget` invalidates every stored signature, all cross-store convergence, the co-signature
  transcription path, and the golden byte fixtures. There is no partial reshape.

This ADR is written **abstraction-down**: Shoreline is a canonical tamper-evident *agent-work
code-review* journal. The substrate's generality beyond code review is an open, falsifiable test
(the owner does not in practice route prose through the journal), **not** an
established platform. Nothing here is decided as if a general coordination substrate were proven.

Breaking changes are welcome: Shoreline has only a very early release, is unpromoted, and the owner is
the sole user. The migration's in-scope stores are the owner's working repos; their signing keys are **all
held locally**, so re-signing and co-signature re-attestation are real but **lossless** and the signature
blast radius does not constrain the reshape. The migration is handled by the one-time owner-run store
migrations.

## Decision

### A1. Reshape `EventTarget` to a non-optional addressed triple

`EventTarget` becomes exactly three fields:

```
EventTarget {
    journal_id: JournalId,          // the journal this event files into — a named logical scope over the store-log (renamed from session_id; see A4)
    subject:   TargetRef,         // non-optional; the single typed address into the work object
    track_id:  Option<TrackId>,   // attribution lane; absent for journal-scoped events (init/carrier)
}
```

The eight-optional bag is gone (only `track_id` survives, now the new triple's one optional field).
`subject` is a non-optional, externally-tagged `TargetRef`
(`src/model/work_object.rs:38-43`) — its variant *is* the domain regime, so "which regime" becomes
type-driven instead of "which optionals happen to be populated." The genuinely subject-less detached
co-signature carrier (which addresses its target through its payload's `target_event_id` +
`target_event_record_hash`, `target.rs:93-110`) gets a single, fieldless, journal-scoped variant
**`TargetRef::Journal`** — the carrier files into its journal by the envelope's `journal_id`, and its
co-signed target stays addressed by content in the payload, never duplicated onto the envelope. This is
the one carrier shape used throughout; `subject` is never `None`.

### A2. `subject` is the single shared identity; retire `work_unit_id`; fold the substrate pair

- **Retire `work_unit_id`** and its `"work:default"` sentinel (a pre-substrate vestige carrying no
  semantic load). It is *written* by note import on both `ReviewInitialized` and every
  `ReviewNoteImported` event (both use `EventTarget::new(session_id, work_unit_id)`, `workflow/import.rs:71`)
  and is embedded in the imported-note idempotency key (`import.rs:187-192`); but it is *read* only through
  the two sentinel guards (`projection/state.rs:169,216`), never by a projection decision. Retiring it drops
  the field and changes the imported-note key scheme (so those note files re-key/rename) — folded into the
  one-time owner-run store migration.
- **Fold `work_object_id` / `work_object_type` into `subject`.** "Is this a task attempt" becomes
  `matches!(subject, Task(..))`, and the object id lives inside the variant. The review and task domains
  now address through *one* `TargetRef`. This is the actual realization of the "shared identity" the
  thesis claimed but never delivered — it is `subject`, not a `WorkObjectId` sibling field.

### A3. Separate the redundant object/revision pair; object identity is content-only and git-optional

Today `revision_id ≡ snapshot_id` (the same descriptor hash) and both ride sibling envelope fields. This
ADR **separates** them into two layers, **both carried in the generative move's payload, not the
envelope**:

- **Object identity** = `sha256` of a **content-only projection** of the diff (paths, statuses, rename
  `old_path`, row kind+text, intrinsic flags). It **drops** git blob OIDs, modes, line/hunk numbers,
  `FileId`/`HunkId`, and the local-only `source_repo_namespace` from identity (they remain as provenance
  in the stored artifact), and **sorts the file set by path** so identity is set-based, not array-order
  dependent (`fingerprint.rs:232,253` currently hash a `Vec` positionally). This makes two clones of
  identical content **converge** to the same object id, and makes a non-git object (e.g. a markdown set)
  expressible — generality becomes *robust* rather than accidental.
- **Revision** is a **distinct** logical layer that folds the object id plus **optional** git provenance
  (base/target commit+tree OIDs). It is the carrier of "this content positioned at these (optional) git
  endpoints," and is what supersession references (ADR-0018).
- **Per-diff git endpoints live on the Revision** (the code binds endpoints 1:1 with the snapshot and
  mints `revision_id` from the same hash, `fingerprint.rs:179-206`). The **only** genuinely
  activity-scoped git fact — the capture-time branch ref (`RevisionRefAssociated` in current source — the
  ADR-0014 wire rename has landed; `capture.rs:274-302`) — stays scoped to the
  engagement/journal and is already non-identity. Object
  identity folds neither.

### A4. Naming and the typed Engagement: `Journal`, `Engagement {Review|Task}`, `Revision` (the work object), `Object` (its content sub-layer)

*(Revised 2026-06-19 to absorb the owner-ratified Engagement/Revision layering refinement. The §A1–A3
decisions are unchanged; this sharpens the names and folds `ReviewUnit` into `Revision`.)*

The word "session" is overloaded — `SessionId` (`src/model/ids.rs:22`, renamed from `reviewId`) names the
per-event top scope, and it collides with the "working session" activity. Rename to free the word and name
each layer. The physical container is the **store** — the single append-only event log every envelope files
into (§A1); within it the logical layers nest **Journal ⊃ Engagement ⊃ Revision ⊃ (Object)**, the journal
being the top *logical* scope over the log (see *Journal cardinality* and *Why `Journal`* below):

| Layer | Old name(s) | New name |
|---|---|---|
| top logical scope over the store-log (the envelope's `*_id`) | `SessionId` | **`Journal` / `JournalId`** (A1) |
| the domain activity around an object — **typed** | *(none — `session_id` was a dead sentinel)* | **`Engagement` / `EngagementId`**, with an **`EngagementType { Review, Task }`** |
| the captured, fact-carrying **work object** (`WorkObjectType` variant) | `ReviewUnit` / `review_unit_id` | **`Revision` / `RevisionId`** — `ReviewUnit` folds into it; `WorkObjectType { Revision, TaskAttempt }` |
| the **content sub-identity** of a Revision (not a work object) | `SnapshotId` | **`Object` / `ObjectId`** — `Revision { id, object_id, git_provenance }` (A3) |

**Journal cardinality — a store holds *one-to-many* journals.** The **store** is the single append-only
event log (`.git/shore`, **unpartitioned** — one content-addressed event set); a **journal** is a *named
logical scope over that log*, and `journal_id` is a **per-event scope** (a filter applied on read), not a
store constant or a physical partition. One store may therefore carry events addressed to several
`journal_id`s: the review path currently defaults to a single `journal:default` (so plain `shore review`
work is **one journal per store today**), but import adapters mint a **distinct journal per imported
session** (e.g. `journal:claude:<uuid>`, `adapter/claude_code/parse.rs`), and ADR-0019's *whole-`Journal`*
liveness scope presumes the multiplicity. So the journal is the **top logical scope within** the store, not
the store itself; engagements group *within* a journal (above), and that grouping is journal-bounded.
Whether the log should ever be **physically** partitioned by journal (vs. the current filter-on-read over
one log) is a separate open layout question, deliberately **out of scope here** — this ADR decides the
naming and the logical layering, not the on-disk layout (tracked in issue #202).

**Why `Journal`, not `Ledger` or `Log` (rationale refreshed 2026-06-22).** The layering separates two
things one name had blurred: the **store** is the single **append-only event log** — the durable substrate
every envelope files into — and a **journal** is a *named logical scope over that log* (a *topic*, in the
generic named-stream sense; one store, one-to-many journals, above). Naming the **scope** layer is the open
question, and two facts settle it:

- **`Log` names the substrate, not the scope.** The append-only log *is* the store, and we already call
  that layer the **store** (`shore store`, `.shore`, `store.json`) — so "log" is the store's role, not a
  second name for the scope, and minting a `Log` type would collide head-on with the diagnostic logging
  already in the tree (`tracing` / `tracing-subscriber`). The scope riding over the log is what we are
  naming, and `Journal` names it precisely: the chronological *book of original entry* for that scope, whose
  derived views (`SessionState`, the current-assessment / supersession-DAG / per-revision projections) are
  the *ledger-like aggregations*.
- **`Ledger` named the derivation, and cued blockchain.** In accounting the journal is the source and the
  ledger is the posted derivation, so "Ledger" named the append-only scope after its own derived views; it
  also strongly cues *distributed-ledger / blockchain*, which Shoreline explicitly is **not** (ADR-0019: no
  executive controller; convergent, not consensus).

`Journal` is accurate for a scope over an append-only log, carries no blockchain connotation, and is an
established systems term (journaling FS / DB journal / event-sourcing journal) with no collision in the
tree. The store-as-log / journal-as-scope split is a layering *intuition*, **not** an adoption of any log
system's machinery: Shoreline does not physically partition the log per journal (an open layout question,
#202), and a journal keys on **provenance** (the review default; a per-import session) rather than
subject-matter, so the named-stream analogy is structural only. The container is **internal + wire
vocabulary** (`journal_id`, `TargetRef::Journal`) — **no primary `shore review` CLI/UI surface change** (the
`shore inspect` substrate-inspector excepted — §A4 (viii)); the wire rename rides a
second signed-store break (§A6), whose throwaway re-keying migrator maps `ledger_id`→`journal_id` (and the
value prefix `ledger:`/`session:` → `journal:`). (The earlier choice
"Ledger" was only to free "session" and because it was the dialogue's word, never a
technical claim, so nothing load-bearing is lost.)

**`Review` is the `Engagement` type, not the unit.** "Review" names the *domain/activity* (sibling to
`Task`), and is the precise home for the noun — distinct from the `shore review` verb and from the
captured unit. A **Revision is to review what a TaskAttempt is to task**: one captured instance of the
domain's work, carrying observations/assessments, what supersession operates over, what `File`/`Range`
anchors key on.

**`ReviewUnit` folds into `Revision` — and the convergence is from the reshape, not a rename.** This is
load-bearing, so state the mechanism precisely. Today the ids fold *different* material:
`revision_id = rev:git:sha256:<snapshot_hash>` (the *snapshot descriptor* hash — which today still folds
git blob OIDs and, for commit ranges, the `base_tree_oid`/`target_tree_oid` pair, `fingerprint.rs:224-253`;
"content-only" is what the **reshaped Object** becomes after A3's content projection, not this hash) while
`review_unit_id = sha256(source_repo_namespace, source, base, target, snapshot_id)` folds the **endpoints**
on top (`fingerprint.rs:186-200`). They converge **only because A3 redefines the layers**: the **endpoints
move into the Revision** (as `git_provenance`) and the **content moves into the Object**, after which
review_unit and revision fold *identical* material (`object_id` + git provenance). `source_repo_namespace`
leaves identity (A3); the `source` worktree-vs-range bit is recoverable from the endpoint *types* and the
single-variant capture-mode discriminants retire, so nothing review_unit folded
survives outside the reshaped Revision. The **semantic clincher**: ADR-0018 removes the lineage container, so
no handle survives *above* the captured position — the position **is** the unit. Therefore
`ReviewUnit ≡ Revision`. (The wire-level rename — `review_unit_id`→`revision_id`, `ReviewTargetRef::ReviewUnit`
→`Revision`, `WorkObjectType {Revision, TaskAttempt}`, the idempotency-key prefixes — rode the first
signed-store break (the `EventTarget` reshape); the residual frozen `reviewUnitId` content-id digest keys
complete in the third break (§A6). Each rides a reshape break, not a separate migration.)

**`Object` is a content sub-layer of `Revision`, not a `WorkObjectType` sibling.** `Object` is the
git-optional content hash (A3) that lets two clones' revisions converge and that `--object` groups by; it
is referenced by `Revision.object_id` (**many revisions → one object**, preserving dedup), never addressed
or fact-attached directly. Promoting `Object` to a third work-object kind would re-introduce the
object/revision/work-object three-way the reshape removes.

**`TargetRef` is two-level, with one type-enforced domain axis (the diverged-`WorkObjectId` guard, applied
one layer up).** The outer variant is the *domain* (`Review` | `Task`, the same word as `EngagementType`);
the inner is the *work object* (`Revision`, plus the `File`/`Range` sub-anchors). So
`TargetRef::Review(ReviewTarget::Revision{ revision_id })` replaces the old
`Review(ReviewTargetRef::ReviewUnit{ review_unit_id })`; `File`/`Range` key on `revision_id`. The domain
now appears structurally in two places — `EngagementType` and the `TargetRef` outer variant — and **they
must never disagree.** Keep **one domain axis with a single source of truth**: the subject's domain is
**structurally derived from / type-checked against its engagement** (a `Review` engagement addresses only
`Revision` subjects, never `TaskAttempt`), **never an independently-asserted wire value**. This is exactly
the `WorkObjectId`-claimed-but-diverged failure (one fact, two encodings) that this ADR exists to fix —
do not re-create it one layer up.

**The typed `Engagement` is abstraction-down, not a platform.** `EngagementType` is closed at
`{ Review, Task }` because those are the **two domains that exist** — code review and the task-supervision
prototype the substrate thesis keeps as a flat work object. A third type (e.g. prose) is a future *stress
test* the enum could admit **without claiming or building it**. Add no machinery that only a hypothetical
third type would need.

`engagement_id` is **carried in the generative move's payload, not the envelope** (owner-ratified
2026-06-19: `journal_id` on the envelope, `engagement_id` in payload). It is the activity binding that
co-locates competing revisions of the same engagement; projections may use it as a **hint/index**, but the
**authoritative grouping derives from the supersession DAG** (see the engagement-derivation rules below).
This keeps the envelope
minimal, matches the `supersedes`-in-payload placement (ADR-0018), and follows the substrate's
project-over-store discipline; the surface is indifferent to the placement.

**An Engagement is *derived*, never *initiated*.** `capture` is the revision-layer generative move — it records a `Revision`; it does **not**
"open" an activity. There is **no `EngagementInitiated` event and no stored "current open activity"
scalar**; the Engagement is the **supersession-connected component of revisions within a `Journal`**
(ADR-0018), a **derived, authoritative** grouping. The payload `engagement_id` is a **write-time-derived
hint/cache** of it, **never an actor-supplied value**. The rules, implementation-complete:

1. **Write-time derivation of the `engagement_id` hint.** A **root** capture (empty `supersedes`) derives
   its `engagement_id` from its own (root) revision. A superseding capture whose predecessors are
   **present** **inherits** their shared `engagement_id` (exact). The writer computes it from the
   `supersedes` edge; an actor never supplies it.
2. **Dangling predecessor — no gate.** Because ADR-0018 accepts a `supersedes` target not yet in the store (a
   `supersession_target_missing` diagnostic, **never** a write rejection — ADR-0003), the writer cannot
   always read a predecessor's `engagement_id`. Rule: **accept the write**; derive a **deterministic
   provisional** `engagement_id` from canonical inputs available at write time (the `supersedes` *target
   ids*, e.g. the lexicographically-least); emit the missing-target diagnostic. On **backfill** the
   grouping projection reconciles via the same self-heal path — the connected component absorbs the
   revision, and a provisional hint that now disagrees is overridden by the projection (and surfaced, rule
   3), **without re-stamping** the immutable events.
3. **Cross-engagement bridge — surface, do not gate.** A capture whose `supersedes` spans revisions in
   **two engagements** is likewise **not rejected**. Its stored `engagement_id` is a deterministic
   representative (lexicographically-least of its predecessors'); the grouping projection **unifies the
   connected component** and emits an **`engagements_merged { merged_ids, by_revision }`** diagnostic.
   Older revisions are **never re-stamped**.

**Single source of truth.** The supersession DAG (within the `Journal`) is **authoritative**;
`engagement_id` is a derived hint — **exact when predecessors are present, provisional + diagnosed when
they are not, never independently asserted, and never *silently* divergent** (every disagreement surfaces
as a diagnostic). This is the §A4 domain-axis discipline applied to engagement membership, honoring
ADR-0003's no-write-gate and ADR-0018's surface-don't-reject posture.

The "I'm engaging with object X" cue *before* any revision is produced is **ADR-0019 derived attention
state**, not an engagement write — it never mints journal state.

**The engagement lifecycle is *observed*, not declared.** "Open / in-progress / closed" is a **derived
projection**, never lifecycle events. An engagement's **start** is its root revision (the supersedes-empty
generative move); its **end**, *where a domain defines one*, is the domain-natural terminal. A `Review`
engagement is terminal when its **current-assessment projection resolves to a single un-replaced
`Accepted`** (`src/session/event/assessment.rs` `ReviewAssessment::Accepted`); a *competing/ambiguous*
current-assessment set keeps the engagement **in-progress** (ADR-0018 / ADR-0008 ambiguity preservation —
ambiguity is not a terminal). A `Task` engagement has **no terminal in V1**: the task vocabulary
(attempt / checkpoint / observation) defines no completion move, so its derived lifecycle is start +
in-progress only, and a task terminal is **deferred** to a future task-completion evaluative move (never a
generic `EngagementClosed`). *In-progress* is an engagement with un-superseded current heads and no
resolved terminal. There is **no
`EngagementOpened`/`Closed` event**: an empty "opened" engagement would be the current-state scalar the
substrate forbids, and "closed" would be either a derivable signal or a write-gate (append-only journals
have no enforceable "end"). The three move kinds bracket the lifecycle — generative opens/advances,
evaluative (assessment) is the terminal, coordinative pauses — so Shoreline **observes** the lifecycle from
the domain moves rather than imposing one. (A future need for an *authoritative* archived state is a named
ADR-0003 executive-policy Revisit Trigger, not a default.)

**The generative move is internally `WorkObjectProposed`; `capture` is a surface verb only.** The
domain-neutral generative move/event is **`WorkObjectProposed`** — it produces a **work object** (a
`Revision` in the review domain, a `TaskAttempt` in the task domain: the two `WorkObjectType` variants),
and the `…Proposed` suffix foregrounds the advisory-generative rule (§A5: the produced work object is a
*proposal*, never operative). The name **ranges over `WorkObjectType { Revision, TaskAttempt }` rather than
either variant**, because the generative move is **not review-specific**: the task
domain's capture is equally a generative move over a different object. It replaces the per-domain
`ReviewUnitCaptured` / `TaskAttemptCaptured` families §A2 collapses into one generative family; *which*
work object a given move produced is carried by the payload's `subject` (`TargetRef`), not encoded in the
event name. **`shore review capture` stays the user-facing review surface verb**: the surface stays
domain-named while internal vocabulary differs. "capture" / "the generative move" wherever it appears in
§A2/§A5, ADR-0018, and the ADR-0007 amendment all denote `WorkObjectProposed`. (Naming the move
`RevisionProposed` after the **review** work object — a *sibling* of `TaskAttempt`, not their genus — is
**rejected**: a single collapsed move named after one `WorkObjectType` variant privileges one domain and
re-introduces, in the move name, the very one-domain-favoring asymmetry §A4 removes from identity; the
`TargetRef` would disambiguate, so it is a naming-clarity defect, not a correctness one. `start`/`open` are
likewise **not** used for this move — they would smuggle in the lifecycle events rejected above.)

**Wire-naming convention (the one rule `WorkObjectProposed` and every domain-named family both follow).**
An event-type name reaches for the **abstract `WorkObject` term only when the event is genuinely
cross-domain** — i.e. it ranges over **both** `WorkObjectType` variants **and** that cross-domain symmetry is
*load-bearing* (the move is the single home of machinery that would otherwise be duplicated per domain).
**Exactly one event meets this:** the generative move `WorkObjectProposed`, the single home of `supersedes`
(ADR-0018), the derived `engagement_id` (above), and the advisory-generative default (§A5). **Every other event
names the most specific accurate thing** — a **domain work object** (`Review…` / `Task…`, including the
`Revision{Ref,Commit}{Associated,Withdrawn}` association family, ADR-0014 amendment) when it is tied to one
domain, or its **own non-work-object concept** (`InputRequest…`, `ValidationCheckRecorded`,
`EventSignatureRecorded`, `ArtifactRemoved`) when it is not about a work object at all. So
`WorkObjectProposed` is **not an exception**: it is the sole current member of the *cross-domain,
load-bearing* category, and domain-specific families are **not** to be renamed up to `WorkObject*` — that
would be the abstraction-up the reshape rejects, claiming a generality that does not exist (e.g. there
is no task-domain commit/ref association). Internal wire vocabulary may be substrate-flavored (`object_id`,
`engagement_id`); the **primary surface** stays domain-named regardless (the `shore inspect`
substrate-inspector is the one deliberate exception — see the surface rule below).

**"review" stays the only user-facing surface verb** (`shore review …`). The `unit` noun retires: within
the `shore review` namespace the captured unit is a **`revision`** (`shore review capture` creates one;
`shore review revisions` / `show <revision>` / `--revision` / `--object` per ADR-0018). `Journal` /
`Engagement` / `EngagementType` and the generative/evaluative/coordinative move vocabulary are
**internal**; `object` / `revision` are permitted domain surface terms. None of the internal
vocabulary reaches a CLI flag or a rendered UI label **on the primary `shore review` surface**. The
**`shore inspect` substrate-inspector is a deliberate exception**: its served documents and UI intentionally
expose substrate vocabulary (`journal`, `engagement`, `revision`, `object`) because inspecting the substrate
is their purpose — e.g. the inspect served key is `journalId`. The primary surface's journal-*selector* (how a
user scopes a command to a journal) stays a separate, **domain-named** question that does **not** adopt
`journal` literally.

**Internal naming follows the layer.** Where the surface rule above fixes the *user-facing* terms
(`review`/`revision`/`object` only) and the wire-naming convention fixes *event* names, this fixes the
*internal* identifiers (types, fields, fns, modules) so the vocabulary stays single-sourced: name a symbol
for **the layer it denotes**. The **domain/activity** is `review` (the `EngagementType`); a **single captured
work object** is `revision` (the `ReviewUnit`→`Revision` fold — the `unit` vocabulary retires *entirely*,
internal as well as surface); the **supersession-connected component** is `engagement` (the derived
grouping); and the **content identity** is `object` (the `obj:` hash). One distinction is load-bearing: a
set of revisions **grouped by shared content/base** (ADR-0018's base auto-grouping) is a **`revision` set**
(e.g. `grouped_revision_ids`), **not** an `engagement` — `engagement` is reserved for the
*supersession*-connected component, a different relation, so the two groupings never share a name. And
`object` names the **content identity and its stored artifact** (`ObjectArtifact`, the `artifacts/objects/`
store, the `/api/object` route — the addressable/stored content layer); the **diff-body render model**
(`DiffSnapshot`) keeps the `snapshot` name — a captured point-in-time diff, used even where no stored object
exists (live rendering), distinct from the `object` identity/storage layer.

### A5. Capture is the generative move, performed by any actor, and stays Advisory

`capture` is the **generative move**, and the substrate already lets any actor perform it — it accepts an
arbitrary `--actor` and ties identity to content, not the writer (`capture.rs:95-103,792-817`). The
author/reviewer asymmetry is **policy in the skills**, not a substrate property; a reviewer may
counter-propose by capturing a revision that supersedes the author's (ADR-0018), with no new event type and
no CLI enforcement.

**Generative moves default to, and remain, Advisory; they are never promotable to Operative.** A
`supersedes`-carrying proposal that a projection could mark "approved — implement it" is one projection
from the workflow engine the substrate refuses (`docs/substrate-language.md`, ADR-0003). This is the
highest-stakes constraint in the reshape. It **amends ADR-0003** — whose Revisit Trigger 4 ("a concrete
multi-agent workflow needs scope-bounded authority…") is the named hook — to state advisory-first
applies to generative moves, and **relates to ADR-0007** (the review act tracks the move kind; role
stays a derived persona, not a substrate fact). Note (as-built flag for those amendments):
`ReviewAssessmentRecorded` today defaults *Operative* (`src/session/event/mod.rs:73-78`); the generative
move must default and stay Advisory.

### A6. The reshape is a clean signed-store break — no shim, no dual-read, no versioned target

Because `target` is in both the TBS view and `event_record_hash`, the reshape invalidates every
signature, all `event_record_hash` convergence, the co-signature transcription branch, and the golden
fixtures — by design. There is **no** compatibility shim, **no** dual-read of the old target shape, and
**no** versioned target. In practice the reshape lands in **three** clean signed-store breaks rather
than one: first the `EventTarget` reshape (the identity triple, lineage retirement, and the generative-move
and `review_unit`→`revision` event-type/wire renames); then the `Ledger`→`Journal` container rename together
with the stored `snapshot_id`→`object_id` object-field rename; then the deferred wire-key renames — the
residual frozen `reviewUnitId` content-id digest keys (completing `review_unit`→`revision`) and the
snapshot-artifact → object-artifact concept (the artifact `schema` `"shore.snapshot"`→`"shore.object"`,
`snapshotArtifactContentHash`→`objectArtifactContentHash`, `artifacts/snapshots/`→`artifacts/objects/`,
`/api/snapshot`→`/api/object`). Each stage was decided after the prior break had already landed, so each rides its
own break rather than being folded backward. Each is a clean break with the same posture, and each is
migrated once by a throwaway re-keying migrator that is deleted afterward — a one-time owner-run migration per break;
all signer keys are held locally, so re-signing and co-signature re-attestation are real but
lossless. Legacy-rejection tests for the old shapes are deleted and the golden vectors regenerated at each
break. The reshaped
store's read path stays strict — a loud rejection of any stray old-shape file is a feature.

## Consequences

### Accepted

- **One shared identity, finally.** Review and task objects address through a single non-optional
  `subject`; the diverged `WorkObjectId` claim is resolved by the type, not papered over. The
  substrate-thesis-summary doc must reflect that correction.
- **Clone convergence and git-optional generality.** Dropping `source_repo_namespace` and git OIDs from
  identity makes two clones converge and makes a non-git object expressible. (Generality remains an open
  test — this enables it; it does not prove it.)
- **Distinct object/revision cardinality** replaces the frozen 1:1:1, enabling one object to carry many
  revisions over an engagement (the substrate for fork-tolerant supersession, ADR-0018).
- **A smaller, type-safe envelope** and ~2 constructors instead of 5; illegal "which optionals are set"
  states become unrepresentable.
- **A cleaner surface.** The CLI's one substrate-leaking noun (`lineage`) is removed by ADR-0018, and no new
  substrate vocabulary reaches the **primary `shore review` surface** (the `shore inspect` substrate-inspector
excepted — §A4 (viii)); "review" stays the only verb.
- **Costs accepted:** a total signed-store break (a one-shot, owner-run migrator); every `EventTarget`
  constructor, projection, and idempotency-key builder is touched; the golden fixtures and the
  legacy-rejection tests are regenerated/deleted; `shore review history` output changes (the
  `change_id` and duplicated-`subject` fields are decision-dead but output-live — their removal is a
  visible output diff taken at the break).

### Rejected

- **Candidate B (an explicit `object_id` sibling field on the envelope alongside `subject`).** It
  duplicates, for the whole-object case, the id already inside `subject` — exactly the duplication this
  reshape removes (`for_review_unit` commits it today, `target.rs:61-67`). `subject` is the single
  identity.
- **Keeping the flat optional triple / a versioned or dual-read target.** Dual-read is the tool for
  un-re-signable bound bytes (other actors' co-signatures); the owner has none, so a one-shot migrator
  strictly dominates. The store is unreleased; there is no reader to keep compatible.
- **`engagement_id` on the signed envelope.** It would enlarge the signed/convergence-bound target for
  envelope-level grouping the projection provides anyway; the payload placement is smaller and the
  surface is indifferent.
- **Framing this as a general coordination substrate.** Abstraction-up is the failure mode the earlier
  challenges name; this ADR hardens the agent-work code-review journal and leaves generality an open test.

## Revisit Triggers

- A second object-domain is admitted for real and passes the substrate thesis's marginal-cost bar, then needs
  identity or addressing shapes this `subject`/`Object`/`Revision` model
  cannot express without a per-domain special case — i.e. the layering would merely *rename* a new
  divergence.
- A non-local repo-namespace model lands and convergence needs a namespace concept back **outside** the
  object identity.
- The advisory-generative line is pressured: a real workflow needs a proposal treated as operative —
  reopen via ADR-0003's executive-policy exception, naming it directly, never as ordinary metadata.
- A released contract or a second user appears, so "breaking changes welcome / one-shot migrate" no
  longer holds and a compatibility strategy is genuinely required.

## Related Docs

- [Substrate thesis summary](../substrate-thesis-summary.md)
- [Substrate language](../substrate-language.md)
- In-repo `docs/adr/`: **ADR-0003** advisory-first (amended by §A5), **ADR-0007**
  writer-act vocabulary (amended), **ADR-0005** lineage (superseded by ADR-0018).
- In-repo `docs/`: `substrate-language.md`, `substrate-thesis-summary.md` — the thesis summary's
  diverged-`WorkObjectId` claim is the load-bearing correction.
