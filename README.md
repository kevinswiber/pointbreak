# Shoreline

[![Crates.io](https://img.shields.io/crates/v/shoreline.svg)](https://crates.io/crates/shoreline)
[![Documentation](https://docs.rs/shoreline/badge.svg)](https://docs.rs/shoreline)
[![CI](https://github.com/kevinswiber/shoreline/actions/workflows/ci.yml/badge.svg)](https://github.com/kevinswiber/shoreline/actions/workflows/ci.yml)

Shoreline is a local terminal review tool for code changes that humans and coding agents work on
together. Capture a change, record observations and input requests, assess the review, and resume
from durable local state when the session continues later.

Install the `shoreline` crate; it provides the `shore` command:

```bash
cargo install shoreline
shore --help
```

## Quick Start

Start with the first-review walkthrough:

- [docs/getting-started.md](docs/getting-started.md)

The short path is:

```bash
cd path/to/git-worktree
shore review capture
shore review unit show --pretty
```

Then record what you learn:

```bash
shore review observation add --track human:local --title "Check error handling"
shore review input-request open --track human:local --title "Need decision" \
  --reason manual-decision-required --mode advisory
shore review assessment add --track human:local --assessment needs-clarification \
  --summary "Small change, but one decision is still open."
```

Shoreline stores local review facts in `.shore/`. Command output JSON is the integration surface;
raw event files, artifact paths, and `.shore/state.json` are internal storage details unless a
command explicitly documents them.

## Current Commands

The current executable surfaces are:

- `shore show`
- `shore dump`
- `shore review capture`
- `shore review observation add/list`
- `shore review input-request open/list/fetch/respond`
- `shore review assessment add/show`
- `shore review history`
- `shore review unit list/show`
- `shore notes apply`

See [docs/cli-reference.md](docs/cli-reference.md) for command options, output documents, schema
names, and V1 limitations.

## Documentation

For users:

- [Getting started](docs/getting-started.md) - first local review from a scratch Git repository.
- [CLI reference](docs/cli-reference.md) - commands, options, output JSON, and V1 boundaries.
- [Review workflow](docs/review-workflow.md) - when to use capture, observations, input requests,
  assessments, history, and unit show.

For contributors and maintainers:

- [CONTRIBUTING.md](CONTRIBUTING.md) - setup, hooks, branch names, commits, tests, and PR flow.
- [docs/releasing.md](docs/releasing.md) - release planning and publish automation.
- [docs/manual-testing.md](docs/manual-testing.md) - maintainer spot-check recipes.

Architecture and model notes:

- [docs/storage-model.md](docs/storage-model.md) - durable events, artifacts, and rebuildable
  projections.
- [docs/input-request-model.md](docs/input-request-model.md) - operative and advisory input
  requests.
- [docs/assessment-model.md](docs/assessment-model.md) - review assessments and replacements.
- [docs/adr/](docs/adr/) - architectural decision records.

## Project Status

Shoreline v0.1.0 is an experimental Rust-native review core. The package is named `shoreline`
because a shoreline is the boundary where tool-assisted changes meet human review. The installed
command stays `shore` because command names should remain short and practical.

The current focus is a headless, durable review model first:

- Git working-tree capture into a ReviewUnit
- append-only local events under `.shore/events/`
- immutable snapshot and note-body artifacts under `.shore/artifacts/`
- rebuildable projections and command-output JSON
- a read-only terminal view over the same model

## Design Principles

- Keep the review stream as a pure, headless data layer.
- Let rendering, scrolling, navigation, note placement, and session state derive from one explicit
  review model.
- Prefer direct Shoreline commands for durable review facts.
- Treat sidecars as import/transport adapters, not as the authoritative store.
- Keep public command output JSON stable enough for automation and tests.

## Non-Goals For V1

Shoreline should not start as:

- a general Git porcelain
- a complete review platform
- a web review UI
- a summarizer detached from code
- a daemon, notification system, or multi-session broker
- a terminal framework experiment

The narrow goal is a reliable local review surface for tool-assisted or review-heavy changesets.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request. The short validation path is:

```bash
just setup-hooks
just check
```

Security-sensitive reports should follow [SECURITY.md](SECURITY.md), not public issues.
