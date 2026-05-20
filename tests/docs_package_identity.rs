use std::path::Path;

#[test]
fn readme_explains_package_command_split() {
    let readme = std::fs::read_to_string("README.md").expect("read README");

    assert!(readme.contains("cargo install shoreline"));
    assert!(readme.contains("provides the `shore` command"));
    assert!(!readme.contains("cargo install shore\n"));
    assert!(!readme.contains("cargo install shore "));
}

#[test]
fn readme_has_release_badges_for_shoreline() {
    let readme = std::fs::read_to_string("README.md").expect("read README");

    assert!(readme.contains("https://crates.io/crates/shoreline"));
    assert!(readme.contains("https://img.shields.io/crates/v/shoreline"));
    assert!(readme.contains("https://docs.rs/shoreline"));
    assert!(readme.contains("https://docs.rs/shoreline/badge.svg"));
    assert!(readme.contains("https://github.com/kevinswiber/shore/actions/workflows/ci.yml"));
    assert!(readme.contains("actions/workflows/ci.yml/badge.svg"));
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
