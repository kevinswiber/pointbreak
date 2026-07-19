# Development and validation guide

Pointbreak uses `just` as the maintainer entrypoint for development, verification, packaging, and
product-evidence commands. Run `just` or `just --list` to see recipes, and use
`just --list --group <name>` to focus on one surface.

The `Justfile` owns executable composition. This guide owns the decision about which gate applies to
a change and how to interpret its result. Script-level ownership and side effects live in
`scripts/README.md`.

## Start with the changed surface

| Change surface | Minimum local gate | Add when applicable | Expected outcome |
| --- | --- | --- | --- |
| Rust library, CLI, or headless behavior | `just check` | Focused `just test-file <name>` or `just test -E '<filter>'` while iterating | Commit range, build, format, Clippy, and nextest suite pass |
| Platform-conditional Rust code | `just check` | `just check-types` on relevant non-Linux hosts/CI | All workspace targets and feature-gated arms type-check |
| Inspector `web/src` | `just check`, `just web-check`, `just web-verify` | `just web-test` while iterating; `just web-build` when intentionally refreshing the bundle | Rust gate passes, front-end lint/types/tests pass, and committed `assets/app.js` matches source |
| VS Code extension | `just check`, `just extension-check` | `just extension-package` when packaging, binary selection, or extension delivery changes | Rust and extension checks pass; optional host VSIX contains the intended binary |
| GitHub Actions, binary targets, packaging, or release identity | `just workflow-lint`, `just package-archive-selftest` | `just release-bump-selftest` for Cocogitto/tag changes; `just installer-selftest` for acquisition changes | Workflow syntax and shell contracts pass without publication |
| Unix or Windows installer | `just installer-selftest` on the current host | Opposite-platform CI/live evidence required by `docs/releasing.md` | Hermetic acquisition, identity, upgrade, and rollback cases pass |
| Canonical Review example | `just review-example-verify` | Materialize into an empty repository when changing export/import behavior | Manifest, documents, projection identity, and source test agree |
| Review decision continuity | `just review-decision-browser-verify <empty-root>` | Inject the exact binary with `POINTBREAK_BINARY` for release evidence | Disposable canonical/synthetic stores pass browser behavior and viewport checks |
| Product screenshots or marketing capture | Appropriate capture recipe plus asset tests | Marketing synchronization/check workflow and real visual review | Captures, manifest, canonical example, and visible product state agree |
| Agent Skills | `just skills-validate` | `just skills-link` for a local installation check | Each skill validates against the pinned validator and links remain controlled |

These are minimum gates, not substitutes for a task-specific acceptance matrix. A change that crosses
surfaces inherits every affected row.

## What `just check` covers

`just check` runs, in order:

1. `commit-check` for the configured commit range;
2. a debug Rust build;
3. Rust formatting and Clippy across the workspace, all targets, and all features; and
4. the Rust nextest suite.

It deliberately does **not** install or run Node, build the Inspector bundle, check the VS Code
extension, lint workflows, exercise installers, or run real-browser evidence. Those surfaces have
separate prerequisites and failure meanings, so the repository does not provide a misleading
`check-all` recipe.

During an uncommitted first edit, `commit-check` may have no task commit to inspect. Run focused tests
while iterating, then run `just check` after creating the reviewable commit range or pass the intended
range to `just commit-check` explicitly.

The differential git-backend parity harness is a separate gate. Git access runs through a typed
backend seam (ADR-0040): the in-process `gix` backend ships in the default build (the `gix`
feature, on by default) and the qualified read/scalar classes route to it, while the capture diff
and write-tree stay on subprocess `git` permanently. `POINTBREAK_GIT_BACKEND=subprocess` is the
runtime escape hatch, and `--no-default-features` builds the subprocess-only backend. `just check`
covers the `gix` code (Clippy runs `--all-features`) but does not run the parity harness, which is
gated on `--features gix-parity` and exercised by `just git-parity` — and by a dedicated CI lane on
macOS and Windows. Run `just
git-parity` when you change the git seam or either backend; `just git-bench` prints the per-operation
subprocess-vs-gix win. See `docs/adr/adr-0040-git-backend-seam-and-hybrid.md`.

## Generated and protected artifacts

Some commands are intentionally mutating:

| Command | Writes | Rule |
| --- | --- | --- |
| `just fix` | Rust source formatting and Clippy fixes | Inspect every edit; it allows dirty/staged input |
| `just web-build` | Committed Inspector `assets/app.js` | Run only after editing web source; finish with `just web-verify` |
| `just extension-package` | Local VSIX/package output | Treat as disposable dogfood output unless a task explicitly preserves it |
| `just review-example-export …` | Canonical example output | Export from an explicit source repository, inspect the pack, then verify it |
| Screenshot capture recipes | PNG files and optional provenance manifest | Capture committed UI from the intended record; visually inspect both themes |
| `just migrate-store-common-dir …` | Pointbreak store placement | Non-destructive and idempotent, but still durable state; inspect the target repo first |

Freshness commands such as `just web-verify`, `just review-example-verify`, and marketing lock checks
are not regeneration instructions. If one fails, establish whether the derivative is stale or the
claimed source identity is wrong before writing anything.

## Release and publication boundary

The release self-tests are nonpublishing mechanics checks. They do not authorize or perform a public
release. Current commands, credentials, exact-parent requirements, and the explicit owner gate live
in `docs/releasing.md`.

In particular:

- `just package-archive-selftest` validates Cargo/package/archive mechanics locally;
- `just release-bump-selftest` validates Cocogitto and signed-tag mechanics in temporary repos;
- `just installer-selftest` validates the current host installer against local fixtures; and
- `just workflow-lint` validates workflow and shell contracts.

Public truth is established only by the owner-authorized release workflow followed by published-
release verification and any required installed-product or browser evidence.

## Interpreting failures

| Class | Typical signal | First response |
| --- | --- | --- |
| Prerequisite/environment | Command or tool missing, dependency not installed, browser unavailable, credentials rejected | Repair the environment and rerun; do not weaken the check |
| Stale generated artifact | `git diff --exit-code` reports a generated file changed | Regenerate through the owning command and inspect why the bytes changed |
| Identity/provenance mismatch | Commit, tag, digest, manifest, archive, installer, or binary fields disagree | Stop and reconcile the exact source; never edit the identity field in isolation |
| Contract drift | Schema/layout/target/transaction assertion fails | Decide whether implementation or reviewed contract is wrong, then change the owner |
| Behavior regression | Valid fixture reaches the product but semantic/browser assertion fails | Preserve evidence and debug the affected behavior |

Do not turn a fail-closed identity or protected-proof check into a write path merely to make it green.

## Prerequisites and setup

- Run `just setup-hooks` once per clone for Cocogitto commit and branch checks.
- Run `just web-install` before Inspector Node commands and `just extension-install` before VS Code
  extension commands.
- `just workflow-lint` requires `actionlint`, `shellcheck`, and `jq`.
- Browser evidence requires the repository's supported browser tooling and an empty disposable root.
- Commands that accept `POINTBREAK_BINARY` require an absolute executable path. Release evidence
  should inject the exact installed or exact-tag binary instead of rebuilding implicitly.

See `scripts/README.md` for script inventory, mutation boundaries, and script-specific failure
classes.
