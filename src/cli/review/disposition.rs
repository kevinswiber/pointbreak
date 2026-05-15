use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use shore::model::{DispositionId, InterventionId, ObservationId, ReviewTargetRef, ReviewUnitId};
use shore::session::event::ReviewDisposition;
use shore::session::{
    DispositionAddOptions, DispositionAddResult, DispositionShowFilters, DispositionShowOptions,
    DispositionShowResult, DispositionTargetSelector, record_disposition, show_dispositions,
};

use crate::cli::json;
use crate::cli::review::common::{SideArg, read_body_input};
use crate::cli::review::documents::{CurrentDispositionDocument, DispositionViewDocument};

#[derive(Debug, Args)]
pub(super) struct DispositionArgs {
    #[command(subcommand)]
    command: DispositionCommand,
}

#[derive(Debug, Subcommand)]
enum DispositionCommand {
    Add(Box<DispositionAddArgs>),
    Show(DispositionShowArgs),
}

#[derive(Debug, Args)]
struct DispositionAddArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    review_unit: Option<String>,

    #[arg(long)]
    track: String,

    #[arg(long, value_enum)]
    disposition: ReviewDispositionArg,

    #[arg(long, group = "disposition_summary")]
    summary: Option<String>,

    #[arg(long, group = "disposition_summary")]
    summary_file: Option<PathBuf>,

    #[arg(long, group = "disposition_summary")]
    summary_stdin: bool,

    #[arg(long)]
    file: Option<String>,

    #[arg(long, value_enum)]
    side: Option<SideArg>,

    #[arg(long)]
    start_line: Option<u32>,

    #[arg(long)]
    end_line: Option<u32>,

    #[arg(long)]
    observation: Option<String>,

    #[arg(long)]
    intervention: Option<String>,

    #[arg(long)]
    target_disposition: Option<String>,

    #[arg(long = "replaces")]
    replaces: Vec<String>,

    #[arg(long = "related-observation")]
    related_observations: Vec<String>,

    #[arg(long = "related-intervention")]
    related_interventions: Vec<String>,

    #[arg(long = "overrides-observation")]
    overrides_observations: Vec<String>,

    #[arg(long = "overrides-intervention")]
    overrides_interventions: Vec<String>,

    #[arg(long = "overrides-disposition")]
    overrides_dispositions: Vec<String>,

    #[arg(long)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Args)]
struct DispositionShowArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    review_unit: Option<String>,

    #[arg(long)]
    track: Option<String>,

    #[arg(long)]
    all: bool,

    #[arg(long)]
    include_summary: bool,

    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    #[arg(long)]
    compact: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DispositionAddBody {
    review_unit_id: String,
    disposition_id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    disposition: ReviewDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_content_hash: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DispositionShowBody {
    review_unit_id: String,
    filters: DispositionShowFiltersDocument,
    current: CurrentDispositionDocument,
    dispositions: Vec<DispositionViewDocument>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DispositionShowFiltersDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    track_id: Option<String>,
    all: bool,
    include_summary: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum ReviewDispositionArg {
    Accepted,
    AcceptedWithFollowUp,
    NeedsChanges,
    NeedsClarification,
    Overridden,
    Deferred,
    SplitOut,
    Superseded,
}

pub(super) fn run(
    args: DispositionArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        DispositionCommand::Add(args) => {
            let span = tracing::info_span!("shore.review.disposition.add");
            let _entered = span.enter();
            tracing::debug!(command = "review.disposition.add", "command_start");
            review_disposition_add(*args, stdout)
        }
        DispositionCommand::Show(args) => {
            let span = tracing::info_span!("shore.review.disposition.show");
            let _entered = span.enter();
            tracing::debug!(command = "review.disposition.show", "command_start");
            review_disposition_show(args, stdout)
        }
    }
}

fn review_disposition_add(
    args: DispositionAddArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = record_disposition(disposition_add_options(args)?)?;
    let document = disposition_add_document(result);
    json::write_json(stdout, &document, false)
}

fn review_disposition_show(
    args: DispositionShowArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty && !args.compact;
    let result = show_dispositions(disposition_show_options(args));
    let document = disposition_show_document(result?);
    json::write_json(stdout, &document, pretty)
}

fn disposition_add_options(
    args: DispositionAddArgs,
) -> Result<DispositionAddOptions, Box<dyn std::error::Error>> {
    let target = disposition_target(&args)?;
    let summary = read_body_input(
        args.summary.as_deref(),
        args.summary_file.as_deref(),
        args.summary_stdin,
    )?;
    let mut options = DispositionAddOptions::new(&args.repo)
        .with_track(args.track)
        .with_disposition(args.disposition.into())
        .with_target(target);

    if let Some(review_unit) = args.review_unit {
        options = options.with_review_unit_id(ReviewUnitId::new(review_unit));
    }
    if let Some(summary) = summary {
        options = options.with_summary(summary);
    }
    for disposition_id in args.replaces {
        options = options.replacing(DispositionId::new(disposition_id));
    }
    for observation_id in args.related_observations {
        options = options.related_observation(ObservationId::new(observation_id));
    }
    for intervention_id in args.related_interventions {
        options = options.related_intervention(InterventionId::new(intervention_id));
    }
    for observation_id in args.overrides_observations {
        options = options.overriding_observation(ObservationId::new(observation_id));
    }
    for intervention_id in args.overrides_interventions {
        options = options.overriding_intervention(InterventionId::new(intervention_id));
    }
    for disposition_id in args.overrides_dispositions {
        options = options.overriding_disposition(DispositionId::new(disposition_id));
    }
    if let Some(idempotency_key) = args.idempotency_key {
        options = options.with_idempotency_key(idempotency_key);
    }

    Ok(options)
}

fn disposition_show_options(args: DispositionShowArgs) -> DispositionShowOptions {
    let mut options = DispositionShowOptions::new(&args.repo)
        .with_all(args.all)
        .with_include_summary(args.include_summary);
    if let Some(review_unit) = args.review_unit {
        options = options.with_review_unit_id(ReviewUnitId::new(review_unit));
    }
    if let Some(track) = args.track {
        options = options.with_track(track);
    }
    options
}

fn disposition_target(
    args: &DispositionAddArgs,
) -> Result<DispositionTargetSelector, Box<dyn std::error::Error>> {
    let direct_target_count = usize::from(args.observation.is_some())
        + usize::from(args.intervention.is_some())
        + usize::from(args.target_disposition.is_some());
    let file_target_present = args.file.is_some()
        || args.side.is_some()
        || args.start_line.is_some()
        || args.end_line.is_some();
    if direct_target_count > 1 || (direct_target_count == 1 && file_target_present) {
        return Err("target cannot be combined with another target selector".into());
    }
    if let Some(observation_id) = &args.observation {
        return Ok(DispositionTargetSelector::observation(ObservationId::new(
            observation_id.clone(),
        )));
    }
    if let Some(intervention_id) = &args.intervention {
        return Ok(DispositionTargetSelector::intervention(
            InterventionId::new(intervention_id.clone()),
        ));
    }
    if let Some(disposition_id) = &args.target_disposition {
        return Ok(DispositionTargetSelector::disposition(DispositionId::new(
            disposition_id.clone(),
        )));
    }

    if args.end_line.is_some() && args.start_line.is_none() {
        return if args.file.is_some() {
            Err("start line is required when end line is supplied".into())
        } else {
            Err("file is required when selecting disposition lines".into())
        };
    }
    if args.side.is_some() && args.file.is_none() {
        return Err("side requires file".into());
    }

    match (&args.file, args.start_line) {
        (Some(file), Some(start_line)) => Ok(DispositionTargetSelector::range(
            file.clone(),
            args.side.unwrap_or(SideArg::New).into(),
            start_line,
            args.end_line,
        )),
        (Some(file), None) => Ok(DispositionTargetSelector::file(file.clone())),
        (None, Some(_)) => Err("file is required when selecting disposition lines".into()),
        (None, None) => Ok(DispositionTargetSelector::review_unit()),
    }
}

fn disposition_add_document(
    result: DispositionAddResult,
) -> json::EventWriteDocument<DispositionAddBody> {
    json::EventWriteDocument::new(
        "shore.review-disposition-add",
        DispositionAddBody {
            review_unit_id: result.review_unit_id.as_str().to_owned(),
            disposition_id: result.disposition_id.as_str().to_owned(),
            event_id: result.event_id.as_str().to_owned(),
            track_id: result.track_id.as_str().to_owned(),
            target: result.target,
            disposition: result.disposition,
            summary_content_hash: result.summary_content_hash,
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}

fn disposition_show_document(
    result: DispositionShowResult,
) -> json::DiagnosticDocument<DispositionShowBody> {
    json::DiagnosticDocument::new(
        "shore.review-disposition-show",
        DispositionShowBody {
            review_unit_id: result.review_unit_id.as_str().to_owned(),
            filters: DispositionShowFiltersDocument::from(result.filters),
            current: CurrentDispositionDocument::from(result.current),
            dispositions: result
                .dispositions
                .into_iter()
                .map(DispositionViewDocument::from)
                .collect(),
        },
        result.diagnostics,
    )
}

impl From<DispositionShowFilters> for DispositionShowFiltersDocument {
    fn from(filters: DispositionShowFilters) -> Self {
        Self {
            track_id: filters
                .track_id
                .map(|track_id| track_id.as_str().to_owned()),
            all: filters.include_all,
            include_summary: filters.include_summary,
        }
    }
}

impl From<ReviewDispositionArg> for ReviewDisposition {
    fn from(value: ReviewDispositionArg) -> Self {
        match value {
            ReviewDispositionArg::Accepted => ReviewDisposition::Accepted,
            ReviewDispositionArg::AcceptedWithFollowUp => ReviewDisposition::AcceptedWithFollowUp,
            ReviewDispositionArg::NeedsChanges => ReviewDisposition::NeedsChanges,
            ReviewDispositionArg::NeedsClarification => ReviewDisposition::NeedsClarification,
            ReviewDispositionArg::Overridden => ReviewDisposition::Overridden,
            ReviewDispositionArg::Deferred => ReviewDisposition::Deferred,
            ReviewDispositionArg::SplitOut => ReviewDisposition::SplitOut,
            ReviewDispositionArg::Superseded => ReviewDisposition::Superseded,
        }
    }
}
