# Releasing

Pointbreak releases are driven from GitHub Actions through Cocogitto.

The published crate is `pointbreak`; it installs the `shore` command. The crate is licensed
Apache-2.0 through `Cargo.toml` and the repository `LICENSE` file.

Use the **Release Plan** workflow in `plan` mode first. It reports the current commit, recent CI
status, the version Cocogitto will publish, and a changelog preview. For an exact release, set the
optional `version` input, for example `0.1.1`. After checking the plan, re-run the same workflow in
`release` mode with the same version input.

Release mode creates the Cocogitto version commit and tag, pushes both to `main`, and dispatches the
**Release** workflow for that tag. The Release workflow publishes the `pointbreak` crate to crates.io,
then creates the GitHub Release.

## Local helper

```sh
./scripts/run-release-plan.sh
./scripts/run-release-plan.sh plan 0.1.1
./scripts/run-release-plan.sh release 0.1.1
```

Set `RELEASE_PLAN_DIR=.` to keep the downloaded `release-plan.md`.

## Required repository setup

GitHub repository settings:

- Actions workflow permissions must allow **Read and write permissions**.
- Branch protection on `main` must allow this release workflow to push the Cocogitto version commit
  and tag, or the workflow must run with a token/account that is allowed to bypass the protection.

Repository secrets:

- `CARGO_REGISTRY_TOKEN` - crates.io API token with publish access for `pointbreak`.
- `GPG_PRIVATE_KEY` - private key used by the Release Plan workflow to sign the Cocogitto version
  commit and tag.

No Homebrew, npm, or binary-asset secrets are needed for Pointbreak.

## Cocogitto Notes

For normal automatic releases, Cocogitto infers a major bump from a breaking-change conventional
commit such as `feat!:` or a commit with a `BREAKING CHANGE:` footer. Exact releases should use the
workflow `version` input instead of creating artificial breaking-change commits.

The CI release profile amends Cocogitto's generated version bump commit to an unscoped
`chore: v<version>` header before pushing and tagging. Keep that behavior while `cog.toml` has an
empty scopes list.
