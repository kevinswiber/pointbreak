#[test]
fn ci_workflow_runs_project_lint_and_tests() {
    let ci = std::fs::read_to_string(".github/workflows/ci.yml").expect("read CI workflow");

    assert!(ci.contains("name: CI"));
    assert!(ci.contains("branches: [main]"));
    assert!(ci.contains("pull_request:"));
    assert!(ci.contains("actions/checkout@v6"));
    assert!(ci.contains("dtolnay/rust-toolchain@stable"));
    assert!(ci.contains("dtolnay/rust-toolchain@nightly"));
    assert!(ci.contains("taiki-e/install-action@just"));
    assert!(ci.contains("taiki-e/install-action@nextest"));
    assert!(ci.contains("ubuntu-latest"));
    assert!(ci.contains("macos-latest"));
    assert!(ci.contains("windows-latest"));
    assert!(ci.contains("run: just lint"));
    assert!(ci.contains("run: just test-ci"));
}

#[test]
fn release_workflows_target_single_pointbreak_crate() {
    let release_plan =
        std::fs::read_to_string(".github/workflows/release-plan.yml").expect("read release plan");
    let release = std::fs::read_to_string(".github/workflows/release.yml").expect("read release");
    let release_script =
        std::fs::read_to_string("scripts/run-release-plan.sh").expect("read release script");
    let cog = std::fs::read_to_string("cog.toml").expect("read cog config");

    assert!(release_plan.contains("select(.name == \"pointbreak\")"));
    assert!(release_plan.contains("sort_by(.createdAt)"));
    assert!(release_plan.contains("RELEASE_COG_CONFIG"));
    assert!(release.contains("cargo publish -p pointbreak --locked"));
    assert!(release.contains("https://crates.io/api/v1/crates/pointbreak/${VERSION}"));
    assert!(release_script.contains("RELEASE_PLAN_REPO"));
    assert!(release_script.contains("remote get-url origin"));
    assert!(release_script.contains(
        r#"just --justfile "$REPO_ROOT/Justfile" --working-directory "$REPO_ROOT" package-archive-selftest"#
    ));
    let stale_repository = ["kevinswiber", "pointbreak"].join("/");
    assert!(!release_script.contains(&stale_repository));
    assert!(!release.contains("boardwalk"));
    assert!(cog.contains(r#""git commit --amend -m 'chore: v{{version}}'""#));
    assert!(cog.contains(r#""git tag -m 'v{{version}}' v{{version}}""#));
    assert!(!cog.contains("git tag -f"));
    assert!(cog.contains(r#""git push origin HEAD:main""#));
    assert!(cog.contains(r#""git push origin refs/tags/v{{version}}""#));
    assert!(!cog.contains("gh workflow run release.yml"));
    assert!(!cog.contains("gh workflow run release-binaries.yml"));
    assert!(cog.contains(r#"repository = "pointbreak""#));
    assert!(cog.contains(r#"owner = "withpointbreak""#));
    let stale_owner = ["owner = \"", "kevinswiber", "\""].join("");
    assert!(!cog.contains(&stale_owner));
}

#[test]
fn release_workflows_bind_the_reviewed_parent_and_keep_published_state_immutable() {
    let release_plan =
        std::fs::read_to_string(".github/workflows/release-plan.yml").expect("read release plan");
    let release = std::fs::read_to_string(".github/workflows/release.yml").expect("read release");
    let binaries = std::fs::read_to_string(".github/workflows/release-binaries.yml")
        .expect("read release binaries");
    let verify = std::fs::read_to_string(".github/workflows/verify-release.yml")
        .expect("read post-publish verification workflow");
    let helper =
        std::fs::read_to_string("scripts/run-release-plan.sh").expect("read release helper");
    let identity = std::fs::read_to_string("scripts/assert-release-identity.sh")
        .expect("read release identity assertion");
    let docs = std::fs::read_to_string("docs/releasing.md").expect("read release docs");

    assert!(release_plan.contains("expected_source_commit:"));
    assert!(release_plan.contains("required: true"));
    assert!(release_plan.contains("EXPECTED_SOURCE_COMMIT"));
    assert!(release_plan.contains("git rev-parse origin/main"));
    assert!(release_plan.contains("git rev-parse HEAD^"));
    assert!(release_plan.contains("git rev-parse \"refs/tags/${TAG}^{}\""));
    assert!(release_plan.contains("Existing tag conflict"));
    assert!(release_plan.contains("Existing crate conflict"));
    assert!(release_plan.contains("Existing GitHub release conflict"));
    assert!(release_plan.contains("token: ${{ secrets.RELEASE_PUSH_TOKEN }}"));
    assert!(release_plan.contains("select(.headSha == $expected)"));
    assert!(release_plan.contains("unexpected crates.io status"));
    assert!(release_plan.contains("pointbreak-release-workflow/1.0"));
    assert!(release_plan.contains("unexpected GitHub release status"));
    assert!(docs.contains("RELEASE_PUSH_TOKEN"));

    assert!(helper.contains("<plan|release> <version> --expected-source <full-sha>"));
    assert!(helper.contains("expected_source_commit=${EXPECTED_SOURCE_COMMIT}"));
    assert!(helper.contains("git -C \"$REPO_ROOT\" rev-parse origin/main"));

    for tag_only_workflow in [&release, &binaries] {
        assert!(!tag_only_workflow.contains("workflow_dispatch:"));
        assert!(tag_only_workflow.contains("fetch-depth: 0"));
    }
    assert!(release.contains("GitHub release already exists"));
    assert!(release.contains("unexpected crates.io status"));
    assert!(release.contains("unexpected GitHub release status"));
    assert!(!release.contains("already published; skipping"));
    assert!(binaries.contains("fetch-tags: true"));
    assert!(binaries.contains("scripts/assert-release-identity.sh"));
    assert!(binaries.contains("overwrite_files: false"));
    assert!(!binaries.contains("overwrite_files: true"));

    assert!(identity.contains(".build.source == \"git\""));
    assert!(identity.contains(".build.commit == $commit"));
    assert!(identity.contains(".build.describe == $tag"));
    assert!(identity.contains(".build.dirty == false"));

    for required in [
        "ubuntu-latest",
        "macos-latest",
        "windows-latest",
        "alpine",
        "scripts/install.sh",
        "scripts/install.ps1",
        "release-verification",
        "isPrerelease == $prerelease",
    ] {
        assert!(
            verify.contains(required),
            "missing verification term: {required}"
        );
    }
}

#[test]
fn changelog_has_cocogitto_insertion_separator() {
    let changelog = std::fs::read_to_string("CHANGELOG.md").expect("read changelog");

    assert!(
        changelog.lines().any(|line| line == "- - -"),
        "Cocogitto release bumps need the default insertion separator"
    );
}

#[test]
fn commit_check_workflow_reports_shore_examples() {
    let commit_check =
        std::fs::read_to_string(".github/workflows/commit-check.yml").expect("read commit check");

    assert!(commit_check.contains("name: Commit Check"));
    assert!(commit_check.contains("cog check \"${RANGE}\""));
    assert!(commit_check.contains("feat: add review unit discovery"));
    assert!(commit_check.contains("fix: correct input request projection"));
    assert!(!commit_check.contains("boardwalk"));
}
