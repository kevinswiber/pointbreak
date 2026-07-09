# ADR-0029: CLI Output-Mode Convention — JSON-Default Machine Contract, Opt-in `--format text` Human Lane

**Status:** Accepted (owner-approved 2026-07-02); landed 2026-07-02 (grounding issue #96).
**Amended 2026-07-03 (owner): the default-lane flip is withdrawn — compact JSON stays the default
output; the text lane is opt-in via `--format text` / `SHORE_FORMAT=text`. Decisions 1, 6, and 8
(and the "default is text" clauses of 2 and 4) are superseded by the Amendment below; the
`--format` selector, the disposable-text/stable-json posture, and Decisions 5/7/9/10 stand as
landed. Read the Amendment first — the original Context/Decision text is retained as the historical
record.**
**Amended 2026-07-09 (owner): the pre-1.0 legacy `--pretty` and `--compact` aliases are removed.
Use `--format json-pretty` for indented JSON and `--format json` for an explicit compact machine
document.**
**Amended 2026-07-09 (owner): Decision 7's hard core admits ADDITIVE growth of the wire-value
vocabularies within a document's `version`. Appending a new value to the input-request `reasonCode`
set (or another additive-only vocabulary) does not bump `version`, because a soft-shell consumer
selects the values it knows and tolerates the rest; a value that is not on any consumed field-path
is only reachable by a consumer that already opted into reading it. Removing or renaming an existing
value is still a coordinated break that bumps that document's `version`. See the Amendment (Decision
7) block below.**
**Date:** 2026-07-02
**See also:** **ADR-0028** (id-prefix convention — the ids-are-opaque contract and the
display-truncation precedent this ADR extends to CLI text output), **ADR-0030** (named command
surface — the human-named commands that ride this convention), **ADR-0031** (review-surface
grammar — flattens the review family and supersedes Decision 9's input-side clause with
short-id input), the "Command output JSON" and "IDs
are opaque" sections of `docs/review-workflow.md` (the stability promise this ADR re-grades), and
`docs/cli-reference.md` (the per-command reference this convention governs). Grounding issue:
**#96** (human-readable readback). Held open, compatible either way: **shoreline-relay#11**
(fix-or-retire of the relay's non-default `cli-fallback` stdout parser).

## Amendment (2026-07-03): JSON stays the default; the text lane is opt-in

**Owner decision (2026-07-03): the default-lane flip is withdrawn. The compact-JSON machine document
remains the default stdout for every command; the human text lane is reached explicitly via
`--format text` (or `SHORE_FORMAT=text`).** This adopts the posture the original ADR recorded under
*Rejected → "Machine-default + explicit text opt-in"*: that option is now **Accepted**, and the flip
it was weighed against is **not pursued**.

**Why.** The flip's only benefit was cosmetic — a person who does not know `--format text` would get
the readable digest without asking. Its cost was structural: a one-time break of every first-party
stdout consumer (the bundled skills, the loop drivers, the relay `cli-fallback` builders), gated on
shoreline-relay#11 and a coordinated release (original Decision 8). Re-weighed after wave-1 shipped,
that coordination risk is not worth the saved keystroke: the digests already exist and are one flag
away, and keeping JSON the default is **zero-break** — no consumer migrates and no release must be
sequenced.

**Superseded** (retained above for the record; do not implement):
- **Decision 1** (adopt text-default; "breaking change … accepted") — withdrawn; there is no flip.
- **Decision 8** (the flip as one coordinated break, gated on first-party migration) — withdrawn.
  shoreline-relay#11 no longer gates this ADR; the relay keeps parsing `--format json` (today's
  default bytes) unchanged.
- **Decision 6** (the `hint: text-format output on stdout; pass --format json` stderr hint) —
  withdrawn; it only made sense once text was the default. A discoverability nudge in the *other*
  direction (hint a TTY human toward `--format text`) is a possible future affordance, not decided
  here.
- The **built-in default** in **Decision 2** is **`json`**, not `text`; and **Decision 4**'s "the
  default is `text` everywhere, piped or not" is withdrawn. The default is `json` everywhere.

**Stands, exactly as landed** (independent of which lane is default):
- **Decision 2's selector and precedence** — `--format <text|json|json-pretty>`, `SHORE_FORMAT`, and
  `--format` flag > `SHORE_FORMAT` > built-in default — ship as-is; only the built-in default value
  differs (`json`). The 2026-07-09 amendment removes the old boolean aliases now that `--format` is
  the single public selector.
- **Decision 3** — text is formally disposable, the machine lane is the stability surface, and
  un-digested commands fall back to indented JSON on the text lane — stands. (Its clause requiring
  the consumed commands to ship a bespoke text rendering "at the flip" is moot; those renderers
  shipped anyway in wave-1.)
- **Decisions 5** (color precedence), **7** (tiered hard-core / soft-shell stability), **9**
  (opaque-id display truncation), **10** (relay `cli-fallback` stays open), and **Decision 4's core
  rule** (isatty may govern color/pager/stderr, never data shape) — all stand unchanged.

**Net effect.** ADR-0029 now reads as: *one seam, three lanes, `json` the default and the contract,
`text` the opt-in human view, no boolean output aliases — no coordinated break.* Wave-1 (plan 0106, Phases 1–5) delivered the
`--format text` digests on seven commands and is the complete deliverable; the flip half of the plan
is retired. Reviving the flip would be a fresh decision (a new ADR or a superseding amendment) that
re-opens the Decision 8 migration gate.

## Amendment (2026-07-09, Decision 7): wire-value vocabularies grow additively within a `version`

**Owner decision (2026-07-09): Decision 7's hard core is refined — the wire-value vocabularies it
freezes (assessment values, input-request response outcomes, and the input-request `mode`/`reasonCode`
value sets) may gain ADDITIVE values within a document's `version`. Removing or renaming an existing
value remains a coordinated break that bumps `version`.**

**Why.** Freezing the *shape* of a hard-core field-path — that `reasonCode` is present, is a string,
and rides `inputRequests[].reasonCode` — is what a machine consumer depends on. The *set* of values
that field can take is open at the low end: a soft-shell consumer selects the values it recognizes
and tolerates the rest, so appending a value it has never seen cannot break it. A consumer that must
branch on a specific new value has, by definition, already opted into reading it. This mirrors serde
enum evolution: adding a variant is source-compatible for readers that match a subset. Bumping the
whole input-request document family to `version:2` for a zero-shape, append-only change was weighed
and rejected as a break with no consumer to protect.

**Consequence.** The first exercise of this rule is the `insufficient_evidence` input-request
`reasonCode` value, added under `version:1`. The `docs/cli-reference.md` hard-core wording follows
this amendment: hard core = the field-paths and value *shapes*; additive value growth is soft-shell.

## Context

Every document-emitting `shore` command — 37 leaf commands across 8 groups — prints exactly one
JSON document to stdout, single-line compact by default, through one seam (`write_json`,
`src/cli/json.rs:9`). Among the document-emitting commands, the sole non-JSON stdout is
`shore inspect`'s startup banner; the only other stdout in the CLI belongs to the `shore show`
TUI, which owns the terminal interactively (`src/tui/terminal.rs:39`) rather than emitting a
document. There is no TTY detection, no color, no pager, and no table rendering on document
stdout; `--format json-pretty` re-indents the same JSON and is eyeball-debugging, not a human view. The
deliberate human surfaces are the `shore inspect` web UI and the `shore show` TUI (which still
renders the pre-revision working-tree stream).

That leaves the terminal-native human jobs — capture readback, per-track review readback, store
hygiene, landing/association checks — served either by agent-shaped bounded lists or by the
composite `shore review show` document. Measured against a real 102-revision store, that composite
is 1.94 MB of compact JSON for an answer whose human-relevant narrative is 22 rows: issue #96
reproduced live. The fix cannot be "prettier default JSON" — a human view and a machine contract
are different artifacts and must be selected differently.

The actual machine contract is narrow and enumerable. The only stdout reads performed by a
non-human consumer today are:

- `shore review capture` → `.revision.id` (the author skill's `jq` extraction);
- `shore review input-request list` → `inputRequests[].{id,title,mode,reasonCode,trackId}` and
  `shore review input-request respond` → `.inputRequestResponseId`, `.eventId` (the relay's
  **non-default** `cli-fallback` feature; the relay's default path calls the `shoreline` library
  in-process and never touches stdout);
- two wire-value vocabularies that review-loop drivers branch on: the assessment values
  (`accepted`, `accepted_with_follow_up`, `needs_changes`, `needs_clarification` — snake_case on
  the wire; the kebab-case forms are CLI input/display labels, `src/session/event/assessment.rs`)
  and the input-request response outcomes (`approved`, `rejected`, `dismissed`, `superseded`,
  `abandoned`, `src/session/event/input_request.rs:26-32`).

Everything else is guarded only by re-blessable full-document byte snapshots
(`tests/review_document_contract.rs`), while `docs/review-workflow.md` and `docs/cli-reference.md`
declare the entire command-output JSON family "the stable integration surface" — a promise broader
than any real reliance, which freezes shape nobody needs frozen. Every one of those stdout
consumers is first-party (the bundled skills, the relay, the loop drivers, the test suite), which
is what makes a coordinated change of the *default* tractable: the parties that must migrate are
all in-house.

Prior art (git, gh, cargo, jj, kubectl — examined hands-on) converges on one iron rule: **one
output stream is a stability contract and one is explicitly disposable, selected by name so
neither can impersonate the other** (git's porcelain rule is the canonical statement — the
porcelain format is "guaranteed not to change in a backwards-incompatible way between Git
versions or based on user configuration" while the default output's "contents and format are
subject to change at any time", https://git-scm.com/docs/git-status; git even makes the
script-facing format config-immune). The mainstream default is human-first with machine opt-in
(cargo's `--message-format json`,
https://doc.rust-lang.org/cargo/reference/external-tools.html; jj's templated `json()` output,
https://jj-vcs.github.io/jj/latest/templates/; kubectl's `-o json|yaml|jsonpath|…`,
https://kubernetes.io/docs/reference/kubectl/#output-options). gh — the sharpest TTY-adaptive
example — flips *layout* on `isatty(stdout)` and consequently needs `GH_FORCE_TTY` escape hatches
plus "script against `--json`" discipline (https://cli.github.com/manual/gh_help_formatting,
https://cli.github.com/manual/gh_help_environment); every other tool restricts TTY detection to
color and pager, governed by the `NO_COLOR` (https://no-color.org/) and `CLICOLOR`/
`CLICOLOR_FORCE` (https://bixense.com/clicolors/) conventions. The universally safe rule:
**isatty may govern color and pager, never data shape** — decisive for a CLI whose consumers
include agents that may run under a PTY.

The audit's synthesis weighed both defaults and scored keeping machine-default as the lower-risk
posture. The owner decided otherwise: a human at a terminal is a first-class caller of `shore`,
typing a mode flag for every readback is the wrong UX to institutionalize, and agents — which
already thread `--revision`, `--track`, and body flags through every call — absorb an explicit
format selector trivially, especially one an environment variable can pin once per script. That
decision accepts a one-time breaking change to the default and is recorded here.

A stderr affordance precedent already exists in-repo: `shore identity enroll`/`attest` print a
one-line advisory hint to stderr alongside successful JSON stdout
(`src/cli/identity/attest.rs:84-85`).

## Decision

1. **Text-default is adopted: bare `shore …` speaks to a person.** Every document-emitting
   command's default stdout becomes its **text lane** — a human-readable rendering, bounded and
   domain-named once bespoke (Decision 3 defines the content and the transition fallback). The
   machine document moves behind an explicit selector (Decision 2) — this is
   git's porcelain rule in its native orientation: the text default evolves freely; the named
   machine format is the contract. **This is a breaking change to today's default and is
   accepted**; Decision 8 defines the migration. The break is invocation-level only — the machine
   documents themselves do not change shape, so no document `version` bumps.

2. **The selector is `--format <text|json|json-pretty>`, with `SHORE_FORMAT` as the scripted
   default.** Precedence: explicit `--format` flag > `SHORE_FORMAT` environment variable >
   built-in default (`text`).
   - `--format json` emits the machine document — the **machine lane** — byte-identical to
     today's compact default output. Consumers migrate by adding one flag or exporting one
     variable; nothing about the documents, field names, or values changes.
   - `--format json-pretty` is the same document, indented: the eyeball-debugging form. The byte
     contract is pinned on `json` only.
   - `--format text` explicitly selects the text lane (it out-ranks an exported
     `SHORE_FORMAT=json` for one call).
   - `SHORE_FORMAT` accepts the same values and exists so a script or agent harness pins the
     machine lane once instead of repeating the flag on every call. CI and byte-pinned tests
     should prefer the per-call flag, which is immune to environment leakage.
   - The `--pretty`/`--compact` flags are retired with the flip; their jobs are absorbed
     (`json` is compact; `json-pretty` is indented; `text` is readable). An unknown or invalid
     `SHORE_FORMAT` value is a hard error, not a silent fallback.
     The 2026-07-09 amendment reaches the same alias-removal endpoint without adopting the text
     default flip.
   - The convention applies uniformly to reads and write acks. Exit-code semantics are shared
     across lanes and remain contract.

3. **Text-lane output is formally disposable; the machine lane is the stability surface.**
   Wording, layout, truncation, ordering, and color of the text lane may change in any release
   without notice, and nothing — no script, skill, or relay — may parse it. **Transition
   fallback:** a command that does not yet have a bespoke text rendering emits the indented
   JSON document as its interim text lane; because the text lane is disposable, replacing that
   fallback with a real digest later is not a break. The fallback is
   **parseable-but-non-contract** — it happens to be valid JSON, but parsing it is as forbidden
   as parsing any text rendering — and it is **not available to the consumed commands**: any
   command whose document carries a hard-core consumed field-path (Decision 7) must ship a real,
   non-JSON text rendering at the flip, so an unmigrated parser of those commands fails loudly
   instead of silently reading disposable output that will later change shape. Once the machine lane is selected, its
   stdout bytes are **presentation-invariant**: serialization form, field shape, and byte-level
   rendering are a pure function of the command, its arguments, and the recorded content — no
   TTY detection, terminal environment, or presentation configuration may alter them, and the
   machine lane is **never colored**, even at a TTY, even under a future `--color=always` (gh's
   cosmetic TTY-adaptive JSON is explicitly not copied — ANSI-wrapped JSON corrupts a PTY-bound
   agent's parse). Semantic inputs that legitimately feed document *content* — writer identity
   (`SHORE_ACTOR_ID`, git identity, `src/session/identity/writer.rs:46`), signing and store
   configuration, git/worktree state, and clock-derived defaults (e.g. `validFrom`,
   `src/cli/identity/enroll.rs:67`) — are outside this rule: they change what is recorded, never
   how it is rendered.

   Implementation guidance (non-binding): text renderings enter through a sibling of the
   `write_json` seam so lane selection stays a single decision point per command.

4. **isatty may govern color, pager, and stderr hints — never data shape.** The default is
   `text` everywhere, piped or not; the lane never flips on TTY detection (a piped bare
   `shore …` receives text bytes — the caller's cue to select `--format json`, exactly git's
   posture for its default output). On the text lane, color and paging may key on
   `isatty(stdout)`. A text rendering that pages does so only at a TTY; pager selection respects
   `SHORE_PAGER` then `PAGER`, and any paging command must offer a `--no-pager` escape. The
   machine lane never pages. Whether a given human surface colors or pages at all is per-command
   design; the invariant this ADR pins is *where* those affordances may exist (text lane and
   stderr only) and *what* they may never do (change data shape).

5. **Presentation precedence is a total order: `--color` > `NO_COLOR` > `CLICOLOR_FORCE` >
   isatty.** Shore honors the no-color.org rule verbatim (a present, non-empty `NO_COLOR`
   suppresses ANSI color) — unlike git, like gh/cargo/jj. When the first colored surface ships it
   adds `--color <auto|always|never>`; the explicit flag value beats everything. Below the flag,
   `NO_COLOR` (disable) beats `CLICOLOR_FORCE` (non-zero forces color when piped) — when both are
   set, color is off; disabling wins ties. Below both, isatty auto-detection decides. None of
   this applies to the machine lane, which is exempt from color entirely per Decision 3.

6. **A one-line stderr hint guards the piped-human trap.** When (a) stdout is **not** a TTY,
   (b) neither a `--format` flag nor `SHORE_FORMAT` selected the lane, and (c) `SHORE_NO_HINT`
   is unset or empty, the command prints exactly one line to stderr, of the form:

   > `hint: text-format output on stdout; pass --format json (or set SHORE_FORMAT=json) for the stable machine format`

   Humans at a TTY get the readable default and need no hint; the hazard under a text default
   is the *script* that pipes without selecting, and the hint reaches its author on stderr
   without corrupting the pipe. The wording is non-contract — stderr is never contract. This
   extends the existing `identity enroll`/`attest` stderr-hint precedent.

7. **The stability promise is re-graded to a tiered posture on the machine lane, with the
   per-document `version` field as the bump lever.**
   - **Hard core — frozen within a document's `version`:** the envelope discriminators (`schema`,
     `version`) on every document; the consumed field-paths enumerated in Context
     (`.revision.id` on capture; `inputRequests[].{id,title,mode,reasonCode,trackId}` on
     input-request list; `.inputRequestResponseId` and `.eventId` on input-request respond); and
     the wire-value vocabularies (assessment values, input-request response outcomes, and the
     input-request `mode`/`reasonCode` value sets that ride the consumed paths). Changing any of
     these is a coordinated break: bump that document's `version` and migrate consumers — never
     mutate in place.
   - **Soft shell — everything else in the machine documents:** stable by default and
     additive-evolvable within a `version`. Fields may be added; consumers must select by field
     name and tolerate unknown fields. Removing, renaming, or reshaping existing fields bumps
     that document's `version`.
   - The full-document byte snapshots remain the **internal drift alarm**, not the external
     promise: they re-pin against `--format json` invocations at the flip; a deliberate additive
     change re-blesses; an accidental diff is caught. Docs are narrowed to match this posture
     (naming the hard core explicitly and downgrading the blanket "stable integration surface"
     claim); the doc edit itself is follow-on hygiene work, but the posture is decided here.

8. **The flip is one coordinated break, gated on first-party migration.** The default does not
   flip until every first-party stdout consumer selects the machine lane explicitly: the author
   skill's `capture` extraction, the reviewer/author-response skills' readback invocations, the
   loop drivers' assessment/outcome reads, the relay `cli-fallback` command builders (if
   retained per shoreline-relay#11), the byte-snapshot suite, and the documented examples. Each
   adds `--format json` (or exports `SHORE_FORMAT=json` at the top of its script). Because the
   machine bytes are unchanged, migration is mechanical and verifiable — an unmigrated consumer
   fails loudly on first parse of text output, and the stderr hint (Decision 6) names the fix.
   Sequencing lives in the implementation plan; the gate lives here.

9. **Opaque-id display truncation (ADR-0028 extended to the CLI text lane).** Text renderings
   may display-truncate prefixed ids exactly as the inspector does (`shortId` — the 12-char
   digest tail — and `shortRef` — `rev:1ace028b` — `src/cli/inspect/web/src/refs.ts:29-40`).
   The machine lane always emits full ids; a text rendering must keep the full id one step away
   (the machine lane, or an explicit widening flag), and a truncated form must never look
   parseable or be accepted back as an argument. Consumers pass ids back verbatim from the
   machine lane only, per ADR-0028 and `docs/review-workflow.md`.

10. **The relay `cli-fallback` question stays open without blocking this convention
    (shoreline-relay#11).** Both resolutions hold under this ADR: if the fallback is
    **retained**, its command builders adopt `--format json` (Decision 8) and the three
    composite-document paths it reads after re-pointing its stale `/reviewUnit/*` pointers
    (`revision.id`, `revision.revisionId`, `summary.observationCount` on
    `pointbreak.review-revision`) join the hard core; if it is **retired**, `pointbreak.review-revision`
    has zero stdout consumers and evolves in the soft shell behind a re-bless and the `version`
    discipline. The hard core enumerated in Decision 7 is the floor either way.

## Consequences

### Accepted

- **The human becomes a first-class caller.** Bare `shore …` answers a person — the #96 digests
  are the *default* readback, not an opt-in a human must remember. This matches the muscle
  memory of every mainstream dual-audience CLI (git, gh, cargo, jj, kubectl).
- **One coordinated, tractable break.** Every consumer that must migrate is first-party and
  enumerable (four field-paths, two enum vocabularies, the snapshot suite); the machine bytes
  under `--format json` are today's bytes, so migration adds a flag/env and reparses nothing.
- **Text output can iterate freely.** The disposable declaration means digests and diff
  renderings improve without contract ceremony; the interim indented-JSON fallback can be
  replaced per command without further breaks.
- **The freeze set is finally named.** The tiered promise legitimizes additive evolution on the
  wide soft shell and concentrates the "never break this" discipline on an enumerable hard core,
  instead of a blanket byte-freeze nobody needs.
- **Accepted cost: the break itself.** Third-party scripts (if any exist) that parse today's
  default stdout break at the flip; the stderr hint names the one-line fix. This is judged
  acceptable now precisely because the consumer census found no third-party stdout consumers.
- **Accepted cost: environment-sensitive lane selection.** `SHORE_FORMAT` trades per-call
  explicitness for script convenience; a script relying on it inherits environment fragility
  (an unset variable yields text bytes). Mitigated by the flag's precedence, the loud parse
  failure, and the hint; byte-pinned tests and CI use the flag form.
- **Accepted cost: interim fallback keeps the #96 trap on some commands.** Until a bespoke
  digest lands, a command's text lane is indented JSON — no better for a human than today
  (`shore review show` stays unreadable until its digest ships). The digest wave closes this
  per family.
- **Accepted cost: two renderings to maintain** on every participating command, with the text
  side untestable-by-contract (its tests may assert behavior, never bytes).

### Rejected

- **Machine-default + explicit text opt-in (extending today's posture — the audit synthesis's
  lower-risk scoring).** Zero-break and honest to the historical agent-majority audience, but it
  institutionalizes the wrong default for people: every human readback forever pays a mode flag,
  and the bare command greets a person with a wall of compact JSON. Rejected by owner decision:
  the break is affordable now (first-party-only consumers), and it will never be cheaper.
- **A boolean `--text` flag as the selector.** A boolean cannot express the machine/pretty/text
  triage, invites a second axis beside pretty/compact booleans instead of absorbing them, and
  cannot be pinned once per script the way `--format`+`SHORE_FORMAT` can. The enum also leaves
  room for future formats (e.g. JSONL) without a new flag.
- **TTY-adaptive data shape (the gh model) — rejected, revisit conditions recorded.** The trap
  is structural: an agent that happens to run under a PTY (sandboxes, `script`, some CI
  harnesses) would silently receive text output, and a human piping to a file would silently
  get machine output — the lane must follow an explicit selection, not a guess. Reopening this
  is gated on the conditions in Revisit Triggers, all of which must hold.
- **Dual-stream rendering (JSON on stdout + text on stderr simultaneously).** Violates stream
  discipline (stderr is diagnostics and hints, not a data channel) and makes the text rendering
  un-pipeable to a pager.
- **Cosmetic TTY-adaptivity on the machine lane (gh pretty-prints and colors `--json` at a
  TTY).** Even "presentation-only" adaptation of machine stdout puts ANSI or reflowed JSON in
  front of a PTY-bound parser. Machine-lane bytes are TTY-invariant, full stop.
- **A config-file default for the output format.** Lane selection stays in the invocation (flag)
  or its immediate environment (variable); a repo- or user-level config that silently flips a
  script's parse target reintroduces git's "porcelain must be config-immune" problem in reverse.

## Revisit Triggers

- **TTY-adaptive default** becomes eligible for reconsideration only when *all* of: (1) the
  piped/non-TTY branch stays byte-identical to the machine lane; (2) the explicit selectors
  (`--format`, `SHORE_FORMAT`) always beat detection; (3) a force-override lets CI and agent
  harnesses pin the mode unconditionally; (4) first-party agent surfaces (skills, relay
  fallback) demonstrably pin the machine lane explicitly rather than relying on any default.
- **A second machine format appears** (e.g. JSONL for `--watch` streams) → it joins the
  `--format` enum as a new value; the enum is the extension point.
- **shoreline-relay#11 resolves** → update the hard-core enumeration (retained: add the three
  composite paths; retired: record `pointbreak.review-revision` as zero-consumer soft shell).
- **The first `version` bump on a consumed document** → confirm the coordinated-break path works
  in practice; if it proves unworkable, re-grade the tiered promise.
- **Evidence of third-party stdout consumers** appearing before the flip lands → re-run the
  consumer census and re-weigh the migration gate in Decision 8.
