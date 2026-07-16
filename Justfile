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

# Self-test Cargo installation and all release archive layouts without publishing.
package-archive-selftest:
    ./scripts/package-release-selftest.sh

# Exercise the release installer for the current host platform without network access.
installer-selftest:
    {{ if os() == "windows" { "pwsh -NoLogo -NoProfile -File scripts/install-selftest.ps1" } else { "./scripts/install-selftest.sh" } }}

# Lint GitHub Actions workflows, the packaging script, and the binary target manifest.
workflow-lint: workflow-actionlint workflow-lint-assertions

workflow-actionlint:
    actionlint

# Run the workflow checks not provided by reviewdog/action-actionlint in CI.
workflow-lint-assertions:
    #!/usr/bin/env bash
    set -euo pipefail
    shellcheck \
      scripts/package-release-archive.sh \
      scripts/package-release-selftest.sh \
      scripts/verify-release-archives.sh \
      scripts/install.sh \
      scripts/install-selftest.sh
    expected="$(cat <<'EOF'
    [
      {"archive":"tar.gz","builder":"cargo","executable":"pointbreak","os":"macos-latest","rust-target":"x86_64-apple-darwin","target":"darwin-x64"},
      {"archive":"tar.gz","builder":"cargo","executable":"pointbreak","os":"macos-latest","rust-target":"aarch64-apple-darwin","target":"darwin-arm64"},
      {"archive":"tar.gz","builder":"zigbuild","executable":"pointbreak","os":"ubuntu-latest","rust-target":"x86_64-unknown-linux-gnu","target":"linux-x64"},
      {"archive":"tar.gz","builder":"zigbuild","executable":"pointbreak","os":"ubuntu-latest","rust-target":"aarch64-unknown-linux-gnu","target":"linux-arm64"},
      {"archive":"tar.gz","builder":"zigbuild","executable":"pointbreak","os":"ubuntu-latest","rust-target":"x86_64-unknown-linux-musl","target":"alpine-x64"},
      {"archive":"tar.gz","builder":"zigbuild","executable":"pointbreak","os":"ubuntu-latest","rust-target":"aarch64-unknown-linux-musl","target":"alpine-arm64"},
      {"archive":"zip","builder":"cargo","executable":"pointbreak.exe","os":"windows-latest","rust-target":"x86_64-pc-windows-msvc","target":"win32-x64"},
      {"archive":"zip","builder":"cargo","executable":"pointbreak.exe","os":"windows-latest","rust-target":"aarch64-pc-windows-msvc","target":"win32-arm64"}
    ]
    EOF
    )"
    jq -e --argjson expected "$expected" \
      'length == 8
       and (map(.target) | unique | length) == 8
       and (map(."rust-target") | unique | length) == 8
       and map(to_entries | sort_by(.key) | from_entries) == $expected' \
      .github/binary-targets.json > /dev/null
    grep -Fq -- '--bin pointbreak' .github/workflows/release-binaries.yml
    grep -Fq -- 'verify-release-archives.sh' .github/workflows/release-binaries.yml
    grep -Fq -- 'package-archive-selftest' .github/workflows/release-plan.yml
    grep -Fq -- 'package-archive-selftest' .github/workflows/release.yml
    grep -Fq -- 'package-archive-selftest' scripts/run-release-plan.sh
    if rg -n 'shore(\.exe)?|--bin shore' \
      .github/binary-targets.json \
      .github/workflows/release-binaries.yml \
      .github/workflows/release-plan.yml \
      .github/workflows/release.yml \
      scripts/package-release-archive.sh \
      scripts/run-release-plan.sh; then
        echo "release surfaces still reference the retired executable" >&2
        exit 1
    fi
    for t in $(jq -r '.[].target' .github/binary-targets.json); do
      grep -q -- "$t" docs/installation.md || { echo "installation docs missing target: $t" >&2; exit 1; }
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
    cargo +stable run --bin pointbreak -- {{ args }}

# Fold a worktree-local .pointbreak/data store into the Git-common-dir pointbreak store.
# Non-destructive + idempotent; refuses an ephemeral/sensitive worktree unless
# you pass include-ephemeral=true. This IS a shipped subcommand (pointbreak store migrate).
migrate-store-common-dir repo="." include-ephemeral="false":
    cargo +stable run --bin pointbreak -- store migrate --repo {{ repo }} \
        {{ if include-ephemeral == "true" { "--include-ephemeral" } else { "" } }}

# Check commit messages, compile, lint, and tests.
check: commit-check build lint test

# Install the Visual Studio Code extension toolchain from its committed lockfile.
extension-install:
    cd extensions/vscode && npm ci

# Node-only; intentionally not part of `just check` so the Rust gate stays Node-free.
extension-check:
    cd extensions/vscode && npm run check

# Build a host-only VSIX with its matching pointbreak binary for local dogfood.
extension-package:
    node extensions/vscode/scripts/package-local.mjs

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
