use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use pointbreak::documents::{validation_add_document, validation_list_document};
use pointbreak::model::{RevisionId, ValidationStatus, ValidationTrigger};
use pointbreak::session::{
    ValidationAddOptions, ValidationListOptions, ValidationListResult, list_validation_checks,
    record_validation_check,
};

use crate::cli::common::{ContentTypeArg, count_label, read_body_input, wire_label};
use crate::cli::output;

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

/// Record a validation check for a revision.
#[derive(Debug, Args)]
struct ValidationAddArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Captured revision head seed.
    #[arg(long, conflicts_with = "exact_revision")]
    revision: Option<String>,

    /// Exact captured revision without following supersession.
    #[arg(long)]
    exact_revision: Option<String>,

    /// Review lane that owns this validation check.
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

    #[arg(long, value_enum, default_value = "text/plain")]
    summary_content_type: ContentTypeArg,

    #[arg(long)]
    started_at: Option<String>,

    #[arg(long)]
    completed_at: Option<String>,

    #[arg(long = "log-content-hash")]
    log_content_hashes: Vec<String>,

    #[arg(long)]
    idempotency_key: Option<String>,

    /// Sign this write with a specific key: a keystore key name or a path to a
    /// key file. Overrides POINTBREAK_SIGNING_KEY. A key that cannot be loaded leaves
    /// the write unsigned (exit 0) with an advisory diagnostic — signing never
    /// blocks.
    #[arg(long)]
    sign_key: Option<String>,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// List validation checks for a revision.
#[derive(Debug, Args)]
struct ValidationListArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    revision: Option<String>,

    /// Only list validation checks from this review lane.
    #[arg(long)]
    track: Option<String>,

    #[arg(long, value_enum)]
    status: Option<ValidationStatusArg>,

    #[arg(long)]
    include_body: bool,

    #[command(flatten)]
    format_args: output::FormatArgs,
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
    let format_explicit = args.format_args.explicit();
    let check_name = args.check_name.clone();
    let (options, skip) = validation_add_options(args, stderr)?;
    let result = record_validation_check(options)?;
    crate::cli::common::surface_best_effort_skip(&skip, stderr);
    let format = output::resolve_format(format_explicit, output::OutputFormat::Json)?;
    // Bespoke text lane: a one-line receipt naming the recorded check. Rendered
    // before the document builder consumes the result; machine lanes pay nothing.
    let text = matches!(format.format, output::OutputFormat::Text).then(|| {
        crate::cli::common::with_advisory_lines(
            format!(
                "recorded validation {} · {} · {} · track {}",
                output::short_ref(result.validation_check_id.as_str()),
                check_name,
                wire_label(&result.status),
                result.track_id.as_str(),
            ),
            &result.diagnostics,
        )
    });
    let document = validation_add_document(result);
    output::write_document(stdout, format, &document, || {
        text.expect("text lane resolves the digest source")
    })
}

fn review_validation_list(
    args: ValidationListArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let format_explicit = args.format_args.explicit();
    let repo = args.repo.clone();
    let result = list_validation_checks(validation_list_options(args)?)?;
    let delegation_map = crate::cli::common::discover_delegation_map(&repo);
    let format = output::resolve_format(format_explicit, output::OutputFormat::Json)?;
    // `validation_list_document` consumes the result by value; render the digest
    // up front on the text lane only, so the machine lanes pay nothing extra.
    let text = matches!(format.format, output::OutputFormat::Text)
        .then(|| render_validation_list_text(&result));
    let document = validation_list_document(result, delegation_map.as_ref());
    output::write_document(stdout, format, &document, || {
        text.expect("text lane resolves the digest source")
    })
}

/// Bespoke text lane for `validation list`: a count headline naming the
/// revision and any active filters, then one scannable line per check — short
/// id, check name, status (with exit code when recorded), trigger, track, and a
/// `stale` marker when the checked revision has been superseded. An empty
/// listing renders a `no validation checks` line, never silence.
fn render_validation_list_text(result: &ValidationListResult) -> String {
    let mut scope = format!("on {}", output::short_ref(result.revision_id.as_str()));
    if let Some(track_id) = &result.filters.track_id {
        scope.push_str(&format!(" · track {}", track_id.as_str()));
    }
    if let Some(status) = &result.filters.status {
        scope.push_str(&format!(" · status {}", wire_label(status)));
    }
    if result.validation_checks.is_empty() {
        return format!("no validation checks {scope}");
    }
    let mut lines = vec![format!(
        "{} {scope}:",
        count_label(
            result.validation_checks.len(),
            "validation check",
            "validation checks"
        )
    )];
    for view in &result.validation_checks {
        let mut line = format!(
            "  {} · {} · {}",
            output::short_ref(view.id.as_str()),
            view.check_name,
            wire_label(&view.status),
        );
        if let Some(exit_code) = view.exit_code {
            line.push_str(&format!(" (exit {exit_code})"));
        }
        line.push_str(&format!(
            " · {} · {}",
            wire_label(&view.trigger),
            view.track_id.as_str(),
        ));
        if !view.superseded_by_revisions.is_empty() {
            line.push_str(" · stale");
        }
        lines.push(line);
    }
    lines.join("\n")
}

fn validation_add_options(
    args: ValidationAddArgs,
    stderr: &mut dyn Write,
) -> Result<(ValidationAddOptions, crate::cli::common::SigningSkip), Box<dyn std::error::Error>> {
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

    if let Some(revision) = &args.revision {
        let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
        options = options.with_revision_id(RevisionId::new(ids.rev(revision)?));
    }
    if let Some(exact_revision) = &args.exact_revision {
        let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
        options = options.with_exact_revision_id(RevisionId::new(ids.rev(exact_revision)?));
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
    options = options.with_summary_content_type(args.summary_content_type.into());
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
        crate::cli::common::resolve_and_surface_signer(&args.repo, args.sign_key.as_deref(), stderr)
    {
        let (signed, signer_skip) = crate::cli::common::apply_resolved_signer(options, resolved);
        options = signed;
        skip = signer_skip;
    }

    Ok((options, skip))
}

fn validation_list_options(
    args: ValidationListArgs,
) -> Result<ValidationListOptions, Box<dyn std::error::Error>> {
    let mut options = ValidationListOptions::new(&args.repo)
        .with_include_body(args.include_body)
        .with_trust_set(crate::cli::common::discover_trust_set(&args.repo));
    if let Some(revision) = &args.revision {
        let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
        options = options.with_revision_id(RevisionId::new(ids.rev(revision)?));
    }
    if let Some(track) = args.track {
        options = options.with_track(track);
    }
    if let Some(status) = args.status {
        options = options.with_status(status.into());
    }
    Ok(options)
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
