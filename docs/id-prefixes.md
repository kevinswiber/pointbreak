# ID Prefixes

**This is an internal contributor document.** Consumers of Pointbreak's CLI output and JSON
documents must keep treating every id as an opaque string — see "IDs are opaque" in
[review-workflow.md](./review-workflow.md). Nothing here is a license to parse ids; it is the
convention Pointbreak's own code follows when it mints them.

## The convention

Ratified by [ADR-0028](./adr/adr-0028-id-prefix-convention.md):

- **Existing prefix strings are frozen as-is.** They feed content-derived ids, so changing one
  changes the ids newly minted for identical content and breaks cross-clone convergence with
  existing stores. The frozen-value test in `src/model/id_prefix.rs` is the tripwire.
- **New prefixes are spelled-out, hyphenated, lowercase domain names** (`[a-z][a-z-]*`, no
  trailing hyphen). The abbreviation set is closed: `evt`, `rev`, `obj`, `obs`, `assess` are
  grandfathered, not precedent (the legacy `snap` abbreviation was retired in #344).
- **`src/model/id_prefix.rs` is the live source of truth.** Production code mints through its
  constants; the `cfg(test)` table beneath them is the enumerable contract the registry and
  drift tests check. The table below is a readable snapshot — on any disagreement, the
  registry wins.

## Current prefixes

| prefix | kind | shape(s) | minted by | linkified |
|---|---|---|---|---|
| `evt` | content id | `evt:sha256:<hex>` | event envelope (`session/event`); re-derived by the event-store validation check | yes |
| `rev` | content id | `rev:sha256:<hex>`, `rev:worktree:sha256:<hex>` | capture fingerprinting (`session/store/fingerprint`) | yes (incl. `git:`/`worktree:` forms) |
| `obj` | content id | `obj:sha256:<hex>`, `obj:git:sha256:<hex>` | capture fingerprinting | yes (non-clickable) |
| `engagement` | content id | `engagement:sha256:<hex>` | capture fingerprinting; Claude Code adapter | yes (non-clickable) |
| `obs` | content id | `obs:sha256:<hex>` | observation workflow; Claude Code adapter | yes |
| `assess` | content id | `assess:sha256:<hex>` | assessment workflow | yes |
| `validation` | content id | `validation:sha256:<hex>` | validation workflow | yes (non-clickable) |
| `input-request` | content id | `input-request:sha256:<hex>` | input-request workflow | yes |
| `input-request-response` | content id | `input-request-response:sha256:<hex>` | input-request workflow | yes |
| `assoc-commit` | content id | `assoc-commit:sha256:<hex>` | association events (`session/event/association`) | yes (non-clickable) |
| `assoc-ref` | content id | `assoc-ref:sha256:<hex>` | association events | yes (non-clickable) |
| `withdraw-commit` | content id | `withdraw-commit:sha256:<hex>` | association events | yes (non-clickable) |
| `withdraw-ref` | content id | `withdraw-ref:sha256:<hex>` | association events | yes (non-clickable) |
| `task-attempt` | content id | `task-attempt:sha256:<hex>` | Claude Code adapter | yes (non-clickable) |
| `checkpoint` | content id | `checkpoint:sha256:<hex>` | Claude Code adapter | yes (non-clickable) |
| `note` | content id | `note:sha256:<hex>`; `note:<explicit_id>`; `note:<file_index>:<note_index>` | note import; sidecar resolution | yes (`note:sha256:` form only) |
| `journal` | structural | `journal:claude:<session_uuid>`; `journal:default` sentinel | Claude Code adapter; capture/removal/state defaults | no |
| `review` | structural | `review:default` sentinel only | capture default | no |
| `actor` | structural | `actor:git-email:<email>`, `actor:git-name:<name>`, `actor:local`, `actor:claude_code:user`, `actor:claude_code:assistant` | writer identity; Claude Code adapter | no |
| `row` | structural | `row:<zero-padded ordinal>` | review-stream and projection row builders | no |
| `object` | artifact ref | `object:sha256:<hex>` | export/inventory | no |
| `body` | artifact ref | `body:sha256:<hex>` | export | no |
| `note-body` | artifact ref | `note-body:sha256:<hex>` | export/inventory | no |
| `file` | artifact ref | `file:sha256:<hex>` (hash of a redacted relative path) | sensitivity scan | no |

"Linkified" means the inspector's reference regex turns the token into a chip; a `(non-clickable)`
note marks a chip that is styled with a tooltip but has no navigation route (it never renders a dead
link). The list is derived from `REF_ID_PREFIXES` in `src/cli/inspect/web/src/classNames.ts` and
mirrored by the registry's `linkified` flags under a drift test. Membership and the shape gaps were
resolved in [#344](https://github.com/withpointbreak/pointbreak/issues/344) (see the *Linkification
Membership Resolved* amendment in ADR-0028): the eight production-minted content ids above linkify as
non-clickable chips, the `rev:worktree:` shape now linkifies, and the legacy `review-unit`/`snap`
display entries were retired.

The legacy event-clock token `unix-ms:<millis>` is not an ID prefix and is no longer minted.
Readers continue to accept it for existing stores; new local `occurredAt` values use RFC 3339 UTC
with millisecond precision (`YYYY-MM-DDTHH:MM:SS.mmmZ`).

## Shape notes

- Infixes (`worktree:`, `git:`) are owned by the mint site, not the prefix: the registry
  freezes only the leading token.
- `actor:agent:<name>` and `actor:env:<…>` arrive from user configuration (`POINTBREAK_ACTOR_ID`)
  and are validated, never minted.
- `review:default` and `journal:default` are fixed sentinels — structural values with no
  digest payload. One consumer string-compares `journal:default`
  (`src/session/projection/state.rs`); it is protected by the frozen-value test.
- `FileId` is path-based and carries **no** prefix by design (distinct from the `file:`
  redaction ref). `HunkId` is likewise path-based; the two reserved sentinel values
  `hunk:stale` / `hunk:orphaned` (`src/stream/build.rs`) are const-declared and sit outside
  the registry — see ADR-0028's consequences.
- Event idempotency keys (`work_object_proposed:…`, `review_observation_recorded:…`) are a
  separate namespace derived from event-type names, not id prefixes, and are out of the
  registry's scope.

## Adding a prefix

1. Pick a spelled-out, hyphenated, lowercase domain name (`[a-z][a-z-]*`, no trailing hyphen).
   No new abbreviations.
2. Add the constant and its table entry in `src/model/id_prefix.rs`; extend the frozen-value
   test with the new constant. The uniqueness/charset/partition tests take care of themselves.
3. Mint through the constant — never an inline literal. (The adoption sweep in the registry's
   history shows the format!-swap patterns.)
4. Decide linkification. If yes: add the prefix to `REF_ID_PREFIXES` in
   `src/cli/inspect/web/src/classNames.ts` (mind overlap ordering — longer prefixes before
   their own prefixes), update the `refs.test.ts` alternation lock in the same edit, set
   `linkified: true` in the registry, and rebuild the bundle (`just web-build` +
   `just web-verify`). If no: `linkified: false` and the drift test stays green untouched.
5. Add a row to the table above.
