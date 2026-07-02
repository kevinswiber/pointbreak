# CLI Reference

This reference covers the public `shore` command surface provided by the `shoreline` crate.

Command output JSON is the integration surface. Raw event files, artifact paths, event filenames,
and the store's `state.json` are internal storage details unless a command explicitly returns them.

## Global Tracing Flags

Most commands accept optional tracing flags:

```bash
--log <filter>
--log-format <compact|pretty|json>
--log-file <path>
```

Tracing writes to stderr by default. When stdout is piped into JSON tools, prefer
`--log-file <path>` so trace lines do not corrupt the JSON stream. `shore show` requires
`--log-file` when tracing is enabled because it runs a raw-mode TUI.

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
list and show commands, `review show`, and the inspector) discover a checked-in delegation map at
`<repo>/.shore/delegates.json` and resolve the human principal an agent wrote on behalf of,
rendering it beside the writer as `claude-code (for kevin@swiber.dev)`. Discovery is presence-based
— absent file, no change. A malformed `.shore/delegates.json` prints a single warning to stderr and
the read proceeds with no resolution (advisory, never blocking). The file format is documented in
[storage-model.md](./storage-model.md).

The **write** side of this config — creating a delegation record or describing an actor's kind/roles —
is the `shore identity` command group (`shore identity enroll` / `shore identity attest`, below).

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
The sole exception is `shore review endorse` (below), where unsigned is a hard error because the
signature *is* the endorsement's content. See
[signing-ux.md](./signing-ux.md) for the human / agent / CI flows and the
`unsigned → untrusted_key → valid` ladder.

## `shore show`

```bash
shore show [--repo <path>] [--review-notes <path>]
```

`shore show` opens a read-only terminal review view over the same headless review stream used by
`shore dump`.

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--review-notes <path>` loads Shoreline-native `review-notes.json`.
- Without an explicit sidecar, repo-only `shore show` auto-loads durable imported notes from the
  worktree's resolved store when one exists.
- Press `r` to reload the working-tree projection without losing cursor position when possible.
- Stale and orphan review notes appear as dedicated rows.
- Explicit sidecar inputs and `--log-file` are command helpers and are excluded from the reviewed
  snapshot for that command.
- The view is read-only; it does not mutate notes or write session state.

Keybindings: `q`/Esc/Ctrl+C quits, `j`/`k` or Up/Down moves by row, `[` and `]` move through diff
sections, and `{` and `}` move through noted sections.

## `shore dump`

```bash
shore dump [--repo <path>] [--review-notes <path>] [--pretty | --compact]
```

`shore dump` emits the JSON contract over the headless review stream.

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--review-notes <path>` loads Shoreline-native `review-notes.json`.
- Without an explicit sidecar, repo-only `shore dump` auto-loads durable imported notes from the
  worktree's resolved store when one exists.
- Output is compact by default; use `--pretty` for human-readable formatting.
- Recoverable review-note diagnostics stay in the JSON document and the command exits
  successfully.
- Fatal errors, such as unreadable files or malformed JSON, are written to stderr and exit
  non-zero.

Each `stream.rows[*].kind` value is the externally tagged JSON representation of the row-kind enum:
a single-key object such as `file_header`, `hunk_header`, `diff`, `metadata`, `note`,
`stale_note`, or `empty_state`.

Row IDs are opaque strings. They are stable and unique within one built review stream and safe as
keys, but callers must not parse them, derive ordering from them, or rely on their prefix/width. Use
the sibling `ordinal` field for numeric position.

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
the review facts anchored to each line. Validation checks recorded with `shore review validation add`
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
validated projections as `shore review history` and `shore review show` rather than parsing raw
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
`targetDisplay` block), not from the snapshot artifact. `shore review revisions` JSON still carries
`target.worktreeRoot`, unchanged.

## `shore review capture`

```bash
shore review capture [--repo <path>] [--base <rev> [--target <rev>]] \
  [--supersedes <revision-id>]...
```

`shore review capture` records the current V1 revision: the base endpoint, target endpoint, and
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
  layout is upgraded with `just migrate-store [<repo>]` (an owner-run one-off driver, not a `shore`
  subcommand) — distinct from `shore store migrate`, which folds a pre-flip worktree-local
  `.shore/data/` store into the shared store; see
  [storage-model.md](./storage-model.md#migrations-and-doctor).
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

V1 storage is local and synchronous. The shared common-dir store may take concurrent writes from
multiple worktrees of the same clone, kept safe by content-addressed writes and a regenerable
projection rather than a lock. V1 does not add a daemon, delivery queue, approval flow, async
storage, remote storage, or note mutation.

By default every worktree of a clone resolves the shared common-dir store at `.git/shore`, so
`shore review capture` lands its capture directly there — no setup step — and the capture is
immediately visible to `review revisions`, `review show`, and `history` from any sibling worktree. An
`ephemeral` worktree instead captures into its own discardable `.shore/data/` store (see
[`shore store`](#shore-store)).

The native review write commands — `shore review observation add`, `shore review input-request open`
and `respond`, `shore review assessment add`, and `shore review validation add` — behave the same
way. They resolve the shared common-dir store, so you can record a
fact against a revision (or related observation, assessment, or request) captured in a sibling
worktree, and the fact lands directly in that shared store, visible to every worktree in place. An
`ephemeral` worktree writes the fact to its own `.shore/data/` store, unchanged.

## `shore store`

```bash
shore store status [--repo <path>] [--pretty]
shore store mode (shared | ephemeral | show) [--repo <path>] [--pretty]
shore store migrate [--repo <path>] [--include-ephemeral] [--retire-source] [--pretty]
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

Every review read command resolves the shared common-dir store: `review revisions` and `review show`,
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
(This is distinct from the owner-run flat-store driver `just migrate-store`, which relocates a
legacy flat `.shore/` store to `.shore/data/`; see
[storage-model.md](./storage-model.md#migrations-and-doctor).)

`shore store remove` retires content-addressed artifacts from the store. It resolves exactly one
selector to a set of content hashes — `--snapshot <id>` (a snapshot's bound artifact), `--revision
<id>` (every artifact a revision references), `--ref <name>` / `--range <a>..<b>` (artifacts of
revisions anchored on the named commit or commit range), or `--orphans` (artifacts of commit-anchored
revisions whose commits are all unreachable) — and records one removal fact per content hash. It emits
`shore.store-remove` JSON listing each `contentHash`, whether it was newly `created`, and
`coReferencingUnits` (other revisions that still name the same shared artifact, reported before the
removal), plus `eventsCreated` and `eventsExisting`. Removal is content-targeted and idempotent:
re-removing a hash reports `created: false`. Removal is a write, so a signed store stays signed —
`--sign-key` selects the signing key exactly as `shore review capture` does. There is deliberately no
`--idempotency-key`; the removal key is derived solely from the content hash. Removal records the
fact; it does not delete bytes — run `shore store gc` / `shore store compact` to reclaim them.

`shore store gc` and `shore store compact` are the same local sweep: they physically delete the
content-addressed blobs whose content hash has been removed, reclaiming disk. The sweep records no
event and is fully re-derivable from the log — re-capturing the same content re-materializes the blob.
It emits `shore.store-compact` JSON listing each swept blob's `contentHash` and `outcome`
(`removed` or `missing`) plus `bytesReclaimed`. Running it again is a no-op (every removed blob is
already `missing`).

Command output is the stable integration surface. Raw store paths, event files, artifact paths,
`.git` paths, `.shore/data` paths, and `state.json` remain internal storage details.

## `shore identity`

```bash
# Stage a delegation record binding an agent to its responsible principal (shore.identity-enroll).
shore identity enroll <agent-actor-id> --principal <principal-actor-id> \
  [--from <RFC3339>] [--until <RFC3339>] [--comment <text>] [--local] [--repo .] [--pretty]

# Stage an actor-attributes entry — kind + roles — for any actor (shore.identity-attest).
shore identity attest <actor-id> --kind <kind> [--role <role>]... \
  [--comment <text>] [--local] [--repo .] [--pretty]
```

`shore identity` writes the actor/principal config the read side (above) resolves. Both subcommands are
possession-style: they stage the working-tree edit only and never invoke git — review and commit the
file to apply it (`git log -p` is the audit trail), exactly like `shore keys enroll`.

- **`enroll`** stages a delegation record into `.shore/delegates.json` binding `<agent-actor-id>` (an
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

## `shore keys`

Manage the user-level signing keystore and stage signer enrollment. Keys live in `~/.shore/keys/`
(honoring `$XDG_DATA_HOME` on Unix and `%APPDATA%\shore` on Windows; `SHORE_HOME` overrides) — never
in the repo `.shore/` or the store. See [storage-model.md](./storage-model.md) for the key home and
allowed-signers format.

```bash
# Generate a human signing key and print its did:key (shore.keys-init).
shore keys init --name default

# List local keys with enrollment status and which is the default (shore.keys-list).
shore keys list --repo .

# Print a key's did:key and/or raw public key (shore.keys-show).
shore keys show default --did
shore keys show default --pubkey

# Adopt an existing SSH Ed25519 key as an agent-backed signer (shore.keys-use-ssh).
# Reuses ssh-agent custody — no new key material. Parallel to `init`.
shore keys use-ssh ~/.ssh/id_ed25519.pub --name default
shore keys use-ssh 'key::ssh-ed25519 AAAA…'   # git user.signingKey literal form

# Stage an allow-list entry binding a key's did:key to an actor (shore.keys-enroll).
# Possession-style: this stages the working-tree .shore/allowed-signers.json edit only;
# review and commit it to authorize the binding.
shore keys enroll default --actor actor:agent:claude-code --repo .
```

`init` refuses to overwrite an existing named key. `use-ssh` adopts an existing SSH **public** key as an
agent-backed `default` signer: it accepts a `*.pub` path or a `key::ssh-ed25519 AAAA…` literal, emits a
`shore.keys-use-ssh` document with the derived `did:key` (the same `.didKey` field `shore keys show
--did` prints) plus an enrollment hint, and (like `init`) refuses to overwrite. Only plain `ssh-ed25519`
keys are accepted; `ed25519-sk`/RSA/ECDSA are rejected with a clear error pointing at `shore keys init`.
`list`/`enroll` take `--repo` (default `.`) to resolve the committed `.shore/allowed-signers.json`;
every subcommand accepts `--pretty`. Enrollment never commits — the human's commit is the authorization.

Each **write** subcommand (`review capture`, `review observation add`, `review assessment add`,
`review validation add`, `review input-request open`/`respond`) accepts `--sign-key <name|path>` to
sign that write with a specific key (highest precedence; overrides `SHORE_SIGNING_KEY`). A key that
cannot be loaded leaves the write unsigned at exit 0 with an advisory diagnostic — signing never
blocks. An agent-backed key resolves through an identities-only ssh-agent pre-flight; if the agent is
unavailable (`signing_agent_unavailable`), does not hold the key (`signing_agent_key_absent`), or fails
the real sign (`signing_agent_sign_failed`), the write is left unsigned at exit 0 — see
[signing-ux.md](./signing-ux.md) for the full never-gates table. This never-gates behavior covers the
ordinary signed review writes listed above; `shore review endorse` is the exception, where an
unresolved signer is a hard error rather than an unsigned write. Only shipped subcommands are listed;
`rotate` and `revoke` are named follow-ons, not yet available.

## `shore review observation`

```bash
shore review observation add --track <track-id> --title <title> \
  [--revision <revision-id>] [target options] [--body-content-type text/plain|text/markdown]
shore review observation list [--revision <revision-id>] [--track <track-id>] \
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
- `--supersedes <observation-id>` records a correction by appending a new observation that names the
  older observation.
- `observation list` replays durable events for the revision and may filter by revision, track,
  file, or tag. It hydrates body text only with `--include-body`.

Output is compact `shore.review-observation-add` or `shore.review-observation-list` JSON by
default. `observation list` also accepts `--pretty` and `--compact`.

## `shore review input-request`

```bash
shore review input-request open --track <track-id> --title <title> --reason <reason> \
  [--revision <revision-id>] [--mode operative|advisory] \
  [--body-content-type text/plain|text/markdown]
shore review input-request list [--revision <revision-id>] [--track <track-id>] \
  [--mode operative|advisory] [--file <path>] [--status open|responded|ambiguous|all] \
  [--include-body] [--pretty|--compact]
shore review input-request fetch <input-request-id> [--include-body]
shore review input-request respond <input-request-id> --outcome <outcome> [reason options] \
  [--reason-content-type text/plain|text/markdown]
```

Input requests are durable pause or decision requests for a captured revision.

- `input-request open` requires `--track`, `--title`, and `--reason`.
- `--revision` pins the request to one captured revision. Without either, the command defaults to the single captured revision and errors if
  multiple captured revisions exist.
- `--mode` defaults to `operative`; `advisory` requests are durable and visible but do not imply a
  cooperative client must pause.
- Targets mirror observations: review-wide by default, captured file, captured range, or an
  existing native observation through `--observation <observation-id>`.
- Request bodies may come from `--body`, `--body-file`, or `--body-stdin`.
- `--body-content-type` defaults to `text/plain`; use `text/markdown` when the request body should
  render as Markdown in the inspector.
- Large request bodies reuse Shoreline-owned `shore.note-body` artifacts while command output keeps
  artifact paths private.
- `input-request list` is the V1 polling read surface and defaults to open requests. It may filter
  by revision, track, mode, file, or status, and hydrates body text only with `--include-body`.
- `input-request fetch <id> --include-body` returns one request and hydrates the body when
  requested.
- `input-request respond <id>` appends an `input_request_responded` event.
- Response reasons may use `--reason-content-type text/markdown`; the default is `text/plain`.
- Response outcomes are `approved`, `rejected`, `dismissed`, `superseded`, and `abandoned`.

Output documents are compact `shore.review-input-request-open`,
`shore.review-input-request-list`, `shore.review-input-request-fetch`, and
`shore.review-input-request-respond` JSON by default. Read commands also accept `--pretty` and
`--compact`.

V1 is durable and polling-friendly. It does not add a daemon, filesystem watch mode, TUI prompt,
notification transport, or cancellation/escalation event.

## `shore review assessment`

```bash
shore review assessment add --track <track-id> --assessment <assessment> \
  [--revision <revision-id>] [target options] [--summary-content-type text/plain|text/markdown]
shore review assessment show [--revision <revision-id>] [--all] [--track <track-id>] \
  [--include-summary] [--pretty|--compact]
```

Assessments record review calls for a captured revision.

- `assessment add` requires `--track` and `--assessment`.
- `--revision` pins the assessment to one captured revision. Without either, the command defaults to the single captured revision and errors if
  multiple captured revisions exist.
- CLI input uses `kebab-case` assessment values: `accepted`, `accepted-with-follow-up`,
  `needs-changes`, and `needs-clarification`. Command JSON output uses the matching `snake_case`
  values: `accepted`, `accepted_with_follow_up`, `needs_changes`, and `needs_clarification`.
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

## `shore review validation`

```bash
shore review validation add --track <track-id> --check-name <name> --status <status> \
  [--revision <revision-id>] [validation options] [--summary-content-type text/plain|text/markdown]
shore review validation list [--revision <revision-id>] \
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

## `shore review endorse`

```bash
shore review endorse <target-event-id> [--sign-key <name|path>] [--actor <id>] [--repo .] [--pretty]
```

`shore review endorse` records a detached co-signature (an endorsement) over an existing target event —
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

## `shore review history`

```bash
shore review history [--repo <path>] [--revision <id>] [--track <track-id>] \
  [--event-type <event-type>]... [--include-body] [--pretty | --compact]
```

`shore review history` reads the chronological ledger of durable Shoreline events.

- History replays the resolved store's `events/` and emits compact `shore.review-history` v1 JSON by
  default.
- `eventSetHash` and `eventCount` describe the full validated event set used to build the output,
  even when filters return only a subset of entries.
- `historyCount` is the number of returned entries after filters.
- Entries are sorted by `occurredAt`, then `eventId`, as display chronology.
- `--revision`, `--track`, and repeated `--event-type` narrow the returned entries.
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

`shore review history`, `shore review show`, and the inspector endpoints render two
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

History is not the full revision row projection. Use `shore review show` for the composite
narrative-first plus snapshot-complete view of one captured revision.

## `shore review revisions`

```bash
shore review revisions [--repo <path>] [--object <object-id>] [--ref <name>] \
  [--all | --orphans] [--pretty | --compact]
```

`shore review revisions` is the discovery surface for captured revisions. It emits
`shore.review-revision-list` JSON with `eventSetHash`, `eventCount`, `revisionCount`, and entries
sorted by capture time. Each entry carries the revision id, the content-only object id, the capture
endpoints, and `objectArtifactContentHash`.

- `--object <object-id>` lists only the revisions that share one content object. Coincident content
  may span supersession threads, so this is a listing/grouping lens, never a head selector.
- `--ref <name>` filters to revisions associated with a ref. The succession view (the supersession
  DAG and a thread's competing heads) is reported by this same projection; there is no separate
  lineage surface.
- `--all` / `--orphans` control whether revisions whose anchored commits are all unreachable
  (orphaned) are shown or shown exclusively; by default orphaned revisions are hidden.

## `shore review show`

```bash
shore review show [--repo <path>] [--revision <id>] [--track <track-id>] \
  [--include-body] [--pretty | --compact]
```

`shore review show` is the composite view for one revision. It emits compact
`shore.review-revision` v1 JSON by default.

- When exactly one revision has been captured, Shoreline selects it automatically.
- If multiple revisions exist, pass `--revision <id>`. The flag is a **head seed**: a current head
  resolves exactly; a superseded revision resolves its thread's current head; and a thread with
  competing heads is reported as competing rather than auto-picked.
- The output includes revision identity, event-set freshness metadata, filters, summary counts,
  current assessment status, native observations, input requests, assessments, validation checks,
  imported adapter notes, projection rows, and diagnostics.
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
  [`shore review history`](#verification-status-and-endorsement-readback) — advisory, render-only,
  resolved against the reader's `.shore/` trust and attributes config.

Revision-scoped selection seeds on `--revision <id>` and resolves that revision's thread head; no
implicit newest capture globally wins. Unscoped current selection with multiple unrelated captured
revisions still errors at the selection boundary, but routine list, history, and exact-revision reads
have no always-on ambiguous-current warning. A thread-level read may surface
`stale_by_superseding_revision` for a revision that a newer revision supersedes. This release has no
interdiff or stack DAG beyond the supersession graph.

Capture and succession facts stay signable under ADR-0004's generic `EventToBeSigned` contract with
the Dead Simple Signing Envelope (DSSE) and pre-authentication encoding rules.

`shore review show` is distinct from `shore review history`: history is the chronological raw
event listing, while `shore review show` is the composite revision view for agents and future
frontends.

## `shore notes apply`

```bash
shore notes apply --repo <path> --review-notes <path>
```

`shore notes apply` imports review notes into Shoreline-owned durable state without publishing a
revision.

- `--repo <path>` defaults to `.` and may point at the repository root or a subdirectory inside it.
- `--review-notes <path>` is required.
- The command initializes the worktree's resolved store when needed, records one immutable durable
  event per imported note, and rebuilds the store's `state.json`.
- Native `review-notes.json` is an import/transport input, not the authoritative persisted
  Shoreline store.
- Large note bodies may be stored as content-addressed note-body artifacts under the store's
  `artifacts/notes/`; small note bodies remain inline in the imported-note event payload.
- Output is compact `shore.notes-apply` JSON with note counts, diagnostics, and `statePath`.
