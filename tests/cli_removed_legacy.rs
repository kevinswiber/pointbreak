mod support;

use support::{dump_repo, shore};

#[test]
fn legacy_hunk_flag_is_rejected_without_shore_mutation() {
    for command in [vec!["dump"], vec!["show"], vec!["notes", "apply"]] {
        let repo = dump_repo();
        let mut args = command;
        args.extend([
            "--repo",
            repo.path().to_str().unwrap(),
            "--legacy-hunk-agent-context",
            "agent-context.json",
        ]);

        let output = shore(args);

        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("unexpected argument"),
            "stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !repo.path().join(".shore").exists(),
            "clap rejection must happen before any writer runs"
        );
    }
}

#[test]
fn removed_review_commands_are_unknown() {
    for command in ["publish", "verdict", "ack"] {
        let output = shore(["review", command, "--help"]);

        assert!(!output.status.success(), "{command} should be removed");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"),
            "stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn help_omits_legacy_hunk_and_removed_review_commands() {
    let dump = shore(["dump", "--help"]);
    let show = shore(["show", "--help"]);
    let notes_apply = shore(["notes", "apply", "--help"]);
    let review = shore(["review", "--help"]);

    for output in [&dump, &show, &notes_apply, &review] {
        assert!(
            output.status.success(),
            "stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    for (name, output) in [
        ("dump", dump),
        ("show", show),
        ("notes apply", notes_apply),
        ("review", review),
    ] {
        let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
        assert!(
            !stdout.contains("--legacy-hunk-agent-context"),
            "{name} help still mentions legacy Hunk input:\n{stdout}"
        );
        assert!(
            !stdout.contains("agent-context.json"),
            "{name} help still mentions agent-context.json:\n{stdout}"
        );
    }

    let review_stdout =
        String::from_utf8(shore(["review", "--help"]).stdout).expect("review help stdout is utf-8");
    for command in ["publish", "verdict", "ack"] {
        assert!(
            !review_stdout.contains(command),
            "review help still mentions {command}:\n{review_stdout}"
        );
    }
}
