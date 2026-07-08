# ADR-0032: Machine-Document Namespace — Emitted Documents Re-Mint Under `pointbreak.*`; At-Rest Substrate Schemas Stay `shore.*`

**Status:** Accepted; landed in-repo via plan 0123.
**Public-safe:** yes, once the name split is public — engineering rationale only.
**Date:** 2026-07-05
**See also:** **ADR-0029** (output-mode convention — Decision 7's tiered posture stands; this ADR
is the authorized coordinated break for the `schema` discriminator *values*, leaving the hard-core
field paths, wire vocabularies, and the per-document `version` lever untouched), **ADR-0031**
(draft — the ratified surface grammar the new names align to), **ADR-0030** (+ Amendment — the
named command surface and the hidden legacy surfaces this ADR re-mints uniformly), **ADR-0028**
(id prefixes — ids and their prefixes are untouched here), **ADR-0004** (event signatures — why
at-rest schemas are frozen), `docs/substrate-language.md` (substrate vocabulary is internal;
product surfaces are domain-named).

## Context

One `shore.*` namespace currently covers two schema populations with opposite physics.

**Schemas at rest, signed, or fed to digests** — the substrate layer. The event envelope schema
string is inside the to-be-signed bytes (`src/session/event/tbs.rs`; ADR-0004), so it is
signature-load-bearing for every stored event (2,680 in this repo's own store). Identity
derivation consumes schema strings as digest inputs (`shore.worktree-fingerprint`,
`shore.revision-identity`, `shore.object-identity` in `src/session/store/fingerprint.rs`;
`shore.event-set.v1` in `src/session/projection/freshness.rs:7`) — changing any of them re-derives
opaque ids, a convergence-gated store break (`docs/store-migration.md`). The rest of the at-rest
set: `shore.event`, `shore.object`, `shore.state`, `shore.note-body` (store files),
`shore.actor-attributes.v1` (`src/session/identity/actor_attributes.rs`), `shore.store-config`,
`shore.sensitivity-config`, and `shore.store-export-manifest` (`src/session/store/bundle.rs`, an
export artifact read back by import). These sit behind the 1.0 store-format floor.

**Schemas on emitted documents** — the product's machine lane. Every `shore review-*`, `keys-*`,
`identity-*`, `store-*`, `notes-apply`, and `dump` invocation wraps its body in the
`{ schema, version: 1, … }` envelope (`src/documents/`, `DiagnosticDocument`) and writes it to
stdout; the inspect HTTP API serves four more (`shore.inspect-threads` / `-revisions` /
`-history` / `-freshness` — the `shore inspect` command itself prints a text banner, no
document). None of these documents is ever persisted: the live store contains
only the four at-rest schemas above. Verified consumer coupling is by ADR-0029's hard-core field
paths (`.revision.id`, `inputRequests[].{…}`), not by the `schema` string — the first-party skills
and the inspector web app match no schema literals.

Two forces make the current naming wrong for the emitted population:

1. **The namespace misstates contract ownership.** `shore` is the internal substrate codename
   (`docs/substrate-language.md`; the `.shore` dotdir and `SHORE_*` env anchor that layer). The
   emitted documents are the product's primary integration contract, and the product's published
   name — crate, repository, brand — is `pointbreak`. A consumer reading `shore.review-capture`
   is told the wrong owner.
2. **ADR-0031 creates permanent name drift.** The flatten renames commands with a zero-wire-drift
   promise, so after it lands, `input-request show` would emit `shore.review-input-request-fetch`,
   the `key` family would emit `shore.keys-*`, and `identity delegate` would emit
   `shore.identity-enroll` — command archaeology frozen into the contract unless a deliberate
   re-mint happens exactly once.

A future second domain family (`Task` is already modeled internally as an engagement axis but has
no product surface) would mint its documents under whichever rule exists when it arrives.

## Decision

### 1. The namespace rule: at rest = `shore.*`, emitted = `pointbreak.*`

Schema strings that rest on disk, ride signed bytes, or feed identity digests are **substrate
schemas** and keep `shore.*` permanently. This is not a preference: renaming them invalidates
signatures and re-derives opaque ids behind the 1.0 format floor. The frozen set is enumerated in
Context; the `eventType` wire vocabulary (including `work_object_proposed`) stays frozen per
ADR-0029.

Schema strings on documents the product **emits** — CLI stdout machine lane and the inspect HTTP
API — are **product contract** and carry the `pointbreak.*` namespace. The namespace answers
"whose contract is this," uniformly; the remainder of the name carries the subject. This applies
to every emitted document regardless of subject family — `pointbreak.store-status` and
`pointbreak.key-list` alongside `pointbreak.review-capture` — so consumers never need a family map
to predict a document's namespace.

### 2. One coordinated re-mint, shape-identical, no dual-emit

Every emitted document schema re-mints once: the `schema` value changes, the body shape is
byte-identical otherwise, and `version` stays `1` under the new name. Old names cease in the same
release — no alias window, no dual-emit — consistent with the hints-only migration posture
(ADR-0031) and the fact that every stdout consumer is first-party (skills, loop drivers, byte
snapshots, relay `cli-fallback` if retained per shoreline-relay#11). ADR-0029 Decision 7 otherwise
stands unamended: this ADR *is* the coordinated break its hard-core rule requires for `schema`
changes, and the version-bump lever remains the mechanism for all future shape changes.

### 3. New names align to the ADR-0031 grammar; every re-mint is one-to-one

The re-mint adopts the ratified surface grammar so document names and command names converge.
Every row is a one-to-one schema-string rename — no document merges, splits, or body changes
ride this ADR (the association family keeps its five distinct schemas for exactly this reason):

| Today | Re-minted |
|---|---|
| `shore.review-capture` / `-history` / `-endorse` | `pointbreak.review-capture` / `-history` / `-endorse` |
| `shore.review-revision`, `shore.review-revision-list` | `pointbreak.review-revision`, `pointbreak.review-revision-list` |
| `shore.review-observation-add` / `-list` | `pointbreak.review-observation-add` / `-list` |
| `shore.review-assessment-add` / `-show` | `pointbreak.review-assessment-add` / `-show` |
| `shore.review-validation-add` / `-list` | `pointbreak.review-validation-add` / `-list` |
| `shore.review-input-request-open` / `-list` / `-respond` | `pointbreak.review-input-request-open` / `-list` / `-respond` |
| `shore.review-input-request-fetch` | `pointbreak.review-input-request-show` |
| `shore.review-association-commit` / `-ref` / `-commit-withdrawn` / `-ref-withdrawn` / `-list` | `pointbreak.review-association-commit` / `-ref` / `-commit-withdrawn` / `-ref-withdrawn` / `-list` — namespace-only; these five bodies differ (`src/documents/association.rs`), so the ADR-0031 implementation's command collapse, if it consolidates them, is a separate **shape** change under ADR-0029 D7 that mints its new names under `pointbreak.*` per Decisions 1/5 |
| `shore.keys-init` / `-list` / `-show` / `-use-ssh` / `-enroll` | `pointbreak.key-init` / `-list` / `-show` / `-use-ssh` / `-enroll` |
| `shore.identity-enroll` | `pointbreak.identity-delegate` |
| `shore.identity-attest` | `pointbreak.identity-attest` |
| `shore.store-status` / `-mode` / `-migrate` / `-remove` / `-compact` | `pointbreak.store-…` (same verbs) |
| `shore.notes-apply`, `shore.dump` | `pointbreak.notes-apply`, `pointbreak.dump` |
| `shore.inspect-threads` / `-revisions` / `-history` / `-freshness` (inspect HTTP API documents) | `pointbreak.inspect-…` (same tails; the bare `shore.inspect` string is a tracing span, not a document — out of scope per Decision 6) |

Hidden legacy surfaces (`dump`, `show`, `notes` — ADR-0030 Amendment) re-mint uniformly: hidden is
not exempt, and their eventual retirement removes documents rather than renaming them. The
implementation plan enumerates the complete set from `src/documents/` and the `DiagnosticDocument`
call sites; any schema missed by the table above follows the rule, not the table.

### 4. The `shore.review-notes` sidecar input: accept both, forever

The notes sidecar is the one emitted-side schema whose instances exist **at rest outside the
store** and outside any single release window (it is an input file authored by external tools;
`src/sidecar/review_notes.rs`). The reader accepts `shore.review-notes` and
`pointbreak.review-notes` indefinitely; documentation names `pointbreak.review-notes` canonical;
writers migrate at leisure.

### 5. Future domains mint under `pointbreak.*` from day one

Any new domain surface (a task family, if/when it reaches the product) mints its documents as
`pointbreak.<domain>-…` from the first release. No new `shore.*` product surface is created after
this ADR.

### 6. Out of scope — unchanged

`SHORE_*` environment variables, the `.shore` dotdir, the `~/.shore` keystore, the `shore` binary
name, tracing span names (`shore.review.capture` etc. — never contract), and test-fixture schemas
(`shore.test*`). Additive `POINTBREAK_*` env aliases remain a later option per research 0027/0028,
not part of this decision.

## Consequences

### Accepted

- One mechanical, single-window consumer break: re-point the `src/documents/` schema constants,
  re-bless the byte-snapshot suite (the internal drift alarm re-pins per ADR-0029), sweep docs,
  and lockstep the relay `cli-fallback` parser if it still exists. Verified: no first-party
  consumer matches schema strings today, so the practical blast radius is the snapshot suite and
  documentation.
- The namespace becomes a contract-ownership statement: `pointbreak.*` = the product promises
  this; `shore.*` = substrate internals you should not be parsing.
- The ADR-0031 command/document name drift is resolved in the same break — one window instead of
  drift-forever or two breaks.
- The substrate keeps a stable, brand-independent name: any future product-brand evolution
  re-mints only emitted documents; stored bytes, signatures, and identities are insulated.

### Rejected

- **Uniform `pointbreak.*` including at-rest schemas** — a full store break: signature
  invalidation plus opaque-id re-derivation behind the 1.0 format floor, for zero benefit at rest
  (stored bytes are not a consumer surface).
- **Subject-scoped split (only `review-*` documents move)** — a mixed emission namespace where
  consumers memorize which families are "domain enough"; `store`/`key`/`identity` are product
  surface (research 0028 keeps them mounted under the product tree).
- **Dual-emit or alias window** — two names for every document with no retirement forcing
  function; contradicts the hints-only precedent, and every consumer is first-party and updated
  in lockstep anyway.
- **Defer until a second domain surfaces** — mints more `shore.*` product surface in the
  meantime, and misses the ADR-0031 alignment window: the fetch/keys/enroll archaeology would
  freeze into the contract, turning one break into two.
- **Keep `shore.*` and document it as a codename** — leaves the internal codename on the
  product's primary integration contract permanently, compounding with every future domain.

## Revisit Triggers

- A second product surface or brand emerges that emits documents (the namespace rule needs a
  per-product prefix decision).
- A store-format major break is undertaken for independent reasons — the only window in which
  re-namespacing at-rest schemas could ever be reconsidered.
- The machine-lane envelope itself is redesigned (ADR-0029 successor), which would subsume this
  naming rule.

## Landing Note

The accepted decision body above is preserved as written. Its Decision 4 sidecar compatibility rule
and the `shore.notes-apply` / `shore.dump` rows in Decision 3 are no-ops in the live code at landing:
the legacy terminal, dump, and review-notes sidecar surfaces were retired before this ADR landed.
No reader or alias is resurrected by this ADR.
