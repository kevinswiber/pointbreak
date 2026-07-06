# ADR-0033: The User-Level Family Store Tier (Direction C)

**Status:** Accepted (owner-approved 2026-07-06); landed via plan 0117 (user-level family store
tier), PR #<FILL IN AT LANDING>.
**Date:** 2026-07-06
**See also:** [ADR-0015](./adr-0015-single-common-dir-store.md) (single-common-dir
topology; precondition 4 gated this tier and §4 earmarked `link`/`unlink` as its surface),
[ADR-0016](./adr-0016-content-targeted-artifact-removal-and-compaction.md)
(content-targeted removal — governs content *within* a store; the `forget` verb here is deliberately
outside it), [ADR-0027](./adr-0027-at-rest-encryption-boundaries.md) (D5
decode-at-boundary applies to the fold), [ADR-0004](./adr-0004-event-signatures.md)
(reader-relative committed trust; the 1.0 store-format floor), ADR-0028 (id prefixes / cross-clone
convergence). Source: internal research 0031 (codex-approved 2026-07-06); deferral record: internal
research 0011 Q7. Resolves the remaining scope of issue #153.

## Context

ADR-0015 collapsed the store topology to a clone-local default (`.git/shore`) with an opt-in
worktree-ephemeral escape, and named a third tier — a user-level store per repository family —
as first-class but gated: on family-identity keying (explicit-link, never remote-URL auto-keying),
on a secrets-safe layout (never bare `~/.shore`; `~/.shore/keys/` outside any store repo by
construction), and on the tier staying opt-in. Issue #153 tracks that residual: review facts should
survive `rm -rf <clone>` and be shared across independent clones on one machine, offline, with zero
deployed infrastructure.

Research 0031 resolved every gate. The relay architecture (research 0030) **complements** rather
than absorbs this tier — every relay stage requires infrastructure the local-only user lacks, and
the substrate the tier needs (verified fold via `import_store_bundle_with_verification`,
payload-hash idempotency, signed-set union, ADR-0008 conflict transcription, ADR-0016 removal,
ADR-0027 D5) is already built and ratified. The durable layer is already multi-writer-safe across
processes (`create_file_exclusive` atomic publish, `src/storage/mod.rs:129-161`; conflict ladder in
`record_event_once`, `src/session/store/event_store.rs:85-146`); independent clones writing one
directory is mechanically the multi-worktree case ADR-0015 shipped. No source-identity change is
needed: content `object_id`s and commit-range `revision_id`s already converge across clones
(`src/session/store/fingerprint.rs:296-307, 126-145`), and the landed commit-OID grouping projection
(`src/session/projection/commit_oid_grouping.rs`) groups committed work on a clone-independent key.
Three claims in issue #153 are corrected by this record: `repository_family_id` is **not** minted
anywhere (`src/session/store/resolution.rs:46,59` is a `None`-only placeholder); the placement is
`<root>/stores/<family>/`, not the flatter `$XDG_DATA_HOME/shore/<family>/`; and the "cross-clone
repository namespace" bullet is over-scoped and struck.

## Decision

### 1. The tier and its placement

A user-level **family store** is an opt-in third resolution tier: one store per repository family
per machine, at `<shore-home-root>/stores/<slug>/`. The root is resolved by the **same** precedence
the keys home already implements (`SHORE_HOME` → `$XDG_DATA_HOME/shore` → `~/.shore` on Unix,
`%APPDATA%\shore` on Windows; `src/keys/home.rs:28-55`), extracted into one shared root resolver —
not reimplemented. The `stores/` segment is load-bearing: it keeps `<root>/{keys,stores}` disjoint
by construction regardless of family naming. The family directory reuses the existing store layout
verbatim (`events/`, `artifacts/notes/`, `artifacts/objects/`, regenerable `state.json` via
`ensure_store_dirs`) plus two new files: a schema-versioned `family.json` manifest (identity stamp,
hard-error on unsupported schema) and a generated `.gitignore` covering the machine-local files
(`state.json`, `registry.json`).

### 2. Resolution: one new arm, no new resolver

`resolve_store` (`src/session/store/resolution.rs:191-229`) grows one branch. Precedence:
**ephemeral opt-in > user-level opt-in > clone-local default**, with the legacy-layout hard-cutover
guard firing before the new arm regardless. `StoreMode` stays two-state (Shared/Ephemeral); the
opt-in is a separate `family_ref: Option<String>` read **only** from the git-excluded
`.shore/store.local.json` — never from the committed `store.json` — so a pulled commit can never
activate the tier and "opted in with no family" is unrepresentable. `StoreResolution` gains a
`ResolvedTier` tag (`Ephemeral | CloneLocal | UserLevel { family_ref }`) threaded through
`store_resolution_for`, so `store status` reports the real tier instead of today's hardcoded
`"local"` (`resolution.rs:41-48`).

### 3. Keying: explicit link, non-identity slug

`shore store link <slug>` promotes a clone to the family tier; `shore store unlink` detaches it —
the promote/detach surface ADR-0015 §4 reserved. The **slug is placement metadata, not an identity
token**: it names the directory and populates `repository_family_ref`, and is never folded into
`revision_id`/`eventId`/signed bytes — so it is freely renameable and entirely outside the format
floor. When `<slug>` is omitted, `link` *suggests* a default (repo basename or canonicalized remote
name) that the human confirms; inference is never the key. Guards at link time: refuse an
Ephemeral-mode worktree without an explicit override (mirroring `store migrate`); refuse a
family-stamp mismatch when the directory already exists (stops two unrelated repos unioning by
slug reuse); run a best-effort, advisory-only history-overlap check; run the sensitivity gate
(Decision 7); best-effort refuse/warn when the store root sits on an unsupported filesystem
(Decision 4).

### 4. Write model: direct shared-directory writes

All linked clones write the family store directly — the same exclusive-create, content-addressed,
payload-hash-idempotent machinery that already carries multiple worktrees writing `.git/shore`.
**No new durable-layer code, no lockfile, no lease, no broker**; per ADR-0015, any future lock must
be store-directory-scoped, and the only reserved candidate is a compaction-only lock (revisit
trigger). Non-guarantees stated plainly: the family store must live on a local POSIX filesystem —
NFS and sync-managed directories (Dropbox/iCloud) are unsupported (a real footgun: `~/.shore` looks
syncable); `compact`/`gc` should run against a quiescent family store (the compaction-vs-writer
race is inherited unchanged from the multi-worktree case, corruption-free, and benign). Per-clone
journals with union-on-read are rejected (re-forks read authority — ADR-0015's rejection carries
over harder); the verified bundle fold remains the documented fallback for odd filesystems; a sync
driver is not built (the 0030 anti-entropy model is the forward-compatible shape for a future
*remote* mirror).

### 5. Relocation semantics, with an optional verified fold

Linking is a **relocation**: exactly one authoritative write store per clone at a time. New writes
land in the family store from link forward. `link` may optionally fold the clone's existing
`.git/shore` history forward via `import_store_bundle_with_verification` (non-destructive by
default, verify-then-retire on explicit flag) — the same mechanism `store migrate` uses, against a
new target. `store migrate` itself keeps its historical worktree→clone scope and grows no `--to
user` mode. Known and accepted (Owner Decision Point A): the fold stamps `IngestVia::BundleApply`
on every folded event (`src/session/store/bundle.rs:528`), which strips ADR-0016's possession arm —
prior **unsigned** removals lose operative suppression in the family store; the recommended
disposition is document-and-re-issue (`shore store remove` re-run natively in the family store is a
fresh possessed event), surfaced in `link`'s output when the fold transports removal events.
Steady-state direct writes are unaffected (possession is store-relative:
`src/session/projection/artifact_removal.rs:192`).

### 6. Lifecycle: registry, `forget`, `store list`

Each family store carries its own `registry.json` (machine-local bookkeeping, outside the event
log, gitignored): entries record the member clone's path and are re-validated **bidirectionally** —
an entry is live iff the recorded path exists as a git repo *and* that clone's `store.local.json`
still names this family back (git-worktree's mutual back-pointer, adapted). `unlink` deregisters
proactively; `rm -rf` is caught by the next liveness sweep. **ORPHANED** (zero live entries) is
binary and re-derived on demand; idleness is reported as raw facts (last-write timestamp,
live-clone count), not a coded state. `shore store forget <family>` is the whole-store destructive
verb — deliberately **outside** ADR-0016 (no store survives to record the removal event in):
dry-run by default, `--yes` to execute, preceded by a `scan_store_inventory` report of exactly what
dies. After `forget`, a dangling `family_ref` is a **hard, actionable error** (never silent store
re-creation, never silent fallback to clone-local). `shore store list` — the first
non-`--repo`-scoped surface — walks `<root>/stores/` and reports each family's id, inventory,
live-clone count, and orphan flag. `store status` populates the already-reserved
`repository_family_ref`/`clone_ref` wire fields (opaque clone id, never a raw path) plus new
lifecycle fields. CLI copy must keep "orphaned family store" distinct from the existing
`store remove --orphans` selector (unreachable-commit content — a different orphan).

### 7. Sensitivity: gate at the one-shot boundary, promise only what holds

The `migrate_store_to_common_dir` gate ports verbatim to link/fold time: `scan_worktree_sensitivity`
with a `block` outcome refuses the link/fold **before any write** to the family store, with an
explicit override flag (fresh name — Owner Decision Point B) and the exclude-glob fix named in the
error. Each writing clone enforces **its own** sensitivity config (no cross-clone registry union;
exclude globs only suppress false positives, so the asymmetry fails safe). Write-time scanning is
explicitly deferred (the full scan is O(worktree) and wrong for routine writes; a per-capture check
scoped to the diff's files is named future work). Relay ingest gets no parallel gate (no worktree to
scan; its admission control is signature/trust). Stated non-promise: the gate scans worktree files
only — it has no lever on the two plaintext absolute-path payload fields
(`WorkObjectProposed.git_provenance.target.worktreeRoot`, `TaskAttempt.project_path`), and
`worktreeRoot` is identity-bearing (folded into `revision_id` and the signed bytes), so at-rest
redaction is structurally unavailable without a format-floor break; wire-side redaction
(`src/cli/inspect/api.rs:1010-1016`) is the ceiling.

### 8. Identity and trust: no changes

No source-identity change ships with this tier. Worktree captures legitimately stay per-worktree
(divergent `revision_id`s; converging `object_id`s and shared deduped artifacts); committed work
converges via clone-stable commit-range ids and the landed commit-OID grouping. V1 trust needs no
new machinery: every verifying read or fold is clone-anchored, so verification stays
reader-relative against the reading clone's committed `allowed-signers.json` (ADR-0004's designed
posture). The non-git trust-root resolver (relay plan 0006 anchors trust via `git_worktree_root`,
which a family store lacks) is required only if a family store is ever served directly — a named
future case.

## Consequences

### Accepted

- Review facts survive clone deletion and are shared across independent clones, offline, with zero
  daemons — as a thin placement tier over ratified substrate (no new sync/trust/conflict machinery).
- The tier is strictly opt-in and locally revocable per clone; a pulled commit can never activate it.
- The slug's non-identity status buys free rename and zero format-floor exposure.
- The compaction-vs-writer race and advisory `state.json` staleness are inherited unchanged from the
  multi-worktree world and documented, not re-engineered.
- A family store's bytes are reclaimable even after every clone is gone (`forget`), at the cost of a
  new destructive verb that deliberately records no event.
- The fold's possession-stripping for prior unsigned removals is accepted and documented (Decision 5).
- Absolute paths inside signed payloads become wider-visible in a machine-level store; the gate
  documents rather than hides this.

### Rejected

- **Infer-from-remote keying** (as the key): unstable, multi-remote-ambiguous, wrong on forks in both
  directions, credential-bearing, impossible for no-remote repos. Kept only as a link-time suggestion.
- **Anchor-hash or UUID family ids**: opaque, un-navigable, and (for anchor hashes) a re-derivation
  treadmill; a non-identity slug needs neither.
- **Committed family binding**: auto-opts-in every clone (violates the opt-in constraint) and
  publishes the family name.
- **A third `StoreMode` variant**: would attach identity to a mode bit and mint a new invalid-config
  case; an orthogonal optional field makes it unrepresentable.
- **Per-clone journals + union-on-read**: re-forks read authority (ADR-0015's rejection, stronger
  cross-clone).
- **A localhost relay mirror as the substitute**: covers the sharing problems only via a standing
  daemon and a loopback network path — the infrastructure this tier exists to avoid.
- **A cross-clone source-identity namespace** (issue #153's original bullet): every candidate pays
  an owner-run, convergence-gated re-derivation to buy nothing V1 needs.
- **A relay-ingest sensitivity gate**: nothing to scan; admission control there is signature/trust.
- **Building the sync driver now**: collapses into the fold locally; earns its keep only for a
  remote mirror (research 0030's tier).

## Owner Decision Points

Owner approval (2026-07-06) adopts the recommended disposition for each point below (A–D).

- **A. Fold possession-stripping disposition** — recommended: accept + document + surface in `link`
  output; users re-issue removals natively in the family store. (Alternatives: a same-owner fold
  carve-out — no existing mechanism, weakens the ingest stamp; fold-less link by default — splits
  the trail.)
- **B. Sensitivity override flag naming/strength** — recommended: a fresh flag (e.g.
  `--include-sensitive`) with wording that acknowledges the destination may later be synced/pushed.
- **C. Idle rendering** — recommended: raw facts only in V1 (last-write, live-clone count); no coded
  idle label.
- **D. `forget` breadcrumb** — recommended: none in V1; a forgotten family reduces to "does not
  exist."

## Revisit Triggers

- The compaction-vs-writer race is observed in practice → build the reserved store-directory-scoped
  compaction lock (never a write lock, never one-clone-one-writer).
- A family store is served directly (relay hub journal, or any non-clone-anchored verification) →
  design the parallel non-git trust-root resolver honoring plan 0006's one-root /
  read-per-verification / degrade-to-empty discipline.
- Users want family stores on synced/NFS storage → revisit the fold fallback as a supported mode.
- Cross-clone read staleness becomes a UX complaint → wire the family-scoped `event_set_hash`
  freshness short-circuit.
- The sync-strategy axis (0011 Q8: git-managed/pushed stores, git-ref substrate) activates → its own
  ADR; V1's posture is local-only, never synced, never pushed.
- Relay S1 durability-export lands on "export to a local durable home" → that home is this tier;
  re-check the `link`-less (store-only) creation path it would need.
