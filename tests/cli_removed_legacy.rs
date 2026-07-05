mod support;

use support::{dump_repo, shore};

struct RemovedPath {
    argv: &'static [&'static str],
    /// Substrings the successor hint must contain (empty = no hint expected).
    hint_contains: &'static [&'static str],
}

// Family/rename tasks APPEND rows here.
const REMOVED_PATHS: &[RemovedPath] = &[
    // Fully removed review verbs: unregistered, no successor hint.
    RemovedPath {
        argv: &["review", "publish", "--help"],
        hint_contains: &[],
    },
    RemovedPath {
        argv: &["review", "verdict", "--help"],
        hint_contains: &[],
    },
    RemovedPath {
        argv: &["review", "ack", "--help"],
        hint_contains: &[],
    },
    // Retired names that point at their post-reshape successors.
    RemovedPath {
        argv: &["review", "intervention", "--help"],
        hint_contains: &["shore input-request"],
    },
    RemovedPath {
        argv: &["review", "lineage", "--help"],
        hint_contains: &["shore capture --supersedes", "shore revision list"],
    },
    RemovedPath {
        argv: &["review", "unit", "--help"],
        hint_contains: &["shore revision list", "shore revision show"],
    },
    // Flattened families: `shore review <verb>` retired, points at `shore <verb>`.
    RemovedPath {
        argv: &["review", "capture", "--help"],
        hint_contains: &["shore capture"],
    },
    RemovedPath {
        argv: &["review", "history", "--help"],
        hint_contains: &["shore history"],
    },
    RemovedPath {
        argv: &["review", "endorse", "--help"],
        hint_contains: &["shore endorse"],
    },
    RemovedPath {
        argv: &["review", "observation", "add", "--help"],
        hint_contains: &["shore observation"],
    },
    RemovedPath {
        argv: &["review", "assessment", "add", "--help"],
        hint_contains: &["shore assessment"],
    },
    RemovedPath {
        argv: &["review", "validation", "add", "--help"],
        hint_contains: &["shore validation"],
    },
];

#[test]
fn removed_review_paths_are_unregistered_and_hint_at_successors() {
    for case in REMOVED_PATHS {
        let output = shore(case.argv.iter().copied());
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(!output.status.success(), "{:?} must be rejected", case.argv);
        assert!(
            stderr.contains("unrecognized subcommand"),
            "{:?} should be unregistered:\n{stderr}",
            case.argv
        );
        for needle in case.hint_contains {
            assert!(
                stderr.contains(needle),
                "{:?} hint missing {needle:?}:\n{stderr}",
                case.argv
            );
        }
    }
}

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
            !repo.path().join(".shore/data").exists(),
            "clap rejection must happen before any writer runs"
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
