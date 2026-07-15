//! The `--format`/`SHORE_FORMAT` output-lane selector across document-emitting
//! commands: precedence, the machine-lane byte contract, the interim text
//! fallback, and the hard error on an invalid env value.

mod support;

#[test]
fn format_json_pretty_preserves_the_document_shape() {
    let repo = support::dump_repo();
    let path = repo.path().to_str().unwrap();
    let via_format = support::shore(["history", "--repo", path, "--format", "json-pretty"]);
    let via_json = support::shore(["history", "--repo", path, "--format", "json"]);
    let pretty: serde_json::Value =
        serde_json::from_slice(&via_format.stdout).expect("pretty JSON parses");
    let compact: serde_json::Value =
        serde_json::from_slice(&via_json.stdout).expect("compact JSON parses");
    assert_eq!(pretty, compact);
    assert!(String::from_utf8_lossy(&via_format.stdout).starts_with("{\n"));
}

#[test]
fn identity_whoami_supports_pretty_json_without_changing_its_shape() {
    let repo = support::dump_repo();
    let path = repo.path().to_str().unwrap();
    let pretty = support::shore([
        "identity",
        "whoami",
        "--repo",
        path,
        "--format",
        "json-pretty",
    ]);
    let compact = support::shore(["identity", "whoami", "--repo", path, "--format", "json"]);
    let pretty_value: serde_json::Value = serde_json::from_slice(&pretty.stdout).unwrap();
    let compact_value: serde_json::Value = serde_json::from_slice(&compact.stdout).unwrap();
    assert_eq!(pretty_value, compact_value);
    assert!(String::from_utf8_lossy(&pretty.stdout).starts_with("{\n"));
}

#[test]
fn legacy_pretty_and_compact_flags_are_removed() {
    let repo = support::dump_repo();
    let path = repo.path().to_str().unwrap();

    for flag in ["--pretty", "--compact"] {
        let output = support::shore(["history", "--repo", path, flag]);
        assert!(!output.status.success(), "{flag} should be rejected");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(flag),
            "stderr should name {flag}:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn format_text_falls_back_to_indented_json_pre_digest() {
    let repo = support::dump_repo();
    let path = repo.path().to_str().unwrap();
    let output = support::shore(["history", "--repo", path, "--format", "text"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Pre-digest fallback: indented JSON (multi-line), same schema tag visible.
    assert!(stdout.lines().count() > 1);
    assert!(stdout.contains("pointbreak.review-history"));
}

#[test]
fn invalid_shore_format_is_a_hard_error() {
    let repo = support::dump_repo();
    let path = repo.path().to_str().unwrap();
    let output = support::shore_env(["history", "--repo", path], &[("SHORE_FORMAT", "bogus")]);
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
