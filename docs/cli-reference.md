# CLI Reference

This reference covers the public `shore` command surface provided by the `shoreline` crate.

Command output JSON is the machine-integration surface, under a **tiered stability promise**. A
narrow **hard core** is frozen within each document's `version`:

- the envelope discriminators (`schema`, `version`) on every document;
- the field-paths a non-human consumer actually reads — `shore capture`'s `revision.id`,
  `shore input-request list`'s `inputRequests[].{id,title,mode,reasonCode,trackId}`, and
  `shore input-request respond`'s `inputRequestResponseId` and `eventId`;
- the wire-value vocabularies — the assessment values, the input-request response outcomes, and the
  input-request `mode` (`operative`/`advisory`) and `reasonCode` value sets that ride the consumed
  `input-request list` field-paths (see the [`assessment`](#shore-assessment) and
  [`input-request`](#shore-input-request) sections).

Changing any hard-core value is a coordinated break: bump that document's `version` and migrate
consumers. Everything else in the documents is **soft shell** — stable but additive-evolvable within
a `version`: fields may be added, consumers must select by field name and tolerate unknown fields,
and removing, renaming, or reshaping an existing field bumps the `version`. Raw event files,
artifact paths, event filenames, and the store's `state.json` are internal storage details unless a
command explicitly returns them.

## Global Tracing Flags

Most commands accept optional tracing flags:

```bash
--log <filter>
--log-format <compact|pretty|json>
--log-file <path>
```

Tracing writes to stderr by default. When stdout is piped into JSON tools, prefer
`--log-file <path>` so trace lines do not corrupt the JSON stream.

When `--log-file <path>` points inside the repository, Shoreline treats that path as command-helper
plumbing for the current command and excludes it from the reviewed snapshot and fingerprint.

## Actor Identity and Delegation

Every write records a writer `actorId`. By default it derives from the local Git identity
(`actor:git-email:<email>`, then `actor:git-name:<name>`, then `actor:local`). Set
`SHORE_ACTOR_ID` to write under an explicit identity — agents use `actor:agent:<agent-name>`:

```bash
export SHORE_ACTOR_ID="actor:agent:claude-code"
```

`SHORE_ACTOR_ID` outranks the Git identity on every CLI write path, including paths without a
per-call override; a malformed value is ignored and falls through rather than corrupting
provenance.

Review read commands (`history`, the `observation` / `input-request` / `assessment` / `validation`
list and show commands, `revision show`, and the inspector) discover a checked-in delegation map at
`<repo>/.shore/delegates.json` and resolve the human principal an agent wrote on behalf of,
rendering it beside the writer as `claude-code (for kevin@swiber.dev)`. Discovery is presence-based
— absent file, no change. A malformed `.shore/delegates.json` prints a single warning to stderr and
the read proceeds with no resolution (advisory, never blocking). The file format is documented in
[storage-model.md](./storage-model.md).

The **write** side of this config — creating a delegation record or describing an actor's kind/roles —
is the `shore identity` command group (`shore identity delegate` / `shore identity attest`, below).

### Signing

Every write may carry an Ed25519 signature. Which key signs (if any) follows this precedence:

1. `--sign-key <name|path>` on the write subcommand (a keystore key name or a path to a key file)
2. `SHORE_SIGNING_KEY` (same shape: a key name or a path)
3. agent-context auto-keygen — under an `actor:agent:*` id, a passphrase-less per-machine key is
   generated on first write (see [agent-authoring.md](./agent-authoring.md))
4. the user-default keystore key named `default`
5. none — the write proceeds unsigned

`SHORE_SIGNING=off` (case-insensitive) disables signing entirely; `SHORE_SIGNING=auto` (the default)
resolves a signer where possible. `SHORE_HOME` overrides the user-level key home (mainly for
tests/CI). **Signing never gates a write** (with one exception, below): any resolution failure (no key,
an unreadable key home, an unsupported algorithm, a malformed configured key, `SHORE_SIGNING=off`)
degrades to an unsigned write at exit 0 with a one-line advisory diagnostic on stderr — it never blocks.
The sole exception is `shore endorse` (below), where unsigned is a hard error because the
signature *is* the endorsement's content. See
[signing-ux.md](./signing-ux.md) for the human / agent / CI flows and the
`unsigned → untrusted_key → valid` ladder.

## Review Lanes

Every recorded fact — an observation, an assessment, a validation check, an input request, a
commit or ref association — is scoped to a **review lane**: a caller-chosen free-text label
naming who or what is doing the reviewing, such as `agent:codex` or a human reviewer's own
identity. There is no fixed vocabulary or required shape; the label is opaque to Shoreline and
exists so a revision's facts can be filtered and grouped by who recorded them.

- On every write command that records a fact (`shore observation add`,
  `shore assessment add`, `shore validation add`,
  `shore association record` / `withdraw`, `shore input-request open`),
  `--track <track-id>` is **required** and
  stamps the lane that owns the new fact.
- On read/list commands (`shore observation list`, `shore input-request list`,
  `shore validation list`, `shore assessment show`, `shore history`,
  `shore revision show`), `--track <track-id>` is **optional** and narrows the results to one
  lane; omitted, all lanes are returned.

## `shore diff`

```bash
shore diff [--repo <path>] [--revision <id>] [--stat] [--color <auto|always|never>] [--theme <theme>]
```

`shore diff` prints a captured revision's diff — base to target, from the **frozen captured
snapshot** — as a text unified diff on stdout. It is the terminal reader for the immutable diff a
revision recorded; its subject is always the captured snapshot, never the live working tree
(`git diff` owns the live tree).

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--revision <id>` selects the captured revision (a head seed): a current head resolves exactly, a
  superseded revision resolves its thread's current head. Omit it to diff the current capture; it is
  required when the store holds more than one candidate.
- `--stat` prints only the diffstat (a per-file summary and totals), not the diff body.
- `--color <auto|always|never>` controls ANSI syntax coloring of the diff body. `auto` (the default)
  colorizes only when stdout is a TTY, honoring `NO_COLOR` and `CLICOLOR_FORCE` (precedence: `--color`
  > `NO_COLOR` > `CLICOLOR_FORCE` > isatty); piped or redirected output stays plain. Color is pure
  presentation — stripping the ANSI reproduces the plain diff exactly.
- `--theme <theme>` picks the truecolor palette: `auto` (the default) detects the terminal
  background — light or dark — and selects the matching built-in palette; `light` / `dark` force a
  built-in; any other value names a bundled syntax theme, matched case-insensitively (bat's
  vocabulary, e.g. `"Monokai Extended"`, `"onehalflight"`, `"nord"`). Environment fallbacks: `SHORE_THEME`, then
  bat's `BAT_THEME` (precedence: `--theme` > `SHORE_THEME` > `BAT_THEME` > detection > dark). An
  unknown name from `--theme`/`SHORE_THEME` is an error listing the valid vocabulary; an unknown
  inherited `BAT_THEME` warns on stderr and falls back. The terminal is queried only when colors
  are on, stdout is a direct truecolor TTY, and the preference is `auto` — piped output never
  probes and stays deterministic. Themes apply on truecolor terminals (`COLORTERM=truecolor`); the
  16-color palette always follows the terminal's own theme. Intraline (changed sub-word) emphasis
  renders as an add/del background tint on truecolor and as an underline on 16-color terminals.
- `shore diff` is a **filter, not a pager**: it writes plain git-diff to any pipe or redirect and
  colorizes only when writing directly to a terminal, so it composes with the tools you already use —
  `shore diff | less -R` to page, `shore diff | delta` (or another diff renderer) to reformat,
  `shore diff > change.diff` to save. There is no built-in pager and no `--no-pager` flag; use
  `--color always` to force color through a pipe (e.g. `shore diff --color always | less -R`). A
  reader that closes the pipe early (`shore diff | head`) is a clean exit.
- The command is **text-only and non-interactive**: it has no `--format` selector and emits no JSON
  (machine consumers read the review documents, e.g. `shore revision show --format json`). Its output
  is **disposable** — wording, layout, and ordering may change between releases, so nothing should
  parse it.
- When a revision's captured content has been removed from the store, `shore diff` prints a short
  "content is unavailable" line (with the removed content's short id) instead of a diff body.

## `shore inspect`

```bash
shore inspect [--repo <path>] [--host <addr>] [--port <n>] [--open]
```

`shore inspect` starts a small local web server that visualizes the worktree's resolved store for
tracing event timelines and outcomes — the kind of inspection that is awkward against the raw JSON
files or per-command output.

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--host <addr>` defaults to `127.0.0.1` (loopback). `--port <n>` defaults to `7878`; use
  `--port 0` to bind an ephemeral port, which is then printed.
- `--open` launches the inspector in the default browser after the server starts.
- The server runs until interrupted with Ctrl-C.

The page provides a chronological event timeline (filterable by track, revision, thread, and
event type, newest-first by default), a per-event detail view, a composite per-revision page showing
the current-assessment status plus grouped observations, input requests, assessments, and
competing-head badges, a supersession-thread list/detail view showing heads, threaded revisions,
diagnostics, and stale superseded-revision facts, and the captured diff for a revision annotated with
the review facts anchored to each line. Validation checks recorded with `shore validation add`
appear throughout — as a labeled timeline event type, a "Validation checks" section on the revision
page (with the check name, status, trigger, and exit code), and on thread cards — shown for context
only; they do not affect the current assessment and carry no merge or acceptance authority. The
reader-relative `verificationStatus` and `endorsements` readback (see
[Verification status and endorsement readback](#verification-status-and-endorsement-readback))
renders beside events and review facts: the per-event signature status as a chip on the timeline,
detail view, and revision fact cards, and the endorsement classification with its resolved
`endorser` and `endorserAttributes` on the detail view and revision page — presented as advisory,
render-only information, never as a gate or verdict. Long IDs render as
truncated, clickable references that navigate to the resource they name, and the page auto-refreshes
when the store changes or a freshness diagnostic appears or clears.

The inspector is a read-only, single-store, localhost developer tool. It reads through the same
validated projections as `shore history` and `shore revision show` rather than parsing raw
storage, and it serves over a synchronous, dependency-free HTTP server with no async runtime. The
small JSON API the page consumes (`/api/history`, `/api/revisions`, `/api/revisions/{id}`,
`/api/threads`, `/api/snapshots/{id}`, `/api/freshness`) is an internal surface for the bundled page,
not a stable contract.

Every worktree of a clone resolves the shared common-dir store (`.git/shore`), so the inspector
renders snapshots captured in sibling worktrees as well as the current one. The `/api/snapshots/{id}`
payload is **content-only**: it carries the immutable diff content and its `contentHash` only — no
`revisionId`, `source`, `base`, or `target`. The captured worktree path is therefore simply absent
from the snapshot wire (there is nothing to redact). Endpoint/target display lives on
`/api/revisions/{id}` and `/api/revisions`, derived from the revision projection (a path-private
`targetDisplay` block), not from the snapshot artifact. `shore revision list` JSON still carries
`target.worktreeRoot`, unchanged.

## `shore capture`

```bash
shore capture [--repo <path>] [--base <rev> [--target <rev>]] \
  [--path <pathspec>]... [--supersedes <revision-id>]...
```

`shore capture` records the current V1 revision: the base endpoint, target endpoint, and
captured diff snapshot. By default V1 captures the local Git worktree from `HEAD` to the working
tree, including untracked files (source `git_worktree`).

- With `--base <rev>`, capture instead records the committed range from `<rev>` to `--target`
  (default `HEAD`) as a `git_commit_range` source. Both revs are resolved with `git rev-parse` to
  commit OIDs at capture time: annotated tags peel to their commit, and a rev that does not exist or
  does not name a commit (a blob or tree) is rejected with an error that names the rev. The snapshot
  is the `base..target` tree diff with no working-tree, index, or untracked involvement, so both
  endpoints serialize as `git_commit` and no worktree path appears in the output. `--target`
  requires `--base`. Re-capturing the same range is idempotent and reports `eventsExisting`, and an
  equivalent rev spelling (`HEAD~1` versus the resolved OID) captures the same revision because
  rev spellings are never stored. Like worktree capture, a range capture lands in the shared
  common-dir store, so it is immediately visible from sibling worktrees (see below).
- Durable state lands in the shared common-dir store at `.git/shore` under the clone's Git common
  directory (the default for every worktree). An `ephemeral` worktree instead keeps its own
  discardable `.shore/data/` store. A legacy flat `.shore/` store from before the `.shore/data/`
  layout is a retired pre-1.0 format: it is detected and refused, not migrated — distinct from
  `shore store migrate`, which folds a pre-flip worktree-local `.shore/data/` store into the shared
  store; see [storage-model.md](./storage-model.md#migrations-and-doctor).
- Opting into ephemeral mode generates a committed `.shore/.gitignore` (two lines: `data/` +
  `*.local.json`) when the paths are not already ignored, so the worktree-local store stays out of
  `git status`; the file is visible, meant to be committed, and survives clone. Writer-initializing
  commands against the ephemeral store generate it too; a shared-store write generates nothing (the
  shared store lives inside `.git/`, which git already ignores) and never mutates the working tree.
  Nothing writes `.git/info/exclude` anymore, and no tracked `.gitignore` is ever modified.
  Committed config siblings (`.shore/delegates.json`, `.shore/actor-attributes.json`,
  `.shore/allowed-signers.json`, `.shore/store.json`) stay tracked; only `.shore/data/` and the
  private `.shore/delegates.local.json`, `.shore/actor-attributes.local.json`, and
  `.shore/store.local.json` overrides are excluded.
- The store subtree (`events/`, `state.json`, and `artifacts/`) is the same wherever it resolves:
  the shared common-dir store at `.git/shore` by default, or an `ephemeral` worktree's own
  `.shore/data/`.
- `events/` stores immutable event files.
- `state.json` is a rebuildable projection, not the authority.
- Full captured snapshots are Shoreline-owned immutable object artifacts under `artifacts/objects/`.
- The `work_object_proposed` event binds to the object artifact's canonical content hash; the
  content-only artifact body carries no revision identity or endpoints (those live on the event).
- Output is compact `shore.review-capture` JSON and includes the revision and object IDs
  plus `objectArtifactContentHash`.
- With `--supersedes <revision-id>` (repeatable, order-independent), the capture records that it
  evolves past the named revisions, extending the supersession DAG. The new revision becomes a thread
  head; when two captures supersede the same predecessor, the competing heads are surfaced rather than
  auto-collapsed.
- With `--path <pathspec>` (repeatable), the capture is scoped to the given git pathspec(s): both
  the tracked diff and untracked-file synthesis include only matching files. This composes with the
  default worktree capture and with `--base`/`--target`. The syntax is native git pathspec —
  including magic such as `:(exclude)...` — executed by git itself, and pathspecs are interpreted
  relative to the repository root regardless of the invoking directory. The recorded set is
  order-independent (sorted, deduped, trailing slashes normalized on plain paths) and is part of
  the captured revision's identity: the same range captured under a different scope is a different
  revision, while identical captured content still shares one content object. A scope that matches
  no changed files is an error — a scoped capture never records an empty revision. The scope is
  visible in `shore revision show`, `shore revision list`, and `shore history` under
  `source.pathspecs`; an unscoped capture carries no `pathspecs` key and its identity is unchanged
  from before the option existed. (This is deliberately git pathspec syntax, not the
  gitignore-style globs of `.shore/sensitivity.json` — capture scoping is executed by git, while
  the sensitivity exclude config is matched by Shore at scan time.)

V1 storage is local and synchronous. The shared common-dir store may take concurrent writes from
multiple worktrees of the same clone, kept safe by content-addressed writes and a regenerable
projection rather than a lock. V1 does not add a daemon, delivery queue, approval flow, async
storage, remote storage, or note mutation.

By default every worktree of a clone resolves the shared common-dir store at `.git/shore`, so
`shore capture` lands its capture directly there — no setup step — and the capture is
immediately visible to `revision list`, `revision show`, and `history` from any sibling worktree. An
`ephemeral` worktree instead captures into its own discardable `.shore/data/` store (see
[`shore store`](#shore-store)).

The native review write commands — `shore observation add`, `shore input-request open`
and `respond`, `shore assessment add`, and `shore validation add` — behave the same
way. They resolve the shared common-dir store, so you can record a
fact against a revision (or related observation, assessment, or request) captured in a sibling
worktree, and the fact lands directly in that shared store, visible to every worktree in place. An
`ephemeral` worktree writes the fact to its own `.shore/data/` store, unchanged.

## `shore store`

```bash
shore store status [--repo <path>] [--pretty]
shore store mode (shared | ephemeral | show) [--repo <path>] [--pretty]
shore store migrate [--repo <path>] [--include-ephemeral] [--retire-source] [--pretty]
shore store link [<slug>] [--repo <path>] [--include-ephemeral] [--include-sensitive] [--retire-source] [--pretty]
shore store unlink [--repo <path>] [--pretty]
shore store forget <slug> [--yes] [--force] [--pretty]
shore store list [--pretty]
shore store remove [--repo <path>] (--snapshot <id> | --revision <id> | --ref <name> | --range <a>..<b> | --orphans) [--sign-key <key>] [--pretty]
shore store gc [--repo <path>] [--pretty]
shore store compact [--repo <path>] [--pretty]
```

`shore store` commands inspect, configure, and maintain the review store the current Git worktree
resolves. By default every worktree of a clone — the main worktree and every linked worktree alike —
resolves the same **shared common-dir store** at `.git/shore` (the path under the repo's Git common
directory), automatically and with no setup step. Because linked worktrees share one Git common
directory, a capture in any worktree is immediately visible from its siblings. This is a per-clone
store, not a user-level multi-repository store or remote sync service.

`shore store status` resolves the store and emits `shore.store-status` JSON.

- `mode` is `local` and `storeRef` is `local`. (Both report the resolved local store; there is no
  registration step and no separate "linked" mode.)
- `inventory` reports `eventCount`, `eventBytes`, `artifactCount`, `artifactBytes`, `totalBytes`,
  optional `untrackedBytes`, `largestArtifacts`, and `revisionObjects`. Artifact entries use
  opaque artifact refs rather than filesystem paths. Each `revisionObjects` entry carries a
  `revisionIds` **list** (sourced from the `work_object_proposed` events keyed by `objectId` plus
  `objectArtifactContentHash`), because one object artifact may be referenced by several revisions
  under the shared-store model (#146), while a rebased recapture can store another artifact for the
  same stable object id.
- `sensitivity` reports `policyOutcome` plus redacted findings. Finding references use
  `file:sha256:*` refs, and command output does not print secret values or source file paths.

Sensitivity findings are reported but do not currently abort a write. A hard-blocking policy and
explicit override controls are a forward-looking note for when movement can target a wider store;
blocking findings still name only safe finding kinds, such as `known_token`, and command output does
not print the secret text or file path.

Every review read command resolves the shared common-dir store: `revision list` and `revision show`,
`history`, the observation, input-request, and validation lists, the association list, and
`assessment show` read it from any worktree of the clone, including hydrated bodies and the captured
snapshot, so their `eventCount` and `eventSetHash` reflect that one store.

`shore store mode` reads or sets a per-worktree store mode that controls where the worktree's review
data lands. `shore store mode ephemeral` pins the worktree to a discardable worktree-local
`.shore/data` store — the privacy escape hatch for sensitive or throwaway work whose bytes should
disappear when the worktree is removed. `shore store mode shared` (the default) uses the shared
common-dir store. The mode is written to a committed `.shore/store.json`, and may be overridden
privately by a git-excluded `.shore/store.local.json` (the local file wins, the
`delegates.json` / `delegates.local.json` precedent). A malformed or unsupported config is a hard
error rather than a silent fallback. `shore store mode show` reports the resolved mode without changing
it. All three forms emit `shore.store-mode` JSON with `mode` (`shared` | `ephemeral`) and `source`
(`default` | `committed` | `local`); the body embeds no storage path.

### Sensitivity exclude globs

The worktree sensitivity scan (run by `shore store status` and `shore store migrate`'s consent
gate) supports a committed **`.shore/sensitivity.json`** plus a git-excluded
**`.shore/sensitivity.local.json`** (covered by `.shore/.gitignore`'s `*.local.json` line) listing
path globs the scan skips — the targeted alternative to the blanket `--include-ephemeral` override
when a repo's own test fixtures carry scanner-triggering strings:

```json
{
  "schema": "shore.sensitivity-config",
  "version": 1,
  "excludeGlobs": ["tests/**", "src/session/store/sensitivity.rs"]
}
```

The two files merge by **union** (committed order first, then novel local entries) — deliberately
diverging from the local-replaces-committed rule `store.json`/`delegates.json` use, because this is
a *list*: replace would force copying the whole committed list to add one local entry, union grants
nothing replace couldn't, and the audit counts make any widening visible. Default is empty (scan
everything; opt-in only).

Glob semantics are a documented gitignore-style subset matched against the repo-relative path: a
leading `/` or any interior `/` makes the pattern rooted (`tests/**`, `src/lib.rs`); a slash-free
pattern matches any path component at any depth (`*.pem`, `data`); a trailing `/` marks a directory
pattern matching only paths inside it; `**` spans segments (zero or more interior, one or more
trailing); `*` and `?` never cross `/`. Negation (`!`), empty patterns, and malformed or
unsupported-version config are hard errors naming the offending file — the config gates a
protection, so a misread never silently changes coverage.

An excluded path is **not scanned** — an explicit operator opt-out, kept honest by the audit
surfaces: `shore store status` reports `sensitivity.excludedPathCount` and
`sensitivity.excludeGlobs[{glob, matched}]` (zero-count globs included; a dead glob is itself worth
seeing), and `shore store migrate` reports `sensitivityExcludedPathCount` whenever its gate scan
ran (absent under `--include-ephemeral`, which skips the scan). Excluded paths themselves are never
listed — the scan's redacted `file:sha256:*` posture stands. The gate behavior is unchanged: a
`block` finding outside the excludes still refuses without `--include-ephemeral`.

A pre-flip worktree-local `.shore/data` store on a non-ephemeral worktree (data written before the
shared common-dir default) is detected on any read or write and errors with a hint to run
`shore store migrate`. `shore store migrate` folds that legacy store into the shared common-dir store
**non-destructively** by default: it copies events and artifacts forward and leaves `.shore/data` in
place, so you can verify the result and then remove `.shore/data` yourself to finish the switch. It
is idempotent (re-running reports the already-present facts as existing), and it refuses an ephemeral
or sensitivity-flagged worktree unless you pass `--include-ephemeral`.

**`--retire-source`** completes the switch in one command: after the fold, an independent
verification walks every durable file in the source store (`events/` and `artifacts/`, recursively;
only in-flight `*.tmp` files are excluded — the regenerable store-root `state.json` sits outside
those trees, and a nested file merely named `state.json` is verified like any other) and requires each to be
present in the shared store with identical content — byte-identical for artifacts, canonically
identical modulo the import's own ingest-provenance stamp for events. Only then is `.shore/data`
deleted, so the very next read resolves. On **any** missing or divergent file — including an orphan
artifact no event references, which the fold deliberately does not carry — the command errors,
names the offending paths, and deletes nothing. A source with no durable files at all (only the store-root
`state.json` or the empty directories the writer pre-creates) is removed as a husk without a fold;
a source holding artifact files but no event files is refused outright. Classification is by file
counts, never directory existence.

It emits `shore.store-migrate` JSON with `eventsCreated`, `eventsExisting`, `artifactsCreated`,
`artifactsExisting`, `sourceEmpty`, `sourceRetired`, `verifiedEvents`, and `verifiedArtifacts`.
(This is distinct from the legacy flat `.shore/` layout, a retired pre-1.0 format that is detected
and refused rather than migrated; see
[storage-model.md](./storage-model.md#migrations-and-doctor).)

`shore store link [<slug>]` promotes this clone into the opt-in **user-level family store** at
`<shore-home-root>/stores/<slug>/`, a per-machine store shared across independent clones of the same
repository family so review facts survive removing any one clone. The binding is recorded only in the
git-excluded `.shore/store.local.json`, so it never travels in a commit. Before any family write,
`link` runs its gates in order: it refuses an ephemeral worktree (override `--include-ephemeral`) and
a sensitivity-flagged worktree (override `--include-sensitive`), refuses a slug already stamped for a
different family, and warns (without blocking) on a sync-managed filesystem path or when the clone
shares no git history with an existing family. It then folds the clone-local `.git/shore` history
forward with independent verification and flips the binding last, so an interrupted link leaves the
clone still resolving its clone-local store. Omitting `<slug>` fails with a suggestion rather than
picking one silently. `--retire-source` deletes the clone-local store only after the fold is
verified. When the fold carries prior unsigned `shore store remove` events, a diagnostic discloses
that they lost possession-based suppression and should be re-issued in the family store. It emits
`shore.store-link` JSON (`familyRef`, `cloneRef`, `createdFamily`, the `folded*` counts,
`sourceRetired`, and any warnings). `shore store unlink` detaches this clone (clearing the binding
and deregistering it) without moving any data, and survives a family store that was already forgotten.

`shore store forget <slug>` is the whole-store destructive verb for a family store, deliberately
outside `shore store remove`'s content-targeted removal (no store survives a forget to hold a removal
event). It is dry-run by default: it previews the inventory and live-clone count that would be lost
and deletes nothing. `--yes` performs the deletion, but only for a family with zero live clones (an
**orphaned** family store — a different notion from `shore store remove --orphans`, which targets
unreachable-commit content); a family with live clones additionally requires `--force`. `shore store
list` is the one repo-less surface: it takes no `--repo` flag, never resolves a git repo, and walks
`<shore-home-root>/stores/` reporting each family's `familyRef`, inventory, `liveCloneCount`,
`orphaned` flag, and `lastWrite`. Against an empty home it returns an empty `families` array.

`shore store remove` retires content-addressed artifacts from the store. It resolves exactly one
selector to a set of content hashes — `--snapshot <id>` (a snapshot's bound artifact), `--revision
<id>` (every artifact a revision references), `--ref <name>` / `--range <a>..<b>` (artifacts of
revisions anchored on the named commit or commit range), or `--orphans` (artifacts of commit-anchored
revisions whose commits are all unreachable) — and records one removal fact per content hash. It emits
`shore.store-remove` JSON listing each `contentHash`, whether it was newly `created`, and
`coReferencingUnits` (other revisions that still name the same shared artifact, reported before the
removal), plus `eventsCreated` and `eventsExisting`. Removal is content-targeted and idempotent:
re-removing a hash reports `created: false`. Removal is a write, so a signed store stays signed —
`--sign-key` selects the signing key exactly as `shore capture` does. There is deliberately no
`--idempotency-key`; the removal key is derived solely from the content hash. Removal records the
fact; it does not delete bytes — run `shore store gc` / `shore store compact` to reclaim them.

`shore store gc` and `shore store compact` are the same local sweep: they physically delete the
content-addressed blobs whose content hash has been removed, reclaiming disk. The sweep records no
event and is fully re-derivable from the log — re-capturing the same content re-materializes the blob.
It emits `shore.store-compact` JSON listing each swept blob's `contentHash` and `outcome`
(`removed` or `missing`) plus `bytesReclaimed`. Running it again is a no-op (every removed blob is
already `missing`).

Removed content renders as an explained state on every read surface, never as an error. For
note-shaped bodies (observation and input-request bodies, response reasons, assessment and
validation summaries, imported note bodies), `shore revision show`, the leaf `list`/`fetch`/`show`
commands, and `shore history` omit the body text and carry a `bodyContentState` /
`summaryContentState` / `reasonContentState` field beside the content hash — `suppressed_present`
while the bytes are still stored (a compact would reclaim them) or `physically_removed` after the
sweep — plus `body_content_suppressed_present` / `body_content_physically_removed` diagnostics.
The field is omitted entirely while content is present, and a body that is missing *without* a
recorded removal still fails the read with the `import referenced artifacts` guidance.

Command output is the machine-integration surface, under the tiered stability promise described at
the [top of this reference](#cli-reference) (a frozen hard core; an additive-evolvable soft shell).
Raw store paths, event files, artifact paths, `.git` paths, `.shore/data` paths, and `state.json`
remain internal storage details.

## `shore identity`

```bash
# Stage a delegation record binding an agent to its responsible principal (shore.identity-enroll).
shore identity delegate <agent-actor-id> --principal <principal-actor-id> \
  [--from <RFC3339>] [--until <RFC3339>] [--comment <text>] [--local] [--repo .] [--pretty]

# Stage an actor-attributes entry — kind + roles — for any actor (shore.identity-attest).
shore identity attest <actor-id> --kind <kind> [--role <role>]... \
  [--comment <text>] [--local] [--repo .] [--pretty]
```

`shore identity` writes the actor/principal config the read side (above) resolves. Both subcommands are
possession-style: they stage the working-tree edit only and never invoke git — review and commit the
file to apply it (`git log -p` is the audit trail), exactly like `shore key enroll`.

- **`delegate`** stages a delegation record into `.shore/delegates.json` binding `<agent-actor-id>` (an
  `actor:agent:<name>` id) to a responsible **non-agent** `--principal` (the human/actor that answers
  for the agent; the depth-0 rule rejects an agent principal). `--from` defaults to now in RFC 3339 UTC;
  `--until` defaults to an open window; `--comment` is free text for diff readers. Emits a
  `shore.identity-enroll` document and a stderr hint to commit.
- **`attest`** stages an actor-attributes entry into `.shore/actor-attributes.json` for `<actor-id>`
  (any persisted actor id). `--kind` is required — exactly one kind per actor; the reserved well-known
  kinds are `human`, `agent`, `service`, and `reviewer-model`, but any lowercase-kebab token is
  accepted. `--role` is repeatable; kind and roles are normalized to lowercase-kebab and roles are
  deduped + sorted. Re-attesting **replaces** the actor's entry (kind, roles, and comment) — it is not
  additive. Emits a `shore.identity-attest` document.
- **`--local`** writes the private `.local.json` sibling instead of the committed file and git-excludes
  it via the generated, committed `.shore/.gitignore` (its `*.local.json` line covers every private
  override). The layers merge git-config style: a local entry **fully replaces** the
  committed entry for that key on this machine (never a merge), and the command surfaces that
  full-replace caveat on stderr.
- `--repo` (default `.`) may be the repository root or a path inside it; the entry always lands at the
  worktree-root `.shore/`. Inputs are validated against the same grammar the readers enforce, so a
  staged file always re-reads and a rejected input writes nothing.

The delegation-map format is documented in [storage-model.md](./storage-model.md); the models and
decisions are in [ADR-0010](./adr/adr-0010-actor-identity-and-delegation.md) (delegation) and
[ADR-0012](./adr/adr-0012-actor-attributes-and-roles.md) (actor attributes).

## `shore key`

Manage the user-level signing keystore and stage signer enrollment. Keys live in `~/.shore/keys/`
(honoring `$XDG_DATA_HOME` on Unix and `%APPDATA%\shore` on Windows; `SHORE_HOME` overrides) — never
in the repo `.shore/` or the store. See [storage-model.md](./storage-model.md) for the key home and
allowed-signers format.

```bash
# Generate a human signing key and print its did:key (shore.keys-init).
shore key init --name default

# List local keys with enrollment status and which is the default (shore.keys-list).
shore key list --repo .

# Print a key's did:key and/or raw public key (shore.keys-show).
shore key show default --did
shore key show default --pubkey

# Adopt an existing SSH Ed25519 key as an agent-backed signer (shore.keys-use-ssh).
# Reuses ssh-agent custody — no new key material. Parallel to `init`.
shore key use-ssh ~/.ssh/id_ed25519.pub --name default
shore key use-ssh 'key::ssh-ed25519 AAAA…'   # git user.signingKey literal form

# Stage an allow-list entry binding a key's did:key to an actor (shore.keys-enroll).
# Possession-style: this stages the working-tree .shore/allowed-signers.json edit only;
# review and commit it to authorize the binding.
shore key enroll default --actor actor:agent:claude-code --repo .
```

`init` refuses to overwrite an existing named key. `use-ssh` adopts an existing SSH **public** key as an
agent-backed `default` signer: it accepts a `*.pub` path or a `key::ssh-ed25519 AAAA…` literal, emits a
`shore.keys-use-ssh` document with the derived `did:key` (the same `.didKey` field `shore key show
--did` prints) plus an enrollment hint, and (like `init`) refuses to overwrite. Only plain `ssh-ed25519`
keys are accepted; `ed25519-sk`/RSA/ECDSA are rejected with a clear error pointing at `shore key init`.
`list`/`enroll` take `--repo` (default `.`) to resolve the committed `.shore/allowed-signers.json`;
every subcommand accepts `--pretty`. Enrollment never commits — the human's commit is the authorization.

Each **write** subcommand (`capture`, `observation add`, `assessment add`,
`validation add`, `association record`/`withdraw`, `input-request open`/`respond`)
accepts `--sign-key <name|path>` to
sign that write with a specific key (highest precedence; overrides `SHORE_SIGNING_KEY`). A key that
cannot be loaded leaves the write unsigned at exit 0 with an advisory diagnostic — signing never
blocks. An agent-backed key resolves through an identities-only ssh-agent pre-flight; if the agent is
unavailable (`signing_agent_unavailable`), does not hold the key (`signing_agent_key_absent`), or fails
the real sign (`signing_agent_sign_failed`), the write is left unsigned at exit 0 — see
[signing-ux.md](./signing-ux.md) for the full never-gates table. This never-gates behavior covers the
ordinary signed review writes listed above; `shore endorse` is the exception, where an
unresolved signer is a hard error rather than an unsigned write. Only shipped subcommands are listed;
`rotate` and `revoke` are named follow-ons, not yet available.

## `shore observation`

```bash
shore observation add --track <track-id> --title <title> \
  [--revision <revision-id>] [target options] [--body-content-type text/plain|text/markdown] \
  [--tag <tag>]... [--confidence low|medium|high] [--supersedes <observation-id>]... \
  [--responds-to <observation-id>]...
shore observation list [--revision <revision-id>] [--track <track-id>] \
  [--file <path>] [--tag <tag>] [--include-body] [--pretty|--compact]
```

Observations are append-only review notes for a captured revision.

- `observation add` requires `--track` and `--title`.
- `--revision` pins the observation to one captured revision. Without either, the command defaults to the single captured revision and errors if
  multiple captured revisions exist.
- Tracks are review lanes, not actor or producer provenance.
- Without `--file`, the observation targets the whole revision.
- With `--file <path>`, it targets a captured file.
- With `--file <path> --start-line <n> [--end-line <n>]`, it targets a range on `--side <old|new>`
  where the default side is `new`.
- Bodies may come from `--body`, `--body-file`, or `--body-stdin`.
- `--body-content-type` defaults to `text/plain`; use `text/markdown` when the body should render
  as Markdown in the inspector.
- Large bodies are stored as Shoreline-owned `shore.note-body` artifacts while command output keeps
  artifact paths private.
- `--supersedes <observation-id>` (repeatable) records a correction by appending a new observation
  that names the older observation.
- `--responds-to <observation-id>` (repeatable) records that this observation responds to an
  existing observation — a fact-to-fact relationship (a derived `responded_by` back-pointer is
  surfaced on the target). It does not supersede or mutate the target.
- `--confidence <low|medium|high>` records an optional confidence level on the observation.
- `--tag <tag>` (repeatable) attaches free-form tags used by the `observation list` `--tag` filter.
- `observation list` replays durable events for the revision and may filter by revision, track,
  file, or tag. It hydrates body text only with `--include-body`.

Output is compact `shore.review-observation-add` or `shore.review-observation-list` JSON by
default. `observation list` also accepts `--pretty` and `--compact`.

## `shore input-request`

```bash
shore input-request open --track <track-id> --title <title> --reason <reason> \
  [--revision <revision-id>] [--mode operative|advisory] \
  [--body-content-type text/plain|text/markdown]
shore input-request list [--revision <revision-id>] [--track <track-id>] \
  [--mode operative|advisory] [--file <path>] [--status open|responded|ambiguous|all] \
  [--include-body] [--pretty|--compact]
shore input-request show <input-request-id> [--include-body]
shore input-request respond <input-request-id> --outcome <outcome> [reason options] \
  [--reason-content-type text/plain|text/markdown]
```

Input requests are durable pause or decision requests for a captured revision.

- `input-request open` requires `--track`, `--title`, and `--reason`.
- `--revision` pins the request to one captured revision. Without either, the command defaults to the single captured revision and errors if
  multiple captured revisions exist.
- `--mode` defaults to `operative`; `advisory` requests are durable and visible but do not imply a
  cooperative client must pause. The `mode` (`operative`/`advisory`) and `reasonCode` values surface
  on the consumed `input-request list` field-paths, so — like the response outcomes below — they are
  part of the frozen hard core: stable within `version:1`, changed only by a coordinated `version`
  bump.
- Targets mirror observations: review-wide by default, captured file, captured range, or an
  existing native observation through `--observation <observation-id>`.
- Request bodies may come from `--body`, `--body-file`, or `--body-stdin`.
- `--body-content-type` defaults to `text/plain`; use `text/markdown` when the request body should
  render as Markdown in the inspector.
- Large request bodies reuse Shoreline-owned `shore.note-body` artifacts while command output keeps
  artifact paths private.
- `input-request list` is the V1 polling read surface and defaults to open requests. It may filter
  by revision, track, mode, file, or status, and hydrates body text only with `--include-body`.
- `input-request show <id> --include-body` returns one request and hydrates the body when
  requested.
- `input-request respond <id>` appends an `input_request_responded` event.
- Response reasons may use `--reason-content-type text/markdown`; the default is `text/plain`.
- Response outcomes are `approved`, `rejected`, `dismissed`, `superseded`, and `abandoned`. These
  wire values are part of the frozen hard core (review-loop drivers branch on them): stable within
  `version:1`, changed only by a coordinated `version` bump.

Output documents are compact `shore.review-input-request-open`,
`shore.review-input-request-list`, `shore.review-input-request-fetch`, and
`shore.review-input-request-respond` JSON by default. Read commands also accept `--pretty` and
`--compact`.

V1 is durable and polling-friendly. It does not add a daemon, filesystem watch mode, TUI prompt,
notification transport, or cancellation/escalation event.

## `shore assessment`

```bash
shore assessment add --track <track-id> --assessment <assessment> \
  [--revision <revision-id>] [target options] [--summary-content-type text/plain|text/markdown]
shore assessment show [--revision <revision-id>] [--all] [--track <track-id>] \
  [--include-summary] [--pretty|--compact]
```

Assessments record review calls for a captured revision.

- `assessment add` requires `--track` and `--assessment`.
- `--revision` pins the assessment to one captured revision. Without either, the command defaults to the single captured revision and errors if
  multiple captured revisions exist.
- CLI input uses `kebab-case` assessment values: `accepted`, `accepted-with-follow-up`,
  `needs-changes`, and `needs-clarification`. Command JSON output uses the matching `snake_case`
  values: `accepted`, `accepted_with_follow_up`, `needs_changes`, and `needs_clarification`. The
  `snake_case` wire values are part of the frozen hard core (review-loop drivers branch on them):
  stable within `version:1`, changed only by a coordinated `version` bump.
- Targets mirror the revision ledger: review-wide by default, captured file, captured range,
  native observation, native input request, or another assessment.
- Summaries may come from `--summary`, `--summary-file`, or `--summary-stdin`.
- `--summary-content-type` defaults to `text/plain`; use `text/markdown` when the summary should
  render as Markdown in the inspector.
- Large summaries reuse Shoreline-owned `shore.note-body` artifacts while command output keeps
  artifact paths private.
- `--replaces <assessment-id>` is the only V1 relationship that removes an older assessment from
  the current set.
- `--related-observation` and `--related-input-request` record evidence links; they do not mutate
  observations or close input requests.
- `assessment show` reports current status as `unassessed`, `resolved`, or `ambiguous`. It may
  filter by revision or track, include replaced assessments with `--all`, and hydrate summaries
  with `--include-summary`.

Output documents are compact `shore.review-assessment-add` and `shore.review-assessment-show` JSON
by default. `assessment show` also accepts `--pretty` and `--compact`.

State-change outcomes such as deferred, split-out, overridden, and superseded are ordinary review
observations when needed.

## `shore validation`

```bash
shore validation add --track <track-id> --check-name <name> --status <status> \
  [--revision <revision-id>] [validation options] [--summary-content-type text/plain|text/markdown]
shore validation list [--revision <revision-id>] \
  [--track <track-id>] [--status <status>] [--include-body] [--pretty | --compact]
```

Validation checks record local test, lint, build, or other verification evidence for a captured
revision. They are advisory review context only: they do not accept, reject, merge, block, or
replace a review assessment.

- `validation add` requires `--track`, `--check-name`, and `--status`.
- `--revision` pins the check to one captured revision. Without either, the command defaults to the single captured revision and errors if
  multiple captured revisions exist.
- Validation targets are revision-only. There are no file or path target flags.
- Status values are `passed`, `failed`, `errored`, and `skipped`.
- `--command`, `--exit-code`, `--source-fingerprint`, `--started-at`, `--completed-at`, and
  repeatable `--log-content-hash` record evidence metadata without exposing artifact paths.
- `--trigger` defaults to `manual`; accepted values are `manual`, `push`, and `pull-request`.
- Summaries may come from `--summary`, `--summary-file`, or `--summary-stdin`.
- `--summary-content-type` defaults to `text/plain`; use `text/markdown` when the summary should
  render as Markdown in the inspector.
- Large summaries reuse Shoreline-owned `shore.note-body` artifacts while command output keeps
  artifact paths private.
- `validation list` replays durable events for the revision and may filter by revision, track,
  or status. It hydrates summaries only with `--include-body`.

Output documents are compact `shore.review-validation-add` and
`shore.review-validation-list` JSON by default. `validation list` also accepts `--pretty` and
`--compact`.

## `shore endorse`

```bash
shore endorse <target-event-id> [--sign-key <name|path>] [--actor <id>] [--repo .] [--pretty]
```

`shore endorse` records a detached co-signature (an endorsement) over an existing target event —
for example a captured revision's `work_object_proposed` event. The resolved signer is the attesting
signer and the carrier's envelope writer is the **endorser's own actor** (`--actor`, else the resolved
writing identity), never the target's author.

- **Unsigned is a hard error.** Unlike every other write — where signing never gates — an endorsement
  has no unsigned form, because the signature *is* its content. The signer is resolved first (before the
  target); if none resolves (`SHORE_SIGNING=off`, no key, an unreadable key), the command exits non-zero
  and writes nothing. Signer precedence otherwise follows the **Signing** rules above.
- Idempotent: re-endorsing the same target with the same signer is a no-op (`eventsCreated: 0`,
  `eventsExisting: 1`, same carrier `eventId`).
- The emitted `shore.review-endorse` document reports carrier facts (`eventId`, `targetEventId`,
  `targetEventRecordHash`, `attestingSigner`, `actorId`, and write counts) — **not** a trust verdict.
  Whether an endorsement classifies as trusted is reader-relative (resolved against the reader's
  allow-list at read time), not stamped at write time.

The endorsement record and its read-side classification are decided in
[ADR-0013](./adr/adr-0013-endorsement-record-and-classification.md).

## `shore association`

```bash
shore association record --track <track-id> (--commit <rev> | --ref <name> --head <oid>) \
  [--revision <revision-id>] [--sign-key <name|path>] [--repo <path>]
shore association withdraw <association-id> --track <track-id> \
  [--revision <revision-id>] [--sign-key <name|path>] [--repo <path>]
shore association list [--revision <revision-id>] [--axis commit|ref] [--current] \
  [--repo <path>] [--pretty | --compact]
```

`shore association` records and withdraws the commit-graph associations of a captured
revision as append-only associate/withdraw events on two axes — commit and ref. This is how a
revision is tied to the commits and branches that carry it; recording a landed commit
(`record --commit`) is the association half of the ADR-0014 lifecycle
([ADR-0014](./adr/adr-0014-reviewunit-commit-range-lifecycle.md)).

- **`record`** takes exactly one axis. `--commit <rev>` (resolved to an OID) binds the revision on
  the commit axis; `--ref <name>` (a short branch name is normalized to its full ref) at the
  explicit `--head <oid>` (never inferred) binds it on the ref axis. `--commit` and `--ref` are
  mutually exclusive, and `--ref` requires `--head`.
- **`withdraw <association-id>`** retracts an earlier association by its id. The id is positional
  and must carry its prefix — `assoc-commit:…` or `assoc-ref:…` — because the prefix selects which
  axis is withdrawn; a prefixed short form like `assoc-commit:<hex-fragment>` resolves against the
  store. Withdrawal is terminal: a later re-association of the same target does not revive the
  withdrawn edge.
- Both writes are signable — `--sign-key <name|path>` selects the signing key exactly as
  `shore capture` does, and signing never gates the write.
- `--revision <revision-id>` pins the target revision; without it the command defaults to the single
  captured revision and errors if multiple captured revisions exist.
- **`list`** reports both axes unless `--axis commit|ref` narrows to one, and `--current` excludes
  withdrawn associations, showing only what currently holds. It emits `shore.review-association-list`
  JSON.
- The write forms emit `shore.review-association-commit`, `shore.review-association-commit-withdrawn`,
  `shore.review-association-ref`, and `shore.review-association-ref-withdrawn` JSON with the new
  association id and write counts.

Divergent or dangling associations surface as advisory diagnostics on the read surfaces
(`divergent_commit_association` when two or more distinct current commit OIDs claim one revision;
`retraction_target_missing` when a withdrawal names an association that never appeared); they are
render-only and never gate a write.

## `shore history`

```bash
shore history [--repo <path>] [--revision <id>] [--track <track-id>] \
  [--event-type <event-type>]... [--ref <name> [--by label|liveness]] \
  [--limit <n>] [--cursor <cursor>] [--watch [--poll-ms <ms>]] \
  [--include-body] [--pretty | --compact]
```

`shore history` reads the chronological ledger of durable Shoreline events.

- History replays the resolved store's `events/` and emits compact `shore.review-history` v1 JSON by
  default.
- `eventSetHash` and `eventCount` describe the full validated event set used to build the output,
  even when filters return only a subset of entries.
- `historyCount` is the number of returned entries after filters.
- Entries are sorted by `occurredAt`, then `eventId`, as display chronology.
- `--revision`, `--track`, and repeated `--event-type` narrow the returned entries.
- `--ref <name>` filters to events of revisions associated with a ref (a short branch name is
  normalized to its full ref). `--by` chooses how `--ref` matches: `label` (the recorded label,
  offline; the default) or `liveness` (reachability from the ref's live tip).
- `--limit <n>` returns at most N entries as a forward page (from the start, or from `--cursor`); the
  response carries a `nextCursor` to continue. `--cursor <cursor>` continues from a previous
  response's opaque `nextCursor`. Omit both for the full history.
- `--watch` re-renders whenever the store's liveness changes, polling client-side at `--poll-ms`
  (default 3000). It is pull-only — no daemon and no filesystem watch — and is cancelled with
  Ctrl-C; under `--watch` the same `--limit` page is re-rendered on each liveness change.
- Succession and commit/ref association filters include `revision-captured`,
  `revision-commit-associated`, `revision-commit-withdrawn`, `revision-ref-associated`, and
  `revision-ref-withdrawn`.
- Body-like text is omitted by default. `--include-body` hydrates observation bodies, input request
  bodies, input request response reasons, assessment summaries, validation summaries, and
  imported-note bodies. Native Markdown/plain content-type fields are included for those body-like
  event fields when they are not plain text.
- Duplicate semantic events remain visible as separate entries while shared duplicate diagnostics
  are included in the document.

### Verification status and endorsement readback

`shore history`, `shore revision show`, and the inspector endpoints render two
reader-relative, **advisory** facts beside each event. They render only — they never gate a write or
change an exit code, and the temporal `require-verified-endorsement` tier is out of scope.

- `verificationStatus` ∈ `valid | invalid | untrusted_key | unsigned` — the per-event signature
  ladder, resolved against the **reader's** `.shore/allowed-signers.json`. An event signed by a key
  the reader has not enrolled reads `untrusted_key`; an event with no signature reads `unsigned`.
- `endorsements[]` — for an endorsed (co-signed) target event, one entry per endorsement
  attestation (co-signature member). Because signatures are deterministic, one signer yields one
  attestation per target, so this is normally one entry per endorsing signer; an actor who endorses
  the same target with more than one enrolled key surfaces one entry per key (each is a distinct
  attestation, not collapsed):
  - `classification` ∈ `endorsement-trusted | unknown_endorser | ambiguous_endorser`.
  - `endorser` — the resolved actor, present only when `endorsement-trusted`.
  - `endorserAttributes` — the endorser's attested `kind`/`roles` from
    `.shore/actor-attributes.json`. This is a **sibling enrichment** rendered beside the
    classification; it is **not** an input to how the classification is decided.

Both are **reader-relative**: the same endorsement carrier may read `endorsement-trusted` for a
reader who has enrolled the endorser and `unknown_endorser` for a reader who has not. A field is
omitted when empty — no verification policy configured, no endorsement on the target, or no attested
attributes for the endorser. The classification rules are decided in
[ADR-0013](./adr/adr-0013-endorsement-record-and-classification.md).

```json
{
  "eventId": "evt:sha256:…",
  "eventType": "work_object_proposed",
  "verificationStatus": "unsigned",
  "endorsements": [
    {
      "classification": "endorsement-trusted",
      "endorser": "actor:git-email:kevin@swiber.dev",
      "endorserAttributes": { "kind": "human", "roles": ["reviewer"] }
    },
    { "classification": "unknown_endorser" }
  ]
}
```

History is not the full revision row projection. Use `shore revision show` for the composite
narrative-first plus snapshot-complete view of one captured revision.

## `shore revision list`

```bash
shore revision list [--repo <path>] [--object <object-id>] [--ref <name> [--by label|liveness]] \
  [--integration-ref <name>] [--worktree <path>] [--all | --orphans] [--pretty | --compact]
```

`shore revision list` is the discovery surface for captured revisions. It emits
`shore.review-revision-list` JSON with `eventSetHash`, `eventCount`, `revisionCount`, and entries
sorted by capture time. Each entry carries the revision id, the content-only object id, the capture
endpoints, and `objectArtifactContentHash`.

- `--object <object-id>` lists only the revisions that share one content object. Coincident content
  may span supersession threads, so this is a listing/grouping lens, never a head selector.
- `--ref <name>` filters to revisions associated with a ref (a short branch name is normalized to
  its full ref). `--by` chooses how `--ref` matches: `label` (the recorded label, offline; the
  default) or `liveness` (reachability from the ref's live tip). The succession view (the
  supersession DAG and a thread's competing heads) is reported by this same projection; there is no
  separate lineage surface.
- `--integration-ref <name>` sets the reachability target for the `merged` status: a revision is
  `merged` only when it is an ancestor of this ref. It defaults to broad reachability (any live tip).
- `--worktree <path>` scopes the listing to captures belonging to the worktree at that path.
- `--all` / `--orphans` control whether revisions whose anchored commits are all unreachable
  (orphaned) are shown or shown exclusively; by default orphaned revisions are hidden.

## `shore revision show`

```bash
shore revision show [REVISION] [--repo <path>] [--track <track-id>] \
  [--include-body] [--pretty | --compact]
```

`shore revision show` is the composite view for one revision. It emits compact
`shore.review-revision` v2 JSON by default.

- When exactly one revision has been captured, Shoreline selects it automatically.
- If multiple revisions exist, pass the `[REVISION]` positional. It is a **head seed**: a current
  head resolves exactly; a superseded revision resolves its thread's current head; and a thread with
  competing heads is reported as competing rather than auto-picked.
- The output includes revision identity, event-set freshness metadata, filters, summary counts,
  current assessment status, native observations, input requests, assessments, validation checks,
  projection rows, and diagnostics.
- Rows are narrative-first, then snapshot-complete.
- `--track <track-id>` filters narrative facts without changing the selected revision,
  event-set freshness metadata, or captured snapshot completeness.
- Body-like text is omitted by default. `--include-body` hydrates observation bodies, input request
  bodies and response reasons, assessment summaries, validation summaries, and imported-note bodies.
  Native Markdown/plain content-type fields are included for those body-like event fields when they
  are not plain text.
- Each narrative member (observations, input requests and their responses, assessments, validation
  checks) and the `revision` identity (the capture event) carry the same reader-relative
  `verificationStatus` and `endorsements` (with `endorserAttributes`) readback documented under
  [`shore history`](#verification-status-and-endorsement-readback) — advisory, render-only,
  resolved against the reader's `.shore/` trust and attributes config.

Revision-scoped selection seeds on the `[REVISION]` positional and resolves that revision's thread
head; no implicit newest capture globally wins. Unscoped current selection with multiple unrelated captured
revisions still errors at the selection boundary, but routine list, history, and exact-revision reads
have no always-on ambiguous-current warning. A thread-level read may surface
`stale_by_superseding_revision` for a revision that a newer revision supersedes. This release has no
interdiff or stack DAG beyond the supersession graph.

Capture and succession facts stay signable under ADR-0004's generic `EventToBeSigned` contract with
the Dead Simple Signing Envelope (DSSE) and pre-authentication encoding rules.

`shore revision show` is distinct from `shore history`: history is the chronological raw
event listing, while `shore revision show` is the composite revision view for agents and future
frontends.
