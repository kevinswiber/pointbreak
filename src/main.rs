use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand, ValueEnum};
use shore::dump::{DumpDocument, DumpOptions};
use shore::model::{AcknowledgementId, ActorId, ReviewArtifactId, RevisionId};
use shore::session::event::{AcknowledgementNextAction, VerdictDecision};
use shore::session::{
    AcknowledgeReviewOptions, AcknowledgeReviewResult, ImportNotesOptions, ImportNotesResult,
    ProjectionDiagnostic, PublishOptions, PublishResult, PublishVerdictOptions,
    PublishVerdictResult, acknowledge_review, import_notes, publish_verdict,
    publish_worktree_review,
};
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
    Notes(NotesArgs),
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

#[derive(Debug, Args)]
struct NotesArgs {
    #[command(subcommand)]
    command: NotesCommand,
}

#[derive(Debug, Subcommand)]
enum NotesCommand {
    Apply(NotesApplyArgs),
}

#[derive(Debug, Subcommand)]
enum ReviewCommand {
    Publish(PublishArgs),
    Verdict(VerdictArgs),
    Ack(AckArgs),
}

#[derive(Debug, Args)]
struct PublishArgs {
    #[command(flatten)]
    input: ReviewInputArgs,
}

#[derive(Debug, Args)]
struct VerdictArgs {
    #[arg(long)]
    repo: PathBuf,

    #[arg(long, value_enum)]
    decision: VerdictDecisionArg,

    #[arg(long, group = "verdict_summary")]
    summary: Option<String>,

    #[arg(long, group = "verdict_summary")]
    summary_file: Option<PathBuf>,

    #[arg(long)]
    target_revision: Option<String>,

    #[arg(long = "replaces", value_name = "REVIEW_ARTIFACT_ID")]
    replaces: Vec<String>,

    #[arg(long)]
    reviewer_id: Option<String>,
}

#[derive(Debug, Args)]
struct AckArgs {
    #[arg(long)]
    repo: PathBuf,

    #[arg(long)]
    review_artifact: String,

    #[arg(long, value_enum)]
    next_action: NextActionArg,

    #[arg(long, group = "ack_reason")]
    reason: Option<String>,

    #[arg(long, group = "ack_reason")]
    reason_file: Option<PathBuf>,

    #[arg(long)]
    actor_id: Option<String>,
}

#[derive(Debug, Args)]
struct NotesApplyArgs {
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

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NotesApplyDocument {
    schema: &'static str,
    version: u32,
    note_count: usize,
    notes_created: usize,
    notes_existing: usize,
    diagnostics: Vec<shore::session::ProjectionDiagnostic>,
    state_path: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct VerdictDocument {
    schema: &'static str,
    version: u32,
    review_artifact_id: ReviewArtifactId,
    events_created: usize,
    events_existing: usize,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AckDocument {
    schema: &'static str,
    version: u32,
    acknowledgement_id: AcknowledgementId,
    events_created: usize,
    events_existing: usize,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum VerdictDecisionArg {
    Pass,
    PassMinorNit,
    RequestChanges,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum NextActionArg {
    Accept,
    Address,
    Defer,
    Obsolete,
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
            dump(args, &cli.tracing, stdout)
        }
        Command::Notes(args) => notes(args, stdout),
        Command::Review(args) => review(args, &cli.tracing, stdout),
        Command::Show(args) => {
            tracing::debug!(command = "show", "command_start");
            show(args, &cli.tracing)
        }
    }
}

fn dump(
    args: DumpArgs,
    tracing: &TracingArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let document = document_for_dump(&args, tracing)?;
    let json = if should_pretty_print(&args) {
        serde_json::to_string_pretty(&document)?
    } else {
        serde_json::to_string(&document)?
    };
    writeln!(stdout, "{json}")?;
    Ok(())
}

fn show(args: ShowArgs, tracing: &TracingArgs) -> Result<(), Box<dyn std::error::Error>> {
    let document = document_for_show(&args, tracing)?;
    let input = args.input.clone();
    let tracing = tracing.clone();
    let viewport = ViewportSpec::new(80, 24);
    let app = tui::app::TuiApp::new(document, viewport);
    let repo = input.repo.clone();
    let load_document = move || load_dump_document(&input, dump_options(&input, &tracing));
    tui::terminal::run(app, &repo, load_document)
}

fn notes(args: NotesArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        NotesCommand::Apply(args) => {
            tracing::debug!(command = "notes.apply", "command_start");
            notes_apply(args, stdout)
        }
    }
}

fn review(
    args: ReviewArgs,
    tracing: &TracingArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        ReviewCommand::Publish(args) => {
            tracing::debug!(command = "review.publish", "command_start");
            review_publish(args, tracing, stdout)
        }
        ReviewCommand::Verdict(args) => {
            tracing::debug!(command = "review.verdict", "command_start");
            review_verdict(args, stdout)
        }
        ReviewCommand::Ack(args) => {
            tracing::debug!(command = "review.ack", "command_start");
            review_ack(args, stdout)
        }
    }
}

fn notes_apply(
    args: NotesApplyArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = import_notes(notes_apply_options(&args.input)?)?;
    let document = NotesApplyDocument::from(result);
    writeln!(stdout, "{}", serde_json::to_string(&document)?)?;
    Ok(())
}

fn review_publish(
    args: PublishArgs,
    tracing: &TracingArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = publish_worktree_review(publish_options(&args.input, tracing))?;
    let document = PublishDocument::from(result);
    writeln!(stdout, "{}", serde_json::to_string(&document)?)?;
    Ok(())
}

fn review_verdict(
    args: VerdictArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = publish_verdict(verdict_options(&args)?)?;
    let document = VerdictDocument::from(result);
    writeln!(stdout, "{}", serde_json::to_string(&document)?)?;
    Ok(())
}

fn review_ack(args: AckArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let result = acknowledge_review(ack_options(&args)?)?;
    let document = AckDocument::from(result);
    writeln!(stdout, "{}", serde_json::to_string(&document)?)?;
    Ok(())
}

fn document_for_dump(args: &DumpArgs, tracing: &TracingArgs) -> shore::error::Result<DumpDocument> {
    load_dump_document(&args.input, dump_options(&args.input, tracing))
}

fn document_for_show(args: &ShowArgs, tracing: &TracingArgs) -> shore::error::Result<DumpDocument> {
    load_dump_document(&args.input, dump_options(&args.input, tracing))
}

fn load_dump_document(
    args: &ReviewInputArgs,
    options: DumpOptions,
) -> shore::error::Result<DumpDocument> {
    let document = match (&args.review_notes, &args.legacy_hunk_agent_context) {
        (Some(review_notes), None) => {
            DumpDocument::from_review_notes_file_with_options(&args.repo, review_notes, options)?
        }
        (None, Some(agent_context)) => {
            DumpDocument::from_legacy_hunk_agent_context_file_with_options(
                &args.repo,
                agent_context,
                options,
            )?
        }
        (None, None) => DumpDocument::from_repo_with_options(&args.repo, options)?,
        (Some(_), Some(_)) => unreachable!("clap rejects mutually exclusive sidecar flags"),
    };
    Ok(document)
}

fn dump_options(args: &ReviewInputArgs, tracing: &TracingArgs) -> DumpOptions {
    let mut options = DumpOptions::new();
    if let Some(review_notes) = &args.review_notes {
        options = options.exclude_helper_path(review_notes);
    }
    if let Some(agent_context) = &args.legacy_hunk_agent_context {
        options = options.exclude_helper_path(agent_context);
    }
    if let Some(log_file) = &tracing.log_file {
        options = options.exclude_helper_path(log_file);
    }
    options
}

fn publish_options(args: &ReviewInputArgs, tracing: &TracingArgs) -> PublishOptions {
    let mut options = PublishOptions::new(&args.repo);
    if let Some(review_notes) = &args.review_notes {
        options = options.with_review_notes(review_notes);
    }
    if let Some(agent_context) = &args.legacy_hunk_agent_context {
        options = options.with_legacy_hunk_agent_context(agent_context);
    }
    if let Some(log_file) = &tracing.log_file {
        options = options.with_excluded_helper_path(log_file);
    }
    options
}

fn notes_apply_options(
    args: &ReviewInputArgs,
) -> Result<ImportNotesOptions, Box<dyn std::error::Error>> {
    let mut options = ImportNotesOptions::new(&args.repo);
    match (&args.review_notes, &args.legacy_hunk_agent_context) {
        (Some(review_notes), None) => {
            options = options.with_review_notes(review_notes);
            Ok(options)
        }
        (None, Some(agent_context)) => {
            options = options.with_legacy_hunk_agent_context(agent_context);
            Ok(options)
        }
        (None, None) => Err("exactly one review-notes input is required".into()),
        (Some(_), Some(_)) => unreachable!("clap rejects mutually exclusive sidecar flags"),
    }
}

fn verdict_options(
    args: &VerdictArgs,
) -> Result<PublishVerdictOptions, Box<dyn std::error::Error>> {
    let mut options = PublishVerdictOptions::new(&args.repo).with_decision(args.decision.into());
    if let Some(summary) =
        read_optional_body(args.summary.as_deref(), args.summary_file.as_deref())?
    {
        options = options.with_summary(summary);
    }
    if let Some(target_revision) = &args.target_revision {
        options = options.with_target_revision(RevisionId::new(target_revision.clone()));
    }
    if !args.replaces.is_empty() {
        options = options.replacing(
            args.replaces
                .iter()
                .cloned()
                .map(ReviewArtifactId::new)
                .collect(),
        );
    }
    if let Some(reviewer_id) = &args.reviewer_id {
        options = options.with_reviewer_id(ActorId::new(reviewer_id.clone()));
    }
    Ok(options)
}

fn ack_options(args: &AckArgs) -> Result<AcknowledgeReviewOptions, Box<dyn std::error::Error>> {
    let mut options = AcknowledgeReviewOptions::new(
        &args.repo,
        ReviewArtifactId::new(args.review_artifact.clone()),
    )
    .with_next_action(args.next_action.into());
    if let Some(reason) = read_optional_body(args.reason.as_deref(), args.reason_file.as_deref())? {
        options = options.with_reason(reason);
    }
    if let Some(actor_id) = &args.actor_id {
        options = options.with_actor_id(ActorId::new(actor_id.clone()));
    }
    Ok(options)
}

fn read_optional_body(
    inline: Option<&str>,
    file: Option<&std::path::Path>,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    match (inline, file) {
        (Some(_), Some(_)) => Err("body and body file are mutually exclusive".into()),
        (Some(inline), None) => Ok(Some(inline.to_owned())),
        (None, Some(path)) => Ok(Some(std::fs::read_to_string(path)?)),
        (None, None) => Ok(None),
    }
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

impl From<ImportNotesResult> for NotesApplyDocument {
    fn from(result: ImportNotesResult) -> Self {
        Self {
            schema: "shore.notes-apply",
            version: 1,
            note_count: result.note_count,
            notes_created: result.notes_created,
            notes_existing: result.notes_existing,
            diagnostics: result.diagnostics,
            state_path: result.state_path.to_string_lossy().into_owned(),
        }
    }
}

impl From<PublishVerdictResult> for VerdictDocument {
    fn from(result: PublishVerdictResult) -> Self {
        Self {
            schema: "shore.review-verdict",
            version: 1,
            review_artifact_id: result.review_artifact_id,
            events_created: result.events_created,
            events_existing: result.events_existing,
            diagnostics: result.diagnostics,
        }
    }
}

impl From<AcknowledgeReviewResult> for AckDocument {
    fn from(result: AcknowledgeReviewResult) -> Self {
        Self {
            schema: "shore.review-ack",
            version: 1,
            acknowledgement_id: result.acknowledgement_id,
            events_created: result.events_created,
            events_existing: result.events_existing,
            diagnostics: result.diagnostics,
        }
    }
}

impl From<VerdictDecisionArg> for VerdictDecision {
    fn from(value: VerdictDecisionArg) -> Self {
        match value {
            VerdictDecisionArg::Pass => VerdictDecision::Pass,
            VerdictDecisionArg::PassMinorNit => VerdictDecision::PassMinorNit,
            VerdictDecisionArg::RequestChanges => VerdictDecision::RequestChanges,
        }
    }
}

impl From<NextActionArg> for AcknowledgementNextAction {
    fn from(value: NextActionArg) -> Self {
        match value {
            NextActionArg::Accept => AcknowledgementNextAction::Accept,
            NextActionArg::Address => AcknowledgementNextAction::Address,
            NextActionArg::Defer => AcknowledgementNextAction::Defer,
            NextActionArg::Obsolete => AcknowledgementNextAction::Obsolete,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::process::{Command, ExitCode};

    use shore::dump::DumpInputSource;
    use shore::session::ImportNotesOptions;

    use super::cli_tracing::{LogFormatArg, TracingArgs};
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
        super::import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar_path))
            .expect("notes import succeeds");

        let input = ReviewInputArgs {
            repo: repo.path().to_owned(),
            review_notes: None,
            legacy_hunk_agent_context: None,
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
            legacy_hunk_agent_context: None,
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
            legacy_hunk_agent_context: None,
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
            legacy_hunk_agent_context: None,
        };

        document_for_show(&ShowArgs { input }, &tracing_args(None)).expect("show document builds");

        assert!(!repo.path().join(".shore").exists());
    }

    #[test]
    fn dump_and_show_prefer_explicit_review_notes_over_durable_notes() {
        let repo = dump_repo();
        let durable_sidecar = write_native_review_notes(&repo);
        super::import_notes(
            ImportNotesOptions::new(repo.path()).with_review_notes(&durable_sidecar),
        )
        .unwrap();

        let explicit_path = repo.path().join("override-review-notes.json");
        fs::write(&explicit_path, explicit_review_notes_json()).expect("write explicit notes");

        let input = ReviewInputArgs {
            repo: repo.path().to_owned(),
            review_notes: Some(explicit_path),
            legacy_hunk_agent_context: None,
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
