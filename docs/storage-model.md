# Storage Model

## Status

This is architecture guidance for Shoreline's durable review/session state. It describes constraints
the first filesystem persistence release should preserve, even when the implementation starts small.

## Goal

Shoreline should make durable state boring: write facts once, rebuild projections, and keep output,
storage, and notification side effects behind explicit seams. The storage model should avoid the
common failure modes of long-running coordination tools: hidden in-memory authority, direct delivery
before persistence, shared mutable JSON files, unbounded retries, and helper bypasses.

## Storage Authority

Shoreline V1 intentionally uses a filesystem-backed `events/` + `artifacts/` store as the
authoritative local store. The authoritative store is the one the worktree **resolves**: the shared
common-dir store at `.git/shore` by default (the same store for every worktree of a clone), or a
worktree-local `.shore/data` store when the worktree is ephemeral. The on-disk layout is identical
regardless of location. This is a deliberate split between canonical immutable facts and derived
projections, not a temporary gap waiting to be replaced by a database.

**Path convention.** Store paths below (`events/`, `artifacts/`, `state.json`, …) are shown relative
to the resolved store directory — `.git/shore` by default, or `.shore/data` when the worktree is
ephemeral. They are not absolute repo paths; only the ephemeral opt-out and the legacy-migration
notes name `.shore/data` literally.

**Authoritative facts.** Durable history lives in two places within the resolved store:

- `events/` — append-only, immutable per-fact event files. Events are independently written
  and never moved, retried in place, or rewritten on read.
- `artifacts/` — immutable or content-addressed support records, including captured
  revision snapshots and large note-shaped bodies for native observations, input requests, and
  assessments.

These are the only authoritative durable storage in V1. Everything else is a cache or projection.

**Rebuildable projections.** `state.json`, command-output views such as `shore.review-history` and
`shore.review-revision`, and any future read indexes are derived from durable events and artifacts.
They may be deleted and regenerated. Freshness against the current event set is verified through
`eventSetHash`, not through the projection's existence or `eventCount` alone.

**Consumer contract.** Stable automation should depend on Shoreline commands and named JSON documents,
not on raw storage paths. Commands and documents expose semantic IDs, content hashes, and freshness
metadata as the public surface. Event filenames, artifact paths, fan-out layout, the internal shape
of `state.json`, raw storage envelopes, and row or hunk identifier formatting are Shoreline-owned
storage details. They may change without a deprecation cycle unless a later design explicitly
promotes them to a stable contract.

**Deferred options.** SQLite-backed read indexes, content-address fan-out, snapshot compaction or
delta packs, store manifests, and retention policy are implementation choices Shoreline may add later
as derived layers. None of them are current authority, and none of them are part of the consumer
contract until a later design explicitly promotes them.

## Storage Layers

Use distinct storage concepts for distinct semantics. The authoritative store resolves to one of two
locations — the shared common-dir store by default, or a worktree-local ephemeral store — while the
committed config siblings always live under the worktree's `.shore/`:

```text
.git/shore/               shared common-dir store (default; one per clone, shared by every worktree)
  events/                 immutable event log
  state.json              rebuildable projection
  artifacts/              immutable or content-addressed support records
    notes/                optional content-addressed note-body records
    objects/              immutable captured revision object artifacts (content-only snapshots)

.shore/                   per-worktree config (and the ephemeral store, when opted in)
  data/                   ephemeral worktree-local store (git-excluded; same events/ + artifacts/ layout)
  store.json              committed store-mode config (shared | ephemeral)
  store.local.json        locally-excluded private store-mode override
  delegates.json          committed delegation map (shared default)
  delegates.local.json    locally-excluded private delegation override
  actor-attributes.json   committed actor-attributes map (shared default)
  actor-attributes.local.json  locally-excluded private attributes override
  allowed-signers.json    committed allowed-signers trust set
```

By default the store lives at `.git/shore` under the clone's Git common directory, so every worktree
of the clone resolves the same store; an ephemeral worktree instead keeps its store under its own
`.shore/data/`. Either way the store's on-disk layout (`events/`, `state.json`, `artifacts/…`) is the
same. The worktree's `.shore/` directory always holds the committed config siblings (`store.json`,
`delegates.json`, `actor-attributes.json`, `allowed-signers.json`). Only the ephemeral store subtree
and the private `.local.json` overrides are kept out of Git, via a committed
**`.shore/.gitignore`** carrying exactly two lines (`data/` and `*.local.json`) — never a wholesale
`.shore/` exclude, which would hide the committed config, and never the hidden, per-clone
`.git/info/exclude`. (`allowed-signers.json` is committed-only and has no `.local.json` override, by
deliberate trust-set-locality decision.) Shoreline generates the file when something first needs
covering — opting into ephemeral mode (`shore store mode ephemeral` or a write to an ephemeral
store) or staging a `--local` identity override — and skips generation entirely when the paths are
already ignored by any standard source, so user-managed ignore files are respected. A shared-store
write generates nothing and never touches the working tree (the shared store lives inside `.git/`,
which git already ignores).

Clones that predate the committed `.shore/.gitignore` may still carry the retired mechanism's
narrow entries (`.shore/data/`, `.shore/*.local.json`-style lines) in `.git/info/exclude`; those
are harmless — redundant with the committed file — and can be removed by hand. A legacy
**wholesale `.shore/`** line is different: it hides committed `.shore/` config and suppresses the
generated `.shore/.gitignore`, so a clone carrying one should delete that line by hand.

`events/` is the authoritative log. Events are immutable, independently written, and never moved to
`failed/`, retried in place, or rewritten on read.

`state.json` is a cache/projection. It must be rebuildable from durable records. If it is missing,
stale, or invalid, Shoreline should rebuild it rather than treating it as authority.

Revision capture follows the same authority split:

- `work_object_proposed` events in `events/` carry durable capture facts
- a revision carries the base endpoint, target endpoint, and captured diff snapshot, and references a
  content-only **object** identity; the Git endpoints are optional provenance, so a revision is
  git-optional
- V1 captures the local Git worktree from `HEAD` to the working tree
- full captured snapshots live as Shoreline-owned immutable **object artifacts** under
  `artifacts/objects/`
- `work_object_proposed` events bind to the internal object artifact's canonical `contentHash`;
  the event/projection — not the artifact body — is the source of `revisionId`, `objectId`, `source`,
  `base`, and `target` (the content-only object artifact carries none of the revision's identity or
  endpoints)
- the separation is deliberate: the **revision id** is the captured unit's identity and the **object
  id** is a hash of its captured content alone, so two clones capturing identical content converge on
  one object while keeping distinct revisions
- bounded `state.json` may summarize revision count and current unambiguous revision ID, but it
  is not the source of revision identity or snapshot content

`shore capture` returns `shore.review-capture` JSON as the command-output contract. The
command reports the revision and object IDs plus the object artifact content hash, without making
object artifact paths a user-facing API.

Revision succession follows the same event/projection split. A capture records `supersedes` forward
pointers naming the already-stored revisions it evolves past; it never edits captured revision payloads
or object artifacts. There is no separate lineage event family — succession rides the
`work_object_proposed` event's `supersedes` field. Derived read documents build the **supersession
DAG**: a thread is the connected component of the `supersedes` graph, and the projection surfaces every
live **head** rather than resolving to a single scalar head. When two captures supersede the same
predecessor, the resulting **competing heads** are surfaced as competing, never nulled or tie-broken by
timestamp.

Succession identity must not depend on worktree paths, raw `.git` layout, raw `.shore/data` paths, or
raw shared-store paths. The `supersedes` pointers carry opaque revision ids only. Succession facts
remain ordinary producer facts signable by the generic `EventToBeSigned` contract from
[ADR-0004](./adr/adr-0004-event-signatures.md), including its Dead Simple Signing Envelope (DSSE)
and pre-authentication encoding rules.

Succession has scoped current semantics with no global winner. A revision-scoped read seeds on
`--revision <id>` and resolves that revision's thread head; an intra-thread fork surfaces as competing
revisions. The content-only `--object` lens groups revisions by identical content — which may span
threads — and is a listing aid only, never a head selector. Routine list, history, and exact-revision
reads have no always-on ambiguous-current warning, but unscoped current selection still fails clearly
when the caller asks for one current revision in a store with multiple unrelated captures. The
`stale_by_superseding_revision` diagnostic is a thread-level freshness fact for a revision that a newer
revision supersedes, not an exact-revision read error.

This first release has no interdiff or stack DAG beyond the supersession graph itself. Public export,
relay/network forwarding, visual stack rendering, and stacked-work graph semantics remain out of scope.

When the inspector lists captured revisions, it shows a derived label for each working-tree
target — the worktree's name together with the short base commit — instead of a generic
"working tree". This label is computed at read time from the capture's existing endpoint data; the
captured record itself is unchanged, and the full worktree path is not shown. Opening the complete
detail for a revision captured in a different worktree is future work.

`ObjectArtifact.contentHash` is a canonical hash of the artifact body excluding the
self-referential `contentHash` field. The body is `{schema: "shore.object", version, snapshot,
contentHash}`, so the hash covers only the content-only `{schema, version, snapshot}` — the **full
captured row inventory** (every `DiffFile`, `FileMetadataRow`, `ReviewHunk`, and `DiffRow`) and nothing
worktree- or identity-specific. The `snapshot` body field is the captured diff itself (a
`DiffSnapshot`); it is kept. Two clones capturing the same content therefore produce **byte-identical
artifacts that dedup** into one object instead of colliding; revision identity and endpoints live in
the `work_object_proposed` event/projection, never in the artifact. The hash is not a raw JSON file
checksum.

An earlier artifact body embedded the captured unit's identity and endpoints
(`revisionId`/`source`/`base`/`target`) and folded them into the hash, which is what made two
worktrees' artifacts collide. The identity-reshape break removed that shape along with the transitional
dual-read that once accepted both bodies: the strict reader now rejects the old-schema artifact
outright, and a one-shot migrator re-hashed every artifact into the content-only object form (see
[store-migration.md](./store-migration.md)). New captures write only the object artifact, and no
in-store legacy body remains to read. Any future elision plan must again bump the object artifact
version; see [ADR-0002](./adr/adr-0002-large-snapshot-artifact-policy.md).

The retired imported-notes pipeline left one storage remnant: old stores may carry immutable
`review_note_imported` events in `events/` (and, for large imported bodies, note-body artifacts
under `artifacts/notes/`). The event kind keeps its reserved type code forever — the type-code
registry is append-only — so those stores still load, but no surface projects the kind anymore
(ADR-0030, second Amendment). `state.json` keeps its `noteCount` field for wire-shape stability;
it is structurally zero.

Native observations follow the revision ledger model:

- immutable `review_observation_recorded` events in `events/` carry durable observation facts
- each observation targets a revision plus an optional file or line range in that captured
  snapshot
- each observation belongs to a required track; tracks are review lanes, while actor/producer provenance
  remains in the event writer envelope
- bounded `state.json` may summarize observation state, such as `observationCount`, but it does not
  embed observation history or body content

Observations are append-only. Corrections are new `review_observation_recorded` events that name
older observations through `supersedesObservationIds`; standalone retraction is deferred.

Observation read projections use `observationId` as the logical identity. If multiple durable
events carry the same observation ID, Shoreline preserves those events but returns one observation row
and emits a duplicate semantic diagnostic.

Observation bodies use inline-or-artifact mechanics. Bodies under or
equal to `BODY_INLINE_LIMIT` (4096 bytes today) stay inline in the event payload; bodies above the
threshold are externalized to `artifacts/notes/<sha256(body)>.json` with the `shore.note-body`
envelope (schema `shore.note-body`, version `1`), keeping `state.json` bounded and avoiding
unbounded event payload growth.

The direct read surface is `shore observation list`, which replays events and can optionally
hydrate bodies. Body artifact paths, event filenames, and `state.json` paths are internal storage
details, not command-output API. Native observations also appear in the composite
`shore revision show` projection.

Native input requests follow the same revision ledger model:

- immutable `input_request_opened` events in `events/` carry durable request facts
- immutable `input_request_responded` events in `events/` carry durable response facts
- each request targets a revision, captured file or range, or native observation in that same
  revision
- each request belongs to a required track; actor/producer provenance remains in the event writer
  envelope
- bounded `state.json` summarizes input request state with `inputRequestCount`,
  `openInputRequestCount`, and `openOperativeInputRequestCount`, but it does not embed request
  history, response history, body content, or reason content

Request `reasonCode` and response `outcome` are intentionally separate classification axes.
Multiple different response events remain append-only facts; read surfaces report that
input request as ambiguous instead of choosing a timestamp winner.

Input request read projections use semantic IDs rather than event filenames as logical identity.
Multiple `input_request_opened` events with the same `inputRequestId` collapse to one request row
with a duplicate semantic diagnostic. Multiple `input_request_responded` events with the same
`inputRequestResponseId` collapse to one response row and do not make the input request
ambiguous. Distinct response IDs remain distinct facts and can still make the input request
ambiguous.

Input request bodies and response reasons use the shared inline-or-artifact mechanics. Text under
or equal to `BODY_INLINE_LIMIT` (4096 bytes today) stays inline in the event payload; text above
the threshold is externalized to `artifacts/notes/<sha256(body)>.json` with the `shore.note-body`
envelope (schema `shore.note-body`, version `1`), keeping `state.json` bounded and avoiding
unbounded event payload growth.

The direct read surfaces are `shore input-request list` and `shore input-request show`,
which replay events and can optionally hydrate bodies. Body artifact paths, reason artifact paths,
event filenames, and `state.json` paths are internal storage details, not command-output API. Native
input requests also appear in the composite `shore revision show` projection.

Native assessments follow the same revision ledger model:

- immutable `review_assessment_recorded` events in `events/` carry durable assessment facts
- each assessment targets a revision, captured file or range, native observation, native input
  request, or native assessment in that same revision
- each assessment belongs to a required track; actor/producer provenance remains in the event writer
  envelope
- bounded `state.json` summarizes assessment state with `assessmentCount`, but it does not embed
  assessment history, summaries, relationship graphs, or current-assessment candidates

Assessment values are closed in V1. Stored event JSON and command JSON use `snake_case`: `accepted`,
`accepted_with_follow_up`, `needs_changes`, and `needs_clarification`. CLI input and human-facing
display use the matching `kebab-case` spelling: `accepted`, `accepted-with-follow-up`,
`needs-changes`, and `needs-clarification`.

Assessment replacement is explicit. `replacesAssessmentIds` is the only V1 relationship that
removes an older assessment from the current set. Related observation and input-request references
are evidence links; they do not change current/replaced status.

Assessment read projections use semantic IDs rather than event filenames as logical identity.
Multiple `review_assessment_recorded` events with the same `assessmentId` collapse to one
assessment row with a duplicate semantic diagnostic. Multiple unreplaced assessment IDs remain
append-only facts; read surfaces report the current state as ambiguous instead of choosing a
timestamp winner.

Assessment summaries use the shared inline-or-artifact mechanics. Summaries under or equal to
`BODY_INLINE_LIMIT` (4096 bytes today) stay inline in the event payload; summaries above the
threshold are externalized to `artifacts/notes/<sha256(body)>.json` with the `shore.note-body`
envelope (schema `shore.note-body`, version `1`), keeping `state.json` bounded and avoiding
unbounded event payload growth.

The direct read surface is `shore assessment show`, which replays events and can optionally
hydrate summaries. Summary artifact paths, event filenames, and `state.json` paths are internal
storage details, not command-output API. Native assessments also appear in the composite
`shore revision show` projection.

State-change outcomes such as deferred, split-out, overridden, and superseded are represented as
native observations tagged with `state-change:*`, not as assessment values.

Validation evidence follows the same revision ledger model:

- immutable `validation_check_recorded` events in `events/` carry durable facts about completed
  checks
- each validation check targets one exact captured revision through opaque, content-addressed
  revision identity
- each validation check belongs to a required track; actor/producer provenance remains in the event
  writer envelope
- bounded `state.json` summarizes validation evidence with `validationCheckCount`, but it does not
  embed validation history, summary content, logs, or reports

Validation evidence is advisory. It may support review judgment in `shore revision show`,
`shore history`, and `shore validation list`, but it never grants review acceptance,
merge authority, or write authority. It never changes `currentAssessment`, assessment ambiguity,
operative input-request counts, or any other operative projection.

Validation identity is path-free. Event targets, validation targets, and stable identity fields carry
opaque IDs such as `revisionId`, `trackId`, and `validationCheckId`; they must not derive from
worktree paths, raw `.git` layout, raw `.shore/data` paths, raw shared-store paths, raw artifact paths,
or machine-local route names.

Validation summaries use the shared inline-or-artifact mechanics. Summaries under or equal to
`BODY_INLINE_LIMIT` (4096 bytes today) stay inline in the event payload; summaries above the
threshold are externalized to `artifacts/notes/<sha256(body)>.json` with the `shore.note-body`
envelope (schema `shore.note-body`, version `1`). Large logs and reports are referenced by
`sha256:<hex>` content hashes only; they are never inlined in validation events.

Validation events remain ordinary producer facts signable by the generic `EventToBeSigned` contract
from [ADR-0004](./adr/adr-0004-event-signatures.md). The validation family adds no signing payload
type, `sigVersion`, or family-specific signing path. See
[ADR-0006](./adr/adr-0006-validation-evidence.md) for the accepted validation evidence contract.

Review history is the chronological read surface over durable events:

- `shore history` returns `shore.review-history` JSON derived from a validated scan of the
  resolved store's `events/`
- `eventSetHash` and `eventCount` describe the full event set read for the command, not only the
  returned entries after filters
- `historyCount` describes the filtered entry count
- entries are sorted by `occurredAt`, then `eventId`, as display chronology only
- revision, track, and event-type filters narrow entries without changing freshness metadata or
  suppressing full-event-set diagnostics
- `--include-body` hydrates body-like text from inline payloads or `artifacts/notes/`, while the
  default output keeps large text omitted

History preserves raw append-only facts. It does not collapse duplicate semantic events, choose
current assessments, resolve input-request lifecycles, or build the full revision row projection.
Shared state diagnostics are still included so callers can see duplicate semantic facts while
inspecting the underlying events. Raw event files, artifact paths, event filenames, and `state.json`
are storage details, not history output API.

`shore revision show` is the composite read surface for one captured revision:

- `shore revision show` returns `shore.review-revision` JSON derived from a validated scan of the
  resolved store's `events/` plus the bound immutable object artifact for the selected revision
- `eventSetHash` and `eventCount` describe the full event set read for the command, not only the
  selected revision's returned narrative facts
- the output includes revision identity, filters, summary counts, current assessment, native
  observations, input requests, assessments, projection rows, and
  diagnostics
- rows are narrative-first plus snapshot-complete: reviewed ledger material appears first, and the
  snapshot remainder still includes every captured file, metadata row, hunk header, and diff row
- track filters narrow narrative facts only; they do not mutate revision selection, freshness
  metadata, or captured snapshot completeness
- `--include-body` hydrates body-like text from inline payloads or `artifacts/notes/`, while the
  default output keeps large text omitted

`shore.review-revision` is command-output API. Object artifacts, note body artifacts, event files,
event filenames, and `state.json` remain Shoreline-owned storage details and are not exposed as stable
paths.

The review stream also surfaces stale and orphan notes as dedicated rows so reviewers can park the
cursor on them; the stream emits an additional synthetic file header for orphan notes when at least
one is present.

## Shared Common-Dir Store Selection

The default durable store is the **shared common-dir store** at `.git/shore`, the path under the
clone's Git common directory. It is the default for **every** worktree of a clone — the main
worktree and every linked worktree alike — automatically, with no setup step. Because all linked
worktrees of a clone share one Git common directory, they all resolve the same `.git/shore` store,
so a capture in any worktree is immediately visible from its siblings. The store stays flat —
store-only, with no committed-config sibling to separate from. This default store is per-clone, not a
user-level multi-repository store or remote sync service; a clone may opt into a separate,
machine-wide **user-level family store tier** (see
[User-Level Family Store Tier](#user-level-family-store-tier) below), which resolves the
multi-repository case [issue #153](https://github.com/kevinswiber/shoreline/issues/153) named.

Public commands expose the resolved store as command JSON using opaque refs. Callers must not depend
on raw store paths, event filenames, artifact paths, `.git` paths, `.shore/data` paths, or
`state.json` layout — the JSON never prints them.

The writer contract is direct. Review capture and the native review write commands — recording an
observation, an input request open or response, an assessment, or validation evidence — write their
event, artifacts, and rebuilt `state.json` directly into the shared common-dir
store, the same store every read surface resolves. The fact is therefore visible to every worktree
of the clone in place, with no setup step, and a write can attach a fact to a revision (or relate
it to an observation, assessment, or request) captured in a sibling worktree. An **ephemeral**
worktree (`shore store mode ephemeral`) instead pins its writes to a discardable worktree-local
`.shore/data/` store — the privacy escape hatch for sensitive or throwaway work whose bytes should
disappear when the worktree is removed.

A pre-flip worktree-local `.shore/data/` store on a non-ephemeral worktree (data written before the
shared common-dir default) is detected on any read or write and errors with a hint to run
`shore store migrate`. `shore store migrate` folds that legacy store into the shared common-dir store
**non-destructively** by default: it copies events and artifacts forward with strict content-hash
validation and leaves `.shore/data/` in place, so the operator can verify the result and then remove
`.shore/data/` to finish the switch — or completes the switch in one command with
`--retire-source`, which independently re-verifies the fold from disk (a physical walk requiring
every durable source file present with identical content in the shared store — never a
manifest-driven check, which cannot see orphan/unreferenced files) and only then deletes
`.shore/data/`; on any divergence it errors and deletes nothing. It is idempotent — re-running
reports already-present facts as existing — and refuses an ephemeral or sensitivity-flagged worktree
unless `--include-ephemeral` is passed. It scans for sensitivity findings before moving data and
reports them in the command document. (`shore store migrate` folds the ephemeral `.shore/data/`
store into the shared common-dir store; it is unrelated to the legacy flat `.shore/` layout, a
retired pre-1.0 format that is detected and refused rather than migrated; see
[Migrations And Doctor](#migrations-and-doctor).)

The legacy-store guard deliberately keeps firing after a plain (no-retire) migrate until
`.shore/data/` is removed: the guard is not auto-suppressed once the fold looks like a subset,
because that check would run on **every** read and write (each opens the store), and a
suppressed-but-present `.shore/data/` would silently diverge the moment anything wrote to it.
`--retire-source` is the supported completion; the guard's hint names it.

`shore store status` is the public health and inventory surface for the resolved store. Its
`inventory` reports event and artifact byte counts, total bytes, optional Git untracked bytes,
largest artifact refs, and revision snapshot byte accounting; its redacted sensitivity scan
findings are hashed `file:sha256:*` values and do not disclose secret contents or source file paths.
Sensitivity findings are reported but do not currently abort a write; a hard-blocking policy and
explicit override controls are a forward-looking note for when movement can target a wider store.

Known-safe paths that trip the scanner (a repo's own test fixtures, for example) can be excluded
with a committed `.shore/sensitivity.json` (plus a git-excluded `.shore/sensitivity.local.json`
override, merged by union) — the targeted alternative to the blanket `--include-ephemeral`
override, which disables the migrate gate wholesale. An excluded path is not scanned; the scan
reports the excluded-path count and per-glob match counts so an over-broad exclude stays visible.
See [cli-reference.md](./cli-reference.md#sensitivity-exclude-globs) for the format and glob
semantics.

Reads resolve the shared common-dir store on every review read surface. `shore revision list` and
`show`, `shore history`, the observation, input-request, and validation lists,
`shore assessment show`, the association list, and the inspector API all
read it from any worktree of the clone, including object artifacts and large note-shaped bodies, so
their `eventCount` and `eventSetHash` reflect that one store.

Reload is a read-side projection refresh. The durable event log remains immutable; reload re-runs
the order-independent projection against the current worktree state and lowers anchor-stale
conditions into the read surface via `reload_diagnostics`. If reload encounters a parse or ingest
error partway through, the prior projection survives because the read-side primitive never mutates
the resolved store.

A future delivery queue is a separate subsystem. Queue concepts such as `pending/`, `failed/`,
retry counts, backoff, and circuit breakers do not belong in the store's `events/`.

## User-Level Family Store Tier

A clone may opt into a **user-level family store**: one store per repository family per machine, at
`<shore-home-root>/stores/<slug>/`. It exists so review facts survive removing any single clone
(`rm -rf` of a checkout) and are shared across independent clones of the same family — offline, with
no daemon and no sync service. The tier is opt-in per clone and additive: a clone that never links
keeps resolving its clone-local `.git/shore` store exactly as before.

**Placement and shared root.** The shore-home root is resolved by the same precedence the key home
already uses — `SHORE_HOME`, then `$XDG_DATA_HOME/shore`, then `$HOME/.shore` on Unix or
`%APPDATA%\shore` on Windows (see [Signature Allow-List and Key
Home](#signature-allow-list-and-key-home)); both the key home and the family stores now derive that
root from one shared resolver rather than two copies. The `stores/` path segment keeps
`<root>/{keys,stores}` disjoint from the key home by construction, so a family named `keys` still
lands at `<root>/stores/keys` and can never collide with the keystore. A family directory reuses the
existing store layout verbatim — `events/`, `artifacts/notes/`, `artifacts/objects/`, and the
regenerable `state.json` — plus two new files, `family.json` and a generated `.gitignore`.

**Resolution precedence.** `resolve_store` grows one branch, giving the order **ephemeral opt-in >
user-level opt-in > clone-local default** (the legacy flat-layout hard-cutover guard still fires
before any of these, unconditionally). The opt-in is a `familyRef`/`cloneRef` pair read from the git
common dir's `shore.link.json` (inside `.git/`, shared by every worktree of one physical clone), with
the legacy per-worktree `.shore/store.local.json` honored as a back-compat fallback — never from the
committed `.shore/store.json`, so a pulled commit can never activate the tier for anyone else. Because
the binding lives in the common dir, a single `shore store link` binds the main checkout and every
`git worktree` of the clone, and `cloneRef` keys on the common dir so one physical clone is one
registry member. A binding in the committed document is a hard error, and one of the pair without the
other is a hard error too — "opted in with no family" is unrepresentable. The ephemeral opt-out stays
per-worktree, so an ephemeral worktree still escapes to `.shore/data` even when the binding is
present. (This per-physical-clone relocation is the issue #402 amendment to ADR-0033.)

**The link/unlink/forget/list surface.** `shore store link <slug>` promotes the current clone: it
refuses an Ephemeral-mode worktree and a sensitivity-`block` worktree unless explicitly overridden,
refuses a slug already stamped for a different family, warns (without blocking) on a sync-managed
filesystem path, and warns — advisory only — when the clone shares no git history with the family it
is joining. It then folds the clone-local history forward by default, optionally retiring the source
after a verified fold, and flips the local binding last. When a worktree writes to its clone-local
store while a sibling worktree of the same clone is linked, `shore store status` and `shore capture`
surface a one-line advisory pointing at `shore store link <slug>` — the split is signalled, never
silent. `shore store unlink` detaches the clone back to clone-local, moving no data. `shore store forget <slug>` is the whole-store destructive verb,
dry-run by default and `--yes` to execute (refused while any clone is still live, unless `--force`);
it sits deliberately outside the content-targeted removal model in [Content Removal and
Compaction](#content-removal-and-compaction) — no store survives a forget to hold a removal event in.
`shore store list` is the first store surface with no `--repo` input: it walks every family on the
machine and reports each one's inventory, live-clone count, and orphan status.

**`family.json` / `registry.json` / `.gitignore`.** `family.json` is the schema-versioned manifest
(`"shore.family-manifest"`, version 1), written eagerly at link time; a bound family whose directory
lacks it is a *forgotten* family — a hard, actionable error, never a silent re-create and never a
silent clone-local fallback. `registry.json` (`"shore.family-registry"`, version 1) is machine-local
membership bookkeeping outside the event log: it records each member clone's path, re-validated
bidirectionally (the path is a git repo *and* that clone's local config still names the family back),
so `list`/`forget`/`status` derive liveness on demand rather than trusting stale entries. The family
directory's generated `.gitignore` covers exactly `state.json` and `registry.json` — both
machine-local, neither meant to be shared even if the family directory were ever placed under version
control.

**Non-guarantees.**

- The family store must live on a **local POSIX filesystem**. Network filesystems (NFS) and
  sync-managed directories (Dropbox, iCloud / Mobile Documents, OneDrive, Google Drive) are
  unsupported: `~/.shore` looks syncable, which is exactly the footgun a best-effort path warning
  calls out at link time.
- `compact` / `gc` should run against a **quiescent** family store. The compaction-versus-writer race
  is inherited unchanged from the existing multi-worktree case — corruption-free, but benign
  staleness is possible mid-race — and is not re-engineered for this tier.
- Linking **folds** a clone-local history forward through the same verified-import machinery `store
  migrate` uses. That fold stamps every folded event as bundle-applied, which strips the possession
  arm of content-targeted removal (see [Content Removal and
  Compaction](#content-removal-and-compaction)): prior **unsigned** removals in the folded source lose
  operative suppression in the family store. This is accepted and documented, not fixed — the
  recommended recovery is to re-issue `shore store remove` natively in the family store, surfaced as a
  diagnostic whenever a fold transports removal events. Steady-state direct writes to an
  already-linked family store are unaffected.
- `registry.json` writes are an atomic whole-set read-modify-write with **no lock**. Two clones
  linking the same family at the same instant can lose one entry to the race; that is accepted
  (liveness is always re-derived on demand from the bidirectional check, so a lost entry is a missed
  listing, never corruption) rather than solved with a new lockfile.

## Content Removal and Compaction

Because the shared common-dir store persists captured bytes that a removed worktree would once have
discarded, the store has an explicit, never-automatic content-removal path. It is content-targeted:
the durable fact is an `ArtifactRemoved { content_hash }` event keyed solely on the content hash, so
two peers removing the same content converge on one byte-identical fact and the same shared blob is
removed for every revision that references it (object artifacts dedup on content, so one blob is
shared by many revisions; targeting content rather than a revision is the only coherent granularity). The
event is journal-anchored — it carries no revision target — and the event log stays immutable: the
removal event never rewrites or tombstones the capture event. Read projections join the capture event
with any `ArtifactRemoved` over its content hash and render an explained **"content removed"** in place
of the missing bytes, distinguishing a removed artifact from one that is merely not-yet-synced or
corrupt. See [ADR-0016](./adr/adr-0016-content-targeted-artifact-removal-and-compaction.md) for the
decision.

The explained render covers snapshot content and note-shaped bodies alike (observation and
input-request bodies, response reasons, assessment and validation summaries):
an operative removal renders as `suppressed_present` (removal recorded, bytes still stored until a
compact) or `physically_removed` (bytes swept), never as an error — while bytes that are missing
*without* an operative removal keep the hard `import referenced artifacts` error, so a removed
artifact is never confused with a genuinely missing one. On the JSON read surfaces the body twin
appears as a `bodyContentState` / `summaryContentState` / `reasonContentState` field beside the
content hash (omitted entirely while present, so unaffected documents are byte-identical), plus
`body_content_suppressed_present` / `body_content_physically_removed` diagnostics. One
deliberate boundary: inline bodies (at or below the externalization threshold) live in the immutable
event log itself, and content-targeted removal does not cover event-payload bytes (the deferred
Tier-2 in ADR-0016) — an operative removal over a matching hash never suppresses an inline render.
This body-side render is read-surface consistency, not a privacy gate: the sensitive captured bytes
are the snapshot artifacts, and their removal path is the one described above.

Removal is two-phase. `shore store remove` appends the removal event (cheap, convergent, auditable);
the marked bytes survive on disk until `shore store gc` / `shore store compact` runs a local,
non-event maintenance sweep that physically deletes the removed and unreferenced blobs. Because a
removed content hash has no live referrer by construction, the sweep needs no reference-count wait and
is re-derivable from the event log. **Compaction — not removal — is the point of no return:** removal
is one-way (there is no append-only un-remove), and once bytes are compacted they cannot be recovered
by an event, only re-captured or re-imported. The operator rule for sensitive data is therefore
**remove, then compact**.

This is complete only **before** the artifact is pushed or mirrored. The removal event converges to
peers (they learn the content is removed and may collect their own copy), but **bytes already
mirrored to another store cannot be un-sent** — privacy is something you secure before push, and the
already-mirrored case is a documented limitation, not something removal can repair. GC is deliberately
not an event: "I deleted my local bytes" is a local maintenance fact, not a shared review fact, so it
is never converged to peers.

## Event Files

Every durable event must carry a non-null `idempotencyKey`. The key should be derived from canonical
event content, not generated randomly at the call site.

Use a hash of the idempotency key as the event filename:

```text
events/<sha256(idempotencyKey)>.json
```

Keep the readable idempotency key inside the event envelope. The filename is fixed-width and safe;
the event remains inspectable.

Event creation should be exclusive. If the file already exists for the same idempotency key, the
write is idempotent. If the filename exists with conflicting content, that is a corruption or
conflict error, not a merge.

Same-key retry checks should compare the canonical event payload hash, not the full event bytes.
Envelope fields such as `occurredAt` may differ across attempts while the durable fact is still the
same. A matching `payloadHash` is idempotent; a different `payloadHash` is a conflict.

Idempotency keys control write identity. Semantic IDs control logical projection identity. A caller
that repeats the same logical fact with different idempotency keys creates multiple durable events,
not a storage overwrite. Read projections collapse same-semantic-ID events to one logical row and
surface a duplicate semantic diagnostic so the raw append-only history remains inspectable.

Any hash that contributes to durable identity should use Shoreline's canonical JSON path, with object
keys sorted recursively before hashing. Do not rely on incidental serde_json map ordering or local
construction order for event payload hashes, revision fingerprints, snapshot fingerprints, or future
content-derived IDs.

Do not add a global sequence number until Shoreline has a concrete allocator that does not create a
shared mutable counter. Deterministic event ordering can start from event metadata and filenames.

## Ingest Provenance

Events that enter a store through a foreign-event seam carry an optional top-level envelope
sibling stamped by the local importer
([ADR-0009](adr/adr-0009-resumption-binding-trust-source.md)):

```json
"ingest": { "via": "ingest-events", "receivedAt": "unix-ms:1760000000000" }
```

`via` is a bounded vocabulary naming the seam: `ingest-events` (the `ingest_events` /
`import_event` workflow) or `bundle-apply` (store bundle import). `receivedAt` uses the store's
`unix-ms:` timestamp format. Consumers read presence; `via` and `receivedAt` are operator-facing
detail.

Both import seams stamp unconditionally and **overwrite** any inbound stamp. A stamp in arriving
bytes is some other store's bookkeeping — the same honesty rule that applies to `sourceRef`: hop
metadata from elsewhere is not a fact.

The stamp participates in nothing that identifies or authenticates the event. It is excluded from
the to-be-signed view, so stamping a signed event cannot invalidate its signature, and it
contributes to neither idempotency keys nor `eventId`. Exclusive event creation gives the stamp
first-stored-wins mechanically: a locally authored stored event can never acquire a stamp after
the fact, and an ingested event can never lose or swap its first stamp on re-ingest.

The marker is local bookkeeping written by the store owner's own importer. It is trustworthy to
this store under the single-writer contract; it is never a signed fact, and it is never
trustworthy to a third party reading a mirrored or copied store. Note the seam boundary: bundle
apply stamps, but a wholesale filesystem copy (`cp -r`) carries unstamped events into the new
store. Stores whose possession does not imply authorship should prefer the `verified-only`
binding posture described in
[ADR-0009](adr/adr-0009-resumption-binding-trust-source.md).

Events imported before the marker landed are unstamped and indistinguishable from local-authored
events — a store owner who imported events earlier possesses a store whose history they chose.
The marker discriminates from its landing forward.

## Artifact Files

Artifact filenames follow two deliberate rules, paired to what the file represents:

- **Object artifacts** use the artifact's canonical `contentHash` as the filename stem:
  `artifacts/objects/<sha256(object-artifact-body)>.json`. The readable `objectId` stays inside the
  artifact body, and each `work_object_proposed` event binds the two values together with
  `objectId` plus `objectArtifactContentHash`. This lets a rebased recapture keep the same stable
  content object while storing a different concrete artifact envelope when line geometry or blob OIDs
  change. The read path recomputes and compares `contentHash`, so tamper or transcription errors are
  caught at load time. The body inlines every captured row but no worktree identity, so the
  `contentHash` covers the full row inventory only. See [ADR-0002](./adr/adr-0002-large-snapshot-artifact-policy.md)
  for the content decoupling that separates the object's content hash from the revision's identity
  (#146).
- **Content-addressed artifacts** use a hash of the artifact body as the filename stem. Note-body
  artifacts live at `artifacts/notes/<sha256(body)>.json`. Hashing the body gives deterministic
  addressing and deduplication across observations, input requests, and assessments that share
  text. Native-recorded payloads may carry a payload-level body hash
  (`body_content_hash` / `reason_content_hash` / `summary_content_hash`) so future read paths or
  repair tools can verify the artifact against the event ledger; imported-note payloads do not
  carry such a hash and instead rely on the content-addressed filename plus the referring event's
  `body_artifact_path`. Identifier-hashed artifacts do not gain the same dedup benefit, because
  their underlying ID is already unique.

The asymmetry is intentional: identifier-hashed naming protects filenames from arbitrary ID
characters, while content-addressed naming earns its keep through deterministic dedup. Read paths
should not mix the two rules — locate object artifacts by their `objectId` and note-body
artifacts by the relative path recorded in the referencing event.

Artifact filenames remain Shoreline-owned storage details. The consumer contract is the command-output
JSON (`shore.review-capture`, `shore.review-revision`, and friends), which exposes semantic IDs and the
object artifact's canonical `contentHash`. Filename derivation rules may change without a
deprecation cycle, but artifacts are V1 authority alongside events — the event log alone cannot
reconstruct snapshot rows or large note bodies. A future rule change must therefore rename or
migrate the affected files in place, keep a compatibility read path during transition, or
regenerate the directory from the original source (worktree capture, sidecar import) where that is
possible. Shoreline does not promise dual-read of legacy filenames implicitly.

## Atomic Writes

All durable writes should go through one storage helper. The helper owns:

- temp file in the same directory as the target
- deterministic temp filename prefix
- file mode suitable for local review/session data
- temp file fsync for durable writes
- atomic rename into place
- parent directory fsync for durable writes
- stale temp file sweep

Any helper that can create temp files must also participate in sweeping them. Cleanup should not be
limited to queue code. On load, Shoreline should remove stale temp files matching its known prefixes and
older than the configured safety threshold.

Rebuildable projections may use a non-durable write mode that skips fsync, but they still should use
the same temp/rename path to avoid partial reads.

## Bounded Projections

`state.json` must stay bounded. It should summarize current state, cursors, and active projections;
it should not grow linearly with the event log.

If a projection needs unbounded history, split it into paged or content-addressed records under
`artifacts/` and keep `state.json` as an index or summary. A large `state.json` is a design smell
because it becomes a shared mutable file, a slow health check, and a crash-recovery hazard.

Imported review-note bodies follow this rule directly: bodies under or equal to `BODY_INLINE_LIMIT`
(4096 bytes today) stay inline in the event payload; bodies above the threshold are externalized to
`artifacts/notes/<sha256(body)>.json` with the `shore.note-body` envelope (schema `shore.note-body`,
version `1`), so the authoritative event and rebuildable projection remain bounded.

## Note Body Materialization

Shoreline stores note-shaped event bodies (observations, input request bodies, input request response
reasons, assessment summaries, imported review notes) using a threshold split, not as a uniform
artifact-per-body materialization.

- **Inline path.** Bodies whose byte length is at most `BODY_INLINE_LIMIT` (4096 bytes today, defined
  at `src/session/store/body_artifact.rs:8`) remain inline in the event payload. The on-disk event
  carries the body bytes verbatim under its `body` (or `summary` / `reason`) field.
  `body_artifact_path` stays `None`. The materialization discriminator is `body` vs
  `body_artifact_path`, not `body_byte_size`: native ledger payloads (observations, input requests,
  assessments) currently set `body_byte_size = Some(_)` on the inline arm via the shared
  `staged_body` helper, while imported-note payloads leave `body_byte_size = None` inline. Consumers
  that need an inline length should read it from the inline string directly.
- **Artifact path.** Bodies above the threshold are externalized to
  `artifacts/notes/<sha256(body)>.json` under the `shore.note-body` envelope
  (`{"schema":"shore.note-body","version":1,"body":"..."}`). The event payload's `body` field is
  `None`; its `body_artifact_path` carries the relative path and `body_byte_size` carries the body's
  length. Native-recorded payloads (observations, input requests, input request responses,
  assessments) additionally carry `body_content_hash` / `reason_content_hash` /
  `summary_content_hash`; imported-note payloads do not. `load_body_artifact` validates the path
  shape and the envelope's `schema` / `version` fields, not the body bytes themselves — hash-based
  cross-validation against the event payload, where available, is a caller's responsibility.

### What `artifacts/notes/` is — and isn't

- It is a content-addressed **overflow store**, not a complete inventory of note bodies. Small-body
  notes have no corresponding file in this directory. The directory may be empty for a repo that
  has only small notes.
- The authoritative durable record of every note is its event under the resolved store's `events/`.
  Replay (`EventStore::list_events()` followed by `load_body_artifact` for any `body_artifact_path`)
  is the only supported read primitive for note state.
- Tooling that wants a complete list of note bodies must replay events; walking `artifacts/notes/`
  alone is not sufficient and is not a supported authority.

### Why threshold-based

- Most observations and assessment summaries are short. Externalizing every body would emit one
  additional fsync per body-bearing event (both inline event and artifact use `Durability::Durable`
  writes), with proportional file-count growth.
- The body's identity does not depend on materialization: native-recorded payloads (observations,
  input requests, assessments) already carry a `*_content_hash`, and imported-note artifacts are
  content-addressed by `sha256(body)` in their filenames. Materializing every body would not
  strengthen those guarantees, only change where canonical bytes live.
- Artifact-only enumeration is not a supported read path. Even if all bodies were materialized, an
  artifact file under `artifacts/notes/<hash>.json` carries the body and the envelope schema /
  version but no referrer identity — it cannot answer "which note / observation / assessment does
  this body belong to?" without joining back to the event ledger.

### Threshold is tunable

The 4096-byte threshold is internal storage tuning and may change without a deprecation cycle. The
**inline-or-artifact bifurcation itself** is the stable contract: storage consumers must accept
that any given note-shaped body may be either inline or referenced by a `body_artifact_path`, and
resolve both arms.

See [ADR-0001](./adr/adr-0001-note-body-materialization.md) for the decision rationale.

## Large Object Artifact Policy

Shoreline stores captured revision diffs inline in content-hash-keyed object artifacts under
`artifacts/objects/<objectArtifactContentHash>.json`. The artifact body is one JSON object per snapshot
and carries every captured file, every metadata row, every hunk, and every diff row. There is no
elision threshold, no generated-file detection, and no metadata-only marker for "too-large" or
"elided" files.

- **Row inventory.** A captured snapshot for a newly added 10,000-line text file produces one
  artifact with roughly 10,000 inline `DiffRow` objects. V1 does not elide.
- **Metadata rows.** `FileMetadataKind` is `{ BinarySummary, ModeChange, RenameSummary,
  SubmoduleSummary }` today. `BinarySummary` is the V1 *content-omission* marker — binary
  patches set `is_binary = true`, get a `BinarySummary` row, and leave `hunks` empty. The other
  three variants carry file-level Git facts (rename, mode change, submodule pointer change) and
  also leave `hunks` empty, but they are not content-omission markers. There is no `ElidedFile`
  or `GeneratedFile` variant.
- **Read surface.** `shore revision show` is narrative-first plus snapshot-complete: reviewed
  ledger material appears first, and the snapshot remainder includes every captured file, metadata
  row, hunk header, and diff row. No flag omits row bodies.
- **Content-hash scope.** `ObjectArtifact.contentHash` covers the full row inventory. The object
  artifact body carries content only — the revision's identity and endpoint fields were dropped from
  the body and hash at the content-decoupling (#146) and identity-reshape breaks. Any future elision
  must again change the artifact version so a consumer can tell which scope produced a given hash on
  inspection.

The V1 policy is intentionally minimal: every question issue #64 asks ("elide?", "detect
generated?", "metadata-only rows?", "omit-on-show?", "hash scope?") receives an explicit answer in
[ADR-0002](./adr/adr-0002-large-snapshot-artifact-policy.md). Each answer's reversal — what would
have to change to flip it — is recorded in the ADR's "Future Reversal" section.

## Projection Freshness

`state.json` records `eventSetHash` as derived freshness metadata for the event set used to build
the projection. `eventCount` remains a cheap count, but it does not prove that a cached projection
matches the current `events/` set in the resolved store.

`eventSetHash` is computed from Shoreline's canonical JSON hash path over sorted `(eventId,
payloadHash)` pairs. It intentionally excludes the full event JSON, event filenames, sequence
numbers, writer metadata, storage paths, and `occurredAt`. The hash describes which durable facts
the projection saw; it is not a causal ordering primitive or a raw event-file checksum.

If a cached projection's `eventSetHash` does not match a fresh scan of the store's `events/`, the
projection is stale and should be rebuilt from the event files. The event files remain authoritative;
`state.json` is still safe to delete and regenerate. `shore history` and
`shore revision show` reuse this freshness primitive, and future derived-index projections should
do the same rather than inventing per-projection hashes.

## Shared Mutable Files

Authoritative facts should not live in read-modify-write shared JSON documents. Per-event files are
a deliberate defense against metadata clobbering:

- two writers can write different events without merging a shared object
- one failed event does not roll back unrelated events
- a projection can be rebuilt after partial failure
- stale projections are recoverable

Shared JSON files are acceptable only for rebuildable projections or configuration whose merge rules
are explicit and tested.

## V1 Writer Contract

V1 has no store-directory lock: Shoreline does not coordinate writers with lockfiles, leases, a
daemon, IPC, or filesystem notifications. Concurrency safety rests on the store primitives instead.
Events and object artifacts are written with content-addressed exclusive file creation, note-body
artifacts are content-addressed by body hash, and `state.json` is a regenerable projection written by
atomic rename. So concurrent writers to one store directory cannot corrupt each other: identical
events converge (already-exists with a matching payload), different events never collide, and
conflicting events under one idempotency key fail loud. A stale `state.json` is never read as
authority because reads rebuild from the event log.

The shared common-dir store depends on this directly. Capture and every native review write land in
the `.git/shore` store every read resolves, so multiple worktrees of the same clone may write that
one store directory concurrently, and the content-addressed/regenerable primitives above keep that
safe without a lock. `shore store migrate` reuses the same import path — content-hash-validated,
artifacts before events — to fold a pre-flip worktree-local `.shore/data/` store forward, scanning
for sensitivity findings before movement and reporting them in its document. Any store-directory lock
added later must be scoped to the store directory, never "one clone, one writer", so a future
cross-clone store inherits it.

Event files remain the append-only authority. They are created with exclusive file creation:
same-key and same-payload retries are idempotent, while same-key and different-payload attempts are
conflicts. Different event files can be written independently, but reducers and projections decide
whether the resulting event set is valid, ambiguous, or conflicting.

`state.json` writes are projection cache writes. If projection writers race, events remain
authoritative and the projection can be rebuilt.

Workflow startup cleanup removes only Shoreline temp files older than the workflow startup threshold.
Preserving fresh `.shore-write.*.tmp` files avoids clobbering an in-flight write, but it is not a
lock or lease and does not make long-running multi-process writes a supported coordination model.

## Legacy Writer Role Events

Earlier development versions of Shoreline wrote a `role` field inside each event's writer
envelope. Current Shoreline does not store a writer role: the review act is derived from
`eventType`, and the conversation speaker is recorded by adapters as a `sourceSpeaker` payload
field. Store reads reject stored events whose writer carries `role`. This is a pre-1.0 shape: 1.0 is
the store-format floor, so such events are not upgraded or migrated — the format is retired and no
longer supported (see [Migrations And Doctor](#migrations-and-doctor)).

## Legacy Writer Tool Events

Earlier development versions of Shoreline wrote a `tool` object inside each event's writer
envelope. Current Shoreline names the producing software under `producer` (`{name, version}`); per
the [ADR-0010](./adr/adr-0010-actor-identity-and-delegation.md) vocabulary rule, "agent" names
acting software, "producer" names software that writes events, and the word "tool" is reserved for
the model-API/MCP sense and is no longer an envelope field. The rename rides the pre-adoption
hard-break policy ([ADR-0007](./adr/adr-0007-writer-act-vocabulary.md)): the golden to-be-signed
bytes, the embedded signatures, and `sigVersion: 1` are all untouched.

Store reads reject stored events whose writer carries `tool` with a typed
`UnsupportedEventEnvelope` error naming the retired field (`writer.tool`) and this anchor — which
names the replacement, `writer.producer` — rather than an opaque missing-field error. This is a
pre-1.0 shape: 1.0 is the store-format floor, so such events are not rewritten or migrated — the
format is retired and no longer supported (see [Migrations And Doctor](#migrations-and-doctor)).

## Actor Identity and Delegation

Every event's writer carries an `actorId`. Human writers derive theirs from Git identity
(`actor:git-email:<email>` or `actor:git-name:<name>`); agents write under their own
`actor:agent:<agent-name>` id, set with `SHORE_ACTOR_ID` (see
[agent-authoring.md](./agent-authoring.md)). The actor id is reported in projections but is never
the basis of a binding decision — a writer cannot make a claim trustworthy by asserting it
([ADR-0007](./adr/adr-0007-writer-act-vocabulary.md)).

Who an agent acts *on behalf of* is answered by a checked-in delegation map at
`.shore/delegates.json` — a sibling of `.shore/allowed-signers.json`, deliberately separate so that
key rotation never touches delegation. It is human-committed JSON:

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

- The top-level key is `delegates`; unknown top-level keys are ignored for forward compatibility.
- Each key is an `actor:agent:<name>` id mapping to an array of windowed records.
- `principal` must be a valid **non-agent** actor id in v1 (delegation chains have depth 0).
- `validFrom` is required and `validUntil` is null or an RFC 3339 UTC instant (`Z` offset only,
  e.g. `2026-06-10T00:00:00Z`); the window is half-open `[validFrom, validUntil)`.
- `comment` is free text for diff readers and is never authority.

Resolution is projection-time, replay-stable, and git-free: it selects the record whose window
contains the event's `occurredAt`. **Revocation closes a window** (`validUntil` set) — events
inside the closed window keep resolving, so history stays stable — while **deleting a record is
disavowal**: events that previously resolved deliberately resolve to nothing. `git log -p
.shore/delegates.json` is the audit trail; the file's history, not a mechanism, records who was
enrolled when.

Resolution config is reader-supplied, exactly like the allowed-signers trust set. A consumer
without the map — a mirror, an exported bundle — degrades to `principal: none`, never a wrong
answer. The CLI discovers `.shore/delegates.json` at the worktree root; a malformed file warns once
to stderr and proceeds with no map (advisory, never blocking). Overlapping windows with distinct
principals resolve as `ambiguous` and are surfaced, never auto-picked.

A locally-excluded `.shore/delegates.local.json` override may sit beside the committed
`.shore/delegates.json`. The two layer git-config style: for each agent present in the local file,
its records **fully replace** the committed records for that agent (including replacement with an
empty array, which disavows the agent locally); agents absent from the local file inherit the
committed map; either file may exist alone, and a malformed local file is advisory — it never
poisons the committed default. The committed file stays the shared, portable audit/authority root;
the local override is a private, non-portable convenience. Shoreline keeps
`.shore/delegates.local.json` out of Git via the committed `.shore/.gitignore`'s `*.local.json`
line (the committed `.shore/delegates.json` and `.shore/allowed-signers.json` are deliberately
tracked). `shore identity delegate --local` generates that file automatically before staging the
override; if you hand-create the override instead, commit a two-line `.shore/.gitignore` (`data/`
+ `*.local.json`) yourself — or run the enroll command once — so it does not show up in
`git status`.

In this release, delegation entries are created by editing `.shore/delegates.json` directly (or by
an agent proposing a working-tree edit); the human's review-and-commit is the authorization. The
symmetric signature trust set `.shore/allowed-signers.json` is staged the same possession-style way —
by `shore key enroll` or a hand edit — and documented in the next section.

Pre-cutover honesty: agent events written before the `actor:agent:` cutover carry the human's
git-email id and remain exactly what they claimed at write time. The `agent:*` *track* name is a
heuristic ("written on an agent track"), never re-attribution; recapture is the hard-break escape
hatch.

## Signature Allow-List and Key Home

The committed signature trust set lives at `.shore/allowed-signers.json`, a sibling of
`.shore/delegates.json`. It is a **custom Shoreline JSON document** — **not the OpenSSH**
`allowed_signers` line format despite the filename echo — mapping each actor to the `did:key`s
authorized to sign on its behalf:

```json
{
  "allowedSigners": {
    "actor:git-email:alice@example.com": [
      "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd"
    ],
    "actor:agent:claude-code": [
      "did:key:z6MkpTHR8VNsBxYAAWHut2Geadd9jSwuBV8xRoAnwWsdvktH"
    ]
  }
}
```

Like the delegation map, it is **reader-supplied trust config**, never store content: a consumer
without the file — a mirror, an exported bundle — renders a signed event as `untrusted_key`, never a
wrong `valid`. Entries are staged possession-style (`shore key enroll` writes the working-tree
file; the human's commit is the authorization) and the file is deliberately tracked in Git.

Private **keys never live in the repo `.shore/` or the store** — both are copyable, linkable, or
mirrored surfaces and would ship the private key. They live in a user-level **key home**,
`~/.shore/keys/`, resolved as `SHORE_HOME` (verbatim override, mainly for tests/CI), then
`$XDG_DATA_HOME/shore`, then `$HOME/.shore` on Unix or `%APPDATA%\shore` on Windows. On Unix the key
home is created `0700` and each private-key file `0600`; on Windows mode bits are advisory and the
directory inherits the parent ACL (documented caveat). A private-key file is a minimal
Shoreline-native JSON document carrying the raw 32-byte Ed25519 seed (`{ "version", "alg", "seed" }`,
base64); a `<name>.pub` sidecar records the derived `did:key`.

A key may instead be **agent-backed**: a custody-tagged reference adopted with `shore key use-ssh`,
where ssh-agent custodies the private key and the keystore stores only the **public** key — no seed.
Its on-disk document is `{ "version", "alg", "custody": "agent", "publicKey" }` (the public key
base64); it lives at the same `~/.shore/keys/<name>` with the same `<name>.pub` did:key sidecar, never
in the repo `.shore/` or the store. The change is additive — an existing `{ version, alg, seed }` file
still loads as file custody. Because the reference carries the public key, it is not secret, so the
`0600` mode is not applied (the no-clobber-on-create policy still is); and its `did:key` derives from
the stored public material with **no agent and no private key**, so `enroll` / `list` / `show` work
offline.

## Projection Ordering

Event filenames are derived from idempotency-key hashes. Listing event files therefore does not imply
causal publication order.

Reducers should be order-independent unless the model has introduced an explicit ordering primitive
for the events they consume. A projection may collect facts and derive state at the end, but it
should not depend on "apply this event before that event" just because one filename sorts earlier.
If a future feature needs causal order, add the ordering mechanism first and test the projection
against shuffled event input.

## Storage API Shape

Keep the primitive storage API bytes-shaped first, with JSON as a convenience layer:

```text
storage::read_bytes
storage::read_bytes_if_exists
storage::write_bytes_atomic
storage::create_file_exclusive
storage::list_dir
storage::sweep_temp_files

storage::read_json
storage::write_json_atomic

event_store::write_event
event_store::read_event
event_store::list_events
event_store::event_exists
```

This keeps the lower layer useful for manifests, JSON, future binary artifacts, and exact conflict
checks. Event filename construction should live in `event_store`, not in command handlers.

The first local durable-state stage should stay synchronous and local. Do not introduce async traits
or a runtime until a remote backend, subscription API, or second storage backend forces that
decision.

## Output Boundary

CLI output is also a side effect and should have a seam.

Domain, storage, and workflow code should return values, diagnostics, or events. CLI code should
decide how to write those values to stdout and stderr. Avoid burying `println!` or `eprintln!`
inside workflow logic.

A small boundary such as `run_with_io(args, stdout: &mut dyn Write, stderr: &mut dyn Write)` is
enough. This is not a multi-channel delivery framework; it is a testability and side-effect
boundary.

Machine-readable commands should work through ordinary pipes without terminal allocation. Formatting
should be explicit for automation-oriented JSON output; do not make command semantics depend on
ambient process TTY state unless the command is inherently interactive and fails clearly without one.

## Notifications And Delivery

Notifications are hints, not authority. The durable event must land before any notification fires.
Clients that receive a notification should re-read durable state before acting.

If Shoreline later adds a delivery queue, every retry path must have:

- a maximum attempt count
- backoff policy
- permanent vs. transient failure classification
- a terminal failed state that removes the entry from active rotation
- target-liveness checks before resume or apply actions

The local durable-state stage should not implement this queue. Local event writes should fail loudly
rather than loop.

## Migrations And Doctor

Runtime code should read canonical storage. Legacy repair and migration belong in a future
`shore doctor` or equivalent explicit command.

The legacy flat `.shore/` layout — an early pre-1.0 store that kept `events/`, `artifacts/`, and
`state.json` directly under `.shore/` instead of nesting them under `.shore/data/` — is a **retired
pre-1.0 format**. 1.0 is the store-format floor: a flat store is still detected (by the same
flat-store-marker set the resolve guard uses) and the resolve path surfaces it as a typed,
actionable error, but it is no longer relocated or upgraded — the format is retired and no longer
supported, so the error offers no migration to run. (This retired flat layout is distinct from
`shore store migrate`, which folds a pre-flip `.shore/data/` store into the shared common-dir store;
see [Shared Common-Dir Store Selection](#shared-common-dir-store-selection).) The still-future
`shore doctor`
([issue #9](https://github.com/kevinswiber/shoreline/issues/9)) remains a separate, read-only
diagnostic bundle concern — it is not built.

Migration and repair work should commit independently. One successful fix should not be rolled back
because an unrelated later validation failed. This mirrors the event-log rule: one durable fact, one
independent commit.

## Lock Discipline

The first local event-store implementation should not need locks. If a future change introduces
locks, follow these constraints:

- keep critical sections short
- do not perform long I/O while holding a lock when it can be avoided
- use lock-acquisition timeouts
- record enough state on disk to recover after process death
- do not rely on process-exit cleanup for correctness

## Health And Status

Health checks and status commands should exercise the real path:

- load the manifest or storage root
- list `events/`
- read event envelopes through the event store
- derive fresh state
- compare or refresh the projection

A lightweight probe that bypasses event loading and state derivation can report healthy while the
real workflow is broken. The health path should be the same code path users depend on.
