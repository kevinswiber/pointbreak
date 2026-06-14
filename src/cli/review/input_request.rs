use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use shoreline::documents::{
    input_request_fetch_document, input_request_list_document, input_request_open_document,
    input_request_respond_document,
};
use shoreline::model::{InputRequestId, ObservationId, ReviewUnitId, ReviewUnitLineageId};
use shoreline::session::event::{
    AssertionMode, InputRequestReasonCode, InputRequestResponseOutcome,
};
use shoreline::session::{
    InputRequestFetchOptions, InputRequestListOptions, InputRequestOpenOptions,
    InputRequestRespondOptions, InputRequestStatusFilter, InputRequestTargetSelector,
    fetch_input_request, list_input_requests, open_input_request, respond_input_request,
};

use crate::cli::json;
use crate::cli::review::common::{SideArg, read_body_input};

#[derive(Debug, Args)]
pub(super) struct InputRequestArgs {
    #[command(subcommand)]
    command: InputRequestCommand,
}

#[derive(Debug, Subcommand)]
enum InputRequestCommand {
    Open(InputRequestOpenArgs),
    List(InputRequestListArgs),
    Fetch(InputRequestFetchArgs),
    Respond(InputRequestRespondArgs),
}

#[derive(Debug, Args)]
struct InputRequestOpenArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    review_unit: Option<String>,

    #[arg(long)]
    lineage: Option<String>,

    #[arg(long)]
    track: String,

    #[arg(long)]
    title: String,

    #[arg(long, value_enum)]
    reason: InputRequestReasonArg,

    #[arg(long, value_enum, default_value = "operative")]
    mode: InputRequestAssertionModeArg,

    #[arg(long, group = "input_request_body")]
    body: Option<String>,

    #[arg(long, group = "input_request_body")]
    body_file: Option<PathBuf>,

    #[arg(long, group = "input_request_body")]
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
struct InputRequestListArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    review_unit: Option<String>,

    #[arg(long)]
    lineage: Option<String>,

    #[arg(long)]
    track: Option<String>,

    #[arg(long, value_enum)]
    mode: Option<InputRequestAssertionModeArg>,

    #[arg(long)]
    file: Option<String>,

    #[arg(long, value_enum, default_value = "open")]
    status: InputRequestStatusArg,

    #[arg(long)]
    include_body: bool,

    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    #[arg(long)]
    compact: bool,
}

#[derive(Debug, Args)]
struct InputRequestFetchArgs {
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
struct InputRequestRespondArgs {
    input_request_id: String,

    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long, value_enum)]
    outcome: InputRequestOutcomeArg,

    #[arg(long, group = "input_request_reason")]
    reason: Option<String>,

    #[arg(long, group = "input_request_reason")]
    reason_file: Option<PathBuf>,

    #[arg(long, group = "input_request_reason")]
    reason_stdin: bool,

    #[arg(long)]
    idempotency_key: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum InputRequestAssertionModeArg {
    Operative,
    Advisory,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum InputRequestReasonArg {
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
enum InputRequestStatusArg {
    Open,
    Responded,
    Ambiguous,
    All,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum InputRequestOutcomeArg {
    Approved,
    Rejected,
    Dismissed,
    Superseded,
    Abandoned,
}

pub(super) fn run(
    args: InputRequestArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        InputRequestCommand::Open(args) => {
            let span = tracing::info_span!("shore.review.input_request.open");
            let _entered = span.enter();
            tracing::debug!(command = "review.input_request.open", "command_start");
            review_input_request_open(args, stdout)
        }
        InputRequestCommand::List(args) => {
            let span = tracing::info_span!("shore.review.input_request.list");
            let _entered = span.enter();
            tracing::debug!(command = "review.input_request.list", "command_start");
            review_input_request_list(args, stdout)
        }
        InputRequestCommand::Fetch(args) => {
            let span = tracing::info_span!("shore.review.input_request.fetch");
            let _entered = span.enter();
            tracing::debug!(command = "review.input_request.fetch", "command_start");
            review_input_request_fetch(args, stdout)
        }
        InputRequestCommand::Respond(args) => {
            let span = tracing::info_span!("shore.review.input_request.respond");
            let _entered = span.enter();
            tracing::debug!(command = "review.input_request.respond", "command_start");
            review_input_request_respond(args, stdout)
        }
    }
}

fn review_input_request_open(
    args: InputRequestOpenArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = open_input_request(input_request_open_options(args)?)?;
    let document = input_request_open_document(result);
    json::write_json(stdout, &document, false)
}

fn review_input_request_list(
    args: InputRequestListArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty && !args.compact;
    let repo = args.repo.clone();
    let result = list_input_requests(input_request_list_options(args));
    let delegation_map = super::common::discover_delegation_map(&repo);
    let document = input_request_list_document(result?, delegation_map.as_ref());
    json::write_json(stdout, &document, pretty)
}

fn review_input_request_fetch(
    args: InputRequestFetchArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty && !args.compact;
    let delegation_map = super::common::discover_delegation_map(&args.repo);
    let result = fetch_input_request(
        InputRequestFetchOptions::new(&args.repo, InputRequestId::new(args.input_request_id))
            .with_include_body(args.include_body),
    );
    let document = input_request_fetch_document(result?, delegation_map.as_ref());
    json::write_json(stdout, &document, pretty)
}

fn review_input_request_respond(
    args: InputRequestRespondArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = respond_input_request(input_request_respond_options(args)?)?;
    let document = input_request_respond_document(result);
    json::write_json(stdout, &document, false)
}

fn input_request_open_options(
    args: InputRequestOpenArgs,
) -> Result<InputRequestOpenOptions, Box<dyn std::error::Error>> {
    let target = input_request_target(&args)?;
    let body = read_body_input(
        args.body.as_deref(),
        args.body_file.as_deref(),
        args.body_stdin,
    )?;
    let mut options = InputRequestOpenOptions::new(&args.repo)
        .with_track(args.track)
        .with_title(args.title)
        .with_reason_code(args.reason.into())
        .with_assertion_mode(args.mode.into())
        .with_target(target);

    if let Some(review_unit) = args.review_unit {
        options = options.with_review_unit_id(ReviewUnitId::new(review_unit));
    }
    if let Some(lineage) = args.lineage {
        options = options.with_lineage_id(ReviewUnitLineageId::new(lineage));
    }
    if let Some(body) = body {
        options = options.with_body(body);
    }
    if let Some(idempotency_key) = args.idempotency_key {
        options = options.with_idempotency_key(idempotency_key);
    }

    Ok(options)
}

fn input_request_list_options(args: InputRequestListArgs) -> InputRequestListOptions {
    let mut options = InputRequestListOptions::new(&args.repo)
        .with_status(args.status.into())
        .with_include_body(args.include_body);
    if let Some(review_unit) = args.review_unit {
        options = options.with_review_unit_id(ReviewUnitId::new(review_unit));
    }
    if let Some(lineage) = args.lineage {
        options = options.with_lineage_id(ReviewUnitLineageId::new(lineage));
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

fn input_request_respond_options(
    args: InputRequestRespondArgs,
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

fn input_request_target(
    args: &InputRequestOpenArgs,
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
            Err("file is required when selecting input request lines".into())
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
        (None, Some(_)) => Err("file is required when selecting input request lines".into()),
        (None, None) => Ok(InputRequestTargetSelector::review_unit()),
    }
}

impl From<InputRequestAssertionModeArg> for AssertionMode {
    fn from(value: InputRequestAssertionModeArg) -> Self {
        match value {
            InputRequestAssertionModeArg::Operative => AssertionMode::Operative,
            InputRequestAssertionModeArg::Advisory => AssertionMode::Advisory,
        }
    }
}

impl From<InputRequestReasonArg> for InputRequestReasonCode {
    fn from(value: InputRequestReasonArg) -> Self {
        match value {
            InputRequestReasonArg::AmbiguousState => InputRequestReasonCode::AmbiguousState,
            InputRequestReasonArg::UnsafeAction => InputRequestReasonCode::UnsafeAction,
            InputRequestReasonArg::StaleRevision => InputRequestReasonCode::StaleRevision,
            InputRequestReasonArg::FailedGate => InputRequestReasonCode::FailedGate,
            InputRequestReasonArg::ExternalSideEffect => InputRequestReasonCode::ExternalSideEffect,
            InputRequestReasonArg::ConflictingEvent => InputRequestReasonCode::ConflictingEvent,
            InputRequestReasonArg::MissingPermission => InputRequestReasonCode::MissingPermission,
            InputRequestReasonArg::ManualDecisionRequired => {
                InputRequestReasonCode::ManualDecisionRequired
            }
        }
    }
}

impl From<InputRequestStatusArg> for InputRequestStatusFilter {
    fn from(value: InputRequestStatusArg) -> Self {
        match value {
            InputRequestStatusArg::Open => InputRequestStatusFilter::Open,
            InputRequestStatusArg::Responded => InputRequestStatusFilter::Responded,
            InputRequestStatusArg::Ambiguous => InputRequestStatusFilter::Ambiguous,
            InputRequestStatusArg::All => InputRequestStatusFilter::All,
        }
    }
}

impl From<InputRequestOutcomeArg> for InputRequestResponseOutcome {
    fn from(value: InputRequestOutcomeArg) -> Self {
        match value {
            InputRequestOutcomeArg::Approved => InputRequestResponseOutcome::Approved,
            InputRequestOutcomeArg::Rejected => InputRequestResponseOutcome::Rejected,
            InputRequestOutcomeArg::Dismissed => InputRequestResponseOutcome::Dismissed,
            InputRequestOutcomeArg::Superseded => InputRequestResponseOutcome::Superseded,
            InputRequestOutcomeArg::Abandoned => InputRequestResponseOutcome::Abandoned,
        }
    }
}
