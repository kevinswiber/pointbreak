//! Drift guard: every clap leaf command path must be documented in
//! `docs/cli-reference.md`, and the flags called out in the CLI-audit hygiene
//! issue must appear. Command paths are stable and few; a full every-flag guard
//! is deliberately out of scope while the flag surface churns (a whole-surface
//! flag guard would need a large, brittle allow-list of intentionally
//! undocumented plumbing flags). This catches the headline drift: a whole
//! command family shipping undocumented.

use clap::CommandFactory;

const REFERENCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/docs/cli-reference.md"
));

const REFERENCE_COMMAND: &str = "pointbreak";

/// The audit's named flag gaps — the concrete regression set this guard pins
/// without a churny whole-surface flag allow-list.
const REQUIRED_FLAGS: &[&str] = &[
    "--limit",
    "--cursor",
    "--watch",
    "--poll-ms",
    "--by", // history
    "--integration-ref",
    "--worktree", // revision list
    "--responds-to",
    "--confidence", // observation add
];

fn collect_leaf_paths(cmd: &clap::Command, prefix: &mut Vec<String>, out: &mut Vec<String>) {
    let subs: Vec<&clap::Command> = cmd
        .get_subcommands()
        .filter(|c| c.get_name() != "help")
        .collect();
    if subs.is_empty() {
        if !prefix.is_empty() {
            out.push(prefix.join(" "));
        }
        return;
    }
    for sub in subs {
        prefix.push(sub.get_name().to_owned());
        collect_leaf_paths(sub, prefix, out);
        prefix.pop();
    }
}

#[test]
fn every_leaf_command_is_documented() {
    let cmd = super::Cli::command();
    let mut paths = Vec::new();
    collect_leaf_paths(&cmd, &mut Vec::new(), &mut paths);

    let missing: Vec<String> = paths
        .iter()
        .filter(|path| !REFERENCE.contains(&format!("{REFERENCE_COMMAND} {path}")))
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "commands missing from docs/cli-reference.md:\n{}",
        missing.join("\n")
    );
}

#[test]
fn collect_leaf_paths_walks_hidden_leaves() {
    let hidden_leaf = clap::Command::new("legacy").hide(true);
    let root = clap::Command::new("pointbreak").subcommand(hidden_leaf);
    let mut paths = Vec::new();
    collect_leaf_paths(&root, &mut Vec::new(), &mut paths);
    assert!(
        paths.contains(&"legacy".to_owned()),
        "hidden leaves must still be walked so reference_coverage.rs keeps documenting them: {paths:?}"
    );
}

#[test]
fn audit_flags_are_documented() {
    let missing: Vec<&str> = REQUIRED_FLAGS
        .iter()
        .copied()
        .filter(|flag| !REFERENCE.contains(flag))
        .collect();
    assert!(
        missing.is_empty(),
        "flags missing from docs/cli-reference.md: {missing:?}"
    );
}
