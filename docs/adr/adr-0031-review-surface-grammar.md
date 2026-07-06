# ADR-0031: Review-Surface Grammar — the Review Family Flattens, One Verb/Noun Rule, Short-Id Input

**Status:** Accepted (owner-approved 2026-07-05); landed 2026-07-06.
**Date:** 2026-07-05
**See also:** **ADR-0029** (output-mode convention — lanes, selector, `SHORE_FORMAT`, the
frozen machine envelope, and the display-truncation rules all stand; its Decision 9 clause
rejecting truncated ids **as input** is superseded by Decision 4 here), **ADR-0030** (named
command surface — Decision 1 is the subject rule this ADR completes; Decisions 3/6 carry the
legacy-surfaces-hidden Amendment), **ADR-0028** (opaque id prefixes — the prefix registry,
opacity, and display rules stand; abbreviated id **input** is newly decided here, Decision 4),
**ADR-0010** (delegation records — renamed surface verb here), **ADR-0016** (remove/compact
vocabulary — kept verbatim). Wave-0 hygiene issues: #377, #378, #379, #380.

## Context

ADR-0030 Decision 1 established the surface's subject rule: a bare top-level `shore <verb>` is
about the captured review record, and commands about other subjects live under family nouns.
The 10-subcommand `review` family (`src/cli/review/mod.rs`) predates that rule, and under it
the family noun is redundant: capture, observations, assessments, validation, input requests,
history, and the revision listing *are* the review record — the one subject the bare verbs
already name. The redundancy is not free: it produces the surface's worst reading
(`shore review revisions` — the subject named twice, then a bare plural), and it puts three
extra levels between a user and the most-used commands in the product.

A full 38-leaf enumeration (live-verified against the compiled binary) found five grammar
inconsistencies, all archaeology rather than design: `review revisions` is the only verb-less
pluralized listing (`src/cli/review/revisions.rs`); `association` is the only family with
verb-first hyphenated compounds (`associate-commit`, `withdraw-ref`, …,
`src/cli/review/association.rs`); `review input-request fetch` is the only get-one verb not
named `show` (`src/cli/review/input_request.rs`); `identity enroll` and `keys enroll` reuse
one verb for two materially different trust operations (`src/cli/identity/enroll.rs`,
`src/cli/keys/enroll.rs`); and `keys` and `notes` are the only pluralized family nouns. This
ADR normalizes only `keys` — the `notes` family is hidden pending the ADR-0030 Amendment's
design pass, which owns its fate (including its name).

Two measurements bound the change. Every stdout consumer is first-party — the bundled skills,
docs, and tests; CI never invokes the binary by argv at all — and the tests funnel through
shared harness helpers, so a rename sweep is mechanical. And the repo's own store shows the
write surface is ~96% agent-authored (observation and validation records alone are 78% of
2,680 events), so write-command renames cost skill and test edits, not human retraining; the
human-ergonomics budget belongs on the read side. The machine contract is
invocation-independent: the `shore.*` schema discriminators, the consumed field-paths, and the
wire enum vocabularies ride library serde (`src/cli/json.rs`, the documents builders) and are
frozen per ADR-0029 — command renames cannot touch them.

Finally, an input/display asymmetry: the inspector display-truncates ids (`shortRef` —
`rev:1ace028b` — and the 12-char `shortId`, `src/cli/inspect/web/src/refs.ts`), and ADR-0029's
text lane inherits that display rule, but no surface accepts a truncated form back. The
product shows ids that nothing will take as input.

## Decision

### 1. The review family flattens to the top level

Every `shore review <x>` becomes `shore <x>`. ADR-0030 Decision 1 is restated in its completed
form: **a bare verb or bare noun family at the top level reads or writes the captured review
record; commands about any other subject remain family-scoped nouns** — `store`, `key`,
`identity`, and the hidden `notes` family (per the ADR-0030 Amendment). The `review` noun
retires from the tree with the standard removed-command hints; `diff` and `inspect` already
live at the top level under the same rule.

### 2. The grammar rule and the renames

**Workflow moves are bare verbs; everything else is `<singular-noun> <verb>`, with `list` =
many and `show` = one.** Ids whose object is the command's subject are positional;
`--revision` / `--track` style selectors remain flags.

Bare verbs (unchanged): `capture`, `diff`, `history`, `endorse`, `inspect`.

Renames (each retired path gets a removed-command hint and a `cli_removed_legacy.rs` guard):

| Before | After |
|---|---|
| `review revisions` | `revision list` |
| `review show` | `revision show [REVISION]` |
| `review input-request fetch <ID>` | `input-request show <ID>` |
| `review association associate-commit` / `associate-ref` | `association record --commit <oid>` \| `--ref <name> --head <oid>` (one command, exclusive flag groups per the `store remove` precedent) |
| `review association withdraw-commit` / `withdraw-ref` | `association withdraw <ASSOCIATION_ID>` |
| `identity enroll` | `identity delegate <AGENT> --principal <P>` |
| `keys …` | `key …` (subverbs unchanged: `init`, `list`, `show`, `use-ssh`, `enroll`) |
| `review observation/assessment/validation/input-request/association …` | same families, one level up |

Unchanged on purpose: `observation add|list`, `assessment add|show`, `validation add|list`,
`input-request open|list|respond` (lifecycle verbs earn their names), the `store` family
(ADR-0016's remove-claims/compact-erases vocabulary kept; help-text clarification only, #380),
`endorse` (a cross-cutting countersign; bare verb is correct), `history` (not `log` — avoids
both the `--log` tracing family and git's commit-log muscle memory). Bare top-level `show`
remains unassigned, preserving ADR-0030 Decision 3's posture — the revision digest lives at
`revision show`.

The resulting tree:

```
shore
├── capture · diff · history · endorse · inspect
├── revision      list | show [REVISION]
├── observation   add | list
├── assessment    add | show
├── validation    add | list
├── input-request open | list | show <ID> | respond <ID>
├── association   record | withdraw <ID> | list
├── store         status | mode | migrate | remove | compact | gc
├── key           init | list | show | use-ssh | enroll
├── identity      delegate | attest
└── notes         apply                (hidden — ADR-0030 Amendment; the deferred design
                                        pass owns the TUI's fate, including any `notes show`)
```

The hidden legacy surfaces (`show` — the TUI, `dump`, `notes apply`) remain functional but
unadvertised per the ADR-0030 Amendment; this ADR creates no new command in that family.

### 3. Migration is hints-only, in one window

The flatten and renames land **once**, together with the full first-party sweep (skills, docs,
tests, the reference-coverage guard). Every retired path gets the standard removed-command
hint; no hidden `review` compat alias ships (a zombie namespace that would preserve two
spellings of everything indefinitely). External first-party automation updates in lockstep.
The machine documents do not change shape, so no document `version` bumps ride this change.

### 4. Short-id input: id-taking arguments accept abbreviations, resolved unique-or-error

Today every id must be passed in full (`rev:sha256:<64-hex>`). That asymmetry — the product
displays short ids no surface will accept — ends here (owner decision, 2026-07-04). Every
id-taking argument (positional ids and the id flags: `--revision`, `--observation`,
`--input-request`, `--target-assessment`, `--supersedes`, `--replaces`, `--responds-to`,
`--withdraws`, the `endorse` target, …) accepts three input forms:

1. **Full id** — `rev:sha256:<64-hex>`; the machine form, always valid.
2. **Prefixed short id** — kind prefix + leading hex fragment, with the digest tag optional:
   `rev:40c47f97` and `rev:sha256:40c47f97` are equivalent.
3. **Bare fragment** — `40c47f97`, accepted **only** where the argument implies exactly one
   id kind (`revision show <ID>` ⇒ `rev:`, `input-request show <ID>` ⇒ input-request ids,
   `--revision` ⇒ `rev:`, …). An argument that can legitimately take more than one kind
   requires the prefixed form.

Resolution rules (git's unique-prefix model, with a fixed floor):

- A fragment is lowercase hex and matches by **digest prefix** (never substring). Resolution
  scans the id space of the implied kind in the resolved store: exactly one match resolves;
  zero is a not-found error; more than one is a **hard error listing the full candidate ids**
  — never auto-picked, per the ambiguity-preservation discipline. Growth can therefore only
  turn a once-valid fragment into an ambiguity error asking for more characters; it can never
  silently resolve to a different object.
- **Minimum fragment: 4 hex characters** (git's own floor), rejected as too-short below that
  even when unique. *(Revised 8 → 4, owner decision 2026-07-05.)* The display round-trip only
  requires floor ≤ display — the inspector's forms stay 8/12 (ADR-0029's display rules
  untouched), so any id the product displays remains re-enterable verbatim — and every safety
  property (prefix-only, kind-scoped, unique-or-error, full-id storage) is floor-independent:
  a low floor can only make a loud "type more characters" retry more likely, never a wrong
  resolution. Short fragments are what humans remember and type; in per-repo stores the
  ambiguity odds stay small (≈2% per 4-hex fragment against ~1,200 ids of one kind in this
  repo's own heavily-used store).
- One shared resolver at the CLI argument layer — a single decision point, like the shared
  format seam — so every id-taking argument behaves identically.
- The machine lane is untouched **on output**: documents emit full ids only. On input,
  abbreviation is available to **every caller — human or agent** (owner decision 2026-07-04:
  no full-id mandate for agents; because ambiguity fails loud and never silently
  mis-resolves, short input is as safe for automation as for people, and it cuts noise).
  The invariant that matters lives one layer down: any **recorded fact** that references
  another object (`--supersedes`, `--responds-to`, `--replaces`, `--withdraws`, anchor and
  target ids) stores the **resolved full id** — resolution happens at the argument boundary,
  never in stored content. This decision **supersedes the input-side clause of ADR-0029
  Decision 9** ("a truncated form must never … be accepted back as an argument") and narrows
  ADR-0028's pass-ids-back-verbatim consumer guidance to the output side; ADR-0029's
  display-truncation rules stand unchanged.
- The bare-fragment form depends on this ADR's positional-id grammar (Decision 2) and lands
  with it; the prefixed form has no such dependency.

### 5. Contract invariants — what this ADR must not touch

The `shore.*` schema discriminators (including internal wire event-type names), the hard-core
consumed field-paths, the assessment/outcome enum vocabularies, the `SHORE_*` env family, the
`.shore/` dotdir, and the `~/.shore` keystore are all invariant under this reshape. They are
wire and storage identity, not argv.

## Consequences

### Accepted

- **The subject rule finishes its work:** the most-used commands sit at the top level, the
  family noun stops double-naming the product's one subject, and `shore review revisions`
  becomes `shore revision list`.
- **Every catalogued grammar inconsistency is resolved by one rule.**
- **Callers type what they see — human or agent:** any displayed short id (`rev:40c47f97`, a
  12-char `shortId` tail) is valid input, with wrong-resolution structurally impossible
  (unique-or-error); stored references stay full-id by construction.
- **Accepted cost:** a large one-time mechanical sweep of first-party skills/docs/tests;
  retired spellings answered by hints rather than aliases; a store-scan resolver on id-taking
  arguments (bounded by per-repo store size).

### Rejected

- **A hidden `review` compat alias:** two permanent spellings of every command is the sprawl
  this reshape exists to remove; hints are the project's proven mechanism.
- **Verb-first write commands** (`observe`, `assess`, `validate`): reads well in prose but
  splits each fact family across a verb (write) and a noun (read), doubling the top-level name
  count for no read-side gain.
- **Keeping `revisions` as a bare plural:** perpetuates the tree's one verb-less listing —
  the inconsistency the reshape is for.
- **Renaming wire/schema/env identifiers alongside the surface:** frozen contract (ADR-0029);
  argv-only change by design.
- **Substring or fuzzy id matching, and auto-picking among ambiguous candidates:** prefix
  match with a hard ambiguity error is the only resolution that stays deterministic as the
  store grows; anything looser can silently change which object an old command line names.
- **A git-style auto-scaling minimum:** a moving floor makes yesterday's valid invocation
  invalid tomorrow for no user-visible reason; the floor is a fixed constant (4, git's own
  minimum) that only an explicit decision moves.

## Revisit Triggers

- **The 4-hex floor** — if per-repo stores grow to where short-fragment ambiguity errors
  become routine, raise the floor (keeping display ≥ input floor so shown ids stay
  re-enterable).
- **The hidden `notes` family and the TUI** — the ADR-0030 Amendment's design pass decides
  their fate; this ADR only positions them (hidden, family-scoped).
- **Family-noun placement** (`store`, `key`, `identity`) — revisit only if a subject appears
  that is genuinely not the review record's (none exists today).
