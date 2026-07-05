use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{Parser, Subcommand};

use crate::cli_tracing::TracingArgs;

mod assessment;
mod association;
mod capture;
pub(crate) mod common;
mod diff;
mod dump;
mod endorse;
mod history;
mod id_resolver;
mod identity;
mod input;
mod input_request;
mod inspect;
mod json;
mod keys;
mod notes;
mod observation;
mod output;
mod revision;
mod show;
mod store;
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
    Capture(capture::CaptureArgs),
    Diff(diff::DiffArgs),
    #[command(hide = true)]
    Dump(dump::DumpArgs),
    Endorse(endorse::EndorseArgs),
    History(history::HistoryArgs),
    Identity(identity::IdentityArgs),
    InputRequest(Box<input_request::InputRequestArgs>),
    Inspect(inspect::InspectArgs),
    Keys(keys::KeysArgs),
    #[command(hide = true)]
    Notes(notes::NotesArgs),
    Observation(Box<observation::ObservationArgs>),
    Revision(revision::RevisionArgs),
    #[command(hide = true)]
    Show(show::ShowArgs),
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
    // The catch-all for the retired `review` namespace; must stay LAST so every
    // verb-specific window above wins first.
    (
        HintPredicate::LeadingToken("review"),
        "The `review` family flattened to the top level. Use `shore capture`, \
         `shore revision list`, `shore revision show`, `shore observation …`, etc.",
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
    if matches!(cli.command, Command::Show(_))
        && crate::cli_tracing::tracing_enabled(&cli.tracing)
        && cli.tracing.log_file.is_none()
    {
        return Err("shore show requires --log-file when tracing is enabled".into());
    }

    crate::cli_tracing::init_tracing(&cli.tracing)?;

    match cli.command {
        Command::Assessment(args) => assessment::run(*args, stdout, stderr),
        Command::Association(args) => association::run(*args, stdout, stderr),
        Command::Capture(args) => capture::run(args, &cli.tracing, stdout, stderr),
        Command::Diff(args) => diff::run(args, stdout),
        Command::Dump(args) => {
            tracing::debug!(command = "dump", "command_start");
            dump::run(args, &cli.tracing, stdout)
        }
        Command::Endorse(args) => endorse::run(args, stdout, stderr),
        Command::History(args) => history::run(args, stdout),
        Command::Identity(args) => identity::run(args, stdout, stderr),
        Command::InputRequest(args) => input_request::run(*args, stdout, stderr),
        Command::Inspect(args) => inspect::run(args, stdout),
        Command::Keys(args) => keys::run(args, stdout),
        Command::Notes(args) => notes::run(args, stdout),
        Command::Observation(args) => observation::run(*args, stdout, stderr),
        Command::Revision(args) => revision::run(args, stdout),
        Command::Show(args) => {
            tracing::debug!(command = "show", "command_start");
            show::run(args, &cli.tracing)
        }
        Command::Store(args) => store::run(args, stdout, stderr),
        Command::Validation(args) => validation::run(args, stdout, stderr),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::process::{Command, ExitCode};

    use shoreline::dump::DumpInputSource;
    use shoreline::session::ImportNotesOptions;

    use super::dump::{DumpArgs, document_for_dump};
    use super::input::ReviewInputArgs;
    use super::run_with_io;
    use super::show::{ShowArgs, document_for_show};
    use crate::cli_tracing::{LogFormatArg, TracingArgs};

    #[test]
    fn dump_writes_json_to_supplied_stdout() {
        let repo = dump_repo();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let exit = run_with_io(
            [
                "shore",
                "--log",
                "off",
                "dump",
                "--repo",
                repo.path().to_str().unwrap(),
                // This in-process run reads the ambient SHORE_FORMAT; pin the machine
                // lane so the compact-JSON byte assertion is deterministic.
                "--format",
                "json",
            ],
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(exit, ExitCode::SUCCESS);
        assert!(stderr.is_empty());
        assert!(
            String::from_utf8(stdout)
                .unwrap()
                .starts_with("{\"schema\":\"shore.dump\"")
        );
    }

    #[test]
    fn help_writes_to_supplied_stdout_with_success() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let exit = run_with_io(["shore", "--help"], &mut stdout, &mut stderr);

        assert_eq!(exit, ExitCode::SUCCESS);
        assert!(stderr.is_empty());
        assert!(
            String::from_utf8(stdout)
                .unwrap()
                .contains("Usage: shore [OPTIONS] <COMMAND>")
        );
    }

    #[test]
    fn error_path_writes_to_supplied_stderr() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let exit = run_with_io(
            [
                "shore",
                "--log",
                "off",
                "dump",
                "--repo",
                "/definitely/missing",
            ],
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(exit, ExitCode::FAILURE);
        assert!(stdout.is_empty());
        assert!(!stderr.is_empty());
    }

    #[test]
    fn dump_and_show_use_the_same_review_notes_loader() {
        let repo = dump_repo();
        let sidecar_dir = tempfile::tempdir().expect("create sidecar tempdir");
        let sidecar_path = sidecar_dir.path().join("review-notes.json");
        fs::write(&sidecar_path, native_review_notes_json()).expect("write review notes");
        let input = ReviewInputArgs {
            repo: repo.path().to_owned(),
            review_notes: Some(sidecar_path),
        };

        let tracing = tracing_args(None);
        let dump_document = document_for_dump(
            &DumpArgs {
                input: input.clone(),
                pretty: false,
                compact: true,
                format_args: Default::default(),
            },
            &tracing,
        )
        .expect("dump document builds");
        let show_document =
            document_for_show(&ShowArgs { input }, &tracing).expect("show document builds");

        assert_eq!(show_document, dump_document);
    }

    #[test]
    fn dump_and_show_load_durable_notes_by_default() {
        let repo = dump_repo();
        let sidecar_dir = tempfile::tempdir().expect("create durable tempdir");
        let sidecar_path = sidecar_dir.path().join("review-notes.json");
        fs::write(&sidecar_path, native_review_notes_json()).expect("write review notes");
        shoreline::session::import_notes(
            ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar_path),
        )
        .expect("notes import succeeds");

        let input = ReviewInputArgs {
            repo: repo.path().to_owned(),
            review_notes: None,
        };
        let tracing = tracing_args(None);

        let dump_document = document_for_dump(
            &DumpArgs {
                input: input.clone(),
                pretty: false,
                compact: true,
                format_args: Default::default(),
            },
            &tracing,
        )
        .expect("dump document builds");
        let show_document =
            document_for_show(&ShowArgs { input }, &tracing).expect("show document builds");

        assert_eq!(dump_document.input.source, DumpInputSource::Durable);
        assert_eq!(dump_document.summary.note_count, 1);
        assert_eq!(dump_document, show_document);
    }

    #[test]
    fn dump_and_show_use_the_same_filtered_review_notes_loader() {
        let repo = dump_repo();
        let sidecar_path = repo.path().join("review-notes.json");
        fs::write(&sidecar_path, native_review_notes_json()).expect("write review notes");
        let input = ReviewInputArgs {
            repo: repo.path().to_owned(),
            review_notes: Some(sidecar_path),
        };
        let tracing = tracing_args(None);

        let dump_document = document_for_dump(
            &DumpArgs {
                input: input.clone(),
                pretty: false,
                compact: true,
                format_args: Default::default(),
            },
            &tracing,
        )
        .expect("dump document builds");
        let show_document =
            document_for_show(&ShowArgs { input }, &tracing).expect("show document builds");

        assert_eq!(show_document, dump_document);
        assert!(
            dump_document
                .snapshot
                .files
                .iter()
                .all(|file| file.new_path.as_deref() != Some("review-notes.json"))
        );
    }

    #[test]
    fn show_loader_does_not_create_shore_state() {
        let repo = dump_repo();
        let input = ReviewInputArgs {
            repo: repo.path().to_owned(),
            review_notes: None,
        };

        document_for_show(&ShowArgs { input }, &tracing_args(None)).expect("show document builds");

        assert!(!repo.path().join(".shore/data").exists());
    }

    #[test]
    fn show_loader_with_in_repo_sidecar_does_not_create_shore_state() {
        let repo = dump_repo();
        let sidecar_path = repo.path().join("review-notes.json");
        fs::write(&sidecar_path, native_review_notes_json()).expect("write review notes");
        let input = ReviewInputArgs {
            repo: repo.path().to_owned(),
            review_notes: Some(sidecar_path),
        };

        document_for_show(&ShowArgs { input }, &tracing_args(None)).expect("show document builds");

        assert!(!repo.path().join(".shore/data").exists());
    }

    #[test]
    fn dump_and_show_prefer_explicit_review_notes_over_durable_notes() {
        let repo = dump_repo();
        let durable_sidecar = write_native_review_notes(&repo);
        shoreline::session::import_notes(
            ImportNotesOptions::new(repo.path()).with_review_notes(&durable_sidecar),
        )
        .unwrap();

        let explicit_path = repo.path().join("override-review-notes.json");
        fs::write(&explicit_path, explicit_review_notes_json()).expect("write explicit notes");

        let input = ReviewInputArgs {
            repo: repo.path().to_owned(),
            review_notes: Some(explicit_path),
        };

        let tracing = tracing_args(None);
        let dump_document = document_for_dump(
            &DumpArgs {
                input: input.clone(),
                pretty: false,
                compact: true,
                format_args: Default::default(),
            },
            &tracing,
        )
        .expect("dump document builds");
        let show_document =
            document_for_show(&ShowArgs { input }, &tracing).expect("show document builds");

        assert_eq!(dump_document.input.source, DumpInputSource::ReviewNotes);
        assert_eq!(dump_document, show_document);
        assert_eq!(dump_document.notes[0].title, "Explicit sidecar title");
    }

    fn tracing_args(log_file: Option<std::path::PathBuf>) -> TracingArgs {
        TracingArgs {
            log: None,
            log_format: LogFormatArg::Compact,
            log_file,
        }
    }

    fn dump_repo() -> GitRepo {
        let repo = GitRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo
    }

    fn native_review_notes_json() -> &'static str {
        r#"{
  "schema": "shore.review-notes",
  "version": 1,
  "summary": "CLI review notes",
  "files": [
    {
      "path": "src/lib.rs",
      "notes": [
        {
          "id": "note:lib",
          "title": "Review lib",
          "body": "Review this change.",
          "target": {
            "side": "new",
            "startLine": 1,
            "endLine": 1
          },
          "author": "human reviewer",
          "source": "reviewer"
        }
      ]
    }
  ]
}"#
    }

    fn write_native_review_notes(repo: &GitRepo) -> std::path::PathBuf {
        let path = repo.path().join("durable-review-notes.json");
        fs::write(&path, native_review_notes_json()).expect("write durable review notes");
        path
    }

    fn explicit_review_notes_json() -> &'static str {
        r#"{
  "schema": "shore.review-notes",
  "version": 1,
  "summary": "Explicit override review notes",
  "files": [
    {
      "path": "src/lib.rs",
      "notes": [
        {
          "id": "note:explicit",
          "title": "Explicit sidecar title",
          "body": "This is from the explicit sidecar.",
          "target": {
            "side": "new",
            "startLine": 1,
            "endLine": 1
          },
          "author": "explicit reviewer",
          "source": "reviewer"
        }
      ]
    }
  ]
}"#
    }

    struct GitRepo {
        root: tempfile::TempDir,
    }

    impl GitRepo {
        fn new() -> Self {
            let repo = Self {
                root: tempfile::tempdir().expect("create temp git repository directory"),
            };
            repo.git(["init"]);
            repo.git(["config", "user.name", "Shore Tests"]);
            repo.git(["config", "user.email", "shore-tests@example.com"]);
            repo.git(["config", "commit.gpgsign", "false"]);
            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
            let path = self.root.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directories");
            }
            fs::write(path, contents).expect("write test repository file");
        }

        fn commit_all(&self, message: &str) {
            self.git(["add", "--all"]);
            self.git(["commit", "-m", message]);
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            let args = args
                .into_iter()
                .map(|arg| arg.as_ref().to_owned())
                .collect::<Vec<_>>();
            let output = Command::new("git")
                .args(&args)
                .current_dir(self.root.path())
                .output()
                .unwrap_or_else(|error| panic!("run git {:?}: {error}", args));
            assert!(
                output.status.success(),
                "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
                args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
