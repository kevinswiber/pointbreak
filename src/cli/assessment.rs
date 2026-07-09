use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use pointbreak::documents::{assessment_add_document, assessment_show_document};
use pointbreak::model::{AssessmentId, InputRequestId, ObservationId, RevisionId};
use pointbreak::session::event::ReviewAssessment;
use pointbreak::session::{
    AssessmentAddOptions, AssessmentRecordStatus, AssessmentShowOptions, AssessmentShowResult,
    AssessmentTargetSelector, record_assessment, show_assessments,
};

use crate::cli::common::{ContentTypeArg, SideArg, read_body_input};
use crate::cli::output;

#[derive(Debug, Args)]
pub(super) struct AssessmentArgs {
    #[command(subcommand)]
    command: AssessmentCommand,
}

#[derive(Debug, Subcommand)]
enum AssessmentCommand {
    Add(Box<AssessmentAddArgs>),
    Show(AssessmentShowArgs),
}

/// Record an assessment for a revision.
#[derive(Debug, Args)]
pub(super) struct AssessmentAddArgs {
    /// Repository path to read/write Pointbreak review state for.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Captured revision to assess; defaults to the single captured revision.
    #[arg(long)]
    revision: Option<String>,

    /// Review lane that owns this assessment.
    #[arg(long)]
    track: String,

    /// Assessment value to record.
    #[arg(long, value_enum)]
    assessment: ReviewAssessmentArg,

    /// Inline assessment summary.
    #[arg(long, group = "assessment_summary")]
    summary: Option<String>,

    /// File containing the assessment summary.
    #[arg(long, group = "assessment_summary")]
    summary_file: Option<PathBuf>,

    /// Read the assessment summary from stdin.
    #[arg(long, group = "assessment_summary")]
    summary_stdin: bool,

    #[arg(long, value_enum, default_value = "text/plain")]
    summary_content_type: ContentTypeArg,

    /// Captured file path to assess.
    #[arg(long)]
    file: Option<String>,

    /// Side of a range target; defaults to new when a range is selected.
    #[arg(long, value_enum)]
    side: Option<SideArg>,

    /// First line for a file range target.
    #[arg(long)]
    start_line: Option<u32>,

    /// Last line for a file range target.
    #[arg(long)]
    end_line: Option<u32>,

    /// Existing observation to assess.
    #[arg(long)]
    observation: Option<String>,

    /// Existing input request to assess.
    #[arg(long)]
    input_request: Option<String>,

    /// Earlier assessment to assess.
    #[arg(long)]
    target_assessment: Option<String>,

    /// Earlier assessment replaced by this one.
    #[arg(long = "replaces")]
    replaces: Vec<String>,

    /// Observation that supports this assessment.
    #[arg(long = "related-observation")]
    related_observations: Vec<String>,

    /// Input request that supports this assessment.
    #[arg(long = "related-input-request")]
    related_input_requests: Vec<String>,

    /// Stable key used to make a retry idempotent.
    #[arg(long)]
    idempotency_key: Option<String>,

    /// Sign this write with a specific key: a keystore key name or a path to a
    /// key file. Overrides SHORE_SIGNING_KEY. A key that cannot be loaded leaves
    /// the write unsigned (exit 0) with an advisory diagnostic — signing never
    /// blocks.
    #[arg(long)]
    sign_key: Option<String>,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// Show the current assessments for a revision.
#[derive(Debug, Args)]
pub(super) struct AssessmentShowArgs {
    /// Repository path to read Pointbreak review state from.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Captured revision to read; defaults to the single captured revision.
    #[arg(long)]
    revision: Option<String>,

    /// Only show assessments from this review lane.
    #[arg(long)]
    track: Option<String>,

    /// Include replaced assessments.
    #[arg(long)]
    all: bool,

    /// Hydrate assessment summaries in output.
    #[arg(long)]
    include_summary: bool,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum ReviewAssessmentArg {
    Accepted,
    AcceptedWithFollowUp,
    NeedsChanges,
    NeedsClarification,
}

pub(super) fn run(
    args: AssessmentArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        AssessmentCommand::Add(args) => {
            let span = tracing::info_span!("shore.review.assessment.add");
            let _entered = span.enter();
            tracing::debug!(command = "review.assessment.add", "command_start");
            review_assessment_add(*args, stdout, stderr)
        }
        AssessmentCommand::Show(args) => {
            let span = tracing::info_span!("shore.review.assessment.show");
            let _entered = span.enter();
            tracing::debug!(command = "review.assessment.show", "command_start");
            review_assessment_show(args, stdout)
        }
    }
}

fn review_assessment_add(
    args: AssessmentAddArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let format_explicit = args.format_args.explicit();
    let (options, skip) = assessment_add_options(args, stderr)?;
    let result = record_assessment(options)?;
    crate::cli::common::surface_best_effort_skip(&skip, stderr);
    let document = assessment_add_document("pointbreak.review-assessment-add", result);
    let format = output::resolve_format(format_explicit, output::OutputFormat::Json)?;
    output::write_document_json_fallback(stdout, format, &document)
}

fn review_assessment_show(
    args: AssessmentShowArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let format_explicit = args.format_args.explicit();
    let repo = args.repo.clone();
    let format = output::resolve_format(format_explicit, output::OutputFormat::Json)?;
    let result = show_assessments(assessment_show_options(args)?)?;
    let delegation_map = crate::cli::common::discover_delegation_map(&repo);
    // `assessment_show_document` consumes the result by value; the text lane
    // reads the same result, so clone it only when that lane will render.
    let text_source = matches!(format.format, output::OutputFormat::Text).then(|| result.clone());
    let document = assessment_show_document(
        "pointbreak.review-assessment-show",
        result,
        delegation_map.as_ref(),
    );
    output::write_document(stdout, format, &document, || {
        render_assessment_show_text(
            text_source
                .as_ref()
                .expect("text lane resolves the assessment source"),
        )
    })
}

/// Bespoke text lane for `assessment show` (INV-5): the current call, the
/// track and recorded-at of each current record, its summary when hydrated, and
/// the replaced count when `--all` reveals history. Reads only the public
/// `AssessmentShowResult`; ids truncate via `output::short_ref`.
fn render_assessment_show_text(result: &AssessmentShowResult) -> String {
    let mut lines = vec![crate::cli::common::current_call_line(
        &result.current.status,
    )];
    for record in &result.current.records {
        lines.push(format!(
            "  track {} · recorded {}",
            output::short_ref(record.track_id.as_str()),
            record.created_at,
        ));
        if let Some(summary) = &record.summary {
            lines.push(format!("  summary: {summary}"));
        }
    }
    let replaced = result
        .assessments
        .iter()
        .filter(|record| record.status == AssessmentRecordStatus::Replaced)
        .count();
    if replaced > 0 {
        let noun = if replaced == 1 {
            "replaced assessment"
        } else {
            "replaced assessments"
        };
        lines.push(format!("{replaced} {noun}"));
    }
    lines.join("\n")
}

pub(super) fn assessment_add_options(
    args: AssessmentAddArgs,
    stderr: &mut dyn Write,
) -> Result<(AssessmentAddOptions, crate::cli::common::SigningSkip), Box<dyn std::error::Error>> {
    let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
    let observation = args
        .observation
        .as_deref()
        .map(|raw| ids.observation(raw))
        .transpose()?;
    let input_request = args
        .input_request
        .as_deref()
        .map(|raw| ids.input_request(raw))
        .transpose()?;
    let target_assessment = args
        .target_assessment
        .as_deref()
        .map(|raw| ids.assessment(raw))
        .transpose()?;
    let target = assessment_target(
        args.file.as_ref(),
        args.side,
        args.start_line,
        args.end_line,
        observation.as_ref(),
        input_request.as_ref(),
        target_assessment.as_ref(),
    )?;
    let summary = read_body_input(
        args.summary.as_deref(),
        args.summary_file.as_deref(),
        args.summary_stdin,
    )?;
    let mut options = AssessmentAddOptions::new(&args.repo)
        .with_track(args.track)
        .with_assessment(args.assessment.into())
        .with_target_selector(target);

    if let Some(revision) = &args.revision {
        options = options.with_revision_id(RevisionId::new(ids.rev(revision)?));
    }
    if let Some(summary) = summary {
        options = options.with_summary(summary);
    }
    options = options.with_summary_content_type(args.summary_content_type.into());
    for assessment_id in &args.replaces {
        options = options.replacing(AssessmentId::new(ids.assessment(assessment_id)?));
    }
    for observation_id in &args.related_observations {
        options = options.related_observation(ObservationId::new(ids.observation(observation_id)?));
    }
    for input_request_id in &args.related_input_requests {
        options = options
            .related_input_request(InputRequestId::new(ids.input_request(input_request_id)?));
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

pub(super) fn assessment_show_options(
    args: AssessmentShowArgs,
) -> Result<AssessmentShowOptions, Box<dyn std::error::Error>> {
    let mut options = AssessmentShowOptions::new(&args.repo)
        .with_all(args.all)
        .with_include_summary(args.include_summary)
        .with_trust_set(crate::cli::common::discover_trust_set(&args.repo));
    if let Some(revision) = &args.revision {
        let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
        options = options.with_revision_id(RevisionId::new(ids.rev(revision)?));
    }
    if let Some(track) = args.track {
        options = options.with_track(track);
    }
    Ok(options)
}

pub(super) fn assessment_target(
    file: Option<&String>,
    side: Option<SideArg>,
    start_line: Option<u32>,
    end_line: Option<u32>,
    observation: Option<&String>,
    input_request: Option<&String>,
    target_assessment: Option<&String>,
) -> Result<AssessmentTargetSelector, Box<dyn std::error::Error>> {
    let direct_target_count = usize::from(observation.is_some())
        + usize::from(input_request.is_some())
        + usize::from(target_assessment.is_some());
    let file_target_present =
        file.is_some() || side.is_some() || start_line.is_some() || end_line.is_some();
    if direct_target_count > 1 || (direct_target_count == 1 && file_target_present) {
        return Err("target cannot be combined with another target selector".into());
    }
    if let Some(observation_id) = observation {
        return Ok(AssessmentTargetSelector::observation(ObservationId::new(
            observation_id.clone(),
        )));
    }
    if let Some(input_request_id) = input_request {
        return Ok(AssessmentTargetSelector::input_request(
            InputRequestId::new(input_request_id.clone()),
        ));
    }
    if let Some(assessment_id) = target_assessment {
        return Ok(AssessmentTargetSelector::assessment(AssessmentId::new(
            assessment_id.clone(),
        )));
    }

    if end_line.is_some() && start_line.is_none() {
        return if file.is_some() {
            Err("start line is required when end line is supplied".into())
        } else {
            Err("file is required when selecting assessment lines".into())
        };
    }
    if side.is_some() && file.is_none() {
        return Err("side requires file".into());
    }

    match (file, start_line) {
        (Some(file), Some(start_line)) => Ok(AssessmentTargetSelector::range(
            file.clone(),
            side.unwrap_or(SideArg::New).into(),
            start_line,
            end_line,
        )),
        (Some(file), None) => Ok(AssessmentTargetSelector::file(file.clone())),
        (None, Some(_)) => Err("file is required when selecting assessment lines".into()),
        (None, None) => Ok(AssessmentTargetSelector::revision()),
    }
}

impl From<ReviewAssessmentArg> for ReviewAssessment {
    fn from(value: ReviewAssessmentArg) -> Self {
        match value {
            ReviewAssessmentArg::Accepted => ReviewAssessment::Accepted,
            ReviewAssessmentArg::AcceptedWithFollowUp => ReviewAssessment::AcceptedWithFollowUp,
            ReviewAssessmentArg::NeedsChanges => ReviewAssessment::NeedsChanges,
            ReviewAssessmentArg::NeedsClarification => ReviewAssessment::NeedsClarification,
        }
    }
}
