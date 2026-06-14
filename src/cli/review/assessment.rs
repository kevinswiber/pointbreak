use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use shoreline::documents::{assessment_add_document, assessment_show_document};
use shoreline::model::{
    AssessmentId, InputRequestId, ObservationId, ReviewUnitId, ReviewUnitLineageId,
};
use shoreline::session::event::ReviewAssessment;
use shoreline::session::{
    AssessmentAddOptions, AssessmentShowOptions, AssessmentTargetSelector, record_assessment,
    show_assessments,
};

use crate::cli::json;
use crate::cli::review::common::{SideArg, read_body_input};

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

#[derive(Debug, Args)]
pub(super) struct AssessmentAddArgs {
    /// Repository path to read/write Shoreline review state for.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Captured ReviewUnit to assess; defaults to the single captured unit.
    #[arg(long)]
    review_unit: Option<String>,

    /// Assess the current head of one ReviewUnit lineage.
    #[arg(long)]
    lineage: Option<String>,

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
}

#[derive(Debug, Args)]
pub(super) struct AssessmentShowArgs {
    /// Repository path to read Shoreline review state from.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Captured ReviewUnit to read; defaults to the single captured unit.
    #[arg(long)]
    review_unit: Option<String>,

    /// Read assessments from the current head of one ReviewUnit lineage.
    #[arg(long)]
    lineage: Option<String>,

    /// Only show assessments from this review lane.
    #[arg(long)]
    track: Option<String>,

    /// Include replaced assessments.
    #[arg(long)]
    all: bool,

    /// Hydrate assessment summaries in output.
    #[arg(long)]
    include_summary: bool,

    /// Pretty-print JSON output.
    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    /// Force compact JSON output.
    #[arg(long)]
    compact: bool,
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
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        AssessmentCommand::Add(args) => {
            let span = tracing::info_span!("shore.review.assessment.add");
            let _entered = span.enter();
            tracing::debug!(command = "review.assessment.add", "command_start");
            review_assessment_add(*args, stdout)
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
) -> Result<(), Box<dyn std::error::Error>> {
    let result = record_assessment(assessment_add_options(args)?)?;
    let document = assessment_add_document("shore.review-assessment-add", result);
    json::write_json(stdout, &document, false)
}

fn review_assessment_show(
    args: AssessmentShowArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty && !args.compact;
    let repo = args.repo.clone();
    let result = show_assessments(assessment_show_options(args));
    let delegation_map = super::common::discover_delegation_map(&repo);
    let document = assessment_show_document(
        "shore.review-assessment-show",
        result?,
        delegation_map.as_ref(),
    );
    json::write_json(stdout, &document, pretty)
}

pub(super) fn assessment_add_options(
    args: AssessmentAddArgs,
) -> Result<AssessmentAddOptions, Box<dyn std::error::Error>> {
    let target = assessment_target(
        args.file.as_ref(),
        args.side,
        args.start_line,
        args.end_line,
        args.observation.as_ref(),
        args.input_request.as_ref(),
        args.target_assessment.as_ref(),
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

    if let Some(review_unit) = args.review_unit {
        options = options.with_review_unit_id(ReviewUnitId::new(review_unit));
    }
    if let Some(lineage) = args.lineage {
        options = options.with_lineage_id(ReviewUnitLineageId::new(lineage));
    }
    if let Some(summary) = summary {
        options = options.with_summary(summary);
    }
    for assessment_id in args.replaces {
        options = options.replacing(AssessmentId::new(assessment_id));
    }
    for observation_id in args.related_observations {
        options = options.related_observation(ObservationId::new(observation_id));
    }
    for input_request_id in args.related_input_requests {
        options = options.related_input_request(InputRequestId::new(input_request_id));
    }
    if let Some(idempotency_key) = args.idempotency_key {
        options = options.with_idempotency_key(idempotency_key);
    }

    Ok(options)
}

pub(super) fn assessment_show_options(args: AssessmentShowArgs) -> AssessmentShowOptions {
    let mut options = AssessmentShowOptions::new(&args.repo)
        .with_all(args.all)
        .with_include_summary(args.include_summary);
    if let Some(review_unit) = args.review_unit {
        options = options.with_review_unit_id(ReviewUnitId::new(review_unit));
    }
    if let Some(lineage) = args.lineage {
        options = options.with_lineage_id(ReviewUnitLineageId::new(lineage));
    }
    if let Some(track) = args.track {
        options = options.with_track(track);
    }
    options
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
        (None, None) => Ok(AssessmentTargetSelector::review_unit()),
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
