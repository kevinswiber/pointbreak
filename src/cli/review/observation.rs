use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use shore::model::{ObservationId, ReviewTargetRef, ReviewUnitId};
use shore::session::{
    ObservationAddOptions, ObservationAddResult, ObservationListOptions, ObservationListResult,
    ObservationTargetSelector, list_observations, record_observation,
};

use crate::cli::json;
use crate::cli::review::common::{SideArg, read_body_input};
use crate::cli::review::documents::ObservationViewDocument;

#[derive(Debug, Args)]
pub(super) struct ObservationArgs {
    #[command(subcommand)]
    command: ObservationCommand,
}

#[derive(Debug, Subcommand)]
enum ObservationCommand {
    Add(ObservationAddArgs),
    List(ObservationListArgs),
}

#[derive(Debug, Args)]
struct ObservationAddArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    review_unit: Option<String>,

    #[arg(long)]
    track: String,

    #[arg(long)]
    title: String,

    #[arg(long, group = "observation_body")]
    body: Option<String>,

    #[arg(long, group = "observation_body")]
    body_file: Option<PathBuf>,

    #[arg(long, group = "observation_body")]
    body_stdin: bool,

    #[arg(long)]
    file: Option<String>,

    #[arg(long, value_enum, default_value = "new")]
    side: SideArg,

    #[arg(long)]
    start_line: Option<u32>,

    #[arg(long)]
    end_line: Option<u32>,

    #[arg(long = "tag")]
    tags: Vec<String>,

    #[arg(long, value_enum)]
    confidence: Option<ConfidenceArg>,

    #[arg(long = "supersedes")]
    supersedes: Vec<String>,

    #[arg(long)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Args)]
struct ObservationListArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    review_unit: Option<String>,

    #[arg(long)]
    track: Option<String>,

    #[arg(long)]
    file: Option<String>,

    #[arg(long = "tag")]
    tags: Vec<String>,

    #[arg(long)]
    include_body: bool,

    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    #[arg(long)]
    compact: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationAddBody {
    review_unit_id: String,
    observation_id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    body_content_hash: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationListBody {
    review_unit_id: String,
    filters: ObservationListFiltersDocument,
    observations: Vec<ObservationViewDocument>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationListFiltersDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    track_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    include_body: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum ConfidenceArg {
    Low,
    Medium,
    High,
}

pub(super) fn run(
    args: ObservationArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        ObservationCommand::Add(args) => {
            let span = tracing::info_span!("shore.review.observation.add");
            let _entered = span.enter();
            tracing::debug!(command = "review.observation.add", "command_start");
            review_observation_add(args, stdout)
        }
        ObservationCommand::List(args) => {
            let span = tracing::info_span!("shore.review.observation.list");
            let _entered = span.enter();
            tracing::debug!(command = "review.observation.list", "command_start");
            review_observation_list(args, stdout)
        }
    }
}

fn review_observation_add(
    args: ObservationAddArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = record_observation(observation_add_options(args)?)?;
    let document = observation_add_document(result);
    json::write_json(stdout, &document, false)
}

fn review_observation_list(
    args: ObservationListArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty && !args.compact;
    let result = list_observations(observation_list_options(args));
    let document = observation_list_document(result?);
    json::write_json(stdout, &document, pretty)
}

fn observation_add_options(
    args: ObservationAddArgs,
) -> Result<ObservationAddOptions, Box<dyn std::error::Error>> {
    let target = observation_target(&args);
    let body = read_body_input(
        args.body.as_deref(),
        args.body_file.as_deref(),
        args.body_stdin,
    )?;
    let mut options = ObservationAddOptions::new(&args.repo)
        .with_track(args.track)
        .with_title(args.title)
        .with_target(target);

    if let Some(review_unit) = args.review_unit {
        options = options.with_review_unit_id(ReviewUnitId::new(review_unit));
    }
    if let Some(body) = body {
        options = options.with_body(body);
    }
    for tag in args.tags {
        options = options.with_tag(tag);
    }
    if let Some(confidence) = args.confidence {
        options = options.with_confidence(confidence.as_str());
    }
    for supersedes in args.supersedes {
        options = options.superseding(ObservationId::new(supersedes));
    }
    if let Some(idempotency_key) = args.idempotency_key {
        options = options.with_idempotency_key(idempotency_key);
    }

    Ok(options)
}

fn observation_list_options(args: ObservationListArgs) -> ObservationListOptions {
    let mut options = ObservationListOptions::new(&args.repo).with_include_body(args.include_body);
    if let Some(review_unit) = args.review_unit {
        options = options.with_review_unit_id(ReviewUnitId::new(review_unit));
    }
    if let Some(track) = args.track {
        options = options.with_track(track);
    }
    if let Some(file) = args.file {
        options = options.with_file(file);
    }
    for tag in args.tags {
        options = options.with_tag(tag);
    }
    options
}

fn observation_target(args: &ObservationAddArgs) -> ObservationTargetSelector {
    match (&args.file, args.start_line) {
        (Some(file), Some(start_line)) => ObservationTargetSelector::range(
            file.clone(),
            args.side.into(),
            start_line,
            args.end_line,
        ),
        (Some(file), None) => ObservationTargetSelector::file(file.clone()),
        (None, _) => ObservationTargetSelector::review_unit(),
    }
}

fn observation_add_document(
    result: ObservationAddResult,
) -> json::EventWriteDocument<ObservationAddBody> {
    json::EventWriteDocument::new(
        "shore.review-observation-add",
        ObservationAddBody {
            review_unit_id: result.review_unit_id.as_str().to_owned(),
            observation_id: result.observation_id.as_str().to_owned(),
            event_id: result.event_id.as_str().to_owned(),
            track_id: result.track_id.as_str().to_owned(),
            target: result.target,
            body_content_hash: result.body_content_hash,
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}

fn observation_list_document(
    result: ObservationListResult,
) -> json::DiagnosticDocument<ObservationListBody> {
    json::DiagnosticDocument::new(
        "shore.review-observation-list",
        ObservationListBody {
            review_unit_id: result.review_unit_id.as_str().to_owned(),
            filters: ObservationListFiltersDocument {
                track_id: result
                    .filters
                    .track_id
                    .map(|track_id| track_id.as_str().to_owned()),
                file: result.filters.file,
                tags: result.filters.tags,
                include_body: result.filters.include_body,
            },
            observations: result
                .observations
                .into_iter()
                .map(ObservationViewDocument::from)
                .collect(),
        },
        result.diagnostics,
    )
}

impl ConfidenceArg {
    fn as_str(self) -> &'static str {
        match self {
            ConfidenceArg::Low => "low",
            ConfidenceArg::Medium => "medium",
            ConfidenceArg::High => "high",
        }
    }
}
