use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand};
use shore::dump::DumpDocument;
use shore::session::{PublishOptions, PublishResult, publish_worktree_review};
use shore::stream::ViewportSpec;

mod cli_tracing;
mod tui;

use cli_tracing::TracingArgs;

#[derive(Debug, Parser)]
#[command(name = "shore", version, about = "Inspect review streams")]
struct Cli {
    #[command(flatten)]
    tracing: TracingArgs,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Dump(DumpArgs),
    Review(ReviewArgs),
    Show(ShowArgs),
}

#[derive(Debug, Args)]
struct DumpArgs {
    #[command(flatten)]
    input: ReviewInputArgs,

    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    #[arg(long)]
    compact: bool,
}

#[derive(Clone, Debug, Args)]
struct ReviewInputArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long, conflicts_with = "legacy_hunk_agent_context")]
    review_notes: Option<PathBuf>,

    #[arg(long, conflicts_with = "review_notes")]
    legacy_hunk_agent_context: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ShowArgs {
    #[command(flatten)]
    input: ReviewInputArgs,
}

#[derive(Debug, Args)]
struct ReviewArgs {
    #[command(subcommand)]
    command: ReviewCommand,
}

#[derive(Debug, Subcommand)]
enum ReviewCommand {
    Publish(PublishArgs),
}

#[derive(Debug, Args)]
struct PublishArgs {
    #[command(flatten)]
    input: ReviewInputArgs,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishDocument {
    schema: &'static str,
    version: u32,
    review_id: String,
    work_unit_id: String,
    revision_id: String,
    snapshot_id: String,
    events_created: usize,
    events_existing: usize,
    events_created_by_type: std::collections::BTreeMap<String, usize>,
    diagnostics: Vec<shore::session::ProjectionDiagnostic>,
    state_path: String,
}

fn main() -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    run_with_io(std::env::args_os(), &mut stdout, &mut stderr)
}

fn run_with_io<I, S>(args: I, stdout: &mut dyn Write, stderr: &mut dyn Write) -> ExitCode
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
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

fn run_cli(cli: Cli, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    if matches!(cli.command, Command::Show(_))
        && cli_tracing::tracing_enabled(&cli.tracing)
        && cli.tracing.log_file.is_none()
    {
        return Err("shore show requires --log-file when tracing is enabled".into());
    }

    cli_tracing::init_tracing(&cli.tracing)?;

    match cli.command {
        Command::Dump(args) => {
            tracing::debug!(command = "dump", "command_start");
            dump(args, stdout)
        }
        Command::Review(args) => review(args, stdout),
        Command::Show(args) => {
            tracing::debug!(command = "show", "command_start");
            show(args)
        }
    }
}

fn dump(args: DumpArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let document = document_for_dump(&args)?;
    let json = if should_pretty_print(&args) {
        serde_json::to_string_pretty(&document)?
    } else {
        serde_json::to_string(&document)?
    };
    writeln!(stdout, "{json}")?;
    Ok(())
}

fn show(args: ShowArgs) -> Result<(), Box<dyn std::error::Error>> {
    let document = document_for_show(&args)?;
    let viewport = ViewportSpec::new(80, 24);
    let app = tui::app::TuiApp::new(document, viewport);
    tui::terminal::run(app)
}

fn review(args: ReviewArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        ReviewCommand::Publish(args) => {
            tracing::debug!(command = "review.publish", "command_start");
            review_publish(args, stdout)
        }
    }
}

fn review_publish(
    args: PublishArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = publish_worktree_review(publish_options(&args.input))?;
    let document = PublishDocument::from(result);
    writeln!(stdout, "{}", serde_json::to_string(&document)?)?;
    Ok(())
}

fn document_for_dump(args: &DumpArgs) -> Result<DumpDocument, Box<dyn std::error::Error>> {
    load_dump_document(&args.input)
}

fn document_for_show(args: &ShowArgs) -> Result<DumpDocument, Box<dyn std::error::Error>> {
    load_dump_document(&args.input)
}

fn load_dump_document(args: &ReviewInputArgs) -> Result<DumpDocument, Box<dyn std::error::Error>> {
    let document = match (&args.review_notes, &args.legacy_hunk_agent_context) {
        (Some(review_notes), None) => {
            DumpDocument::from_review_notes_file(&args.repo, review_notes)?
        }
        (None, Some(agent_context)) => {
            DumpDocument::from_legacy_hunk_agent_context_file(&args.repo, agent_context)?
        }
        (None, None) => DumpDocument::from_repo(&args.repo)?,
        (Some(_), Some(_)) => unreachable!("clap rejects mutually exclusive sidecar flags"),
    };
    Ok(document)
}

fn publish_options(args: &ReviewInputArgs) -> PublishOptions {
    let mut options = PublishOptions::new(&args.repo);
    if let Some(review_notes) = &args.review_notes {
        options = options.with_review_notes(review_notes);
    }
    if let Some(agent_context) = &args.legacy_hunk_agent_context {
        options = options.with_legacy_hunk_agent_context(agent_context);
    }
    options
}

fn should_pretty_print(args: &DumpArgs) -> bool {
    args.pretty && !args.compact
}

impl From<PublishResult> for PublishDocument {
    fn from(result: PublishResult) -> Self {
        Self {
            schema: "shore.publish",
            version: 1,
            review_id: result.review_id.as_str().to_owned(),
            work_unit_id: result.work_unit_id.as_str().to_owned(),
            revision_id: result.revision_id.as_str().to_owned(),
            snapshot_id: result.snapshot_id.as_str().to_owned(),
            events_created: result.events_created,
            events_existing: result.events_existing,
            events_created_by_type: result.events_created_by_type,
            diagnostics: result.diagnostics,
            state_path: result.state_path.to_string_lossy().into_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::process::{Command, ExitCode};

    use super::{
        DumpArgs, ReviewInputArgs, ShowArgs, document_for_dump, document_for_show, run_with_io,
    };

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
            legacy_hunk_agent_context: None,
        };

        let dump_document = document_for_dump(&DumpArgs {
            input: input.clone(),
            pretty: false,
            compact: true,
        })
        .expect("dump document builds");
        let show_document = document_for_show(&ShowArgs { input }).expect("show document builds");

        assert_eq!(show_document, dump_document);
    }

    #[test]
    fn show_loader_does_not_create_shore_state() {
        let repo = dump_repo();
        let input = ReviewInputArgs {
            repo: repo.path().to_owned(),
            review_notes: None,
            legacy_hunk_agent_context: None,
        };

        document_for_show(&ShowArgs { input }).expect("show document builds");

        assert!(!repo.path().join(".shore").exists());
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
