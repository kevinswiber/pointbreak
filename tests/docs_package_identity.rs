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
    assert!(readme.contains("https://github.com/kevinswiber/pointbreak/actions/workflows/ci.yml"));
    assert!(readme.contains("actions/workflows/ci.yml/badge.svg"));
}

#[test]
fn cargo_metadata_points_to_pointbreak_repository() {
    let manifest = std::fs::read_to_string("Cargo.toml").expect("read Cargo manifest");

    assert!(manifest.contains(r#"homepage = "https://github.com/kevinswiber/pointbreak""#));
    assert!(manifest.contains(r#"repository = "https://github.com/kevinswiber/pointbreak""#));
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
