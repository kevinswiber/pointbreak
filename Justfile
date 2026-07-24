# Pointbreak maintainer entrypoints. Run `just --list` for grouped discovery and read
# `docs/development.md` for the change-to-gate matrix and failure interpretation.
# `just check` is intentionally Rust-only; Inspector, extension, release, and browser
# surfaces have separate groups and prerequisites.

# Bump to upgrade the agentskills.io validator; review the diff at https://github.com/agentskills/agentskills/compare/<old-sha>...<new-sha> before bumping.
export SKILLS_REF_REV := env_var_or_default("SKILLS_REF_REV", "5d4c1fda3f786fff826c7f56b6cb3341e7f3a911")

# Recipes are written for a POSIX shell. Git for Windows bundles one but does not
# put it on PATH, so name it explicitly (the default install location, which the
# GitHub windows runners share). Without this, every recipe fails from
# PowerShell/cmd with "could not find the shell `sh`".
set windows-shell := ["C:/Program Files/Git/bin/sh.exe", "-cu"]

# Rustup users select toolchains explicitly. The Fenix Nix shell overrides both
# commands with direct `cargo`, whose compiler is stable and formatter is nightly.
cargo_stable := env_var_or_default("POINTBREAK_CARGO_STABLE", "cargo +stable")
cargo_nightly := env_var_or_default("POINTBREAK_CARGO_NIGHTLY", "cargo +nightly")

# Host executable suffix: `.exe` on Windows, empty elsewhere. Mirrors the name the
# extension packager derives from .github/binary-targets.json, so the path handed to
# it in `build-all` actually exists on disk.
bin_ext := if os_family() == "windows" { ".exe" } else { "" }

# List available recipes.
[group('help')]
default:
    @just --list

# Run all tests.
[group('core')]
test *args:
    {{ cargo_stable }} nextest run --no-tests pass {{ args }}

# Run all tests (CI mode: no fail-fast, verbose).
[group('core')]
test-ci *args:
    {{ cargo_stable }} nextest run --profile ci --no-tests pass {{ args }}

# Run a specific test file (e.g. just test-file integration).
[group('core')]
test-file name *args:
    {{ cargo_stable }} nextest run --test {{ name }} {{ args }}

# Run the differential subprocess-vs-gix git-backend parity harness (report-only).
[group('core')]
git-parity *args:
    {{ cargo_stable }} nextest run --features gix-parity -E 'test(git_backend_parity)' {{ args }}

# Per-op subprocess-vs-gix microbench behind the read-class flips (gix-parity
# feature; separate from the `bench` feature). Prints the measured per-op win.
[group('core')]
git-bench *args:
    {{ cargo_stable }} nextest run --features gix-parity -E 'test(git_backend_microbench)' --no-capture {{ args }}

# Build (debug).
[group('core')]
build *args:
    {{ cargo_stable }} build {{ args }}

# Build an optimized binary without publishing it.
[group('core')]
release *args:
    {{ cargo_stable }} build --release {{ args }}

# Reject a build profile that is not exactly `debug` or `release`, before any
# dependency runs. Kept private so it stays out of `just --list`.
[private]
_require-build-profile profile:
    @[ "{{ profile }}" = "debug" ] || [ "{{ profile }}" = "release" ] || { echo "build-all: profile must be 'debug' or 'release', got '{{ profile }}'" >&2; exit 2; }

# Build every locally shippable surface for dogfood: the Inspector bundle, the CLI
# binary, and a platform-targeted VS Code VSIX (with that freshly built binary
# bundled in) written to target/vsix/<target>/<profile>/. Profile must be `debug`
# or `release` (default `release`).
[group('core')]
build-all profile="release": (_require-build-profile profile) web-install web-build extension-install
    {{ cargo_stable }} build {{ if profile == "release" { "--release" } else { "" } }}
    POINTBREAK_EXTENSION_PROFILE={{ profile }} POINTBREAK_EXTENSION_BINARY="{{ justfile_directory() }}/target/{{ profile }}/pointbreak{{ bin_ext }}" just extension-package

# Self-test Cargo installation and all release archive layouts without publishing.
[group('release')]
package-archive-selftest:
    ./scripts/package-release-selftest.sh

# Reproduce Cocogitto's native tag lifecycle and the guarded signed-tag finalizer.
[group('release')]
release-bump-selftest:
    ./scripts/finalize-cocogitto-release-tag-selftest.sh

# Exercise the release installer for the current host platform without network access.
[group('release')]
installer-selftest:
    {{ if os() == "windows" { "powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/install-selftest.ps1" } else { "./scripts/install-selftest.sh" } }}

# Lint GitHub Actions workflows, the packaging script, and the binary target manifest.
[group('release')]
workflow-lint: workflow-actionlint workflow-lint-assertions

# Run actionlint against GitHub Actions workflows.
[group('release')]
workflow-actionlint:
    actionlint

# Run the workflow checks not provided by reviewdog/action-actionlint in CI.
[group('release')]
workflow-lint-assertions:
    #!/usr/bin/env bash
    set -euo pipefail
    shellcheck \
      scripts/package-release-archive.sh \
      scripts/package-release-selftest.sh \
      scripts/verify-release-archives.sh \
      scripts/install.sh \
      scripts/install-selftest.sh \
      scripts/assert-release-identity.sh \
      scripts/assert-release-identity-selftest.sh \
      scripts/finalize-cocogitto-release-tag.sh \
      scripts/finalize-cocogitto-release-tag-selftest.sh \
      scripts/run-release-plan.sh \
      scripts/run-release-verification.sh
    ./scripts/assert-release-identity-selftest.sh
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
    grep -Fq -- 'expected_source_commit' .github/workflows/release-plan.yml
    grep -Fq -- 'overwrite_files: false' .github/workflows/release-binaries.yml
    grep -Fq -- 'shell: powershell' .github/workflows/verify-release.yml
    grep -Fq -- 'alpine:3.22' .github/workflows/verify-release.yml
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

# Run Rust formatting checks and Clippy across all targets and features.
[group('quality')]
lint: fmt-check
    {{ cargo_stable }} clippy --workspace --all-targets --all-features -- -D warnings

# Type-check all targets without the full clippy/fmt gate. Used by CI's non-Linux
# legs to keep the cfg(windows)/cfg(not(unix))/feature-gated arms compiled while
# paying the workspace+test compile only once. Linux runs the full `lint` gate.
# Type-check all workspace targets and features without the full lint gate.
[group('core')]
check-types:
    {{ cargo_stable }} check --workspace --all-targets --all-features

# Run clippy with auto-fix.
[group('quality')]
fix *args: fmt
    {{ cargo_stable }} clippy --fix --workspace --all-targets --all-features --allow-dirty --allow-staged -- -D warnings {{ args }}

# Format code.
[group('quality')]
fmt *args:
    {{ cargo_nightly }} fmt --all {{ args }}

# Check Rust formatting without writing files.
[group('quality')]
fmt-check:
    {{ cargo_nightly }} fmt --all -- --check

# Format Nix files with the canonical RFC-166 formatter. Requires Nix.
[group('nix')]
nix-fmt:
    #!/usr/bin/env bash
    set -euo pipefail
    nix run nixpkgs#nixfmt -- $(git ls-files '*.nix')

# Lint and format-check Nix files: nixfmt, statix, deadnix, and `nix flake check`.
# Requires Nix. Deliberately separate from `just lint`/`check`, which stay
# Rust-only so contributors without Nix (mise/manual) can run the core gate.
[group('nix')]
nix-check:
    #!/usr/bin/env bash
    set -euo pipefail
    files=$(git ls-files '*.nix')
    nix run nixpkgs#nixfmt -- --check $files
    nix run nixpkgs#statix -- check .
    nix run nixpkgs#deadnix -- --fail .
    nix flake check

# EXPERIMENTAL: cross-compile a cargo-nextest archive for a Windows msvc target from
# this Linux/macOS host, to run on a real Windows machine (the archive carries prebuilt
# test binaries; the Windows side needs no Rust toolchain). Run inside the Nix
# windows-cross shell: `nix develop .#windows-cross -c just windows-cross-archive`.
# cargo-xwin downloads the MSVC CRT/SDK on first use. See ci-nix-windows-spike.yml.
[group('nix')]
windows-cross-archive target="x86_64-pc-windows-msvc":
    #!/usr/bin/env bash
    set -euo pipefail
    out="target/nextest/pointbreak-{{ target }}.tar.zst"
    mkdir -p "$(dirname "$out")"
    # cargo-xwin emits the per-target CC/AR/linker/lib-search env nextest needs to
    # build (and link) the Windows test binaries; eval it, then archive.
    eval "$(cargo-xwin env --target {{ target }})"
    cargo nextest archive --target {{ target }} --archive-file "$out"
    echo "wrote $out"

# Install git hooks (commit-msg and pre-push validation via cocogitto).
[group('maintenance')]
setup-hooks:
    cog install-hook --all --overwrite

# Symlink repo Agent Skills into project-local or user-level agent skill directories.
[group('skills')]
skills-link *args:
    ./scripts/link-agent-skills.sh {{ args }}

# Remove local symlinks for repo Agent Skills.
[group('skills')]
skills-unlink *args:
    ./scripts/link-agent-skills.sh unlink {{ args }}

# Validate repo Agent Skills with the pinned agentskills.io validator.
[group('skills')]
skills-validate:
    for skill in skills/*; do \
      [ -d "$skill" ] || continue; \
      [ -f "$skill/SKILL.md" ] || continue; \
      uvx --from "git+https://github.com/agentskills/agentskills@${SKILLS_REF_REV}#subdirectory=skills-ref" \
        skills-ref validate "$skill"; \
    done

# Check conventional commits in the selected range.
[group('quality')]
commit-check range='origin/main..HEAD':
    cog check "{{ range }}"

# Run the CLI.
[group('core')]
run *args:
    {{ cargo_stable }} run --bin pointbreak -- {{ args }}

# Fold a worktree-local .pointbreak/data store into the Git-common-dir pointbreak store.
# Non-destructive + idempotent; refuses an ephemeral/sensitive worktree unless
# you pass include-ephemeral=true. This IS a shipped subcommand (pointbreak store migrate).
# Migrate a worktree-local store into the Git-common-dir store without deleting the source.
[group('maintenance')]
migrate-store-common-dir repo="." include-ephemeral="false":
    {{ cargo_stable }} run --bin pointbreak -- store migrate --repo {{ repo }} \
        {{ if include-ephemeral == "true" { "--include-ephemeral" } else { "" } }}

# Run the complete Rust gate: commit check, build, lint, and tests.
[group('quality')]
check: commit-check build lint test

# Run the deterministic cross-candidate fault and native-platform matrix. This
# uses only disposable roots and records raw samples without timing thresholds.
[group('quality')]
store-foundation-qualification-smoke:
    {{ cargo_stable }} bench --features bench --bench store_foundation -- --qualification-smoke

# Run the developer evidence lane with repeated raw performance samples. This
# remains environment evidence rather than a default-test timing gate.
[group('quality')]
store-foundation-qualification:
    {{ cargo_stable }} bench --features bench --bench store_foundation -- --qualification-evidence

# Print and validate the public longitudinal workload and capacity contracts.
[group('quality')]
longitudinal-contract:
    {{ cargo_stable }} bench --locked --features bench --bench store_foundation -- --longitudinal-contract

# Exercise disposable longitudinal construction, pair, preflight, and package mechanics without timing.
[group('quality')]
longitudinal-smoke:
    {{ cargo_stable }} bench --locked --features bench --bench store_foundation -- --longitudinal-smoke

# Recursively verify one completed longitudinal raw-evidence package without editing it.
[group('quality')]
longitudinal-verify-package root:
    {{ cargo_stable }} bench --locked --features bench --bench store_foundation -- \
        --longitudinal-verify-package --longitudinal-package-root="{{ root }}"

# Install the Visual Studio Code extension toolchain from its committed lockfile.
[group('extension')]
extension-install:
    cd extensions/vscode && npm ci

# Check the VS Code extension; intentionally separate from the Rust-only `just check`.
[group('extension')]
extension-check:
    cd extensions/vscode && npm run check

# Build a platform-targeted VSIX with its matching pointbreak binary for local
# dogfood, written to target/vsix/<target>/<profile>/. Honors POINTBREAK_EXTENSION_*
# (BINARY, PROFILE, CLEAN_VERSION); `build-all` sets BINARY and PROFILE for you.
[group('extension')]
extension-package:
    node extensions/vscode/scripts/package-local.mjs

# Install the inspector front-end dev toolchain (Node) from the committed lockfile.
[group('web')]
web-install:
    cd src/cli/inspect/web && npm ci

# Node-only; intentionally NOT part of `just check` (the Rust gate stays Node-free). CI runs this
# as its own ubuntu leg.
# Front-end gate: Biome-lint the served app.js (lint-only) + Biome check (lint+format) the ported TS +
# strict tsc --noEmit + the vitest unit tests.
# Run the Inspector front-end lint, format, type, and unit-test gate.
[group('web')]
web-check:
    cd src/cli/inspect/web && npm run check

# Run the inspector front-end JS unit tests (vitest).
[group('web')]
web-test:
    cd src/cli/inspect/web && npm run test

# Build the inspector front-end bundle (esbuild -> the committed assets/app.js). Run after editing
# web/src so the committed bundle stays fresh; the CI freshness gate fails a PR that forgets.
# Rebuild the committed Inspector bundle after changing its web source.
[group('web')]
web-build:
    cd src/cli/inspect/web && npm run build

# Verify the committed inspector bundle is in sync with web/src (the CI freshness gate, run locally).
# Rebuilds the bundle and fails if it differs from the committed artifact.
# Verify that the committed Inspector bundle matches its source without accepting drift.
[group('web')]
web-verify:
    cd src/cli/inspect/web && npm run build && git diff --exit-code ../assets/app.js

# Refresh the dark/light Pointbreak Review screenshots embedded in README.md.
# Requires a running inspector; pass --url/--revision/--track to override the checked-in framing.
# Refresh README Review screenshots from an explicitly selected running Inspector.
[group('review-evidence')]
capture-inspector-screenshots *args:
    ./scripts/capture-inspector-screenshots.sh {{ args }}

# Refresh the product-owned marketing capture from the verified canonical Review example.
# Requires an inspector serving a materialized example repository.
# Refresh the product-owned marketing capture and provenance manifest from the canonical example.
[group('review-evidence')]
capture-marketing-review-screenshots url="http://127.0.0.1:7878":
    ./scripts/capture-inspector-screenshots.sh --url {{ url }} --example-manifest examples/review/checkout-refactor/manifest.json --manifest assets/marketing/review-interface-capture.json --out-dir assets/marketing --hide-observations

# Export the canonical Review example from a source repository through public Pointbreak APIs.
[group('review-evidence')]
review-example-export source output="examples/review/checkout-refactor":
    {{ cargo_stable }} run --example review_example_pack -- export --repo {{ source }} --output {{ output }}

# Verify the checked canonical Review example pack without depending on store layout.
[group('review-evidence')]
review-example-verify pack="examples/review/checkout-refactor":
    {{ cargo_stable }} run --example review_example_pack -- verify --pack {{ pack }}

# Materialize the canonical Review example into an empty destination repository.
[group('review-evidence')]
review-example-materialize output pack="examples/review/checkout-refactor":
    {{ cargo_stable }} run --example review_example_pack -- materialize --pack {{ pack }} --output {{ output }}

# Materialize the Inspector decision-continuity matrix into an empty, isolated repository.
[group('review-evidence')]
review-decision-matrix-materialize output:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "${POINTBREAK_BINARY:-}" ]; then
      ./scripts/materialize-inspector-decision-matrix.sh "{{ output }}"
    else
      {{ cargo_stable }} build --bin pointbreak
      POINTBREAK_BINARY="$PWD/target/debug/pointbreak" \
        ./scripts/materialize-inspector-decision-matrix.sh "{{ output }}"
    fi

# Materialize both Review evidence stores and verify decision continuity in a real browser.
[group('review-evidence')]
review-decision-browser-verify root:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "${POINTBREAK_BINARY:-}" ]; then
      ./scripts/verify-inspector-decision-continuity.sh --root "{{ root }}"
    else
      {{ cargo_stable }} build --bin pointbreak
      POINTBREAK_BINARY="$PWD/target/debug/pointbreak" \
        ./scripts/verify-inspector-decision-continuity.sh --root "{{ root }}"
    fi
