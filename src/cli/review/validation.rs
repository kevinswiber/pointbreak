use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use shoreline::documents::{validation_add_document, validation_list_document};
use shoreline::model::{ReviewUnitLineageId, RevisionId, ValidationStatus, ValidationTrigger};
use shoreline::session::{
    ValidationAddOptions, ValidationListOptions, list_validation_checks, record_validation_check,
};

use crate::cli::json;
use crate::cli::review::common::read_body_input;

#[derive(Debug, Args)]
pub(super) struct ValidationArgs {
    #[command(subcommand)]
    command: ValidationCommand,
}

#[derive(Debug, Subcommand)]
enum ValidationCommand {
    Add(Box<ValidationAddArgs>),
    List(ValidationListArgs),
}

#[derive(Debug, Args)]
struct ValidationAddArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    review_unit: Option<String>,

    #[arg(long)]
    lineage: Option<String>,

    #[arg(long)]
    track: String,

    #[arg(long)]
    check_name: String,

    #[arg(long, value_enum)]
    status: ValidationStatusArg,

    #[arg(long)]
    command: Option<String>,

    #[arg(long)]
    exit_code: Option<i64>,

    #[arg(long, value_enum, default_value = "manual")]
    trigger: ValidationTriggerArg,

    #[arg(long)]
    source_fingerprint: Option<String>,

    #[arg(long, group = "validation_summary")]
    summary: Option<String>,

    #[arg(long, group = "validation_summary")]
    summary_file: Option<PathBuf>,

    #[arg(long, group = "validation_summary")]
    summary_stdin: bool,

    #[arg(long)]
    started_at: Option<String>,

    #[arg(long)]
    completed_at: Option<String>,

    #[arg(long = "log-content-hash")]
    log_content_hashes: Vec<String>,

    #[arg(long)]
    idempotency_key: Option<String>,

    /// Sign this write with a specific key: a keystore key name or a path to a
    /// key file. Overrides SHORE_SIGNING_KEY. A key that cannot be loaded leaves
    /// the write unsigned (exit 0) with an advisory diagnostic — signing never
    /// blocks.
    #[arg(long)]
    sign_key: Option<String>,
}

#[derive(Debug, Args)]
struct ValidationListArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    review_unit: Option<String>,

    #[arg(long)]
    lineage: Option<String>,

    #[arg(long)]
    track: Option<String>,

    #[arg(long, value_enum)]
    status: Option<ValidationStatusArg>,

    #[arg(long)]
    include_body: bool,

    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    #[arg(long)]
    compact: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum ValidationStatusArg {
    Passed,
    Failed,
    Errored,
    Skipped,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum ValidationTriggerArg {
    Manual,
    Push,
    PullRequest,
}

pub(super) fn run(
    args: ValidationArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        ValidationCommand::Add(args) => {
            let span = tracing::info_span!("shore.review.validation.add");
            let _entered = span.enter();
            tracing::debug!(command = "review.validation.add", "command_start");
            review_validation_add(*args, stdout, stderr)
        }
        ValidationCommand::List(args) => {
            let span = tracing::info_span!("shore.review.validation.list");
            let _entered = span.enter();
            tracing::debug!(command = "review.validation.list", "command_start");
            review_validation_list(args, stdout)
        }
    }
}

fn review_validation_add(
    args: ValidationAddArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let (options, skip) = validation_add_options(args, stderr)?;
    let result = record_validation_check(options)?;
    super::common::surface_best_effort_skip(&skip, stderr);
    let document = validation_add_document(result);
    json::write_json(stdout, &document, false)
}

fn review_validation_list(
    args: ValidationListArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty && !args.compact;
    let repo = args.repo.clone();
    let result = list_validation_checks(validation_list_options(args));
    let delegation_map = super::common::discover_delegation_map(&repo);
    let document = validation_list_document(result?, delegation_map.as_ref());
    json::write_json(stdout, &document, pretty)
}

fn validation_add_options(
    args: ValidationAddArgs,
    stderr: &mut dyn Write,
) -> Result<(ValidationAddOptions, super::common::SigningSkip), Box<dyn std::error::Error>> {
    let summary = read_body_input(
        args.summary.as_deref(),
        args.summary_file.as_deref(),
        args.summary_stdin,
    )?;
    let mut options = ValidationAddOptions::new(&args.repo)
        .with_track(args.track)
        .with_check_name(args.check_name)
        .with_status(args.status.into())
        .with_trigger(args.trigger.into());

    if let Some(review_unit) = args.review_unit {
        options = options.with_review_unit_id(RevisionId::new(review_unit));
    }
    if let Some(lineage) = args.lineage {
        options = options.with_lineage_id(ReviewUnitLineageId::new(lineage));
    }
    if let Some(command) = args.command {
        options = options.with_command(command);
    }
    if let Some(exit_code) = args.exit_code {
        options = options.with_exit_code(exit_code);
    }
    if let Some(source_fingerprint) = args.source_fingerprint {
        options = options.with_source_fingerprint(source_fingerprint);
    }
    if let Some(summary) = summary {
        options = options.with_summary(summary);
    }
    if let Some(started_at) = args.started_at {
        options = options.with_started_at(started_at);
    }
    if let Some(completed_at) = args.completed_at {
        options = options.with_completed_at(completed_at);
    }
    for content_hash in args.log_content_hashes {
        options = options.with_log_artifact_content_hash(content_hash);
    }
    if let Some(idempotency_key) = args.idempotency_key {
        options = options.with_idempotency_key(idempotency_key);
    }
    let mut skip = None;
    if let Some(resolved) =
        super::common::resolve_and_surface_signer(&args.repo, args.sign_key.as_deref(), stderr)
    {
        let (signed, signer_skip) = super::common::apply_resolved_signer(options, resolved);
        options = signed;
        skip = signer_skip;
    }

    Ok((options, skip))
}

fn validation_list_options(args: ValidationListArgs) -> ValidationListOptions {
    let mut options = ValidationListOptions::new(&args.repo).with_include_body(args.include_body);
    if let Some(review_unit) = args.review_unit {
        options = options.with_review_unit_id(RevisionId::new(review_unit));
    }
    if let Some(lineage) = args.lineage {
        options = options.with_lineage_id(ReviewUnitLineageId::new(lineage));
    }
    if let Some(track) = args.track {
        options = options.with_track(track);
    }
    if let Some(status) = args.status {
        options = options.with_status(status.into());
    }
    options
}

impl From<ValidationStatusArg> for ValidationStatus {
    fn from(value: ValidationStatusArg) -> Self {
        match value {
            ValidationStatusArg::Passed => Self::Passed,
            ValidationStatusArg::Failed => Self::Failed,
            ValidationStatusArg::Errored => Self::Errored,
            ValidationStatusArg::Skipped => Self::Skipped,
        }
    }
}

impl From<ValidationTriggerArg> for ValidationTrigger {
    fn from(value: ValidationTriggerArg) -> Self {
        match value {
            ValidationTriggerArg::Manual => Self::Manual,
            ValidationTriggerArg::Push => Self::Push,
            ValidationTriggerArg::PullRequest => Self::PullRequest,
        }
    }
}
