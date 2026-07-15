use std::path::Path;

#[test]
fn readme_explains_package_command_split() {
    let readme = std::fs::read_to_string("README.md").expect("read README");

    assert!(readme.contains("cargo install pointbreak"));
    assert!(readme.contains("provides the `shore` command"));
    assert!(!readme.contains("cargo install shore\n"));
    assert!(!readme.contains("cargo install shore "));
}

#[test]
fn readme_has_release_badges_for_pointbreak() {
    let readme = std::fs::read_to_string("README.md").expect("read README");

    assert!(readme.contains("https://crates.io/crates/pointbreak"));
    assert!(readme.contains("https://img.shields.io/crates/v/pointbreak"));
    assert!(readme.contains("https://docs.rs/pointbreak"));
    assert!(readme.contains("https://docs.rs/pointbreak/badge.svg"));
    assert!(
        readme.contains("https://github.com/withpointbreak/pointbreak/actions/workflows/ci.yml")
    );
    assert!(readme.contains("actions/workflows/ci.yml/badge.svg"));
}

#[test]
fn cargo_metadata_points_to_pointbreak_repository() {
    let manifest = std::fs::read_to_string("Cargo.toml").expect("read Cargo manifest");

    assert!(manifest.contains(r#"homepage = "https://github.com/withpointbreak/pointbreak""#));
    assert!(manifest.contains(r#"repository = "https://github.com/withpointbreak/pointbreak""#));
}

#[test]
fn living_metadata_uses_the_canonical_organization_repository() {
    let stale_repository = ["kevinswiber", "pointbreak"].join("/");
    let canonical_repository = "withpointbreak/pointbreak";
    let paths = [
        ".github/ISSUE_TEMPLATE/config.yml",
        "CONTRIBUTING.md",
        "Cargo.toml",
        "README.md",
        "docs/adr/adr-0014-reviewunit-commit-range-lifecycle.md",
        "docs/id-prefixes.md",
        "docs/installation.md",
        "docs/storage-model.md",
        "extensions/vscode/package.json",
        "scripts/install-selftest.ps1",
        "scripts/install-selftest.sh",
        "scripts/install.ps1",
        "scripts/install.sh",
        "skills/README.md",
        "src/cli/inspect/web/test/css-coverage.test.ts",
    ];

    for path in paths {
        let contents = std::fs::read_to_string(path).unwrap_or_else(|error| {
            panic!("read {path}: {error}");
        });
        assert!(
            !contents.contains(&stale_repository),
            "{path} still uses the personal repository owner"
        );
        assert!(
            contents.contains(canonical_repository),
            "{path} does not name the canonical organization repository"
        );
    }
}

#[test]
fn vscode_metadata_keeps_its_identity_and_uses_canonical_support_urls() {
    let package = std::fs::read_to_string("extensions/vscode/package.json")
        .expect("read VS Code package manifest");

    assert!(package.contains(r#""publisher": "pointbreak""#));
    assert!(package.contains(r#""name": "pointbreak""#));
    assert!(package.contains("https://github.com/withpointbreak/pointbreak.git"));
    assert!(package.contains("https://github.com/withpointbreak/pointbreak/issues"));
    assert!(package.contains("https://github.com/withpointbreak/pointbreak#readme"));
}

#[test]
fn readme_drops_branded_hunk_origin_references() {
    let readme = std::fs::read_to_string("README.md").expect("read README");

    for stale in [
        "modem-dev/hunk",
        "kevinswiber/hunk",
        "docs/hunk-feedback.md",
        "Hunk is the practical inspiration",
        "real Hunk review session",
        "hunk fork",
    ] {
        assert!(
            !readme.contains(stale),
            "README still contains stale Hunk reference: {stale}"
        );
    }
    assert!(!Path::new("docs/hunk-feedback.md").exists());
}

#[test]
fn just_run_targets_the_shore_binary() {
    let justfile = std::fs::read_to_string("Justfile").expect("read Justfile");

    assert!(justfile.contains("cargo +stable run --bin shore --"));
}
