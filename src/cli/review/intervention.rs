use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use shore::model::{InputRequestId, ObservationId, ReviewTargetRef, ReviewUnitId};
use shore::session::event::{
    InputRequestMode, InputRequestReasonCode, InputRequestResponseOutcome,
};
use shore::session::{
    InputRequestFetchOptions, InputRequestFetchResult, InputRequestListOptions,
    InputRequestListResult, InputRequestOpenOptions, InputRequestOpenResult,
    InputRequestRespondOptions, InputRequestRespondResult, InputRequestStatusFilter,
    InputRequestTargetSelector, fetch_input_request, list_input_requests, open_input_request,
    respond_input_request,
};

use crate::cli::json;
use crate::cli::review::common::{SideArg, read_body_input};
use crate::cli::review::documents::InterventionViewDocument;

#[derive(Debug, Args)]
pub(super) struct InterventionArgs {
    #[command(subcommand)]
    command: InterventionCommand,
}

#[derive(Debug, Subcommand)]
enum InterventionCommand {
    Request(InterventionRequestArgs),
    List(InterventionListArgs),
    Fetch(InterventionFetchArgs),
    Resolve(InterventionResolveArgs),
}

#[derive(Debug, Args)]
struct InterventionRequestArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    review_unit: Option<String>,

    #[arg(long)]
    track: String,

    #[arg(long)]
    title: String,

    #[arg(long, value_enum)]
    reason: InterventionReasonArg,

    #[arg(long, value_enum, default_value = "blocking")]
    mode: InputRequestModeArg,

    #[arg(long, group = "intervention_body")]
    body: Option<String>,

    #[arg(long, group = "intervention_body")]
    body_file: Option<PathBuf>,

    #[arg(long, group = "intervention_body")]
    body_stdin: bool,

    #[arg(long)]
    file: Option<String>,

    #[arg(long, value_enum, default_value = "new")]
    side: SideArg,

    #[arg(long)]
    start_line: Option<u32>,

    #[arg(long)]
    end_line: Option<u32>,

    #[arg(long)]
    observation: Option<String>,

    #[arg(long)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Args)]
struct InterventionListArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    review_unit: Option<String>,

    #[arg(long)]
    track: Option<String>,

    #[arg(long, value_enum)]
    mode: Option<InputRequestModeArg>,

    #[arg(long)]
    file: Option<String>,

    #[arg(long, value_enum, default_value = "open")]
    status: InterventionStatusArg,

    #[arg(long)]
    include_body: bool,

    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    #[arg(long)]
    compact: bool,
}

#[derive(Debug, Args)]
struct InterventionFetchArgs {
    input_request_id: String,

    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    include_body: bool,

    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    #[arg(long)]
    compact: bool,
}

#[derive(Debug, Args)]
struct InterventionResolveArgs {
    input_request_id: String,

    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long, value_enum)]
    outcome: InterventionOutcomeArg,

    #[arg(long, group = "intervention_reason")]
    reason: Option<String>,

    #[arg(long, group = "intervention_reason")]
    reason_file: Option<PathBuf>,

    #[arg(long, group = "intervention_reason")]
    reason_stdin: bool,

    #[arg(long)]
    idempotency_key: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InterventionRequestBody {
    review_unit_id: String,
    input_request_id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    mode: InputRequestMode,
    reason_code: InputRequestReasonCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_content_hash: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InterventionListBody {
    review_unit_id: String,
    filters: InterventionListFiltersDocument,
    interventions: Vec<InterventionViewDocument>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InterventionFetchBody {
    intervention: InterventionViewDocument,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InterventionResolveBody {
    input_request_id: String,
    input_request_response_id: String,
    event_id: String,
    outcome: InputRequestResponseOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_content_hash: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InterventionListFiltersDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    track_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<InputRequestMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    status: &'static str,
    include_body: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum InputRequestModeArg {
    Blocking,
    Advisory,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum InterventionReasonArg {
    AmbiguousState,
    UnsafeAction,
    StaleRevision,
    FailedGate,
    ExternalSideEffect,
    ConflictingEvent,
    MissingPermission,
    ManualDecisionRequired,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum InterventionStatusArg {
    Open,
    Resolved,
    Ambiguous,
    All,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum InterventionOutcomeArg {
    Approved,
    Rejected,
    Dismissed,
    Superseded,
    Abandoned,
}

pub(super) fn run(
    args: InterventionArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        InterventionCommand::Request(args) => {
            let span = tracing::info_span!("shore.review.intervention.request");
            let _entered = span.enter();
            tracing::debug!(command = "review.intervention.request", "command_start");
            review_intervention_request(args, stdout)
        }
        InterventionCommand::List(args) => {
            let span = tracing::info_span!("shore.review.intervention.list");
            let _entered = span.enter();
            tracing::debug!(command = "review.intervention.list", "command_start");
            review_intervention_list(args, stdout)
        }
        InterventionCommand::Fetch(args) => {
            let span = tracing::info_span!("shore.review.intervention.fetch");
            let _entered = span.enter();
            tracing::debug!(command = "review.intervention.fetch", "command_start");
            review_intervention_fetch(args, stdout)
        }
        InterventionCommand::Resolve(args) => {
            let span = tracing::info_span!("shore.review.intervention.resolve");
            let _entered = span.enter();
            tracing::debug!(command = "review.intervention.resolve", "command_start");
            review_intervention_resolve(args, stdout)
        }
    }
}

fn review_intervention_request(
    args: InterventionRequestArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = open_input_request(intervention_request_options(args)?)?;
    let document = intervention_request_document(result);
    json::write_json(stdout, &document, false)
}

fn review_intervention_list(
    args: InterventionListArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty && !args.compact;
    let result = list_input_requests(intervention_list_options(args));
    let document = intervention_list_document(result?);
    json::write_json(stdout, &document, pretty)
}

fn review_intervention_fetch(
    args: InterventionFetchArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty && !args.compact;
    let result = fetch_input_request(
        InputRequestFetchOptions::new(&args.repo, InputRequestId::new(args.input_request_id))
            .with_include_body(args.include_body),
    );
    let document = intervention_fetch_document(result?);
    json::write_json(stdout, &document, pretty)
}

fn review_intervention_resolve(
    args: InterventionResolveArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = respond_input_request(intervention_resolve_options(args)?)?;
    let document = intervention_resolve_document(result);
    json::write_json(stdout, &document, false)
}

fn intervention_request_options(
    args: InterventionRequestArgs,
) -> Result<InputRequestOpenOptions, Box<dyn std::error::Error>> {
    let target = intervention_target(&args)?;
    let body = read_body_input(
        args.body.as_deref(),
        args.body_file.as_deref(),
        args.body_stdin,
    )?;
    let mut options = InputRequestOpenOptions::new(&args.repo)
        .with_track(args.track)
        .with_title(args.title)
        .with_reason_code(args.reason.into())
        .with_mode(args.mode.into())
        .with_target(target);

    if let Some(review_unit) = args.review_unit {
        options = options.with_review_unit_id(ReviewUnitId::new(review_unit));
    }
    if let Some(body) = body {
        options = options.with_body(body);
    }
    if let Some(idempotency_key) = args.idempotency_key {
        options = options.with_idempotency_key(idempotency_key);
    }

    Ok(options)
}

fn intervention_list_options(args: InterventionListArgs) -> InputRequestListOptions {
    let mut options = InputRequestListOptions::new(&args.repo)
        .with_status(args.status.into())
        .with_include_body(args.include_body);
    if let Some(review_unit) = args.review_unit {
        options = options.with_review_unit_id(ReviewUnitId::new(review_unit));
    }
    if let Some(track) = args.track {
        options = options.with_track(track);
    }
    if let Some(mode) = args.mode {
        options = options.with_mode(mode.into());
    }
    if let Some(file) = args.file {
        options = options.with_file(file);
    }
    options
}

fn intervention_resolve_options(
    args: InterventionResolveArgs,
) -> Result<InputRequestRespondOptions, Box<dyn std::error::Error>> {
    let reason = read_body_input(
        args.reason.as_deref(),
        args.reason_file.as_deref(),
        args.reason_stdin,
    )?;
    let mut options =
        InputRequestRespondOptions::new(&args.repo, InputRequestId::new(args.input_request_id))
            .with_outcome(args.outcome.into());
    if let Some(reason) = reason {
        options = options.with_reason(reason);
    }
    if let Some(idempotency_key) = args.idempotency_key {
        options = options.with_idempotency_key(idempotency_key);
    }
    Ok(options)
}

fn intervention_target(
    args: &InterventionRequestArgs,
) -> Result<InputRequestTargetSelector, Box<dyn std::error::Error>> {
    if let Some(observation_id) = &args.observation {
        if args.file.is_some() || args.start_line.is_some() || args.end_line.is_some() {
            return Err("observation target cannot be combined with file or line target".into());
        }
        return Ok(InputRequestTargetSelector::observation(ObservationId::new(
            observation_id.clone(),
        )));
    }

    if args.end_line.is_some() && args.start_line.is_none() {
        return if args.file.is_some() {
            Err("start line is required when end line is supplied".into())
        } else {
            Err("file is required when selecting intervention lines".into())
        };
    }

    match (&args.file, args.start_line) {
        (Some(file), Some(start_line)) => Ok(InputRequestTargetSelector::range(
            file.clone(),
            args.side.into(),
            start_line,
            args.end_line,
        )),
        (Some(file), None) => Ok(InputRequestTargetSelector::file(file.clone())),
        (None, Some(_)) => Err("file is required when selecting intervention lines".into()),
        (None, None) => Ok(InputRequestTargetSelector::review_unit()),
    }
}

fn intervention_request_document(
    result: InputRequestOpenResult,
) -> json::EventWriteDocument<InterventionRequestBody> {
    json::EventWriteDocument::new(
        "shore.review-intervention-request",
        InterventionRequestBody {
            review_unit_id: result.review_unit_id.as_str().to_owned(),
            input_request_id: result.input_request_id.as_str().to_owned(),
            event_id: result.event_id.as_str().to_owned(),
            track_id: result.track_id.as_str().to_owned(),
            target: result.target,
            mode: result.mode,
            reason_code: result.reason_code,
            body_content_hash: result.body_content_hash,
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}

fn intervention_list_document(
    result: InputRequestListResult,
) -> json::DiagnosticDocument<InterventionListBody> {
    json::DiagnosticDocument::new(
        "shore.review-intervention-list",
        InterventionListBody {
            review_unit_id: result.review_unit_id.as_str().to_owned(),
            filters: InterventionListFiltersDocument {
                track_id: result
                    .filters
                    .track_id
                    .map(|track_id| track_id.as_str().to_owned()),
                mode: result.filters.mode,
                file: result.filters.file,
                status: result.filters.status.as_str(),
                include_body: result.filters.include_body,
            },
            interventions: result
                .input_requests
                .into_iter()
                .map(InterventionViewDocument::from)
                .collect(),
        },
        result.diagnostics,
    )
}

fn intervention_fetch_document(
    result: InputRequestFetchResult,
) -> json::DiagnosticDocument<InterventionFetchBody> {
    json::DiagnosticDocument::new(
        "shore.review-intervention-fetch",
        InterventionFetchBody {
            intervention: InterventionViewDocument::from(result.input_request),
        },
        result.diagnostics,
    )
}

fn intervention_resolve_document(
    result: InputRequestRespondResult,
) -> json::EventWriteDocument<InterventionResolveBody> {
    json::EventWriteDocument::new(
        "shore.review-intervention-resolve",
        InterventionResolveBody {
            input_request_id: result.input_request_id.as_str().to_owned(),
            input_request_response_id: result.input_request_response_id.as_str().to_owned(),
            event_id: result.event_id.as_str().to_owned(),
            outcome: result.outcome,
            reason_content_hash: result.reason_content_hash,
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}

impl From<InputRequestModeArg> for InputRequestMode {
    fn from(value: InputRequestModeArg) -> Self {
        match value {
            InputRequestModeArg::Blocking => InputRequestMode::Blocking,
            InputRequestModeArg::Advisory => InputRequestMode::Advisory,
        }
    }
}

impl From<InterventionReasonArg> for InputRequestReasonCode {
    fn from(value: InterventionReasonArg) -> Self {
        match value {
            InterventionReasonArg::AmbiguousState => InputRequestReasonCode::AmbiguousState,
            InterventionReasonArg::UnsafeAction => InputRequestReasonCode::UnsafeAction,
            InterventionReasonArg::StaleRevision => InputRequestReasonCode::StaleRevision,
            InterventionReasonArg::FailedGate => InputRequestReasonCode::FailedGate,
            InterventionReasonArg::ExternalSideEffect => InputRequestReasonCode::ExternalSideEffect,
            InterventionReasonArg::ConflictingEvent => InputRequestReasonCode::ConflictingEvent,
            InterventionReasonArg::MissingPermission => InputRequestReasonCode::MissingPermission,
            InterventionReasonArg::ManualDecisionRequired => {
                InputRequestReasonCode::ManualDecisionRequired
            }
        }
    }
}

impl From<InterventionStatusArg> for InputRequestStatusFilter {
    fn from(value: InterventionStatusArg) -> Self {
        match value {
            InterventionStatusArg::Open => InputRequestStatusFilter::Open,
            InterventionStatusArg::Resolved => InputRequestStatusFilter::Responded,
            InterventionStatusArg::Ambiguous => InputRequestStatusFilter::Ambiguous,
            InterventionStatusArg::All => InputRequestStatusFilter::All,
        }
    }
}

impl From<InterventionOutcomeArg> for InputRequestResponseOutcome {
    fn from(value: InterventionOutcomeArg) -> Self {
        match value {
            InterventionOutcomeArg::Approved => InputRequestResponseOutcome::Approved,
            InterventionOutcomeArg::Rejected => InputRequestResponseOutcome::Rejected,
            InterventionOutcomeArg::Dismissed => InputRequestResponseOutcome::Dismissed,
            InterventionOutcomeArg::Superseded => InputRequestResponseOutcome::Superseded,
            InterventionOutcomeArg::Abandoned => InputRequestResponseOutcome::Abandoned,
        }
    }
}
