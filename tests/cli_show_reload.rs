mod support;

use support::shore;

#[test]
fn show_cli_help_lists_no_new_flags_after_reload_change() {
    let output = shore(["show", "--help"]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let help = String::from_utf8_lossy(&output.stdout);
    let known_flags = [
        "--repo",
        "--review-notes",
        "--log",
        "--log-format",
        "--log-file",
        "--help",
        "--version",
    ];
    let lines = help
        .lines()
        .filter(|line| line.trim_start().starts_with("--"))
        .collect::<Vec<_>>();

    for line in lines {
        let flag = line.split_whitespace().next().unwrap_or("");
        assert!(
            known_flags.iter().any(|known| flag.starts_with(known)),
            "unexpected flag in shore show --help after reload change: {flag}"
        );
    }
}
