use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{Parser, Subcommand};

use crate::cli_tracing::TracingArgs;

mod assessment;
mod association;
mod attention;
mod capture;
pub(crate) mod common;
mod diff;
mod endorse;
mod history;
mod id_resolver;
mod identity;
mod input_request;
mod inspect;
mod json;
mod key;
mod observation;
mod output;
mod revision;
mod store;
mod theme;
mod validation;

#[cfg(test)]
mod about_bleed_guard;
#[cfg(test)]
mod help_hygiene_guard;
#[cfg(test)]
mod help_vocab_guard;
#[cfg(test)]
mod reference_coverage;

#[derive(Debug, Parser)]
#[command(
    name = "shore",
    bin_name = "shore",
    version,
    about = "Inspect review streams"
)]
struct Cli {
    #[command(flatten)]
    tracing: TracingArgs,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Assessment(Box<assessment::AssessmentArgs>),
    Association(Box<association::AssociationArgs>),
    Attention(attention::AttentionArgs),
    Capture(capture::CaptureArgs),
    Diff(diff::DiffArgs),
    Endorse(endorse::EndorseArgs),
    History(history::HistoryArgs),
    Identity(identity::IdentityArgs),
    InputRequest(Box<input_request::InputRequestArgs>),
    Inspect(inspect::InspectArgs),
    Key(key::KeyArgs),
    Observation(Box<observation::ObservationArgs>),
    Revision(revision::RevisionArgs),
    Store(store::StoreArgs),
    Validation(validation::ValidationArgs),
}

pub(crate) fn run_main() -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    run_with_io(std::env::args_os(), &mut stdout, &mut stderr)
}

fn run_with_io<I, S>(args: I, stdout: &mut dyn Write, stderr: &mut dyn Write) -> ExitCode
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let removed_command_hint = removed_command_hint(&args);
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let exit = if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) {
                let _ = writeln!(stdout, "{error}");
                ExitCode::SUCCESS
            } else {
                let _ = writeln!(stderr, "{error}");
                if error.kind() == ErrorKind::InvalidSubcommand
                    && let Some(hint) = removed_command_hint
                {
                    let _ = writeln!(stderr, "\n{hint}");
                }
                ExitCode::FAILURE
            };
            return exit;
        }
    };

    match run_cli(cli, stdout, stderr) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(stderr, "{error}");
            ExitCode::FAILURE
        }
    }
}

/// A predicate over the raw argv that recognizes a removed/renamed command.
enum HintPredicate {
    /// Two or three adjacent argv tokens, e.g. `["review", "revisions"]`.
    AdjacentWindow(&'static [&'static str]),
    /// The first non-flag argv token — the attempted subcommand. Used for the
    /// bare-family retirements, e.g. a stale `shore review …`.
    LeadingToken(&'static str),
}

impl HintPredicate {
    fn matches(&self, tokens: &[&str]) -> bool {
        match self {
            HintPredicate::AdjacentWindow(seq) => tokens
                .windows(seq.len())
                .any(|window| window.iter().zip(seq.iter()).all(|(a, b)| a == b)),
            HintPredicate::LeadingToken(name) => tokens
                .iter()
                .skip(1) // skip the program name
                .find(|token| !token.starts_with('-'))
                .is_some_and(|token| token == name),
        }
    }
}

/// Removed/renamed command hints, evaluated in order (first match wins). Keep
/// specific `AdjacentWindow` rows before general `LeadingToken` rows so a stale
/// `shore review <verb>` gets the verb-specific hint rather than the family hint.
/// Family/rename tasks append rows; they never change this mechanism.
const REMOVED_COMMAND_HINTS: &[(HintPredicate, &str)] = &[
    (
        HintPredicate::AdjacentWindow(&["identity", "enroll"]),
        "Use `shore identity delegate <AGENT> --principal <P>`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "assessment"]),
        "Use `shore assessment` instead of `shore review assessment`.",
    ),
    // The association compounds collapsed to `record`/`withdraw`; the four
    // verb-specific triples must precede the family pair so they win first.
    (
        HintPredicate::AdjacentWindow(&["review", "association", "associate-commit"]),
        "Use `shore association record --commit <oid>` (or `--ref <name> --head <oid>`).",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "association", "associate-ref"]),
        "Use `shore association record --ref <name> --head <oid>` (or `--commit <oid>`).",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "association", "withdraw-commit"]),
        "Use `shore association withdraw <ASSOCIATION_ID>`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "association", "withdraw-ref"]),
        "Use `shore association withdraw <ASSOCIATION_ID>`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "association"]),
        "The `association` family is now top-level; use \
         `shore association record|withdraw|list`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "capture"]),
        "Use `shore capture` instead of `shore review capture`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "endorse"]),
        "Use `shore endorse` instead of `shore review endorse`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "history"]),
        "Use `shore history` instead of `shore review history`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "input-request", "fetch"]),
        "Use `shore input-request show <ID>`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["input-request", "fetch"]),
        "Use `shore input-request show <ID>`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "input-request"]),
        "The `input-request` family is now top-level; use \
         `shore input-request open|list|show|respond`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "observation"]),
        "Use `shore observation` instead of `shore review observation`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "revisions"]),
        "Use `shore revision list`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "show"]),
        "Use `shore revision show [REVISION]`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "validation"]),
        "Use `shore validation` instead of `shore review validation`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "intervention"]),
        "Use `shore input-request` instead of `shore review intervention`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "lineage"]),
        "`shore review lineage` is removed; record supersession on \
         `shore capture --supersedes <revision>` and read it with `shore revision list`.",
    ),
    (
        HintPredicate::AdjacentWindow(&["review", "unit"]),
        "`shore review unit` is removed; list with `shore revision list` \
         and show one with `shore revision show <revision>`.",
    ),
    // The catch-all for the retired `review` namespace; must stay LAST among the
    // review rows so every verb-specific window above wins first.
    (
        HintPredicate::LeadingToken("review"),
        "The `review` family flattened to the top level. Use `shore capture`, \
         `shore revision list`, `shore revision show`, `shore observation …`, etc.",
    ),
    (
        HintPredicate::LeadingToken("keys"),
        "The `keys` family is now `key`. Use `shore key <sub>`.",
    ),
    // The legacy working-tree surfaces, retired end-to-end (ADR-0030 second
    // amendment). Bare `show` stays unassigned per ADR-0030 Decision 3.
    (
        HintPredicate::LeadingToken("dump"),
        "`shore dump` is retired. Read a captured revision's diff with `shore diff`, \
         inspect deeply with `shore inspect`, or read the review record with \
         `shore revision show` (add `--format text` for the digest).",
    ),
    (
        HintPredicate::LeadingToken("show"),
        "`shore show` is retired. Read a captured revision's diff with `shore diff`, \
         inspect deeply with `shore inspect`, or read the review record with \
         `shore revision show` (add `--format text` for the digest).",
    ),
    (
        HintPredicate::LeadingToken("notes"),
        "The `notes` family is retired and sidecar notes are no longer imported. \
         Record review facts with `shore observation add` and read them with \
         `shore revision show` or `shore inspect`.",
    ),
];

/// A hint for a removed or renamed command, surfaced after clap's
/// invalid-subcommand error so a stale invocation points at its replacement.
fn removed_command_hint(args: &[OsString]) -> Option<&'static str> {
    let tokens: Vec<&str> = args.iter().filter_map(|arg| arg.to_str()).collect();
    REMOVED_COMMAND_HINTS
        .iter()
        .find(|(predicate, _)| predicate.matches(&tokens))
        .map(|(_, hint)| *hint)
}

fn run_cli(
    cli: Cli,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    crate::cli_tracing::init_tracing(&cli.tracing)?;

    match cli.command {
        Command::Assessment(args) => assessment::run(*args, stdout, stderr),
        Command::Association(args) => association::run(*args, stdout, stderr),
        Command::Attention(args) => attention::run(args, stdout),
        Command::Capture(args) => capture::run(args, &cli.tracing, stdout, stderr),
        Command::Diff(args) => diff::run(args, stdout),
        Command::Endorse(args) => endorse::run(args, stdout, stderr),
        Command::History(args) => history::run(args, stdout),
        Command::Identity(args) => identity::run(args, stdout, stderr),
        Command::InputRequest(args) => input_request::run(*args, stdout, stderr),
        Command::Inspect(args) => inspect::run(args, stdout),
        Command::Key(args) => key::run(args, stdout),
        Command::Observation(args) => observation::run(*args, stdout, stderr),
        Command::Revision(args) => revision::run(args, stdout),
        Command::Store(args) => store::run(args, stdout, stderr),
        Command::Validation(args) => validation::run(args, stdout, stderr),
    }
}
