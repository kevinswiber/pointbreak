---
name: pointbreak-reviewer
description: Use when a coding agent should review a Pointbreak handoff or captured revision that another agent left. Read the author's observations, validation evidence, and input requests with bounded list commands, review the live change independently, respond to open operative input requests, record reviewer findings and validation evidence on your own track, open advisory input requests for author decisions, record exactly one assessment, then stand down.
---

# Pointbreak Reviewer Handoff Review

This skill targets the `0.7.0`-and-later flat `pointbreak` CLI. Use only the canonical Pointbreak
environment and storage names; existing installations must complete the offline move described in
`docs/installation.md` before review work resumes. `pointbreak store paths --repo . --format json`
reports the canonical paths, including the Git-common-dir store shared by linked worktrees.

You are the reviewing agent for a Pointbreak revision another agent captured. Your job is to review
the change independently, record durable review findings, answer any open operative requests you can
answer, and make the review call.

Record exactly one assessment with `pointbreak assessment add`. The assessment is the reviewer's
call, so this role owns it. Never write to the author's track.

Do not run `pointbreak revision show --format json-pretty` as a readback surface. It includes the full captured
snapshot and can emit megabytes for a real change. Use bounded list commands for the author's
handoff, your reviewer notes, input requests, and assessment.

## Workflow at a glance

```text
1. Identify the revision and the author's track.
2. Read the author's observations, validation evidence, and input requests with bounded commands.
3. Choose one reviewer track for this review.
4. Review the change independently from the handoff.
5. Record review findings as observations and concrete check results as validation evidence.
6. Respond to open operative input requests when you can answer them.
7. Open advisory input requests for follow-ups that need an author decision.
8. Add exactly one assessment on the reviewer track.
9. Read back the reviewer record with bounded list commands, then stop.
```

Treat the author's handoff as navigation context, not as proof. Re-run relevant checks, read the
diff yourself, and verify the review result from the repository in front of you.

## Identify the revision

If the revision ID is not already known, list captured units and pick the one you were asked to
review:

```bash
pointbreak revision list --format json-pretty
revision_id="<revision-id>"
author_track="<author-track>"
```

If the author track is not supplied, use the bounded read surfaces to find the track that contains
the authored handoff:

```bash
pointbreak observation list --revision "$revision_id" --format json-pretty
pointbreak validation list --revision "$revision_id" --include-body --format json-pretty
pointbreak input-request list --revision "$revision_id" --status open --format json-pretty
```

## Read the author's handoff

Read only the author's track. Include bodies so you can see the substance of the handoff:

```bash
pointbreak observation list \
  --revision "$revision_id" \
  --track "$author_track" \
  --include-body --format json-pretty

pointbreak validation list \
  --revision "$revision_id" \
  --track "$author_track" \
  --include-body --format json-pretty

pointbreak input-request list \
  --revision "$revision_id" \
  --track "$author_track" \
  --status open \
  --include-body --format json-pretty
```

Use those observations and validation checks to orient yourself, then form your own judgment.
Validation evidence is advisory context, not proof and not an assessment. Do not repeat the author's
claims as reviewer findings unless you have independently verified them.

## Choose your track

Choose one reviewer track for the whole review and reuse it for every reviewer write. Use the form
`agent:<agent-name>-<id>`.

`<agent-name>` is your own short lowercase agent name. `<id>` is usually the issue or PR number; use
the branch's distinctive segment as a fallback, and use a short random tag if neither exists. Keep
the part after `agent:` lowercase, hyphenated, and around 15 characters or fewer.

Tracks are review lanes, not actor identity: the unique tag keeps lanes legible, while the actor id
below records writer provenance in the event envelope.

```bash
agent_name="<agent-name>"
run_id="<id>"
reviewer_track="agent:${agent_name}-${run_id}"
export POINTBREAK_ACTOR_ID="actor:agent:${agent_name}"
```

The actor id is your durable identity across sessions and runs — it carries no run id. Use **one
canonical spelling** for your agent name and always the same one (`claude-code`, never also
`claude`): two spellings split one agent's history across two identities. Keep it lowercase and
hyphenated, like the track rule; `/` inside the agent segment is reserved.

**Signing is automatic.** On your first write under this `actor:agent:*` id, Pointbreak generates a
passphrase-less per-machine key and signs the event; it prints a one-line notice with your `did:key`
and `pointbreak key enroll` so a human can add you to the committed allow-list. Your signed response is
the event that closes the binding loop — once enrolled, it verifies and binds. Signing never blocks
a write — if no key can be made the write still succeeds, unsigned. Set `POINTBREAK_SIGNING=off` to
disable signing. A human can instead reuse an existing SSH key via `pointbreak key use-ssh` (agents still
auto-keygen, unchanged).

## Review independently

Before recording a finding, read the repository's applicable agent instructions and inspect the
change directly. Use the project's normal review and verification surfaces: Git diff, targeted
tests, full tests or checks when appropriate, lint, formatting, documentation checks, and remote
status when the project uses it.

The revision snapshot is frozen at the author's capture moment, while your checkout may have moved
since then. Compare the captured unit's endpoints from `pointbreak revision list --format json-pretty` with the
commit or branch head you actually review. If they diverge or you cannot prove they match, record a
reviewer observation that names the live commit and the possible snapshot mismatch.

A moved HEAD or a commit landing after the capture does **not** invalidate the revision and is
never grounds to recapture or mint a superseding revision: the author bridges a landed commit with
`pointbreak association record --commit` on the same revision, and successive landed commits accreting
on one revision are the expected multi-pass shape. Supersession (`pointbreak capture --supersedes`) is
only for a genuinely new content state replacing the reviewed one. Review the live worktree, note
any snapshot mismatch as an observation, and still record your assessment on the existing revision.

```bash
git status --short --branch
git diff --stat
git diff
git rev-parse HEAD
```

## Record reviewer findings

Record durable review findings as observations on the reviewer track. Use anchored observations for
file or range-specific findings, and review-wide observations for verification, commit divergence, or
cross-cutting conclusions.

```bash
pointbreak observation add \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --title "Parser test covers the new token path" \
  --file tests/parser.rs --start-line 42 --end-line 71 \
  --body "Verified the new regression test fails against the old parser behavior and passes with this change."

pointbreak observation add \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --title "Verification reproduced the author's green checks" \
  --body "Ran the repository's targeted parser test and full test suite from the reviewed checkout. Both passed."
```

Plain observations are for facts that need no response. If you need the author to make a decision,
open an advisory input request instead. To acknowledge or dispose of another observation
non-destructively — leaving it standing — add an observation with `--responds-to <observation-id>`
(see [ADR-0026](../../docs/adr/adr-0026-fact-to-fact-response-relationship.md)); use `--supersedes`
only to retire a target you are correcting.

**Body content type — prefer Markdown for review content.** The `--body` (observations, input
requests), `--summary` (validation, assessment), and `--reason` (input-request responses) fields
default to plain text, but the inspector renders Markdown and the human reviews there, so **author
review content as Markdown** whenever it names code. Pass the matching
`--body-content-type`/`--summary-content-type`/`--reason-content-type text/markdown` and:

- backtick file paths, code/identifier/type references, and function signatures
  (e.g. `` `tests/parser.rs` ``, `` `parse_tokens` ``, `` `Option<&str>` ``);
- prefer short bullet lists over long paragraphs.

Keep bodies concise — Markdown is for scannability, not license to write a document.

## Record reviewer validation checks

When you run checks during review, record the concrete result on the reviewer track. Use validation
evidence for command results, and observations for the reasoning around those results.

```bash
pointbreak validation add \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --check-name "just check" \
  --status passed \
  --command "just check" \
  --exit-code 0 \
  --summary "Reproduced the repository check from the reviewed checkout."
```

Validation checks target the whole captured revision. Do not add file, range, or path targets. If
your live checkout differs from the captured snapshot, say so in a reviewer observation before
recording any check result, and avoid implying that a live-only check proves the frozen snapshot.

## Respond to operative input requests

List the author's open operative requests and respond to each one you can answer. If you cannot
answer one, leave it open and reflect that in the assessment.

```bash
pointbreak input-request list \
  --revision "$revision_id" \
  --track "$author_track" \
  --mode operative \
  --status open \
  --include-body --format json-pretty

pointbreak input-request respond <input-request-id> \
  --outcome approved \
  --reason "verified the migration plan against the current test database fixture"
```

Use `approved`, `rejected`, `dismissed`, `superseded`, or `abandoned` for the response outcome. Do
not respond to a request just to make the queue look clean; response events are durable review facts.

## Ask the author for follow-up decisions

When a non-blocking follow-up needs the author to decide, open an advisory input request on the
reviewer track. Do not record decision-seeking follow-ups as plain observations.

```bash
pointbreak input-request open \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --title "Decide whether to split the parser follow-up" \
  --reason manual-decision-required \
  --mode advisory \
  --file src/parser.rs --start-line 84 --end-line 123 \
  --body "The change is acceptable as written, but the parser now has two extension paths. Please decide whether to split the cleanup into a follow-up issue or handle it before landing."
```

If the fact does not need an author response, record it as an observation instead.

## Add exactly one assessment

After you have reviewed the change and recorded your evidence, add one assessment on the reviewer
track. Use `accepted`, `accepted-with-follow-up`, `needs-changes`, or `needs-clarification`.

```bash
pointbreak assessment add \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --assessment accepted-with-follow-up \
  --related-observation <observation-id> \
  --related-input-request <input-request-id> \
  --summary "The implementation is acceptable. I opened an advisory follow-up for the author to decide how to handle parser cleanup."
```

Recording the assessment is the reviewer's role. Do not add a second assessment to clarify prose;
record clarifying facts as observations before the single assessment, or choose
`needs-clarification` if the review call is not ready.

If your judgment supersedes an assessment already on the revision — you are revising your own
earlier call in a later round, or your re-review pass replaces a predecessor track's assessment —
pass `--replaces <assessment-id>` (repeatable) on the new assessment. Replacement is never
implicit, even for the same actor: two unreplaced assessments on one revision read as competing
candidates and surface as an `ambiguous_assessment` attention item. Check what is current first:

```bash
pointbreak assessment show --revision "$revision_id"
```

## Read back and stand down

Verify the reviewer record with bounded read commands:

```bash
pointbreak observation list \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --include-body --format json-pretty

pointbreak validation list \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --include-body --format json-pretty

pointbreak input-request list \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --status all \
  --include-body --format json-pretty

pointbreak assessment show \
  --revision "$revision_id" \
  --track "$reviewer_track" \
  --include-summary --format json-pretty
```

Then stop. Report the revision ID, reviewer track, assessment value, and any open input requests.
Do not continue editing the code as part of the review unless the user explicitly switches you into
an implementation role.

## Common errors

- **Using full revision show for readback.** Use bounded observation, input-request, and
  assessment read commands. Do not use `pointbreak revision show --format json-pretty` for this review loop.
- **Writing on the author's track.** The reviewer uses a separate reviewer track for every write.
- **Rubber-stamping the handoff.** The author's observations are context. Verify claims yourself.
- **Treating validation evidence as an assessment.** Check records are advisory context. The
  reviewer still records exactly one assessment.
- **Hiding reviewer-run checks in observations.** Use `pointbreak validation add` for concrete
  command results, and observations for interpretation or risks.
- **Skipping the live commit check.** If your checkout differs from the captured snapshot, say so in
  a reviewer observation.
- **Superseding instead of assessing.** A commit that landed after the capture or assessment is
  recorded by the author as a commit association on the same revision; it never requires a new
  revision, and it is not a reason to withhold or re-route your assessment.
- **Recording author-decision follow-ups as observations.** Use an advisory input request when the
  author should answer.
- **Adding multiple assessments.** The reviewer records exactly one assessment for the review pass.
- **Re-assessing without `--replaces`.** A revised call — or a re-review of an already-assessed
  revision — records the new assessment with `--replaces <prior-assessment-id>`; omitting it leaves
  two competing candidates that read as ambiguous.
