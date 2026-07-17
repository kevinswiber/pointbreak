# Getting Started

This guide takes one real tracked change from capture to a complete paired author/reviewer loop:
capture the change, open it in Pointbreak Review, record the author's handoff, let a reviewer ask a
question and make a call, answer it, and land the commit on the same revision. Along the way it
introduces the five Review stages in plain language:
`Work -> Claims -> Evidence -> Questions -> Call`.

## Before You Start

Install Pointbreak by the supported route in [installation.md](installation.md), then verify the
binary:

```sh
pointbreak --version
```

You also need Git. The commands below are written for a macOS or Linux shell; on Windows, run them
in Git Bash or WSL.

## 1. Start From A Real Tracked Change

Pointbreak reviews your own work, not a sample. If a repository of yours already has a modified
tracked file, use it and skip the scratch block below.

Otherwise, create a disposable repository with a committed baseline, then modify the tracked file so
there is a real change to review:

```sh
mkdir first-review
cd first-review
git init
git config user.name "First Review"
git config user.email "first-review@example.invalid"
git config commit.gpgsign false

printf '%s\n' 'First useful Review' > onboarding.txt
git add onboarding.txt
git commit -m "chore: add onboarding baseline"

printf '%s\n' 'First useful Review' 'Checks are evidence, not a verdict.' > onboarding.txt
```

`git status --short` now shows ` M onboarding.txt`: a real modification to an already tracked file,
which is exactly what a first review should look at.

## 2. Capture The Change And Open Review

```sh
pointbreak capture --summary "Explain evidence in first-use guidance" --format json
```

The capture freezes the current diff as a revision and prints a JSON document describing it. The
summary is an immutable discovery label: lists and Review use it to identify the revision, and it
never changes revision identity. Later facts attach to the same revision; nothing edits the label
in place.

Keep the revision id — the `id` field under `revision` in the capture output — for the rest of the
walkthrough:

```sh
REVISION_ID="rev:sha256:<the id from your capture output>"
```

Now open Review:

```sh
pointbreak inspect --open
```

Review is a local, read-only projection of the durable review record: it renders revisions, diffs,
and recorded facts, and it never executes commands or writes to the store. Right now it shows one
revision under your summary, with the changed file and rows. That is the first useful Review — and
notice what it did not require: no actor id, no track, no signing setup, no trust configuration,
and no reading of raw JSON.

`inspect` is a foreground server: leave it running, keep Review open in this browser tab, and run
every later command in a second terminal. Each step below appears in Review after a refresh.

## 3. Read The Review In Five Stages

Every Pointbreak review answers five questions, in this order, and each stage is owned by one
flattened command family:

| Stage | Question it answers | Command family |
| --- | --- | --- |
| Work | What changed? | `capture`, `revision`, `inspect` |
| Claims | What does an author or reviewer assert? | `observation` |
| Evidence | What was checked? | `validation` |
| Questions | What still needs judgment? | `input-request`, `attention` |
| Call | What is the current assessment? | `assessment` |

Two supporting nouns complete the picture: `attention` lists outstanding judgment across stages,
and `association` records where the reviewed work landed. After the capture, Work is populated and
the other stages are empty. The rest of this guide fills them in.

## 4. Author Handoff — Claims And Evidence

The first authored fact must say who wrote it and which review lane owns it. An actor answers "who
wrote this?"; a track answers "which review lane owns it?". Set both now — they were deliberately
not needed to see the first Review:

```sh
export POINTBREAK_ACTOR_ID="actor:agent:first-review-author"
AUTHOR_TRACK="agent:first-review-author"
```

Record the author's claim about the change:

```sh
pointbreak observation add \
  --exact-revision "$REVISION_ID" \
  --track "$AUTHOR_TRACK" \
  --title "First-use guidance distinguishes evidence" \
  --body "The tracked change explains that checks are evidence rather than a verdict." \
  --format json
```

The first authored fact may print a diagnostic that a signing key was generated for the actor.
Pointbreak signs writes automatically; signed-but-untrusted is an advisory state, not an error, and
nothing blocks on it. When you later want Review to show this writer as trusted, run
`pointbreak key enroll <name>` — it stages the key in `.pointbreak/allowed-signers.json` for human
review. Enrollment is optional and nothing in this walkthrough requires it.

Now run a real check and record its result:

```sh
git diff --check
pointbreak validation add \
  --exact-revision "$REVISION_ID" \
  --track "$AUTHOR_TRACK" \
  --check-name "git diff --check" \
  --status passed \
  --command "git diff --check" \
  --exit-code 0 \
  --summary "The captured tracked change has no whitespace errors." \
  --format json
```

Validation is evidence, not a verdict: it records that a command actually ran against the captured
content and what it reported. It is never an assessment, a merge signal, or a task-completion
verdict — the Call stage stays empty until a reviewer makes one. Record a validation only for a
command you actually ran.

Refresh Review: Claims and Evidence now carry the author's observation and check, attributed to the
author's track.

## 5. Reviewer Pass — A Question And A Provisional Call

A reviewer reads before writing: open Review, look at Work, Claims, and Evidence first. Reviewer
facts then get their own actor and track, introduced here at the handoff and not before:

```sh
export POINTBREAK_ACTOR_ID="actor:agent:first-review-reviewer"
REVIEWER_TRACK="agent:first-review-reviewer"
```

The reviewer records an independent claim and re-runs the check rather than trusting the author's
record:

```sh
pointbreak observation add \
  --exact-revision "$REVISION_ID" \
  --track "$REVIEWER_TRACK" \
  --title "The added sentence is user-visible" \
  --body "The added line is wording a reader will quote; it should be deliberate." \
  --format json

git diff --check
pointbreak validation add \
  --exact-revision "$REVISION_ID" \
  --track "$REVIEWER_TRACK" \
  --check-name "reviewer git diff --check" \
  --status passed \
  --command "git diff --check" \
  --exit-code 0 \
  --summary "The reviewer independently reran the whitespace check." \
  --format json
```

Keep the reviewer observation id from the first command's output (the `observationId` field):

```sh
REVIEWER_OBSERVATION_ID="obs:sha256:<the id from the observation output>"
```

A question that needs the author's judgment becomes a durable input request, not a chat message:

```sh
pointbreak input-request open \
  --revision "$REVISION_ID" \
  --track "$REVIEWER_TRACK" \
  --title "Confirm the wording boundary" \
  --reason manual-decision-required \
  --mode advisory \
  --body "Is the new sentence the exact wording the guide should quote?" \
  --format json
```

Keep its id (the `inputRequestId` field):

```sh
INPUT_REQUEST_ID="input-request:sha256:<the id from the open output>"
```

The reviewer closes the pass with a provisional call that links what it rests on:

```sh
pointbreak assessment add \
  --exact-revision "$REVISION_ID" \
  --track "$REVIEWER_TRACK" \
  --assessment needs-clarification \
  --summary "The change is sound, but the quoted wording needs an explicit answer." \
  --related-observation "$REVIEWER_OBSERVATION_ID" \
  --related-input-request "$INPUT_REQUEST_ID" \
  --format json
```

Keep the assessment id (the `assessmentId` field):

```sh
PROVISIONAL_ASSESSMENT_ID="assess:sha256:<the id from the assessment output>"
```

Refresh Review: Questions shows the open request, Call shows `needs-clarification`, and the open
question also appears under attention — outstanding judgment, not a gate.

## 6. Author Response

The author answers the question where it was asked. The durable answer lives on the request itself:

```sh
export POINTBREAK_ACTOR_ID="actor:agent:first-review-author"

pointbreak input-request respond "$INPUT_REQUEST_ID" \
  --outcome approved \
  --reason "Yes - the sentence is exactly what the guide should quote." \
  --format json
```

Context worth keeping beyond the answer goes into a follow-up observation on the author's track:

```sh
pointbreak observation add \
  --exact-revision "$REVISION_ID" \
  --track "$AUTHOR_TRACK" \
  --title "Quoted wording is final" \
  --body "The sentence is the exact wording the guide quotes; no further edit is planned." \
  --format json
```

Both writes change the durable review record, not the captured content — so nothing is recaptured
and the revision id does not change.

## 7. The Reviewer Replaces The Call

The provisional call is answered, but one obligation remains open, so the reviewer records it as an
explicit follow-up request before replacing the call:

```sh
export POINTBREAK_ACTOR_ID="actor:agent:first-review-reviewer"

pointbreak input-request open \
  --revision "$REVISION_ID" \
  --track "$REVIEWER_TRACK" \
  --title "Re-check the wording after the next guide edit" \
  --reason insufficient-evidence \
  --mode advisory \
  --body "If the guide sentence changes, re-run this walkthrough against the new wording." \
  --format json
```

Keep the new request id as `FOLLOW_UP_REQUEST_ID`, then replace the provisional call:

```sh
FOLLOW_UP_REQUEST_ID="input-request:sha256:<the id from the open output>"

pointbreak assessment add \
  --exact-revision "$REVISION_ID" \
  --track "$REVIEWER_TRACK" \
  --assessment accepted-with-follow-up \
  --summary "Accepted; the follow-up re-check remains open." \
  --replaces "$PROVISIONAL_ASSESSMENT_ID" \
  --related-input-request "$FOLLOW_UP_REQUEST_ID" \
  --format json
```

`--replaces` is the only thing that retires an earlier assessment. Refresh Review: Call shows
`accepted-with-follow-up` as current with `needs-clarification` visibly replaced — the history is
kept, not rewritten — and the follow-up stays visible under attention until someone resolves it.

## 8. Land The Commit On The Same Revision

The reviewed content is ready to land. Commit it, then record the commit as an association on the
existing revision:

```sh
git add onboarding.txt
git commit -m "docs: clarify first Review evidence"

export POINTBREAK_ACTOR_ID="actor:agent:first-review-author"
pointbreak association record \
  --revision "$REVISION_ID" \
  --track "$AUTHOR_TRACK" \
  --commit HEAD \
  --format json
```

Refresh Review one last time. The revision id is unchanged, the landing shows the exact commit, and
every fact recorded above still reads against the same revision. Committing already-reviewed
content is a landing, never new work: recapture only when the content itself changes. A commit that
lands what was reviewed is always an association on the same revision.

## Where To Go Next

- [CLI reference](cli-reference.md) lists commands, options, output schemas, and V1 limitations.
- [Review workflow](review-workflow.md) explains when to reach for capture, observations, input
  requests, assessments, history, and revision reads in a real review.
- [Storage model](storage-model.md) explains the durable record underneath: append-only review
  facts, immutable snapshots, and rebuildable projections.
- [Input request model](input-request-model.md) explains operative and advisory requests.
- [Assessment model](assessment-model.md) explains assessment values and replacement behavior.
- [Signing UX](signing-ux.md) explains automatic signing, the trust ladder, and enrollment.
- [Agent authoring](agent-authoring.md) explains how coding agents leave the same review record.
