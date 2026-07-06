# ADR-0030: Named Command Surface — `shore diff` Defined, the `show` Collision Resolved, `up`/`session`/`dump` Settled

**Status:** Accepted (owner-approved 2026-07-02); landed 2026-07-02 (grounding issue #96).
**Date:** 2026-07-02
**See also:** **ADR-0029** (CLI output-mode convention — the `--format` output-lane split this
surface rides: JSON default, opt-in text, per its 2026-07-03 amendment), **ADR-0031**
(review-surface grammar — completes Decision 1's subject rule by flattening the review family),
**ADR-0018**
(event-borne supersession — the reshape that made the captured revision the product's subject),
the "Old dump/show stream vs. revision ledger" section of `docs/review-workflow.md` (the two-surface
seam this ADR resolves), and `docs/cli-reference.md` §`shore show`/`shore dump`. Grounding issue:
**#96** (human-readable readback — `shore diff` and the digest layer are its deliverable).

## Context

Shoreline's early product vocabulary named a six-command human surface — `shore diff`, `shore
show`, `shore up`, `shore notes`, `shore session`, `shore dump` — but the surface was never
defined, and the ground has moved under it. The README was repositioned around the durable review
record (`57ace44`) and now names none of `diff`/`show`/`up`/`session`/`dump`; onboarding
(`docs/getting-started.md`) runs through `shore review capture`, `shore review show --pretty`, and
`shore inspect`. Meanwhile the revision ledger shipped: capture, observations, input requests,
assessments, validation, and associations are live, and the captured revision — not the working
tree — is what the product is about.

Two of the six names exist today, and they are one legacy surface with two front-ends: `shore
dump` and `shore show` both build a `DumpDocument` from the **live working-tree diff at run time**
plus imported `review-notes.json` sidecar notes (`src/cli/input.rs:17-28`, `src/dump.rs`), and
neither reads the revision ledger at all. The TUI's row vocabulary (`src/tui/view.rs:7-17`) has no
revision, observation, input-request, assessment, or validation row kind — it is a diff+notes
viewer for the pre-revision workflow. `shore.dump` JSON has **zero found consumers** despite being
documented as an integration surface.

This produces the collision any human-readback direction hits first: **two `show` commands** —
top-level `shore show` (TUI, live tree) and `shore review show` (machine composite, frozen
revision) — same verb, opposite subjects. And it leaves the #96 job unserved: the immutable
captured diff, the actual object under review, has no terminal reader. `shore review show`
interleaves the diff into a 1.94 MB machine document; the TUI renders the wrong subject; the
inspector reads the captured snapshot well but requires a browser.

What the terminal must *not* do is rebuild the inspector: the supersession DAG, the filterable
event timeline, the per-line-annotated cross-file diff, and the endorsement web are relational,
visual, and already served (`src/cli/inspect/`). The terminal owes the human the loop-inline
readbacks — bounded digests (ADR-0029's text lane) and the captured diff itself.

## Decision

### 1. Bare top-level verbs read against the product's subject: the captured review record

A top-level `shore <verb>` must be about the captured review record — the thing shore holds that
nothing else does. Any command about another subject (the live working tree, sidecar notes, keys,
the store) is family-scoped under a noun (`shore notes …`, `shore store …`), where verbs may
repeat without ambiguity (`shore review show` vs `shore notes show` name different subjects;
top-level bare `show` names none). This principle decides every case below.

### 2. `shore diff` — captured-revision human diff readback (the #96 home)

`shore diff` prints a captured revision's diff — base to target, from the frozen captured snapshot
— as a text unified diff on stdout. It is **text-only**: git-diff is its only lane — it offers no
`--format json` initially (passing one is an error), because the output is already git-diff (pipeable
to any diff tool as-is) and a machine wanting structured diff data reads it from the review
documents. Under ADR-0029 (JSON default, opt-in text) it is therefore the one command with no JSON
lane, emitting git-diff regardless of the default. It is non-interactive and a
well-behaved **filter**: piped, redirected, or paged output is plain git-diff bytes, and it applies
its own syntax coloring only when writing directly to a TTY (ADR-0029 Decision 5's
`--color`/`NO_COLOR`/`CLICOLOR_FORCE` precedence; `--color always` forces color through a pipe). It
does **not** spawn a pager — because its output is genuine git-diff, the reader pipes it to any pager
or diff renderer of their choice (`shore diff | less -R`, `shore diff | delta`), which then owns
presentation; this also keeps `shore diff`'s own coloring from stacking on a re-coloring pager. Its
output is formally disposable — nothing parses it. (ADR-0029 Decision 4 leaves whether a surface
pages at all to per-command design; `shore diff` chooses not to, and no `SHORE_PAGER`/`PAGER`/
`--no-pager` machinery exists for it.)

Its subject is a captured revision, never the live working tree: `git diff` already owns the live
tree, and shore's bare verbs read against the review record per Decision 1. Revision selection
follows the review family's convention (explicit `--revision` when the store holds more than one
candidate); because the surface is disposable, more ergonomic head-resolution may evolve without
ceremony. A diffstat header and a stat-only option are expected; exact flags are implementation
design. Anchored review facts (observations pinned to lines) are **not** part of the initial
definition — if ever added, they stay a lightweight cue and do not re-implement the inspector's
annotated-diff lens.

### 3. The `show` collision is resolved: `show` belongs to the review record; the TUI is renamed

- **`shore review show` keeps its name** — the composite over a frozen revision, whose document
  form lives on the machine lane (`--format json`). Its future reshape (e.g. shedding the
  multi-megabyte row geometry) is soft-shell work under ADR-0029 Decisions 7 and 10, not this
  ADR.
- **Top-level `shore show` is retired as a name.** The TUI it fronts is renamed to
  **`shore notes show`** — subject-named for the job that distinguishes it (reading imported
  sidecar review notes anchored on the live working-tree diff, beside the existing
  `shore notes apply`) — and is explicitly marked **experimental** in `--help` and the CLI
  reference: the TUI has not had the investment to carry a stability expectation, and the
  experimental label says so while its fate is decided (Decision 6). As a bare working-tree
  pager it duplicates `git diff` and is not product surface; the notes overlay is why it exists,
  so the notes family is where it lives. The old name gets the standard removed-command
  migration hint (`src/cli/mod.rs:96-114` precedent), pointing to `shore notes show`
  (imported-notes viewing), `shore diff` (captured-revision readback), and `shore inspect`
  (deep reading).
- **Bare top-level `show` is not reused.** If the deferred TUI decision (Decision 6) ever
  produces a revision-era interactive surface, it arrives under an explicit name decided then;
  reserving the bare verb is not a commitment to fill it.

### 4. `shore up` is dropped

No derivable job: every candidate reading (status readout, recapture shortcut, inspector
launcher) collides with an existing surface (`shore store status`, explicit
`shore review capture --supersedes`, `shore inspect`) or with the standing guardrail that capture
modes lower through explicit adapters rather than ad hoc conveniences. The name is dropped from
the surface — not reserved. Any future proposal starts from a product case, not from the name.

### 5. `shore session` is absorbed

The job the name pointed at — reload/freshness status — is already served where the facts live:
`eventSetHash` freshness metadata on the ledger reads and `shore store status`. A thin `session`
wrapper would be a second home for the same facts, and "session" is an overloaded noun in the
internal model. Freshness readback for humans rides ADR-0029's text lane on the commands that
already own the data (the store digest and review digests), not a new verb. Dropped from the
surface.

### 6. `shore dump` is retired; the TUI's fate is deferred behind this ADR

- **`shore dump` is retired.** Zero found consumers; the integration-surface role its docs
  claimed passes to the review document family under ADR-0029's re-graded promise. Retirement
  ships with the standard removed-command hint. The `shore.dump` schema tag retires with the
  command; the `shore.review-notes` *input* sidecar schema (`shore notes apply`) is unaffected.
  The `DumpDocument` model remains internal plumbing for the renamed TUI while it lives.
- **The TUI decision is explicitly deferred behind this ADR; experimental status covers the
  interim.** Whether `shore notes show` is eventually re-plumbed onto the captured revision
  (mechanically bounded: point it at the existing revision projection and widen its row model)
  or retired outright is decided after `shore diff` and the digest layer ship, on two inputs:
  whether an interactive terminal reader still has pull once `shore diff` covers the SSH
  readback slice, and whether the imported-notes viewer job retains standalone value. Because
  the surface is marked experimental, either outcome — re-plumb, further rename, or retirement —
  needs no deprecation ceremony beyond the migration hint. Any future revision-era TUI is bound
  by Decision 7.

### 7. Terminal surfaces do not ASCII-clone the inspector

The digest layer (ADR-0029's text lane) and `shore diff` may mirror the inspector's
revision-page *header* — current assessment, open input requests, fact counts, diffstat — and no
more. The supersession DAG, the event timeline, the annotated cross-file diff with per-line
facts, and the endorsement web stay inspector-only. A terminal surface that starts growing a
lens the inspector already owns is out of scope by decision, not by omission.

## Consequences

### Accepted

- **#96 gets its home**: `shore diff` (this ADR) plus the bounded digests (ADR-0029's text lane)
  are the deliverable the issue's deferral pointed at; the composite `shore review show` stops
  masquerading as a human surface.
- **One `show` concept survives**: `show` means the review record (`shore review show`); the
  live-tree viewer is subject-named under `notes`. The rename costs muscle memory for anyone
  using the legacy TUI, mitigated by the migration hint.
- **The named surface shrinks honestly**: `up` and `session` exit the vocabulary instead of
  waiting indefinitely for jobs; `dump` exits with zero consumer impact. Fewer names, each with a
  defined job.
- **Deferral is recorded, not implied**: the TUI's fate has named decision inputs and a named
  constraint (Decision 7), so the next session inherits a decision point, not a vague hope.
- **Accepted cost**: retiring `shore dump` and renaming `shore show` are user-visible breaks to
  the legacy surface (softened by hints, and by the fact that the README stopped advertising both
  names). The `dump`/`show` byte-parity seam and TUI code remain in-tree until the deferred
  decision, carrying maintenance weight for a surface that may retire.

### Rejected

- **Re-pointing `shore show` at the captured revision now.** That is the TUI re-plumb by another
  name — it would decide the deferred question in the ADR, and it would silently change the
  subject of an existing command (the sharpest kind of muscle-memory break: same name, different
  data).
- **Retiring the TUI outright now.** Kills the imported-notes viewer job before the digest layer
  and `shore diff` demonstrate coverage; the deferral exists to make that call with evidence.
- **`shore diff` over the live working tree (git-diff parity).** Duplicates `git diff`, leaves
  the #96 readback gap unsolved, and violates Decision 1's subject rule.
- **`shore diff` as a rename of `shore dump`.** `dump` is machine JSON over the wrong subject
  (live tree); reusing its identity would drag the legacy model under a product name.
- **Keeping `shore up` or `shore session` as reserved names.** A reserved-undefined name in the
  surface invites planning a command because the name exists — the failure mode this audit found
  (a six-name table outliving the product's actual vocabulary).
- **A terminal DAG/timeline/annotated-diff.** The inspector owns relational and visual readback;
  ASCII clones would be worse tools and a second maintenance surface (Decision 7).

## Revisit Triggers

- **The deferred TUI decision** — after `shore diff` and the first digest wave ship: re-plumb
  `shore notes show` onto the revision projection, retire it, or leave it as the import viewer.
  Inputs per Decision 6.
- **`shore review show` reshape** — once shoreline-relay#11 resolves (per ADR-0029 Decision 10),
  the composite's row-geometry bulk becomes reshapeable; if that reshape lands, revisit whether
  `shore diff` should absorb any of its readback duties.
- **A real job materializes for a dropped name** — `up` or `session` may return only with a
  product case that names a job no existing surface serves.
- **Notes-import workflow evolution** — if sidecar-note import stops being a supported path, the
  renamed TUI and `shore notes` family shrink accordingly.

## Amendment: Legacy Working-Tree Surfaces Hidden Pending Redesign (2026-07-05)

The original decisions stand: `shore dump` is retired as product surface (Decision 6), the TUI's
job belongs under the notes family (Decision 3), and the TUI's ultimate fate is deferred with named
inputs (Decision 6). What changes is the **interim implementation posture**. Neither the `dump`
retirement nor the `show` → `notes show` rename has been executed — both commands are still
shipped and advertised — and executing them piecemeal now would spend rename ceremony on a surface
whose redesign may retire it anyway.

**Decided instead:** `shore show`, `shore dump`, and the `shore notes` family are **hidden from
`--help`** (`#[command(hide = true)]`) while remaining functional. All three are early product
surface — the TUI was never fully fleshed out, and the notes-import workflow sits in the same
bucket — and none of it should be advertised again until a deliberate design pass decides what (if
anything) is promoted. The `show` → `notes show` rename is **deferred into that design pass**
rather than performed now; if the pass retires the TUI, the rename never happens and the
retirement ships with the standard removed-command hint instead.

**Mechanics:**

- Hiding is not removal: no removed-command hints fire, no behavior changes, and
  `tests/cli_removed_legacy.rs` is untouched.
- `src/cli/reference_coverage.rs` (every clap leaf must appear in `docs/cli-reference.md`) gains
  an explicit policy for hidden leaves: they stay documented, marked **legacy — hidden pending
  redesign**, so the guard keeps walking them.
- Docs still carry substantial `shore dump` / top-level `shore show` example references; those
  sweep toward `shore diff`, `shore inspect`, and the digests with this change or an immediate
  follow-up.

Decision 6's deferral inputs are unchanged; Decision 7 (terminal surfaces do not ASCII-clone the
inspector) binds any future design pass. Bare top-level `show` remains unassigned per Decision 3.

**Consequences.** Accepted: the advertised surface stops promising a legacy workflow; the redesign
decision is not pre-empted; zero breakage (hidden ≠ removed). Cost: three functioning commands
become undiscoverable except through docs — intentional, since discovery is what mis-sells them.
Rejected: executing the rename/retirement now (spends a user-visible break on a surface whose fate
is undecided); leaving the commands advertised (keeps mis-selling the pre-revision workflow the
README no longer describes).

**Revisit Trigger.** The deferred TUI/notes design pass (Decision 6's inputs: whether `shore diff`
+ the digests cover the terminal readback slice, and whether the imported-notes viewer job retains
standalone value) — its outcome executes, renames, or retires each hidden surface explicitly.

**Status:** Accepted (owner-approved 2026-07-05); lands with the review-surface reshape
implementation work (issue #379). The original ADR-0030 text above and its top-level
**Status: Accepted** are unchanged.
