# Bump to upgrade the agentskills.io validator; review the diff at https://github.com/agentskills/agentskills/compare/<old-sha>...<new-sha> before bumping.
export SKILLS_REF_REV := env_var_or_default("SKILLS_REF_REV", "5d4c1fda3f786fff826c7f56b6cb3341e7f3a911")

# List available recipes.
default:
    @just --list

# Run all tests.
test *args:
    cargo +stable nextest run --no-tests pass {{ args }}

# Run all tests (CI mode: no fail-fast, verbose).
test-ci *args:
    cargo +stable nextest run --profile ci --no-tests pass {{ args }}

# Run a specific test file (e.g. just test-file integration).
test-file name *args:
    cargo +stable nextest run --test {{ name }} {{ args }}

# Build (debug).
build *args:
    cargo +stable build {{ args }}

# Build (release).
release *args:
    cargo +stable build --release {{ args }}

# Self-test the release archive packaging script against a host debug build.
package-archive-selftest:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo +stable build --bin shore
    out="$(./scripts/package-release-archive.sh local 0.0.0 tar.gz target/debug)"
    if [ "$out" != "pointbreak-0.0.0-local.tar.gz" ]; then
        echo "wrong archive name: $out" >&2
        exit 1
    fi
    tar tzf "$out" | sort | diff - <(printf 'LICENSE\nNOTICE\nshore\n')
    rm -f "$out"
    echo "package-archive selftest ok"

# Lint GitHub Actions workflows, the packaging script, and the binary target manifest.
workflow-lint: workflow-actionlint workflow-lint-assertions

workflow-actionlint:
    actionlint

# Run the workflow checks not provided by reviewdog/action-actionlint in CI.
workflow-lint-assertions:
    #!/usr/bin/env bash
    set -euo pipefail
    shellcheck scripts/package-release-archive.sh
    expected="$(cat <<'EOF'
    [
      {"archive":"tar.gz","builder":"cargo","os":"macos-latest","rust-target":"x86_64-apple-darwin","target":"darwin-x64"},
      {"archive":"tar.gz","builder":"cargo","os":"macos-latest","rust-target":"aarch64-apple-darwin","target":"darwin-arm64"},
      {"archive":"tar.gz","builder":"zigbuild","os":"ubuntu-latest","rust-target":"x86_64-unknown-linux-gnu","target":"linux-x64"},
      {"archive":"tar.gz","builder":"zigbuild","os":"ubuntu-latest","rust-target":"aarch64-unknown-linux-gnu","target":"linux-arm64"},
      {"archive":"tar.gz","builder":"zigbuild","os":"ubuntu-latest","rust-target":"x86_64-unknown-linux-musl","target":"alpine-x64"},
      {"archive":"tar.gz","builder":"zigbuild","os":"ubuntu-latest","rust-target":"aarch64-unknown-linux-musl","target":"alpine-arm64"},
      {"archive":"zip","builder":"cargo","os":"windows-latest","rust-target":"x86_64-pc-windows-msvc","target":"win32-x64"},
      {"archive":"zip","builder":"cargo","os":"windows-latest","rust-target":"aarch64-pc-windows-msvc","target":"win32-arm64"}
    ]
    EOF
    )"
    jq -e --argjson expected "$expected" \
      'map(to_entries | sort_by(.key) | from_entries) == $expected' \
      .github/binary-targets.json > /dev/null
    for t in $(jq -r '.[].target' .github/binary-targets.json); do
      grep -q -- "$t" README.md || { echo "README missing target: $t" >&2; exit 1; }
    done
    echo "workflow-lint assertions ok"

# Run clippy and fmt check.
lint: fmt-check
    cargo +stable clippy --workspace --all-targets --all-features -- -D warnings

# Type-check all targets without the full clippy/fmt gate. Used by CI's non-Linux
# legs to keep the cfg(windows)/cfg(not(unix))/feature-gated arms compiled while
# paying the workspace+test compile only once. Linux runs the full `lint` gate.
check-types:
    cargo +stable check --workspace --all-targets --all-features

# Run clippy with auto-fix.
fix *args: fmt
    cargo +stable clippy --fix --workspace --all-targets --all-features --allow-dirty --allow-staged -- -D warnings {{ args }}

# Format code.
fmt *args:
    cargo +nightly fmt --all {{ args }}

fmt-check:
    cargo +nightly fmt --all -- --check

# Install git hooks (commit-msg and pre-push validation via cocogitto).
setup-hooks:
    cog install-hook --all --overwrite

# Symlink repo Agent Skills into project-local or user-level agent skill directories.
skills-link *args:
    ./scripts/link-agent-skills.sh {{ args }}

# Remove local symlinks for repo Agent Skills.
skills-unlink *args:
    ./scripts/link-agent-skills.sh unlink {{ args }}

# Validate repo Agent Skills with the pinned agentskills.io validator.
skills-validate:
    for skill in skills/*; do \
      [ -d "$skill" ] || continue; \
      [ -f "$skill/SKILL.md" ] || continue; \
      uvx --from "git+https://github.com/agentskills/agentskills@${SKILLS_REF_REV}#subdirectory=skills-ref" \
        skills-ref validate "$skill"; \
    done

# Check commit messages on the current branch.
commit-check range='origin/main..HEAD':
    cog check "{{ range }}"

# Run the CLI.
run *args:
    cargo +stable run --bin shore -- {{ args }}

# Fold a worktree-local .shore/data store into the common-dir store (.git/shore).
# Non-destructive + idempotent; refuses an ephemeral/sensitive worktree unless
# you pass include-ephemeral=true. This IS a shipped subcommand (shore store migrate).
migrate-store-common-dir repo="." include-ephemeral="false":
    cargo +stable run --bin shore -- store migrate --repo {{ repo }} \
        {{ if include-ephemeral == "true" { "--include-ephemeral" } else { "" } }}

# Check commit messages, compile, lint, and tests.
check: commit-check build lint test

# Install the inspector front-end dev toolchain (Node) from the committed lockfile.
web-install:
    cd src/cli/inspect/web && npm ci

# Node-only; intentionally NOT part of `just check` (the Rust gate stays Node-free). CI runs this
# as its own ubuntu leg.
# Front-end gate: Biome-lint the served app.js (lint-only) + Biome check (lint+format) the ported TS +
# strict tsc --noEmit + the vitest unit tests.
web-check:
    cd src/cli/inspect/web && npm run check

# Run the inspector front-end JS unit tests (vitest).
web-test:
    cd src/cli/inspect/web && npm run test

# Build the inspector front-end bundle (esbuild -> the committed assets/app.js). Run after editing
# web/src so the committed bundle stays fresh; the CI freshness gate fails a PR that forgets.
web-build:
    cd src/cli/inspect/web && npm run build

# Verify the committed inspector bundle is in sync with web/src (the CI freshness gate, run locally).
# Rebuilds the bundle and fails if it differs from the committed artifact.
web-verify:
    cd src/cli/inspect/web && npm run build && git diff --exit-code ../assets/app.js

# Refresh the dark/light Pointbreak Review screenshots embedded in README.md.
# Requires a running inspector; pass --url/--revision/--track to override the checked-in framing.
capture-inspector-screenshots *args:
    ./scripts/capture-inspector-screenshots.sh {{ args }}

# Refresh the product-owned marketing capture from the verified canonical Review example.
# Requires an inspector serving a materialized example repository.
capture-marketing-review-screenshots url="http://127.0.0.1:7878":
    ./scripts/capture-inspector-screenshots.sh --url {{ url }} --example-manifest examples/review/checkout-refactor/manifest.json --manifest assets/marketing/review-interface-capture.json --out-dir assets/marketing --hide-observations

# Export the canonical Review example from a source repository through public Pointbreak APIs.
review-example-export source output="examples/review/checkout-refactor":
    cargo +stable run --example review_example_pack -- export --repo {{ source }} --output {{ output }}

# Verify the checked canonical Review example pack without depending on store layout.
review-example-verify pack="examples/review/checkout-refactor":
    cargo +stable run --example review_example_pack -- verify --pack {{ pack }}

# Materialize the canonical Review example into an empty destination repository.
review-example-materialize output pack="examples/review/checkout-refactor":
    cargo +stable run --example review_example_pack -- materialize --pack {{ pack }} --output {{ output }}
