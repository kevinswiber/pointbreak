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
    assert!(ci.contains("run: just lint"));
    assert!(ci.contains("run: just test-ci"));
    assert!(!ci.contains("windows-latest"));
}

#[test]
fn release_workflows_target_single_shoreline_crate() {
    let release_plan =
        std::fs::read_to_string(".github/workflows/release-plan.yml").expect("read release plan");
    let release = std::fs::read_to_string(".github/workflows/release.yml").expect("read release");
    let cog = std::fs::read_to_string("cog.toml").expect("read cog config");

    assert!(release_plan.contains("select(.name == \"shoreline\")"));
    assert!(release.contains("cargo publish -p shoreline --locked"));
    assert!(release.contains("https://crates.io/api/v1/crates/shoreline/${VERSION}"));
    assert!(!release.contains("boardwalk"));
    assert!(cog.contains("git push origin v{{version}}"));
    assert!(cog.contains("gh workflow run release.yml -f tag=v{{version}}"));
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
