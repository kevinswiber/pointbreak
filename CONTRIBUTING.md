# Contributing To Shoreline

Thanks for taking time to improve Shoreline. The project is still early, so small, focused patches
are easiest to review.

## Development Setup

Install a stable Rust toolchain, a nightly Rust toolchain for formatting, `just`, `cargo-nextest`,
and Cocogitto:

```bash
rustup toolchain install stable
rustup toolchain install nightly
cargo install cargo-nextest --locked
cargo install cocogitto --locked
```

Then install the repository hooks:

```bash
just setup-hooks
```

The hooks validate conventional commits and conventional branch names before changes leave your
machine.

## Common Commands

```bash
just build
just lint
just test
just check
just run --help
```

Use `just lint` before sending a patch that changes Rust code. Use `just test` for the normal test
suite. Use `just check` before opening or updating a pull request; it runs the commit check, build,
lint, and tests.

For targeted test work:

```bash
just test-file docs_open_source_readiness
cargo +stable test --test docs_open_source_readiness
```

## Branches

Branches use conventional branch names:

```text
feat/short-description
fix/short-description
hotfix/short-description
release/short-description
chore/short-description
```

Descriptions should use lowercase letters, numbers, and hyphens. For documentation-only work,
`chore/<description>` is the safe branch prefix currently accepted by the repository hook.

## Commits

Use conventional commits:

```text
docs: add getting started guide
fix: correct input request projection
feat: add review unit discovery
```

The commit subject should be lowercase, imperative, and no more than 100 characters. Do not end it
with a period.

Use an unscoped commit unless `cog.toml` grows an explicit scopes list. Today the scopes list is
empty, so scoped commits such as `docs(readme): ...` are rejected.

Check the current branch with:

```bash
cog check origin/main..HEAD
```

## Pull Requests

Keep pull requests narrow:

- describe what changed and why
- include the validation commands you ran
- update docs when user-facing behavior, command output, setup, or release process changes
- keep generated or unrelated files out of the diff
- avoid public references to private planning or local assistant workflow

CI runs formatting, linting, tests, and conventional commit checks across the supported runner
matrix.

## Project Shape

Shoreline is a Rust terminal review tool. Keep the headless review model authoritative and make the
TUI or other surfaces project from that model. Public command output JSON is the integration
surface; raw `.shore/` files are local storage details unless a command explicitly documents them.
