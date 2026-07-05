//! Guard: every clap leaf's `--help` about text is a real description — not
//! missing, not empty, and not bled from `output::FormatArgs`'s own doc
//! comment. The bleed is a flatten-order quirk: when a leaf's own `Args`
//! struct carries no doc comment, clap falls through to the first flattened
//! field that has one, and `FormatArgs` (flattened into nearly every
//! document-emitting command) is that field almost everywhere. Complements
//! `reference_coverage.rs` (docs coverage) and `help_vocab_guard.rs` (retired
//! vocabulary).
//!
//! This walks every leaf, including hidden ones (`#[command(hide = true)]`):
//! hiding only removes a leaf from its parent's `--help` listing, not its own
//! `--help` output, so a hidden leaf's about can still bleed and is still
//! worth catching.

use clap::CommandFactory;

use super::output::FORMAT_ARGS_ABOUT;

fn collect_leaves<'a>(
    cmd: &'a clap::Command,
    prefix: &mut Vec<String>,
    out: &mut Vec<(String, &'a clap::Command)>,
) {
    let subs: Vec<&clap::Command> = cmd
        .get_subcommands()
        .filter(|c| c.get_name() != "help")
        .collect();
    if subs.is_empty() {
        out.push((prefix.join(" "), cmd));
        return;
    }
    for sub in subs {
        prefix.push(sub.get_name().to_owned());
        collect_leaves(sub, prefix, out);
        prefix.pop();
    }
}

#[test]
fn every_leaf_has_a_real_about() {
    let cmd = super::Cli::command();
    let mut leaves = Vec::new();
    collect_leaves(&cmd, &mut Vec::new(), &mut leaves);

    let offenders: Vec<String> = leaves
        .iter()
        .filter_map(|(path, leaf)| {
            let about = leaf.get_about().map(|a| a.to_string());
            match about.as_deref() {
                None => Some(format!("{path}: about is missing")),
                Some("") => Some(format!("{path}: about is empty")),
                Some(text) if text == FORMAT_ARGS_ABOUT => {
                    Some(format!("{path}: about bled from FormatArgs's doc comment"))
                }
                _ => None,
            }
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "leaves with a missing/empty/bled --help about:\n{}",
        offenders.join("\n")
    );
}
