# Pointbreak Review

[![Crates.io](https://img.shields.io/crates/v/pointbreak.svg)](https://crates.io/crates/pointbreak)
[![Documentation](https://docs.rs/pointbreak/badge.svg)](https://docs.rs/pointbreak)
[![CI](https://github.com/kevinswiber/pointbreak/actions/workflows/ci.yml/badge.svg)](https://github.com/kevinswiber/pointbreak/actions/workflows/ci.yml)

Pointbreak Review is a durable, local-first review record for code changes that humans and coding
agents build together. It is designed for the iteration that happens long before a pull request
opens, where you might guide one agent to author a change and another to review it.

Coding agents generate far more activity than anyone can follow. Rather than store or replay full
transcripts, Pointbreak keeps only the facts that move a review forward: what changed and why, the
open questions, and each assessment. It records them as an append-only log you can read in the
terminal, browse in a local web inspector, or consume as JSON.

Every fact carries the actor that asserted it, human or agent, and can be signed with an Ed25519 key.
Signing never blocks a write, but when a signature is present the record becomes tamper-evident, and
a reader can tell whether each fact is merely signed or bound to a trusted identity. See
[docs/signing-ux.md](docs/signing-ux.md).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/kevinswiber/pointbreak/main/assets/shore-inspector-dark.png">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/kevinswiber/pointbreak/main/assets/shore-inspector-light.png">
  <img alt="The Pointbreak Review inspector: a filterable event timeline with per-actor tracks and signature-trust badges, beside an event detail pane" src="https://raw.githubusercontent.com/kevinswiber/pointbreak/main/assets/shore-inspector-light.png" width="800">
</picture>

*Watching a review in the Pointbreak Review inspector opened by `shore inspect`: the event timeline, each fact attributed to its track, with signature-trust badges.*

Install the `pointbreak` crate; it provides the `shore` command:

```bash
cargo install pointbreak
shore --help
```

## Quick Start

Start with the first-review walkthrough:

- [docs/getting-started.md](docs/getting-started.md)

The short path is:

```bash
cd path/to/git-worktree
shore capture
shore revision show --format json-pretty
```

Then record what you learn:

```bash
shore observation add --track human:local --title "Check error handling"
shore input-request open --track human:local --title "Need decision" \
  --reason manual-decision-required --mode advisory
shore assessment add --track human:local --assessment needs-clarification \
  --summary "Small change, but one decision is still open."
```

In a real collaboration each actor records on its own track — the coding agent that authored the
change (`agent:codex`), a reviewer that is a human or another agent, and you (`human:local`) — so
every fact stays attributed to whoever asserted it. See
[the review workflow](docs/review-workflow.md) and
[agent authoring handoffs](docs/agent-authoring.md) for how the author and reviewer hand off.

Or browse the whole store visually — event timeline, per-revision pages, and annotated diffs — in
a local web UI:

```bash
shore inspect --open
```

Pointbreak stores local review facts in `.shore/data/`. Command output JSON is the integration surface;
raw event files, artifact paths, and `.shore/data/state.json` are internal storage details unless a
command explicitly documents them. Consumers that prefer to read and write those facts in process
can use the supported library API instead of the CLI — see [docs/library-api.md](docs/library-api.md).

## Commands

The `shore` command surface is still taking shape and will change before v1. See
[docs/cli-reference.md](docs/cli-reference.md) for the current commands, their options, output
documents, schema names, and V1 limitations.

## Agent Skills

Pointbreak ships a portable author-handoff skill under [skills/](skills/README.md). Install it with:

```bash
npx skills add kevinswiber/pointbreak
```

## Documentation

For users:

- [Getting started](docs/getting-started.md) - first local review from a scratch Git repository.
- [CLI reference](docs/cli-reference.md) - commands, options, output JSON, and V1 boundaries.
- [Review workflow](docs/review-workflow.md) - when to use capture, observations, input requests,
  assessments, history, and revision show.
- [Agent authoring handoffs](docs/agent-authoring.md) - how a coding agent captures a durable
  handoff record before declaring implementation work done.
- [Agent skills](skills/README.md) - install the portable Pointbreak author-handoff skill.
- [Library API](docs/library-api.md) - the supported in-process library surface (reads, attributed
  writes, event ingest, documents) and its stability contract.
- [Signing UX](docs/signing-ux.md) - human, agent, and CI signing flows and the
  unsigned/untrusted_key/valid verification ladder.

For contributors and maintainers:

- [CONTRIBUTING.md](CONTRIBUTING.md) - setup, hooks, branch names, commits, tests, and PR flow.
- [docs/releasing.md](docs/releasing.md) - release planning and publish automation.
- [docs/manual-testing.md](docs/manual-testing.md) - maintainer spot-check recipes.
- [TRADEMARKS.md](TRADEMARKS.md) - trademark use for names and logos.

Architecture and model notes:

- [docs/storage-model.md](docs/storage-model.md) - durable events, artifacts, and rebuildable
  projections.
- [docs/input-request-model.md](docs/input-request-model.md) - operative and advisory input
  requests.
- [docs/assessment-model.md](docs/assessment-model.md) - review assessments and replacements.
- [docs/adr/](docs/adr/) - architectural decision records.

## Project Status

Pointbreak Review is experimental and under active development. The published crate is `pointbreak`;
the installed command stays `shore` because command names should remain short and practical.

The current focus is a headless, durable review model first:

- Git working-tree or commit-range (`--base`) capture into a revision
- append-only local events under `.shore/data/events/`
- immutable snapshot and note-body artifacts under `.shore/data/artifacts/`
- rebuildable projections and command-output JSON
- read-only terminal and local web views over the same model

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request. The short validation path is:

```bash
just setup-hooks
just check
```

Security-sensitive reports should follow [SECURITY.md](SECURITY.md), not public issues.

## License And Trademarks

This repository's source code is licensed under Apache-2.0. See [LICENSE](LICENSE).

Pointbreak, Pointbreak Review, and the Pointbreak logo are trademarks of Kevin Swiber.
Trademark rights are reserved; see [NOTICE](NOTICE) and [TRADEMARKS.md](TRADEMARKS.md).
The private Pointbreak debugger codebase is not part of this Apache-2.0 repository unless
it is separately published under its own license.
