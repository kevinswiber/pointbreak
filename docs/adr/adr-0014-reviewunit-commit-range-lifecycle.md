# ADR-0014: ReviewUnit Commit-Range Lifecycle — Ref/Commit Association Events

**Status:** Accepted (owner-approved 2026-06-19); landed via the ReviewUnit commit-range lifecycle
implementation work.
**Date:** 2026-06-19
**See also:** [ADR-0004](./adr-0004-event-signatures.md) (generic `EventToBeSigned` — consumed unchanged,
no new `sigVersion`), [ADR-0005](./adr-0005-review-unit-lineage.md) (lineage supersession — kept
distinct), [ADR-0008](./adr-0008-cross-peer-conflict-policy.md) (cross-peer conflict policy — first
consumer of the reserved class-(c) codes), ADR-0015 (single-common-dir store/topology — independent axis;
this family supplies the reachability projection its §5 names).

## Context

A ReviewUnit's relationship to the commit graph is fixed at capture and cannot evolve afterward. A
**worktree capture** is born *floating*: its `target` is `ReviewEndpoint::GitWorkingTree { worktree_root }`
with **no commit OID**, while a **commit-range capture** carries two `GitCommit` endpoints. So a worktree
capture cannot be placed on the commit graph by reachability, cannot get a structural merge-status, and
cannot be branch-filtered by liveness — only by a stored label, which does not exist today. There is **no
structured commit-association event** (the only workaround is freeform observation prose, which a
deterministic projection cannot read), and there are **no git reachability wrappers** (`merge-base
--is-ancestor`, `branch/for-each-ref --contains`) in `src/git/`.

Three substrate facts constrain the design:

- **Capture is immutable and keyed by id only.** `review_unit_captured_idempotency_key` is
  `review_unit_captured:<review_unit_id>`; `event_id` derives from the idempotency key **alone**;
  `record_event_once` **hard-conflicts** on same-key/different-`payloadHash`. Therefore any capture-context
  or commit relationship **must be a separate event**, never a field grafted onto
  `ReviewUnitCapturedPayload` (the capture-context identity constraint).
- **Convergence is over `{event_id, payload_hash}`.** `eventSetHash` is computed over
  `{event_id, payload_hash}`; load-bearing meaning must live in the identity/idempotency key, never an
  excluded-from-identity payload field (ADR-0004 convergence invariant).
- **Vocabulary is collision-checked**: `associate`/`associated` is clean; rejected as already-loaded —
  `finaliz*` (closure connotation), `anchor*` (the note/diff `Anchor` struct), `abandoned`
  (`InputRequestResponseOutcome::Abandoned`), bare `withdrawn`/`retract*` (substrate retraction,
  `docs/substrate-language.md`). `orphaned` (`ResolutionStatus::Orphaned`) is reused **only** as a derived
  graph condition.

## Decision

### 1. A symmetric four-event family on two axes

Add four append-only events (house style `ReviewUnit<Thing><PastVerb>` → snake_case wire, registered in
`src/session/event/kind.rs` with the `as_str()` mirror and the all-variants round-trip test):

| variant | wire | axis |
|---|---|---|
| `ReviewUnitRefAssociated` | `review_unit_ref_associated` | ref / provenance |
| `ReviewUnitRefWithdrawn` | `review_unit_ref_withdrawn` | ref / provenance |
| `ReviewUnitCommitAssociated` | `review_unit_commit_associated` | commit graph |
| `ReviewUnitCommitWithdrawn` | `review_unit_commit_withdrawn` | commit graph |

Payloads (new module `src/session/event/association.rs`, each referencing the unit via
`EventTarget.review_unit_id` + `target: ReviewTargetRef::ReviewUnit{…}`, no new `ReviewTargetRef` variant):

- **`ReviewUnitRefAssociatedPayload`** — `{ ref_association_id, target, ref_name, head_oid }`. Stores
  **both** the ref name (humans query by it) and the head OID (ref names are mutable/reusable).
- **`ReviewUnitCommitAssociatedPayload`** — `{ commit_association_id, target, commit: ReviewEndpoint }`,
  reusing `ReviewEndpoint::GitCommit { commit_oid, tree_oid }` verbatim. A born-floating worktree capture
  **acquires** its target commit via this event ("the work landed as commit X").
- **`ReviewUnitRefWithdrawnPayload`** / **`ReviewUnitCommitWithdrawnPayload`** —
  `{ <axis>_withdrawal_id, target, <axis>_association_id }`, naming the association they retract.

### 2. Capture-identity-safe idempotency keys; the distinguisher is in the key; writer and track are excluded

- Associations follow the house key shape with the **edge distinguisher in the key**:
  `review_unit_<axis>_associated:<review_unit_id>:<source_key>`, where `source_key` is the edge
  distinguisher — `commit_oid` (commit axis) or `ref_name@head_oid` (ref axis). Distinct edges → distinct
  keys → distinct `event_id`; a true re-record hits the same key and `payloadHash` → `Existing`. **`track`
  is resolved out of the key and the id** (envelope-only, recorded for provenance): an association is one
  shared edge fact that must converge across independently-authored copies, so — exactly like the writer —
  the track never enters identity.
- Withdrawals key on the **association id** they retract (the `input_request_responded:<id>:…` precedent),
  so a withdrawal is a separable, idempotent member that content-references its association.
- Each `*_association_id` is a sha256 over `{ review_unit_id, edge distinguisher }` and **EXCLUDES the
  writer** (unlike `build_assessment_id`, which folds writer). Rationale: an association is a *structural
  edge* that must converge across independently-authored copies — two writers recording the same edge must
  compute the same `event_id` and `payloadHash` (ADR-0004 convergence invariant). The writer remains
  envelope-only.

### 3. Withdraw-only — no supersession-by-reference

There is **no `replaces_*_ids` machinery** on this family. An association is a content-idempotent
structural claim, not a competing judgement, so it has no "which-of-two-current-judgements wins" problem
for supersession to solve (contrast assessment `replaces_assessment_ids`, which disambiguates judgements).
A **correction** is `Withdrawn(old) + Associated(new)` — two append-only facts whose `associated −
withdrawn` fold equals a supersession link with less machinery. A withdrawal **records unconditionally** (no
write-time referent check; a missing referent is the expected cross-peer case, handled by the projection
diagnostic in §7, not by rejecting the write — which `validate_assessment_relationships` does and which is
federation-wrong). Withdrawal is **terminal**: a later identical `Associated` returns `Existing` and does
**not** revive a withdrawn edge (set-subtraction, no timestamp ordering — ADR-0008 forbids timestamp
arbitration); reviving requires a genuinely new edge.

### 4. Every status is derived; nothing is stored

The recorded events of this family are **exactly** the four above; no lifecycle status is stored. The
projection folds `ReviewUnitCaptured` (to seed capture-time endpoints) **plus** the four association
events. The *current commit set* per unit is the **captured target commit** (for commit-range captures,
whose `GitCommit` target is already in `ReviewUnitCapturedPayload`) **∪** `associated − withdrawn` (the
`open_input_request_count` positive-minus-negative fold). From it:

- `anchored` ≡ the current commit set is non-empty — i.e. a **commit-range capture is born `anchored`**
  from its captured `GitCommit` target (no auto-association), and a **worktree capture starts `floating`**
  until a `ReviewUnitCommitAssociated` lands. `floating`/`anchored` are **events-only, no git**.
- `merged` / `live` / `orphaned` — derived from the current commit set **plus** git reachability and a
  **read-time integration ref** (a parameter/config, **not** stored — `merged` is relative to which ref
  you ask about and would go stale if persisted). The breadth of `merged` is **broad-by-default,
  narrow-when-configured** (see Resolved in implementation).

No status is persisted; a canonical/derived value is **withheld** under ambiguity (the lineage-head
discipline). The read surface presents the **full per-OID matrix** plus a headline that is withheld under
ambiguity (see Resolved in implementation).

### 5. A dedicated, git-free projection + a read-time reachability enrichment

Add `ReviewUnitCommitRangeProjection::from_events` in a new `src/session/projection/commit_range.rs`,
modeled on `ReviewUnitLineageProjection::from_events`: a pure, deterministic single-pass fold, **never
touching git**, into a `BTreeMap<ReviewUnitId, …>`. Its inputs are `ReviewUnitCaptured` (seeding
capture-time endpoints — so commit-range captures seed `anchored`) plus the four association events; all
other event types are no-ops. The four events are also **no-ops in `SessionState`** (the existing lineage
no-op dispatch precedent), so `SessionState` stays status-free. The per-unit view carries the current
ref/commit sets, `anchored`/`floating`, the withdrawn-edge history, and per-unit `diagnostics`; any
canonical/derived value is **withheld under ambiguity**.

Reachability (`merged`/`live`/`orphaned`) is a **separate read-time enrichment** (`enrich_liveness(view,
repo, integration_ref)`) the read surface invokes when a repo is in hand — it never enters the pure fold.
The graph condition is a **new `CommitGraphCondition::Orphaned` enum**, distinct from the note-anchor
`ResolutionStatus::Orphaned`.

### 6. Net-new git reachability plumbing

Add to `src/git/command.rs` (status-tolerant, mirroring `git_path_is_ignored`'s `[0,1]` probe):

- `git_is_ancestor(repo, ancestor, descendant) -> Ancestry { Ancestor, NotAncestor, MissingObject }` over
  `merge-base --is-ancestor` (exit 128 / bad object → `MissingObject`).
- `git_for_each_ref(repo, patterns) -> Vec<RefEntry>` to enumerate live-ref tips.
- `git_object_exists(repo, oid)` over `cat-file -e` for a precise orphan reason.

Reuse `git_worktree_list` for worktree HEADs. **Default live-ref scope** = local branches (matched by the
`refs/heads/` prefix so nested names are included) + all linked-worktree HEADs + remote-tracking refs
(`refs/remotes/`) **when present**. A **missing/gc'd/rebased object degrades to `orphaned`, never an
error**; an unavailable repo yields "reachability unknown" (omit the merged/live fields). Compute batched
per repo, deduped by OID, in an in-process ancestry cache; **never persist** reachability.

### 7. Conflict diagnostics realize ADR-0008's reserved class-(c) codes

This family is the **first consumer** of ADR-0008's reserved `ambiguous_supersession` /
`retraction_target_missing`. Both are **shoreline projection diagnostics** (computable from the unioned
event set), surfaced never persisted, self-healing on backfill, with **no timestamp arbitration**:

- **`divergent_commit_association`** — the concrete code realizing the reserved `ambiguous_supersession`:
  two un-withdrawn commit associations for the same unit with different OIDs. Surface **both**, withhold the
  headline condition, pick no winner (the `lineage_forked_successor` / `append_duplicate_semantic_diagnostics`
  surface-all posture).
- **`retraction_target_missing`** — a withdrawal whose named association has not backfilled; the withdrawal
  has no effect yet and the diagnostic vanishes when the association arrives.

### 8. Signing & convergence — unchanged contracts

All four events sign under ADR-0004's generic `EventToBeSigned::from_event` (fully generic over
`event_type`) with **no new `sigVersion`**, and converge under the existing `record_event_once` /
`eventSetHash` machinery. No amendment to ADR-0004/0005/0008 is required.

### 9. CLI surface & worktree-only auto-record

The CLI provides these capabilities:

- **Record and withdraw** a commit association (by commit rev, resolved to a `GitCommit` endpoint) and a
  ref association (by ref name + head OID), each reusing the write house shape
  (`--repo`/`--review-unit`|`--lineage`/`--track`/`--sign-key`, JSON out) — **but no `--idempotency-key`**:
  the association/withdrawal key is the canonical, **non-overridable** edge key (an arbitrary override
  would defeat cross-writer convergence). A reader lists current/all associations per axis. **No
  `--replaces`** — corrections are withdraw-then-re-associate (§3).
- **Auto-record `ReviewUnitRefAssociated`** (current ref name + head OID) inside `capture_review` on the
  **worktree arm only**, after the capture event; **skip on detached HEAD**, best-effort, never blocking
  capture. Commit-range captures get no auto-association.
- **Branch-filtered history** ("history for `feat/X`") as a read-side filter selecting units by ref
  association, with a **label-vs-liveness** selector (label = recorded ref association, offline; liveness =
  reachability), and the four events filterable in `review history`. This **supersedes** the earlier
  `worktree_root` scoping proxy.

The landed spelling is a `shore review association` noun with flat verbs `associate-commit` /
`withdraw-commit` / `associate-ref` / `withdraw-ref` / `list`; `--ref` (alias `--branch`) + `--by
{label,liveness}` on `review history` / `review unit list`.

### 10. Distinct from ADR-0005 lineage

A commit association is the *same* unit's work landing as commit X; lineage supersession is a *new* unit
replacing an *old* one as a thread head. The association family must never set or move a lineage head, and
does not touch the `review_unit_lineage_*` events.

## Consequences

### Accepted

- Worktree captures bridge onto the commit graph after the fact (structural merge-status, branch-by-
  liveness), and "review history for `feat/X`" becomes answerable by recorded label and by reachability.
- The recorded vocabulary stays minimal (four events, zero stored statuses), and the design is
  federation-correct: same edge converges across writers; missing referents self-heal; no winner-picking.
- Net-new cost is bounded: the pure fold is O(events) like the existing reducers; reachability is a
  batched, cached, read-time enrichment paid only when liveness is requested.
- Landed **independent of** the ADR-0015 topology collapse (no topology dependency, no new `sigVersion`, no
  ADR amendment); resolves the capture-context identity problem and supplies ADR-0015 §5's reachability
  projection.

### Rejected

- **A "finalization" event / `finalized`/`abandoned` statuses** — "finalize" implies closure, contradicting
  append-only/never-closes; the terms also collide (`abandoned` = an input-request outcome). Replaced by
  the association/withdrawal framing.
- **Supersession-by-reference (`replaces_*_ids`) for this family** — an association has no competing-
  judgement to disambiguate; write-time referent validation is federation-wrong. Withdraw-only is simpler
  and ADR-0008-aligned (§3).
- **Storing any lifecycle status** — re-introduces stale-persisted-status and breaks convergence (load-
  bearing meaning outside the identity key). All statuses derived (§4).
- **Folding `writer` or `track` into the association id** — would prevent the same edge from converging
  across writers. Both are envelope-only (§2).
- **Folding this into ADR-0015** — topology (where the store lives) and lifecycle (how a unit relates to the
  commit graph) are independent axes; folding would have coupled a parallel-safe decision to unrelated
  commit-grouping, privacy, and store-rebuild preconditions. They are siblings with a consumer/supplier
  cross-reference.
- **Auto-recording a commit association at capture** — a worktree capture is born floating; its target
  commit does not exist yet. Only `ReviewUnitRefAssociated` is auto-recorded (§9).

## Revisit Triggers

- A real **revive-the-same-edge** use case emerges (would reopen the terminal-withdrawal / no-revival rule).
- **Substrate retraction** lands (re-evaluate the domain-scoped withdrawal against the generic primitive).
- **ADR-0015 topology** changes in a way that pressures the reachability-projection consumer seam.

## Resolved in implementation

The open questions the draft carried were resolved as follows:

- **Track scope → track-free.** Association/withdrawal keys and ids fold neither writer nor track; an
  association is one shared structural edge, and a track in the key would split it per-track and defeat
  cross-writer convergence. `--track` is still accepted on the write verbs and recorded envelope-only.
- **Headline vs matrix → per-OID matrix; headline withheld under ambiguity.** Liveness enrichment returns a
  per-`commit_oid` `CommitGraphCondition` vector plus a headline that is `Some` only when all current OIDs
  agree and no diagnostic fired, else `None`.
- **`merged` breadth → broad-by-default, narrow-when-configured.** With no integration ref, `merged` means
  "ancestor of any live-ref tip other than its own tip"; an optional integration ref narrows it to
  "ancestor of that ref."
- **Detached-worktree-HEAD `live_branch` label → `(detached @ <short-oid>)`.** Honest; never fabricates a
  branch name from the worktree basename. Named branches render as their short ref name.
- **CLI surface → the `shore review association` noun** with flat verbs `associate-commit` /
  `withdraw-commit` / `associate-ref` / `withdraw-ref` / `list`; `--ref` (alias `--branch`) + `--by
  {label,liveness}` on `review history` and `review unit list`.

## Amendment: Conditional auto-record of the capture-time ref for commit-range captures whose tip is HEAD (2026-06-19)

**Status unchanged.** ADR-0014 remains **Accepted**. This amendment **widens** §9's capture-time
auto-record; the original decision otherwise stands. No new event type, payload, idempotency key,
projection rule, or `sigVersion` — purely a change to *when* the existing `ReviewUnitRefAssociated`
auto-record fires.

**What changes.** §9 and §4 stated "commit-range captures get no auto-association." Extend the
best-effort capture-time `ReviewUnitRefAssociated` auto-record to **also** fire for a commit-range
capture **when the range's target endpoint OID equals the current HEAD OID** (and HEAD is not detached).
Equivalently, the `CaptureSourceSpec::Worktree`-only gate becomes a **"capture tip == current HEAD"**
gate: always true for a worktree capture (its base is HEAD), and true for a commit-range capture iff
`target.commit_oid == git_head_oid(worktree_root)`. The recorded edge is the current branch ref + head
OID, identical to the worktree path; **detached HEAD still records nothing** (no ref name is fabricated);
still best-effort, never blocking capture, failure degrades to the `ref_association_auto_record_skipped`
diagnostic.

**Why.** The ref axis is **provenance** (§1: `ReviewUnitRefAssociated` = "captured while HEAD was on
branch B at oid X"), distinct from the commit axis that derives `anchored`/`merged` (§4). The original
"no auto-association for ranges" reasoning was specifically about the *commit anchor* — a range is born
`anchored` from its captured `GitCommit` target, so it needs no commit association. But it left ranges
with **no provenance/branch signal**, which (a) makes the common
`shore review capture --base <integration>` form — whose target **defaults to HEAD**
(`src/session/workflow/capture.rs:332`) — un-`--branch`-filterable, and (b) under the ADR-0015
single-common-dir collapse leaves a range capture with **no read-time scoping signal**, so two worktrees
each reviewing *their own* branch's range would collide on a bare `unit show` (an ADR-0008 selection
error) instead of each resolving its own. Recording the branch ref when the range genuinely **ends at
that branch's tip** gives a precise provenance + scoping + branch-filter signal for the common case.

**Why conditional (target == HEAD), not unconditional.** A range capture's checked-out branch is
meaningful provenance only when the range terminates at that branch's tip. An arbitrary historical range
(`--base v1.0 --target v1.1` captured while standing on `main`) has a target ≠ HEAD; recording `main`
there would be provenance **noise** that pollutes `--branch main` with an unrelated range. Such captures
correctly continue to get no auto-association and rely on the **read-time fail-open** (a
capture with neither a worktree path nor a ref association resolves as the current worktree's) plus the
commit-OID grouping — both of which ship regardless and remain the safety net.

**What does not change.** `anchored`/`floating` and `merged`/`live`/`orphaned` stay derived from the
**commit** set and are untouched by this ref edge — a commit-range capture stays **born `anchored`** from
its `GitCommit` target (§4). Withdrawal/terminality (§3), convergence (the deterministic, writer/
track-free `ref_association_id`, §2), idempotency, and signing (§8) are all unchanged. The recorded
vocabulary (the four events) is unchanged.

**Implementation.** Relax the worktree-only capture-time ref auto-record gate from
`matches!(source, Worktree)` to "tip == current HEAD," reusing `auto_record_capture_ref_association`
verbatim (it already resolves the branch ref, skips detached HEAD, and signs). For a range capture, the
tip is `fingerprint.target` (`ReviewEndpoint::GitCommit { commit_oid }`); compare to
`git_head_oid(worktree_root)`. Lands in the same phase as the read-time scoping it serves; the read-time
fail-open stays in place as the complementary safety net.

## Amendment: Rename the Association Event Family `review_unit_*` → `revision_*` (2026-06-21)

**The original decision stands; this is a vocabulary rename only.** ADR-0014's model — the symmetric
four-event family on two axes (§1), the capture-identity-safe writer/track-free idempotency keys (§2), withdraw-only
terminality (§3), all-statuses-derived (§4), the git-free projection + read-time reachability enrichment
(§5/§6), the ADR-0008 conflict diagnostics (§7), signing/convergence with **no new `sigVersion`** (§8), the
CLI surface (§9, the `shore review association` noun), and the prior 2026-06-19 conditional-auto-record
amendment — are **all unchanged**. Only the *names* move.

**Context.** The substrate reshape renamed the review-domain work object `ReviewUnit` → `Revision`
(`WorkObjectType::ReviewUnit`→`Revision`; `ReviewTargetRef::ReviewUnit`→`Revision`; the wire tag
`review_unit`→`revision` — ADR-0017 §A4). This family's payload **bodies** already moved to the reshaped
target (`ReviewTargetRef::Revision`, addressed through ADR-0017's `EventTarget.subject`), but its four
**event-type names**, their **wire `eventType` values**, and their **idempotency-key prefixes** were missed
and **at authoring still carried** `review_unit_*` (`src/session/event/kind.rs:36-39` /
`src/session/event/association.rs:36,61,85,107`). **Implementation status, 2026-06-21: the substrate-reshape implementation has since
landed the wire/EventType/idempotency rename** — current source is now the `Revision*` `EventType` /
`revision_*` wire + idempotency prefixes (`kind.rs:13-16,36-39`), and tests assert the legacy `review_unit_*`
types no longer decode — so this amendment **records and ratifies** that rename (the Rust payload-struct
symbols are a still-pending lockstep cleanup; see below). It was **migration-blocking**: a one-shot migrator's `review_unit`→`revision`
remap (the owner-run migration; ADR-0017 §A6 — the first of its three breaks) over any still-`review_unit_*` legacy event would
otherwise produce a `revision`-keyed event the (now `revision_*`) core would reject. So the rename rides the
**same signed-store break as the `EventTarget` reshape** (the first of §A6's three breaks), not a separate
migration.

**The rename.** The §1 house style `ReviewUnit<Thing><PastVerb>` becomes `Revision<Thing><PastVerb>`:

| old variant | old wire `eventType` | → new variant | new wire `eventType` | axis |
|---|---|---|---|---|
| `ReviewUnitRefAssociated` | `review_unit_ref_associated` | `RevisionRefAssociated` | `revision_ref_associated` | ref / provenance |
| `ReviewUnitRefWithdrawn` | `review_unit_ref_withdrawn` | `RevisionRefWithdrawn` | `revision_ref_withdrawn` | ref / provenance |
| `ReviewUnitCommitAssociated` | `review_unit_commit_associated` | `RevisionCommitAssociated` | `revision_commit_associated` | commit graph |
| `ReviewUnitCommitWithdrawn` | `review_unit_commit_withdrawn` | `RevisionCommitWithdrawn` | `revision_commit_withdrawn` | commit graph |

**Idempotency-key prefixes rename in lockstep** (§2). The association key shape becomes
`revision_<axis>_associated:<revision_id>:<source_key>` (`source_key` still `commit_oid` on the commit axis,
`ref_name@head_oid` on the ref axis); withdrawals become `revision_<axis>_withdrawn:<association_id>`. Because
`event_id` derives from the idempotency key alone, **this changes the derived `event_id` of every event in
this family** — which is exactly what the one-shot owner-run migration re-keys for the whole store (ADR-0017 §A6
/ the migrator), so it is absorbed there and is **not** a separate migration. The capture-identity-safe key discipline
(§2: edge distinguisher in the key; `writer`/`track` excluded; `*_association_id` as a writer-free sha256
over `{ revision_id, edge distinguisher }`) is **unchanged** — only the literal `review_unit_`→`revision_`
prefix moves. The payload **field** names (`ref_association_id`, `commit_association_id`,
`ref_withdrawal_id`, `commit_withdrawal_id`) never carried `review_unit` and are **unaffected**; the Rust
payload-struct symbols (`ReviewUnitRefAssociatedPayload` → `RevisionRefAssociatedPayload`, etc.) rename in
lockstep with their `EventType` variants (implementation mechanics, not a wire change beyond the table).
**Implementation status, 2026-06-21: the wire/EventType/idempotency rename has landed (`kind.rs`), but the
Rust payload-struct symbols are a *still-pending* lockstep cleanup — live source still names them
`ReviewUnit*Payload` (`src/session/event/association.rs:27,51,76,99`).** This is a Rust-symbol rename only
(no wire effect) and finishes within the substrate-reshape implementation work.

**Why `Revision*` and not `WorkObject*`.** The commit-range / ref association lifecycle is
**review-domain-specific**: it associates a *review work object* to the commit graph, and the task domain has
no commit/ref-association analog (verified — no task-domain association events exist). So this family names
the review work object `Revision`, consistent with §A4's domain-specific naming. This is **deliberately
distinct** from the one *cross-domain* generative move, which collapses review **and** task and therefore
took the domain-neutral name `WorkObjectProposed` (ADR-0017 §A4): domain-specific families keep the domain
work-object name; only the collapsed move is domain-neutral. (Abstraction-down: no task-association machinery
is added on speculation.)

**Relationship to the rest of the reshape.**
- ADR-0017 §A4 already moved the **payload target** to `ReviewTargetRef::Revision` addressed via
  `EventTarget.subject`; the original §1:53-54 description (`EventTarget.review_unit_id` +
  `ReviewTargetRef::ReviewUnit{…}`) is superseded by that reshape, and this amendment completes the matching
  **event-type / wire / key** rename so the family is fully `revision`-vocabularied end to end.
- ADR-0018 retires the lineage family; §10's "does not touch the `review_unit_lineage_*` events" is mooted
  (those events no longer exist after ADR-0018), but the association family itself is **orthogonal to and
  unaffected by** lineage retirement — association (this *revision*'s work landed as commit X) remains
  distinct from supersession (a *new* revision replaces an old one), now expressed via ADR-0018's `supersedes`.
- Signing is unchanged: all four events continue to sign under ADR-0004's generic
  `EventToBeSigned::from_event` with **no new `sigVersion`** (the `eventType` string is already fully generic
  in the signing view; only its value changes, which the reshape break re-signs).

**What does not change (restated for the implementer).** No new event type, payload field, projection rule,
diagnostic, CLI verb, or `sigVersion`. The four-event count, the two axes, withdraw-only terminality, the
derived per-OID matrix / withheld-headline-under-ambiguity, broad-default `merged`, the
`divergent_commit_association` / `retraction_target_missing` diagnostics, and the
`shore review association` surface all stand verbatim — read with `revision` substituted for `review_unit`
in the four event names, their wire values, and their idempotency-key prefixes.

## Amendment: Divergence Means Competing Landing Claims — Not Accretion (2026-07-09)

**The original decision stands.** The four-event family (§1), idempotency keys (§2), withdraw-only
terminality (§3), all-statuses-derived (§4), the §10 association-vs-supersession distinction, signing
(§8), and the CLI surface (§9) are all unchanged. This amendment re-scopes **when
`divergent_commit_association` fires and what withholds the landing headline** (§5/§7), adds one git
plumbing helper (§6), and adds a **write-time advisory guard** on `shore association record --commit`.
Tracked as [issue #443](https://github.com/withpointbreak/pointbreak/issues/443).

**What was wrong.** §7's predicate — two un-withdrawn commit associations with different OIDs
(`src/session/projection/commit_range.rs:328-342`) — fires on states the model itself blesses: a
commit-range capture's target plus its landed squash/merge commit, and successive passes landing
commits on one revision over time. Commits accreting on a revision are a history, not a competition.
Compounding it, `headline_for` (`src/session/workflow/commit_range_liveness.rs:443-457`) withholds the
headline whenever *any* diagnostic is present or per-OID conditions disagree, so the expected end state
of a successful review + landing renders as `⚠ commit associations diverge … landing: unknown`
(`src/cli/association.rs:376-391`) — a healthy record that reads as broken, which in practice sends
agents to `shore capture --supersedes` when recording the association was the whole point (§10).
Meanwhile the predicate cannot catch the mistake it guards against: an association to the *wrong*
revision is usually a **single** edge, invisible to any edge-count rule.

**Divergence, re-scoped.** Two current landing claims are divergent only when they **compete to be the
same landing**; everything else is history. Concretely:

- **The capture-target edge never participates.** `CommitEdgeSource::CaptureTarget`
  (`commit_range.rs:57-64`) is provenance — where the review happened — not a landing claim.
  Divergence is assessed among `CommitEdgeSource::Association` edges only. This alone un-flags the
  standard squash-landing pattern.
- **Same tree = rewrite, not divergence.** Two current association edges with distinct commit OIDs but
  the same `tree_oid` (already in the payload) are content-equivalent (rebase/cherry-pick). The pure
  fold — which keeps its §5 git-free discipline — emits the informational
  **`rewritten_commit_association`** for this case and makes **no divergence claim of its own**; the
  fold-level distinct-OID predicate is removed.
- **Ancestry decides, at enrichment time.** The read-time liveness enrichment (§5, already git-backed)
  computes the **maximal** current association claims with one `git merge-base --independent <oids…>`
  call (new status-tolerant helper `git_independent_commits` beside `git_is_ancestor`,
  `src/git/command.rs:319-333`). A chain of successive landings has one maximal claim — never
  divergent. **`divergent_commit_association`** (same code string, new layer) fires iff **two or more
  incomparable maximal claims are each live or merged with distinct trees**. Orphaned claims never
  compete — a rebased-away tip shows in the per-OID matrix (and is worth withdrawing) but is no longer
  a candidate landing. Missing/gc'd objects degrade per §6 (orphaned, never an error, never a
  divergence claim); an unavailable repo makes no divergence claim (headline already degrades to
  unknown). No timestamp arbitration — topology, not clocks — so the ADR-0008 class-(c) posture holds;
  this narrows, and still realizes, the reserved `ambiguous_supersession`.

**Headline, re-scoped.** The landing headline is the condition of the **unique maximal live-or-merged
association claim**; it falls back to the capture target's condition when no association edges exist,
and reads `orphaned` when every current claim is orphaned. It is withheld only under real divergence
(or when liveness is unavailable). The blanket `!diagnostics.is_empty() → None` rule in `headline_for`
is dropped — an unrelated `retraction_target_missing` or `rewritten_commit_association` no longer
blanks the landing status. Enrichment-level divergence surfaces through the same per-unit
`diagnostics` the read surfaces already render; the `association list` ⚠ digest rewords to match (it
must no longer suggest "associate the landed commit" in a state reachable only by having done so).

**Write-time advisory guard.** The moment to catch a wrong-revision association is
`associate_commit` (`src/session/workflow/association/mod.rs:337`), where both the commit and the
revision's bound snapshot are in hand. Compare the commit's changed paths
(`git diff-tree --no-commit-id --name-only -r`) against the captured `DiffSnapshot`'s paths: an empty
intersection adds the advisory **`commit_association_content_mismatch`** to the result document (and a
text-lane warning), naming a better-matching current revision when one exists. **Advisory, never
blocking** (ADR-0003 posture): the association records regardless, and the check degrades silently
when the snapshot bytes are suppressed/removed or git is unavailable. Tree/patch equality short-circuits
as an exact landing.

**What does not change (restated for the implementer).** No new event type, payload field, or
`sigVersion`; nothing new is stored — both new codes are derived diagnostics, and the guard writes
nothing. Withdrawal, idempotency, convergence, and the surface-both/pick-no-winner posture for real
divergence are unchanged. §10 stands verbatim: association = the *same* revision landing as commit X;
supersession = a *new* revision replacing an old one. A commit landing after capture or assessment is
recorded with an association and is never grounds to recapture or supersede.
