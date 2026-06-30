# ADR-0025: Read-Time Diff Syntax Highlighting — View-Only Projection, `syntect` Tokenizer, Cross-Surface Emit

**Status:** Accepted (owner-approved 2026-06-30); implemented in-repo 2026-06-30.
**Date:** 2026-06-30
**See also:** **ADR-0021** (server-side projection direction — highlighting is one such *computed* view the
server owns), **ADR-0020 D11** (the projection/secondary seam "whose substrate is its own future ADR"),
**ADR-0022** (inspector TS build — the web emit ships through the committed `assets/app.js` esbuild
pipeline, and the new CSS rides `classNames.ts` + the `css-coverage` drift test), **ADR-0024/0023** (the
secondary read-index — the content-hash highlight cache here is a *separate, index-independent* sibling on
the same shared-state upgrade). Grounding issues: **#303** (this feature), **#255** (per-request recompute +
the `Arc<RwLock>` shared-state upgrade this cache rides), **#254** (windowing — the prerequisite for the
deferred per-window enrichment), **#299/#305** (the route reshape that renamed `snapshot_json` /
`/api/snapshots/{id}`). Reference-only prior art: `@pierre/diffs` (Apache-2.0; evaluated and rejected as a
dependency — see Rejected).

## Context

Diff rows render with **no syntax highlighting** today: the web inspector emits `escapeHtml(r.text)`
(`src/cli/inspect/web/src/diff/render.ts`), and the TUI paints each row with a single per-kind ratatui
`Style` (`src/tui/render.rs`). The backend already computes structured hunks and stores them as a
**content-addressed** `ObjectArtifact` whose hash covers `{schema, version, snapshot}` — i.e. every
`DiffFile`/`ReviewHunk`/`DiffRow` (`object_artifact_content_hash`, `src/session/store/object_artifact.rs`).
The inspector serves a snapshot at the read seam `snapshot_json` → `/api/snapshots/{id}`
(`src/cli/inspect/api.rs`, post-#299 route reshape): it reads + hash-validates the object-scoped **v2**
artifact and re-serializes a *derived* `serde_json::Value` view through a small shaping hook (a defensive
`remove(["revisionId","source","base","target"])` that is a **no-op on v2 bodies** — the v2
`ObjectArtifact` carries only `{schema, version, snapshot, contentHash}`; identity/endpoints live on
`/api/revisions/{id}` from the projection, never on the shared object artifact). So the seam already emits a
re-serialized view of the decoded artifact rather than the stored bytes verbatim; **D1 is what extends that
derived view with tokens** — via a dedicated DTO, so the stored bytes (and their hash) are never touched.

The owner has fixed the framing: highlighting is **best-effort and non-mission-critical** (any failure must
fall back to today's plain rendering), it is a **view concern** (it must not change what is stored or its
identity), and it must serve the two surfaces that exist today — the web inspector and the TUI (terminal/ANSI
stdout is out of scope). `@pierre/diffs` (the diffs.com engine) was evaluated as a candidate and rejected as
a dependency; its ideas (server-emitted tokens, intraline-as-decoration) are borrowed, its code is not.

This ADR records the decisions that are expensive to reverse. Per-surface design and tuning (TypeScript
coverage, size caps, terminal color-depth, changed-row color treatment) were resolved during implementation
and are recorded under "Resolved during implementation" below.

## Decision

### D1. Highlighting is a read-time, view-only projection — tokens never enter the artifact

Syntax tokens are computed at **read time** and attached only to **inspect-only wire/view types**, never to
the stored, content-addressed structs. For the web, build a typed enriched DTO — `WireObjectArtifact` /
`WireDiffSnapshot` / `WireDiffFile` / `WireReviewHunk` / `WireDiffRow` — at the `snapshot_json` read seam,
**after** decode + hash-validate and **before** serialization, by mapping the decoded `ObjectArtifact` into
the DTO and adding tokens per row. The stored `ObjectArtifact`/`DiffSnapshot`/`DiffFile`/`DiffRow` keep their
exact shape, so the artifact content hash stays **byte-stable by construction**. The wire gains one additive,
optional per-row field (`tokens?`); its absence is byte-identical to today's output. This is
**identity-by-construction**, not identity-by-vigilance. A guard test asserts the stored artifact never
serializes a `tokens` field and that its hash is self-consistent.

### D2. `syntect` is the highlighter; pure-Rust `regex-fancy`, never `onig`

Depend on `syntect` with `default-features = false, features = ["parsing", "regex-fancy"]` (the syntax set
comes from `two-face`, not syntect's stock dump — see the resolved note below). The
pure-Rust `fancy-regex` engine avoids the `onig` C/Oniguruma dependency that the syntect maintainers
themselves flag as a frequent Windows/WASM build failure — directly hostile to Shoreline's Windows-CI long
pole and Node-free/minimal-toolchain posture. Drive the parser directly (`ParseState::parse_line` →
`ScopeStack::apply` → classify scopes) to produce token **kinds**, not `Theme`/`Highlighter` RGB. Highlighting
is **best-effort**: unknown language, parse error, missing input, or any failure yields no tokens and the
surface renders plain — no panics, no user-facing errors. `tree-sitter` is the known-good future alternative
and is explicitly **deferred**.

### D3. One Rust tokenizer core → surface-neutral spans → surface-specific emit

A single in-process tokenizer produces a surface-neutral `TokenSpan { start, end, kind }` where `start`/`end`
are **byte offsets into the raw `DiffRow.text`** and `kind` is a small fixed enum (11:
`keyword, string, comment, number, type, function, constant, operator, punctuation, variable, plain`),
emitting **non-plain spans only**. Each surface has its own emit step over the *same* spans: the **web** wire
translates byte offsets to **UTF-16 code units at the emit seam** (so the JS `String.slice` is exact) and
escapes each sliced segment; the **TUI** consumes byte offsets directly in-process (no JSON round-trip) and
maps kinds to ratatui `Style`. Both emit steps are built as a **generic attributed-segment sweep** (split at
the union of span boundaries, tag each minimal segment) rather than a naive per-token loop, so the deferred
intraline channel (D6) drops in without an emit rewrite. The kind enum is the cross-surface contract; CSS
token classes route through `classNames.ts` + `app.css` + the `css-coverage` drift test, colored by a
`--tok-*` family in `tokens.css` (never raw syntect RGB).

### D4. A content-hash-keyed in-memory projection cache

Cache the highlighted read result in memory, **keyed by the artifact `content_hash`** (not object id — a
rebase changes bytes while keeping the object id; content-hash also dedups byte-identical artifacts across
worktrees). The cache needs **no invalidation**: a content-addressed artifact is immutable, so its highlighted
view is immutable; eviction (bounded) is always safe. It lives on the inspect server's shared state — the
**same `Arc<RwLock<…>>` upgrade #255 proposes**, as a *sibling* cache with a different key (content hash, not
`eventSetHash`). The two are index-independent and compose; whichever lands first justifies the upgrade.

### D5. Correctness default: per-hunk, side-separated streams

Highlight each hunk as **two ordered side-streams** — old = context+removed, new = context+added — each in one
stateful pass, mapping tokens back to rows positionally (**option b**). This needs no git read, is byte-offset
safe by construction, behaves identically across diff sources, and is wrong only when a multi-line construct
opens before a hunk's first row (bounded, non-misleading). Per-line restart (**option c**) is the per-row
fallback. Highlighting a hunk as one mixed text (**option d**) is structurally incoherent and disqualified.
**Blob-backed whole-file highlighting (option a)** — fetch full blobs via `old_oid`/`new_oid` for correct
cross-line state — is a **deferred upgrade**, not in the initial scope. Both oids are `Option` and are `None`
for an all-zero raw sha (`src/model/file.rs`, `src/git/raw.rs`): the default worktree review
(`git diff HEAD`) carries `new_oid = None`, **added** files carry `old_oid = None`, and **synthetic**
untracked files carry both `None` (`src/git/ingest.rs`). So (a) applies **per side, only where that
side's oid is present** — both sides for commit-range modified/renamed files, the **old side for
modified/deleted files** (its immutable blob, the high-value low-risk slice), and **not at all** for added or
synthetic files — every cell without a blob falls back to the option-(b) side-stream.

### D6. Intraline (word/char) diff is a cost-value-gated follow-on

Intraline emphasis is **not** co-shipped with highlighting. It is a separate, lower-risk read-time projection
(pure per-pair string diff via the `similar` crate, `word-alt` heuristic + length guard) on a different visual
channel (background/underline vs syntax foreground). It rides the **same** inspect-only DTO, the same byte→
UTF-16 wire treatment, the same content-hash cache, and the same generic attributed-segment emit (D3) — which
is the one design hook the initial implementation must include so intraline lands as a small increment rather
than an emit rewrite.

## Consequences

### Accepted

- **Object identity is preserved with zero ongoing test budget.** Because tokens live on a distinct wire type
  (D1), no stored write path can ever change the artifact hash; the existing hash-integrity tests plus a
  single guard test remain the guarantee.
- **The build stays C-toolchain-free** (D2) — no `onig`, so the Windows CI long pole is not aggravated.
- **A measured, moderate dependency cost.** The initial implementation adds **+14 marginal runtime crates**
  (~+12%, 120→134); the intraline follow-on (`similar`) adds +1. No `onig`; `regex-automata`/`regex-syntax`/
  `memchr` are shared (already pulled by `tracing-subscriber`); **no new duplicate versions** (the `thiserror`
  v1/v2 split is pre-existing).
- **Binary size grows ~2.5 MiB.** A `cargo build --release` A/B measured the `shore` binary at **8,856,432
  bytes without highlighting vs 11,544,112 bytes with it — +2,687,680 bytes (+2.56 MiB, +30.3%)**, dominated by
  the `two-face` bundled syntax dump and `fancy-regex`. (Using syntect's stock dump alone — fewer languages,
  no TypeScript — was +1.99 MiB; `two-face`'s far larger coverage costs only ~+0.57 MiB more, because dropping
  the stock dump offsets most of its size.) This is the single largest cost of the feature; it is accepted on
  the strength of the strictly-optional/best-effort nature and the C-toolchain-free build, and is noted as a
  revisit trigger if the bundled coverage proves more than warranted.
- **Perf risk is low.** `/api/snapshots/{id}` is on-demand and once-per-open — not on page load and not in the
  3s freshness poll — so highlighting does not touch #255's hot path, and the content-hash cache amortizes its
  cost to ~zero (D4).
- **The two surfaces share one tokenizer** (D3); the TUI pays no JSON round-trip; CSS governance is honored.
- **Fail-safe by design:** absent tokens ⇒ today's exact rendering on both surfaces (D2/D3).

### Rejected

- **Tokens as an optional field on the stored `DiffRow` (gated by invariant tests).** `skip_serializing_if`
  protects the hash only while the field is `None`; the first code path that writes `Some(tokens)` silently
  changes the artifact hash, breaking dedup and cross-worktree convergence. Identity-by-vigilance with a
  growing test surface, for no benefit (the owner has fixed highlighting at read time). **Rejected** in favor
  of D1.
- **Baking tokens into the artifact at capture time.** Would change content identity and bloat the immutable
  artifact; the capture-vs-read fork is decided read-time. **Rejected.**
- **`onig` regex engine (syntect default).** Faster, but reintroduces a C build dependency on the platform
  that is already Shoreline's CI bottleneck. **Rejected** for `regex-fancy` (best-effort highlighting does not
  need the speed).
- **Depending on `@pierre/diffs`.** A browser shadow-DOM Web Component bundling Shiki — fights the
  non-minified committed `app.js` + Node-free cargo invariants (ADR-0022) and our `(side, line)` annotation
  interleaving; its diff-compute half is redundant (the backend already ships hunks); its comment layer is less
  capable than our review-stream. **Reference-only.**
- **Client-side (JS) highlighting.** Violates the one-Rust-tokenizer-core constraint and would diverge from the
  TUI. **Rejected.**
- **Blob-backed whole-file highlighting as the initial default (option a).** Patchy blob availability
  (`new_oid = None` on worktree reviews), a new git-subprocess on the read path, and a freshness/TOCTOU +
  lossy-UTF-8 reconciliation hazard. **Deferred** to a later upgrade behind the cache (D5).
- **A range/window-aware highlight endpoint at the outset (option b for perf).** Requires a client rewrite away
  from render-from-memory lazy bodies and depends on #254/#255 windowing. **Deferred**; the initial
  implementation ships all-row enrichment + opt-outs + cache.

## Resolved during implementation

The per-surface tuning the ADR deliberately left open was resolved as follows:

- **TypeScript coverage: via `two-face`.** syntect's stock bundle has no TypeScript, and the current
  `version: 2` Sublime `.sublime-syntax` files do not load in syntect's YAML loader. Rather than vendor and
  maintain individual grammars, the syntax set is sourced from the **`two-face`** crate
  (`two_face::syntax::extra_newlines()`, `features = ["syntect-fancy"]` — same syntect, no `onig`), which
  bundles the `bat` syntaxes: TypeScript and TSX tokenize, along with the long tail of other languages.
  syntect's own stock dump is dropped (`default-features = false`, no `default-syntaxes`) since `two-face`
  supplies the set, so the two dumps nearly offset.
- **Size cap: reuse `LARGE_FILE_ROWS` (500).** A file whose total diff rows exceed the cap renders plain; the
  content-hash cache makes the cost a one-time pay.
- **TUI color-depth: a `COLORTERM` env-var helper** (no new dependency) — truecolor when
  `COLORTERM ∈ {truecolor, 24bit}`, else a per-kind named-ANSI palette that also renders on 16-color
  terminals.
- **Changed-row color:** the diff add/remove signal becomes a background tint (truecolor) and the code takes
  full syntax foreground; the `+`/`-` sign and gutter retain the add/remove color.

## Revisit Triggers

- **Binary size is judged unacceptable.** The measured +1.99 MiB / +23.6% from D2 is recorded above; if it
  becomes a concern, revisit the embedded-syntax-dump scope (curated subset) or the `syntect`-vs-`tree-sitter`
  choice.
- **Highlighting stops being best-effort.** If any consumer comes to depend on tokens being present/correct
  (e.g. a test asserts specific tokenization), the fail-safe contract (D2) and the no-invariant-tests posture
  (D1) must be reopened.
- **A third highlighting surface appears** (e.g. terminal/ANSI stdout) — confirm it reuses the D3 core rather
  than growing a parallel highlighter.
- **#254/#255 land windowing.** Re-evaluate moving web enrichment from all-row (D5) to per-window
  (the deferred option b), at which point highlighting becomes windowed for free.
- **`two-face` coverage gaps bite** (a language neither syntect's stock bundle nor the `bat` set covers):
  revisit vendoring a specific grammar vs accepting plain fallback. TypeScript/TSX, the original gap, are now
  covered by `two-face`.
- **Grammar accuracy / `fancy-regex` slowness** ever blocks a request: reconsider the engine (the `onig`
  rejection is conditional on best-effort, read-time, cached use).
