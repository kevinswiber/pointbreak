# ADR-0020: Pluggable Durable-Storage Backend Seam (Journal + ContentStore)

**Status:** Accepted (owner-approved 2026-06-23); landed via the durable-storage backend-seam
implementation work. **Grounding re-verified against current `main` (`43bb8c6`):** no decision changed —
the compact re-hash-before-unlink floor and the strict/lenient skip-and-diagnose read surface independently
*reinforce* D2/D3 and the byte-not-typed design (see the notes in D3, D4, D11); a few `file:line` citations
were refreshed for the shifted `event_store.rs`/`state.rs`.
**Date:** 2026-06-23
**See also:** the durable-storage backend analysis that produced this decision. Relates to: **ADR-0004**
(event signatures — the layout-independence this relies on and preserves), **ADR-0015** (single common-dir
store — the `resolve_store` choke point
this threads a backend handle through), **ADR-0016** (content-targeted removal — only the
`ContentStore` removes; the `Journal` is append-only), **ADR-0017/ADR-0018** (object identity layering,
event-borne supersession — the content-address layer this sits beneath, untouched). Composes with
the `contentEncoding` direction (`contentEncoding` sits *below* the identity layer; see O3, composes without
re-keying the store).

## Context

Shoreline persists an event-sourced store as **plain JSON files**: one file per event under `events/`
(named `sha256(idempotencyKey).json`), one file per content object under `artifacts/objects/`,
externalized note bodies under `artifacts/notes/`, and a regenerable `state.json` projection. There are
**no storage traits today** — `EventStore` (`src/session/store/event_store.rs`) and `LocalStorage`
(`src/storage/mod.rs`) are concrete structs constructed **per-operation** from a `store_dir: &Path`,
which four `resolve_*` wrappers (`src/session/store/resolution.rs`) hand to ~25 production consumers,
each following the same shape (`resolve_* → EventStore::open / LocalStorage::new → build event →
record_event_once → SessionState::from_events → write state.json`). `LocalStorage` holds *most* of the
`fs::*` code, but it is **not** the only filesystem user: the object/body **read** paths bypass it with
direct `std::fs::read` (`object_artifact.rs:116`/`:141`/`:149`, `body_artifact.rs:87`) — a coupling the
`ContentStore` wrapper must absorb (a `ContentStore::get`), and one reason the content-store surface is
larger than "swap `LocalStorage`."

The owner wants the flexibility to **swap persistence backends and experiment with new on-disk data
structures, with clear, measured trade-offs** — and to run in-process unit tests against an in-memory
store. The durable-storage backend analysis established the forces:

- **Diffability is an architectural constraint, not a preference.** The durable truth layer stays
  plain-text and `git diff`-able; binary/analytical stores are allowed *only* in the derived,
  regenerable projection layer (out of scope here, D11). "Too many files" is answered by a *diffable*
  NDJSON log, never a binary truth store.
- **Signatures and content-hashes are layout-independent (verified against code).** Every digest
  (signatures, `eventRecordHash`, `payloadHash`, object `content_hash`, note-body hash) is computed from
  typed in-memory fields via `canonical_json`, **never from storage bytes**; read-time re-derivation
  rejects on mismatch (`validate_event`, `verify_event_signature`, `decode_and_validate_object_artifact`),
  so a lossless serde round-trip validates under any backend.
- **"Too many files" is two distinct problems (measured on APFS/SSD).** A *many-small-files* event
  problem (1.93× disk amplification over a 4 KB block floor; **63× slower read-all** at 10 k events:
  144.8 ms file-per-event vs 2.3 ms NDJSON) and a *few-large-files* object problem (1.02× amplification,
  max object 128 KB) — wanting **different** fixes (a diffable NDJSON log vs. transparent compression).
- **The refactor is small in production, test-dominated overall.** ~25 production consumers thread the
  handle; the larger counts (`EventStore::open` ~149×, `LocalStorage::new` ~30–40×) are test call sites
  — why static generics are the wrong dispatch (D6) and the in-memory test-rewrite is its own stage (D10).

This ADR records the **durable-layer** abstraction only; the projection/read-model cache is a
named-but-deferred third seam (D11).

## Decision

### D1. Two byte-oriented traits over opaque bytes

Introduce two **object-safe, byte-oriented** traits (`&[u8]` in/out, not typed
`ShoreEvent`/`ObjectArtifact` operations) below the storage seam. Method names below are the recommended
canonical form for the landing; the substance is what binds.

```rust
enum CreateOutcome { Created, AlreadyExists }   // promoted from CreateFileOutcome (drop the "File"-ism)
enum RemoveOutcome { Removed, Missing }

/// Idempotency-keyed append-with-dedup over OPAQUE event bytes. APPEND-ONLY (no remove).
trait Journal {
    /// Atomic, lock-free, CROSS-PROCESS-SAFE create-if-absent keyed by the logical
    /// idempotency key (the backend maps it to its own address: the file impl hashes
    /// it to a filename; an in-memory impl keys a map on the raw key). See D3.
    fn create_event_once(&self, idempotency_key: &str, bytes: &[u8]) -> Result<CreateOutcome>;
    fn read_event_bytes(&self, idempotency_key: &str) -> Result<Option<Vec<u8>>>;
    fn event_exists(&self, idempotency_key: &str) -> Result<bool>;   // absorbs today's raw Path::exists() leak
    fn list_event_bytes(&self) -> Result<Vec<Vec<u8>>>;              // full-iteration read surface; DETERMINISTIC order (D4)
}

/// Content-addressed CAS over OPAQUE blobs, shared by objects AND note bodies. Supports remove.
trait ContentStore {
    fn put_once(&self, content_ref: &str, bytes: &[u8]) -> Result<CreateOutcome>;
    fn get(&self, content_ref: &str) -> Result<Vec<u8>>;            // absorbs today's direct std::fs::read of object/body artifacts
    fn get_if_exists(&self, content_ref: &str) -> Result<Option<Vec<u8>>>;
    fn remove(&self, content_ref: &str) -> Result<RemoveOutcome>;   // dumb unlink; ADR-0016 gc/compact (the compact re-hash floor is wrapper-level — D3)
    fn list(&self, prefix: &str) -> Result<Vec<String>>;            // store-relative refs, deterministic order (path-shaped — see Open Questions)
}
```

### D2. One backend-agnostic wrapper owns the crypto-sensitive decisions; backends are dumb byte sinks

The idempotency / co-signature classification (`Created` / `Existing` / `ExistingDivergentSignature` /
conflict, today `record_event_once`, `event_store.rs:32-86`) **and** content-hash validation live in
**exactly one** backend-agnostic wrapper above the traits. A backend implements only the byte primitives
in D1 and inherits the crypto-sensitive logic unchanged. The wrapper boundary already exists —
`record_event_once` makes the entire decision above the single `create_file_exclusive` call; the
refactor consolidates the content-hash half (today split across `write_object_artifact_to` /
`decode_and_validate_object_artifact` / `validate_note_body_artifact_bytes`) into the `ContentStore`
wrapper. The backend reports only `CreateOutcome::{Created, AlreadyExists}` (D1); on `AlreadyExists` the
wrapper reads the stored bytes back and maps to today's richer `EventWriteOutcome`
(`Existing` / `ExistingDivergentSignature` / conflict) by the typed `payload_hash` / `event_record_hash`
comparison — the classification is the wrapper's, never the backend's. **Byte-not-typed is decisive**: a
typed surface would tempt a backend to re-canonicalize or
re-serialize on read and silently shift the bytes a content-hash validates over — the frozen-content-id class
of silent signed-store break — and a dumb-byte-sink backend makes that class of
break *structurally impossible to introduce by adding a backend*.

### D3. Backend invariance obligations O1–O4 + the cross-process atomicity clause

A backend swap must break no signature and no content-hash. The verified obligations the wrapper/traits
enforce:

- **O1 — Events may re-serialize** in any byte layout *iff* the `ShoreEvent` serde round-trip is lossless
  (canonical key-sort erases byte order at hash time). The one hazard is `payload: serde_json::Value` —
  the one place a backend could round-trip lossily (float/Unicode normalization) without a compile error.
  A regression test round-trips an edge-case payload and asserts `payloadHash` stability (pins O1).
- **O2 — Objects/note bodies need deterministic, byte-identical serialization** of identical typed
  content across stores/backends — a *determinism* obligation (for #146 cross-worktree dedup
  convergence), **not** "hash the file bytes."
- **O3 — The content address / dedup key is the decoded-content `sha256`,** never raw stored-byte
  equality. Content-hash *validation* already re-derives from decoded canonical content (objects hash
  `{schema, version, snapshot}` with `contentHash` removed and re-canonicalized,
  `object_artifact.rs:197-211`; note bodies hash only `NoteBodyEnvelope.body`, `body_artifact.rs:131-138`).
  **byte-exact `ContentStore` round-trip** (`get(put(b)) == b`) is a *separate* current dedup/storage
  obligation. Stating the address over decoded content is what lets `contentEncoding`
  (which makes *stored* bytes differ per store) compose with this seam **without re-keying the store**.
- **O4 — No digest reads event position.** Signatures, content-hashes, and the `event_set_hash`
  liveness token (which sorts before hashing, `freshness.rs:23-43`) are all order-free. **This does NOT
  mean the materialized `state.json` projection is order-free — see D4.**

The **cross-process atomic create-if-absent** clause (D1) is the hardest obligation and is stated as
**cross-process**, not merely "atomic": there is no store-dir lock by design (`resolution.rs:98-101`),
so multi-worktree write-through correctness rests on it (today `O_CREAT|O_EXCL`; measured as
8-racing → 1 `Created` / 7 `AlreadyExists`). A real alternative *diffable-truth* backend meets it its own
way — e.g. **NDJSON** via an advisory lock or single-writer broker (Open Questions). (A database
`UNIQUE`/`INSERT-OR-IGNORE` is the analogous mechanism, but a binary DB is **rejected for the truth
layer** under diffable-truth — see Rejected — and belongs only to the deferred projection layer, D11; it
is not a durable-truth backend example.) A backend correct only single-process is silently lossy in
production, not just slower; the in-memory backend's
single-process weakening is a deliberately-scoped non-production exception (D7), enforced by the selector
(D8) — never a relaxation of the production contract.

**Content-hash validation also guards the remove path (reinforced by the compact re-hash-before-unlink work
landed on main).**
`compact_store` (`artifact_removal/mod.rs:478`) now **re-verifies a blob's decoded-content hash before it
erases it** (refuse-on-drift → `SweepOutcome::HashMismatchSkipped`, `artifact_removal/mod.rs:439-446`),
and the module is explicit that "identity is over decoded content, not raw bytes" — an independent landing
that *confirms* O3 and D2. The trait split is unchanged: the re-hash-before-unlink is **wrapper logic**
(read decoded bytes via `ContentStore::get`, re-hash, decide) sitting **above** the dumb
`ContentStore::remove` primitive, whose outcome stays `RemoveOutcome { Removed, Missing }` (the
compact-level `SweepOutcome` that adds `HashMismatchSkipped` is the *wrapper's*, not the backend's). A
backend implements only the dumb remove; the content-hash floor rides along in the one wrapper.

### D4. Event listing order is deterministic; the `state.json` projection is order-sensitive

`SessionState::from_events` reduces events in the order the slice is supplied (`state.rs:67`), and at
least one reducer is **first-seen-wins**: `apply_input_request_opened` uses `.or_insert(assertion_mode)`
(`state.rs:225`). Today the file backend's `list_events` (`event_store.rs:129`) returns a **deterministic
hash-sorted** order (`sha256(idempotencyKey)` filenames; the sort is produced by `LocalStorage::list_dir`'s
`paths.sort()`, `storage/mod.rs:215`, surfaced through `list_event_file_names`, `event_store.rs:163`), so
`state.json` is byte-stable.
Therefore, although O4 makes *digests* order-free, the materialized projection is **not** order-free, and
the contract must protect it:

- `Journal::list_event_bytes` MUST return a **deterministic** order. The **default file backend preserves
  today's hash-sorted order**, so `state.json` is byte-identical across the refactor (the zero-format-change
  gate, D10).
- Any **alternative** backend whose listing order differs from the file backend's must be gated by an
  **order-audit** that verifies reducer **outcome** stability — *not merely byte-stability*. A different
  order can select a different first-seen *winner* (`apply_input_request_opened`'s `.or_insert(assertion_mode)`
  would store a *different* `assertion_mode`), a **semantically** different projection, not just different
  bytes. The audit must sweep **all** reducers, not only the one named here (this ADR cites
  `apply_input_request_opened` as a *confirmed* first-seen reducer, not a claim it is the only one).
  **Alternatively**, the wrapper/projection path imposes a canonical order (e.g. sort decoded events by a
  stable key) before reducing, which moots the audit. Either way this is a per-alternative-backend
  obligation, surfaced now so it is not discovered at the first non-file backend.

This refines the research's "order-independent" claim, which was specifically about the `event_set_hash`
liveness token (it sorts), not the full state reducer.

### D5. The projection write stays off the traits; durable writes are always durable

The derived `state.json` write is a **projection, not an event**: it stays a `Durability::Projection`
*filesystem* write on the `LocalStorage` primitive (today `ingest.rs:239-243`) and is **not** routed
through `Journal`. The trait surface never grows a "write a view" method. Consequently the durable traits
**drop the `Durability` parameter** (they are always `Durable`); `Durability::Projection` survives only on
the local-storage primitive the projection writer uses directly. This keeps the diffable truth layer from
absorbing the projection layer's (permitted) freedom to be binary/analytical (D11).

### D6. Dispatch: a closed `StoreBackend` enum at the single `resolve_store` choke point

Dispatch is **dynamic, via a closed `StoreBackend` enum**, introduced at the **single `resolve_store`
choke point** (`resolution.rs:144`) and nowhere else. Because stores are constructed per-open, dispatch
cost is performance-irrelevant; ergonomics decides. **Static generics are rejected** — runtime selection
needs a dynamic branch regardless, and generics would virally parameterize ~180 sites (mostly tests)
without buying the selection. The resolver returns a backend handle instead of a `PathBuf`; consumers
change **only at their construction line**. The traits are kept **object-safe** as cheap insurance even
though a closed enum needs no `dyn`.

### D7. Deliverable impls: default-file + in-memory only; in-memory is test/experiment-only

This work ships **two impls only**: the **default file** backend (today's layout, verbatim) and an
**in-memory** backend (`Mutex<HashMap>`) for in-process tests/experiments. The in-memory backend is the
**honesty test** for the seam — if any contract clause were file-shaped, it could not satisfy it. It
**drops** crash durability (`Durability` no-op) and cross-process visibility, and **keeps** the
**backend-level** clauses (atomic create-if-absent *within the process*, byte-exact round-trip,
`get`/`list`/`remove`). The wrapper-owned crypto logic — the co-signature decision and content-hash
validation (D2) — is **not** the backend's job; it rides along unchanged for *every* backend. The honesty
test is precisely that none of those wrapper clauses turned out to be file-shaped — the in-memory backend
carries them without a filesystem; its only gaps are the two deliberate drops above (crash durability,
cross-process visibility). Its cross-process drop is a deliberately-scoped non-production exception (D3),
enforced by the selector rule in D8.

### D8. Selector: `SHORE_BACKEND` env var for file-shaped backends; in-memory is in-process injection only

Runtime selection without a rebuild uses a **`SHORE_BACKEND` env var**, defaulting to `local`,
**hard-erroring on an unknown value** (borrowing `StoreMode`'s loud posture for the *value*, using the
`SHORE_PERF` env-var *mechanism* — a developer/experiment toggle, not an operative per-store property).
The `.shore/store.json` config-file mechanism is **held in reserve** for if/when a backend becomes a
durable per-store fact (a complete `store mode` template exists to promote it).

The implementable rule for the in-memory/subprocess distinction (the gap F3 named):

- **`SHORE_BACKEND` selects only persistent, file-shaped backends** (`local`; later `ndjson`). These
  cross the subprocess boundary cleanly via `Command::env` (the test harness already does this), so the
  CLI tier can exercise a file-shaped alternative end-to-end (the child re-reads the same on-disk store).
- **The in-memory backend is selected exclusively by in-process programmatic injection** (a test/experiment
  constructs the `StoreBackend::Memory` variant directly, or sets it via a process-local API) — it is
  **never an `SHORE_BACKEND` env value**, so a spawned `shore` child can never inherit it and start an
  empty, lost-on-exit store. (`SHORE_BACKEND=memory` is therefore *not a valid value* and hard-errors,
  consistent with the unknown-value rule.) This makes the distinction a structural property of *how* the
  backend is selected, not a runtime check.

### D9. What stays `LocalStorage`-only, beside/below the trait

Not everything is abstracted. These stay `LocalStorage`-typed by nature:

- The **`store_migrate.rs` flat→nested relocator** — intrinsic directory moves with no cross-backend
  meaning (permanently `LocalStorage`-only).
- The **`.git/info/exclude` + temp-sweep helpers** in `prepare_store_writer_at` — worktree-git concerns
  keyed on `worktree_root`, not store concerns.
- The **common-dir fold** (`store_migrate_common_dir.rs`, which rides `import_store_bundle`, *not*
  intrinsic dir moves) and the **`bundle.rs` / `inventory.rs` walks** stay `LocalStorage`-typed in stage 1;
  they are bundle/inventory **consumers**, abstractable later through a `ContentStore`/`Journal` read
  surface *only if* a non-file backend needs bundle/inventory support — a consumer refactor, not a file-ism.

The **inspector** and **`cli/store.rs`** are projection/repo-keyed and change nothing.

### D10. Zero-format-change: the existing suite is the regression net and the acceptance gate; staging

Introducing the traits with the file impl as default is a **behavior-preserving refactor**: no on-disk
layout change, no new event/schema/store version, **no `sigVersion` bump, no signed-store break**: every
format-bearing constant lives in the typed model *above* storage, and there is no store-version/manifest file
to move. The layout strings (`events`,
`artifacts/objects`, `artifacts/notes`, `state.json`) and the file backend's hash-sorted listing order
(D4) are preserved verbatim, so `state.json` is byte-identical. Consequently the **existing capture /
idempotency / co-signature / gc-compact / inventory / migrate / bundle / resolver / freshness suites pin
observable behavior without asserting the trait shape** — keeping them **green and unchanged is the
acceptance gate**. Stage the work:

1. **Pure default-file refactor** — introduce the traits + the file impl + the wrapper; thread the handle
   through the `resolve_store` seam. The entire existing suite **green and unchanged** is the gate. Keep
   the D9 `LocalStorage`-only items as-is.
2. **In-memory impl** + the bounded **~16-site test-rewrite** (tamper writes + `read_dir` count assertions
   move to a `put_raw`/`insert_raw` backend hook; ~14 layout-seeding `create_dir_all` + 12 `state.json`
   reads stay on `LocalStorage` as the regression net; the 25 `tests/` integration `fs::*` calls are
   permanently subprocess-bound).
3. **Benchmark harness** for the NDJSON candidate (boardwalk fixture + synthetic changesets). Any *real*
   alternative backend's on-disk format is a later, **separately-gated** decision under
   `docs/store-migration.md` §8's convergence rule — never bundled into this trait landing.

### D11. Explicit deferral of the `ProjectionStore` seam (the three-layer model)

The architecture names **three layers**: **journal** (events), **content** (objects + note bodies), and
**projection** (materialized read views — `state.json` + `SessionState::from_events`, plus the on-the-fly
history/show/revisions/inspect reductions). This ADR abstracts the first two. The **projection layer is a
named-but-deferred seam** and is *not* symmetric with the durable traits: `Journal`/`ContentStore`
abstract dumb byte persistence (meaning lives above), while a future `ProjectionStore` would abstract
**query semantics** (the whole point of a DuckDB-style analytical projection is SQL/columnar pushdown,
not where bytes land). Today there is exactly one materialized view (`state.json`), no persistent
secondary index, and no measured query pain, so designing it now is premature.

The read surface keeps it open: full iteration (`list_event_bytes`, D4) plus the pure, order-independent
`event_set_hash` (`freshness.rs:23-43`). Note the read surface now has **strict** (`list_events`,
`event_store.rs:129`) **and lenient** (`list_events_lenient`, `event_store.rs:140`, feeding
`read_events_for_display`, `read.rs:170`) decode variants — the skip-and-diagnose path for
schema-broken events. Both decode the *same* `Journal::list_event_bytes`; the strict-vs-lenient policy is
**wrapper/projection-level, not a `Journal` concern**, which confirms byte-not-typed (D2) — the byte trait
needs no lenient variant. An incremental projector's "events since H" / stable cursor is a
**future additive** `Journal` method, not a redesign, and likely wants a real append-log backend. A
DuckDB-shaped analytical projection is its **own** future research → ADR → plan, triggered by a concrete
slow read path or an analytical query the on-the-fly reducer serves poorly (cross-revision observations,
validation-over-time, supersession-DAG traversal). This is where the diffable-truth boundary is enforced:
the projection layer is *precisely* where a binary/analytical store is allowed, because it is derived and
rebuildable from the journal.

## Consequences

### Accepted

- **Backends become swappable for experiments behind one stable seam**, with the crypto invariants
  enforced once (D2) — adding a backend can never silently break a signature or content-hash (D3), the
  highest-stakes failure mode.
- **The refactor is a pure, zero-format-change rename of the seam** (D10): ~25 production consumers change
  only at their construction line; `state.json` stays byte-identical (D4); the existing suite, green and
  unchanged, is the gate.
- **A diffable answer to "too many files"** is named and measured (D7): NDJSON for events (built later),
  compression for objects — the truth layer stays plain-text and `git diff`-able throughout.
- **In-process unit tests gain an in-memory backend** (D7) — a real but **narrow** speed win (store-heavy
  `event_store.rs` modules; removes ~2 fsyncs + ~4 syscalls per event write); **invisible to the subprocess
  CLI tier**, not pitched as a whole-suite speedup.
- **Costs accepted:** a net `dyn`/enum indirection per store-open (negligible at per-command frequency); a
  bounded ~16-site test-rewrite (D10); a per-alternative-backend order-audit obligation (D4); and the
  cross-process atomicity clause (D3) as a real obligation every production backend must meet.

### Rejected

- **A typed trait surface** (`Journal::append(event)` / `ContentStore::put(artifact)`) — tempts a
  signature/content-hash-perturbing re-serialization on read; the whole point is a dumb byte sink (D2).
- **Static generics for dispatch** — cannot do runtime selection alone and would virally parameterize
  ~180 sites (D6).
- **A binary truth store** (sqlite/redb/sled, or packed binary segments — git packfiles, EventStoreDB
  chunks) — forfeits `git`-diffability; binary belongs only in the deferred projection layer (D11). Packed
  segments survive only as prior art proving NDJSON is the *diffable* way to pack.
- **FastCDC content-defined chunking now** — byte-stable only with deterministic reassembly+rehash (high
  blast radius vs O1–O3) and it destroys per-object diffability; no fixture shows the multi-MB pain it
  solves. Prefer `contentEncoding` compression if object size ever bites (D7).
- **The `.shore/store.json` config-file selector now** — a backend is an experiment toggle, not yet a
  durable per-store property; held in reserve (D8).
- **`SHORE_BACKEND=memory` as an env-reachable value** — it would let a spawned child inherit an empty,
  lost-on-exit store; in-memory is in-process injection only (D8).
- **Weakening cross-process create-if-absent to single-process** (D3) — the in-memory single-process
  weakening is a scoped non-production exception, not a relaxation of the production contract.
- **`remove` on the `Journal`** — the journal is append-only; ADR-0016 removal is content-targeted on the
  `ContentStore` only (D1).
- **Routing `state.json` through the `Journal`** — collapses the truth and projection layers (D5).

## Open Questions

- **NDJSON locking mechanism** — `flock` advisory lock vs an in-process single-writer broker vs a sidecar
  lockfile, against Shoreline's per-operation, no-long-lived-handle construction pattern. Any such lock
  **must be store-directory scoped, never one-clone-one-writer** (`resolution.rs:98-101` states this as a
  forward constraint). (Hand-off to the NDJSON backend plan.)
- **`ContentStore` path-shaped addressing** — `content_ref` and `list(prefix)` are store-relative
  *path-shaped* strings (`artifacts/objects`, `artifacts/notes`), a mild file-layout leak (inherited from
  today's layout) into the otherwise byte-pure trait. A non-file backend can emulate it (a `HashMap`
  prefix-filter, SQL `LIKE`), but a cleaner shape would have the **wrapper** own object-vs-note namespacing
  — an opaque content address + a `kind` discriminator — so the backend never models file prefixes. Decide
  at the first non-file `ContentStore` backend; the file impl is fine as-is.
- **The put/get decoded-vs-encoded boundary** — does `ContentStore::put`/`get`
  operate over decoded bytes (wrapper owns hashing/address; encoding is an internal backend concern) or
  already-encoded bytes? Likely "wrapper computes the decoded-content hash + address; backend stores opaque
  (possibly encoded) bytes under that address and returns decoded bytes." Confirm when `contentEncoding`
  lands (O3).
- **Order-audit vs wrapper-canonicalization (D4)** — for the first non-file backend, is the projection's
  order-sensitivity handled by auditing the few `or_insert`/first-seen reducers, or by sorting decoded
  events to a canonical order in the wrapper before reducing? Decide when an out-of-hash-order backend is
  actually built; the file impl needs neither.
- **Tamper-hook trait shape** — a `#[cfg(test)]`-gated trait method vs a separate `TamperJournal` test
  trait vs a free function. Decide in the implementation work (D10).
- **`store status` reporting the effective backend** — `SHORE_BACKEND` is env-only (no committed source to
  report), but surfacing the *effective* backend aids experiment hygiene. Cheap; defer to the implementation
  work.
- **Cross-store format mismatch under env selection** — a store written by `local` and re-read under
  `ndjson` mismatches on-disk format; the experiment surface needs a documented rule (a distinct on-disk
  subtree per backend, or developer-owned consistency).
- **Append-latency on slow/networked storage** — all append measurements are APFS/SSD, where the two layouts
  are within fsync noise; the file-per-event double fsync could widen on a slow/networked FS. Direction
  unchanged; magnitude unmeasured.

## Revisit Triggers

- A measured read-all or write-amplification pain on a real store crosses a threshold worth NDJSON's
  concurrency cost → build the NDJSON backend as its own implementation effort.
- `contentEncoding` lands and makes *stored* bytes differ per store → confirm the dedup key
  has moved to the decoded-content `sha256` (O3) and the put/get encoded boundary is settled.
- A backend stops being an experiment and becomes a durable per-store fact → promote the selector from the
  `SHORE_BACKEND` env var to the `.shore/store.json` config-file mechanism (D8).
- A concrete slow read path, or an analytical query the on-the-fly reducer serves poorly, emerges → open
  the deferred `ProjectionStore` seam as its own ADR and implementation effort (D11), DuckDB the lead candidate.
- A non-file backend with a different listing order is proposed → resolve the D4 order-audit-vs-canonicalize
  question before it lands.
- Any proposal to add a typed method, a `remove` on the `Journal`, or a binary truth store → reopen here;
  these are the load-bearing rejections (D1, D2, Rejected).

## Related Docs

- The durable-storage backend analysis that produced this decision.
- In-repo `docs/`: `storage-model.md` (the consumer contract this seam preserves), `store-migration.md`
  (the §8 convergence rule any real backend format is gated by), `substrate-language.md`
  (store = log/journal; the three-layer framing).
- In-repo `docs/adr/`: ADR-0004, ADR-0015, ADR-0016, ADR-0017/ADR-0018 (see header).
- Composes with the `contentEncoding` direction below the identity layer, with no re-keying.
