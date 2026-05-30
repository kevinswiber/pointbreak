use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{Parser, Subcommand};

use crate::cli_tracing::TracingArgs;

mod dump;
mod input;
mod inspect;
mod json;
mod notes;
mod review;
mod show;

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
    Dump(dump::DumpArgs),
    Inspect(inspect::InspectArgs),
    Notes(notes::NotesArgs),
    // Boxed because the review subcommands carry much larger argument structs
    // than the other top-level commands.
    Review(Box<review::ReviewArgs>),
    Show(show::ShowArgs),
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
    let removed_input_request_command = invokes_removed_input_request_command(&args);
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
                if error.kind() == ErrorKind::InvalidSubcommand && removed_input_request_command {
                    let _ = writeln!(
                        stderr,
                        "\nUse `shore review input-request` instead of `shore review intervention`."
                    );
                }
                ExitCode::FAILURE
            };
            return exit;
        }
    };

    match run_cli(cli, stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(stderr, "{error}");
            ExitCode::FAILURE
        }
    }
}

fn invokes_removed_input_request_command(args: &[OsString]) -> bool {
    args.windows(2)
        .any(|pair| pair[0].to_str() == Some("review") && pair[1].to_str() == Some("intervention"))
}

fn run_cli(cli: Cli, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    if matches!(cli.command, Command::Show(_))
        && crate::cli_tracing::tracing_enabled(&cli.tracing)
        && cli.tracing.log_file.is_none()
    {
        return Err("shore show requires --log-file when tracing is enabled".into());
    }

    crate::cli_tracing::init_tracing(&cli.tracing)?;

    match cli.command {
        Command::Dump(args) => {
            tracing::debug!(command = "dump", "command_start");
            dump::run(args, &cli.tracing, stdout)
        }
        Command::Inspect(args) => inspect::run(args, stdout),
        Command::Notes(args) => notes::run(args, stdout),
        Command::Review(args) => review::run(*args, &cli.tracing, stdout),
        Command::Show(args) => {
            tracing::debug!(command = "show", "command_start");
            show::run(args, &cli.tracing)
        }
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

        assert!(!repo.path().join(".shore").exists());
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

        assert!(!repo.path().join(".shore").exists());
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
