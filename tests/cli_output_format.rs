//! The `--format`/`SHORE_FORMAT` output-lane selector across document-emitting
//! commands: precedence, the machine-lane byte contract, the interim text
//! fallback, and the hard error on an invalid env value.

mod support;

#[test]
fn format_json_pretty_matches_legacy_pretty() {
    let repo = support::dump_repo();
    let path = repo.path().to_str().unwrap();
    let via_format = support::shore(["dump", "--repo", path, "--format", "json-pretty"]);
    let via_legacy = support::shore(["dump", "--repo", path, "--pretty"]);
    assert_eq!(via_format.stdout, via_legacy.stdout);
}

#[test]
fn format_text_falls_back_to_indented_json_pre_digest() {
    let repo = support::dump_repo();
    let path = repo.path().to_str().unwrap();
    let output = support::shore(["dump", "--repo", path, "--format", "text"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Pre-digest fallback: indented JSON (multi-line), same schema tag visible.
    assert!(stdout.lines().count() > 1);
    assert!(stdout.contains("shore.dump"));
}

#[test]
fn invalid_shore_format_is_a_hard_error() {
    let repo = support::dump_repo();
    let path = repo.path().to_str().unwrap();
    let output = support::shore_env(["dump", "--repo", path], &[("SHORE_FORMAT", "bogus")]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("SHORE_FORMAT"));
}

#[test]
fn write_acks_accept_format_json() {
    // A write-ack command that previously had NO format flags accepts --format json
    // and behaves as the flag-less invocation does.
    let repo = support::dump_repo();
    let with_flag = support::shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    let repo2 = support::dump_repo();
    let without = support::shore(["capture", "--repo", repo2.path().to_str().unwrap()]);
    assert_eq!(with_flag.status.success(), without.status.success());
}
