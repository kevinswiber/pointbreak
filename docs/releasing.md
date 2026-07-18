# Releasing

Pointbreak releases are driven from GitHub Actions through Cocogitto.

The published crate is `pointbreak`; it installs the `pointbreak` command. The crate source is
licensed Apache-2.0 through `Cargo.toml` and the repository `LICENSE` file. Preserve `NOTICE`
and `TRADEMARKS.md` so release artifacts keep the Pointbreak trademark reservation visible.

Release `0.7.0` is the hard executable, environment, and storage-name cutover described in
[ADR-0036](./adr/adr-0036-pointbreak-cli-storage-and-environment-cutover.md). Its notes must direct
operators to stop writers, move owner-controlled state offline, update environment/config references,
verify `pointbreak store paths` and readback, and use the inverse filesystem move for rollback. Do not
promise a fallback, compatibility alias, automatic migration, or migration command.

Use the **Release Plan** workflow in `plan` mode first. Supply an exact version and the full
40-character `origin/main` commit that has been reviewed and approved as the release parent. Plan
mode is side-effect-free: it reports that expected parent, the current checkout and recent CI,
the target version/tag/assets, the changelog preview, and any existing tag, crate, or GitHub Release
conflict. A missing, abbreviated, malformed, moved, or non-`origin/main` parent fails closed.

After checking the plan, explicitly authorize the named commit and version, then re-run the same
workflow in `release` mode with exactly the same inputs. Release mode fetches `origin/main` again
immediately before mutation. It stops if the parent moved or the target tag, crate, or release
already exists.

Release mode creates the Cocogitto version commit as the direct child of the approved parent, creates
an annotated tag that peels to that commit, and pushes both to `main`. The tag push is the sole
publication trigger. The **Release** workflow publishes the `pointbreak` crate to crates.io and creates
the GitHub Release. The **Release Binaries** workflow builds from clean exact-tag checkouts with full
tag history, verifies runnable binaries' build identity, and refuses to overwrite existing assets.

The **Release Binaries** workflow adds eight versioned archives and `checksums.txt` to that release.
The install scripts depend on those exact filenames and fail closed when an archive or checksum is
missing. They also require the binary to report a clean Git build, a full commit, and the exact
requested tag. The sole transition exception is the already-published v0.7.0 binary, which predates
the additive build field. Run `just installer-selftest` after changing installer or release-asset
behavior.

After the crate, release, and all assets exist, run **Verify Published Release** with the same expected
source parent and the new tag. It verifies the immutable tag/release/archive/checksum set and installer
digests, then performs fresh temporary-prefix acquisition on macOS, glibc Linux, musl Linux, and Windows
PowerShell 5.1. Each live row records the installed binary path, SHA-256, and full version document.

## Local helper

```sh
./scripts/run-release-plan.sh plan 0.8.0 --expected-source <full-origin-main-sha>
./scripts/run-release-plan.sh release 0.8.0 --expected-source <same-full-sha>
./scripts/run-release-verification.sh v0.8.0 --expected-source <same-full-sha>
```

Set `RELEASE_PLAN_DIR=.` to keep the downloaded `release-plan.md`.
Use `--output <directory>` with `run-release-verification.sh` to retain all platform reports.

## Required repository setup

GitHub repository settings:

- Actions workflow permissions must allow **Read and write permissions**.
- Install the release GitHub App on this repository with **Contents: Read and write** permission.
- Branch protection on `main` must accept the release workflow's signed, fast-forward Cocogitto
  version commit. The App does not need a bypass for deletion, signed-commit, or force-push rules.

Repository secrets:

- `CARGO_REGISTRY_TOKEN` - crates.io API token with publish access for `pointbreak`.
- `GPG_PRIVATE_KEY` - private key used by the Release Plan workflow to sign the Cocogitto version
  commit and tag.
- `RELEASE_APP_CLIENT_ID` - client ID for the installed release GitHub App.
- `RELEASE_APP_PRIVATE_KEY` - complete PEM private key for the release GitHub App. Release mode uses
  it to mint a short-lived, repository-scoped installation token with Contents write permission for
  the signed version commit and tag pushes. Unlike refs pushed with `GITHUB_TOKEN`, those pushes start
  the tag-only publication workflows.

No Homebrew, npm, or binary-asset secrets are needed for Pointbreak.

## Cocogitto Notes

For normal automatic releases, Cocogitto infers a major bump from a breaking-change conventional
commit such as `feat!:` or a commit with a `BREAKING CHANGE:` footer. Exact releases should use the
workflow `version` input instead of creating artificial breaking-change commits.

The CI release profile amends Cocogitto's generated version bump commit to an unscoped
`chore: v<version>` header before pushing and tagging. Keep that behavior while `cog.toml` has an
empty scopes list.
