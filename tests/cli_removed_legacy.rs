mod support;

use support::{dump_repo, shore};

struct RemovedPath {
    argv: &'static [&'static str],
    /// Substrings the successor hint must contain (empty = no hint expected).
    hint_contains: &'static [&'static str],
}

// Family/rename tasks APPEND rows here.
const REMOVED_PATHS: &[RemovedPath] = &[
    // Fully removed review verbs: unregistered with no verb-specific successor
    // (the retired-`review` family hint is asserted separately).
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
    RemovedPath {
        argv: &["review", "disposition", "--help"],
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
    RemovedPath {
        argv: &["review", "input-request", "open", "--help"],
        hint_contains: &["shore input-request"],
    },
    // The association grammar rewrite: compounds point at the new verbs, and the
    // family path points at the top-level family.
    RemovedPath {
        argv: &["review", "association", "associate-commit", "--help"],
        hint_contains: &["shore association record"],
    },
    RemovedPath {
        argv: &["review", "association", "associate-ref", "--help"],
        hint_contains: &["shore association record --ref"],
    },
    RemovedPath {
        argv: &["review", "association", "withdraw-commit", "--help"],
        hint_contains: &["shore association withdraw"],
    },
    RemovedPath {
        argv: &["review", "association", "withdraw-ref", "--help"],
        hint_contains: &["shore association withdraw"],
    },
    RemovedPath {
        argv: &["review", "association", "list", "--help"],
        hint_contains: &["shore association record|withdraw|list"],
    },
    // The old get-one verb `fetch` at the new top level points at `show`.
    RemovedPath {
        argv: &["input-request", "fetch", "--help"],
        hint_contains: &["shore input-request show"],
    },
    // The revision family: the verb-less plural and the digest both moved.
    RemovedPath {
        argv: &["review", "revisions", "--help"],
        hint_contains: &["shore revision list"],
    },
    RemovedPath {
        argv: &["review", "show", "--help"],
        hint_contains: &["shore revision show"],
    },
    // `identity enroll` renamed to `delegate` (the verb collided with `keys enroll`).
    RemovedPath {
        argv: &["identity", "enroll", "--help"],
        hint_contains: &["shore identity delegate <AGENT> --principal <P>"],
    },
];

#[test]
fn review_namespace_is_retired_with_a_hint() {
    for argv in [vec!["review"], vec!["review", "--help"]] {
        let out = shore(argv.clone());
        assert!(!out.status.success(), "{argv:?} should be unregistered");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("unrecognized subcommand"),
            "{argv:?} stderr:\n{err}"
        );
        // The bare-`review` leading-token hint points at the flattened surface.
        assert!(
            err.contains("flattened to the top level"),
            "{argv:?} stderr:\n{err}"
        );
    }
}

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
fn help_omits_legacy_hunk_input() {
    let dump = shore(["dump", "--help"]);
    let show = shore(["show", "--help"]);
    let notes_apply = shore(["notes", "apply", "--help"]);

    for output in [&dump, &show, &notes_apply] {
        assert!(
            output.status.success(),
            "stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    for (name, output) in [("dump", dump), ("show", show), ("notes apply", notes_apply)] {
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
}
