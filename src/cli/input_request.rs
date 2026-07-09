use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use pointbreak::documents::{
    input_request_fetch_document, input_request_list_document, input_request_open_document,
    input_request_respond_document,
};
use pointbreak::model::{InputRequestId, ObservationId, RevisionId};
use pointbreak::session::event::{
    AssertionMode, InputRequestReasonCode, InputRequestResponseOutcome,
};
use pointbreak::session::{
    InputRequestFetchOptions, InputRequestListOptions, InputRequestListResult,
    InputRequestOpenOptions, InputRequestRespondOptions, InputRequestRespondResult,
    InputRequestStatusFilter, InputRequestTargetSelector, fetch_input_request, list_input_requests,
    open_input_request, respond_input_request,
};

use crate::cli::common::{ContentTypeArg, SideArg, read_body_input};
use crate::cli::output;

#[derive(Debug, Args)]
pub(super) struct InputRequestArgs {
    #[command(subcommand)]
    command: InputRequestCommand,
}

#[derive(Debug, Subcommand)]
enum InputRequestCommand {
    Open(InputRequestOpenArgs),
    List(InputRequestListArgs),
    Show(InputRequestShowArgs),
    Respond(InputRequestRespondArgs),
}

/// Open an input request for a revision.
#[derive(Debug, Args)]
struct InputRequestOpenArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    revision: Option<String>,

    /// Review lane that owns this input request.
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

    #[arg(long, value_enum, default_value = "text/plain")]
    body_content_type: ContentTypeArg,

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

    /// Sign this write with a specific key: a keystore key name or a path to a
    /// key file. Overrides SHORE_SIGNING_KEY. A key that cannot be loaded leaves
    /// the write unsigned (exit 0) with an advisory diagnostic — signing never
    /// blocks.
    #[arg(long)]
    sign_key: Option<String>,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// List input requests.
#[derive(Debug, Args)]
struct InputRequestListArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    revision: Option<String>,

    /// Only list input requests from this review lane.
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

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// Show a single input request by id.
#[derive(Debug, Args)]
struct InputRequestShowArgs {
    input_request_id: String,

    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    include_body: bool,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// Respond to an open input request.
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

    #[arg(long, value_enum, default_value = "text/plain")]
    reason_content_type: ContentTypeArg,

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
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        InputRequestCommand::Open(args) => {
            let span = tracing::info_span!("shore.input_request.open");
            let _entered = span.enter();
            tracing::debug!(command = "input_request.open", "command_start");
            review_input_request_open(args, stdout, stderr)
        }
        InputRequestCommand::List(args) => {
            let span = tracing::info_span!("shore.input_request.list");
            let _entered = span.enter();
            tracing::debug!(command = "input_request.list", "command_start");
            review_input_request_list(args, stdout)
        }
        InputRequestCommand::Show(args) => {
            let span = tracing::info_span!("shore.input_request.show");
            let _entered = span.enter();
            tracing::debug!(command = "input_request.show", "command_start");
            input_request_show(args, stdout)
        }
        InputRequestCommand::Respond(args) => {
            let span = tracing::info_span!("shore.input_request.respond");
            let _entered = span.enter();
            tracing::debug!(command = "input_request.respond", "command_start");
            review_input_request_respond(args, stdout, stderr)
        }
    }
}

fn review_input_request_open(
    args: InputRequestOpenArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let format_explicit = args.format_args.explicit();
    let (options, skip) = input_request_open_options(args, stderr)?;
    let result = open_input_request(options)?;
    crate::cli::common::surface_best_effort_skip(&skip, stderr);
    let document = input_request_open_document(result);
    let format = output::resolve_format(format_explicit, output::OutputFormat::Json)?;
    output::write_document_json_fallback(stdout, format, &document)
}

fn review_input_request_list(
    args: InputRequestListArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let format_explicit = args.format_args.explicit();
    let repo = args.repo.clone();
    let format = output::resolve_format(format_explicit, output::OutputFormat::Json)?;
    let result = list_input_requests(input_request_list_options(args)?)?;
    let delegation_map = crate::cli::common::discover_delegation_map(&repo);
    // `input_request_list_document` consumes the result by value; the text lane
    // reads the same result, so clone it only when that lane will render.
    let text_source = matches!(format.format, output::OutputFormat::Text).then(|| result.clone());
    let document = input_request_list_document(result, delegation_map.as_ref());
    output::write_document(stdout, format, &document, || {
        render_input_request_list_text(
            text_source
                .as_ref()
                .expect("text lane resolves the list source"),
        )
    })
}

fn input_request_show(
    args: InputRequestShowArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let format_explicit = args.format_args.explicit();
    let delegation_map = crate::cli::common::discover_delegation_map(&args.repo);
    let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
    let input_request_id = ids.input_request(&args.input_request_id)?;
    let result = fetch_input_request(
        InputRequestFetchOptions::new(&args.repo, InputRequestId::new(input_request_id))
            .with_trust_set(crate::cli::common::discover_trust_set(&args.repo))
            .with_include_body(args.include_body),
    );
    let document = input_request_fetch_document(result?, delegation_map.as_ref());
    let format = output::resolve_format(format_explicit, output::OutputFormat::Json)?;
    output::write_document_json_fallback(stdout, format, &document)
}

fn review_input_request_respond(
    args: InputRequestRespondArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let format_explicit = args.format_args.explicit();
    let (options, skip) = input_request_respond_options(args, stderr)?;
    let result = respond_input_request(options)?;
    crate::cli::common::surface_best_effort_skip(&skip, stderr);
    let text_source = result.clone();
    let document = input_request_respond_document(result);
    let format = output::resolve_format(format_explicit, output::OutputFormat::Json)?;
    output::write_document(stdout, format, &document, || {
        render_input_request_respond_text(&text_source)
    })
}

/// Bespoke text lane for `input-request list` (INV-5): a header naming the
/// active status filter and count, then one scannable line per request. An empty
/// list renders a `no ... input requests` line, never silence. Reads only the
/// public `InputRequestListResult`; ids truncate via `output::short_ref`.
fn render_input_request_list_text(result: &InputRequestListResult) -> String {
    let status = status_filter_label(result.filters.status);
    if result.input_requests.is_empty() {
        return format!("no {status} input requests");
    }
    let mut lines = vec![format!(
        "{status} input requests ({}):",
        result.input_requests.len()
    )];
    for request in &result.input_requests {
        lines.push(format!(
            "  {} · \"{}\" · {} · {} · {}",
            output::short_ref(request.id.as_str()),
            crate::cli::common::clamp_title(&request.title),
            wire_label(&request.mode),
            wire_label(&request.reason_code),
            request.status.as_str(),
        ));
    }
    lines.join("\n")
}

/// Bespoke text lane for `input-request respond` (INV-5): a one-line
/// confirmation of the recorded outcome. Reads only the public respond result.
fn render_input_request_respond_text(result: &InputRequestRespondResult) -> String {
    let events = result.events_created;
    let noun = if events == 1 { "event" } else { "events" };
    format!(
        "responded {} to {} ({events} {noun} created)",
        wire_label(&result.outcome),
        output::short_ref(result.input_request_id.as_str()),
    )
}

/// The snake_case wire spelling of a simple serde enum, for disposable text
/// labels (INV-3) — `Approved` → `approved`, `ManualDecisionRequired` →
/// `manual_decision_required`.
fn wire_label<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn status_filter_label(filter: InputRequestStatusFilter) -> &'static str {
    match filter {
        InputRequestStatusFilter::Open => "open",
        InputRequestStatusFilter::Responded => "responded",
        InputRequestStatusFilter::Ambiguous => "ambiguous",
        InputRequestStatusFilter::All => "all",
    }
}

fn input_request_open_options(
    mut args: InputRequestOpenArgs,
    stderr: &mut dyn Write,
) -> Result<(InputRequestOpenOptions, crate::cli::common::SigningSkip), Box<dyn std::error::Error>>
{
    let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
    let observation = match &args.observation {
        Some(raw) => Some(ids.observation(raw)?),
        None => None,
    };
    args.observation = observation;
    let revision = match &args.revision {
        Some(raw) => Some(ids.rev(raw)?),
        None => None,
    };
    args.revision = revision;
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

    if let Some(revision) = args.revision {
        options = options.with_revision_id(RevisionId::new(revision));
    }
    if let Some(body) = body {
        options = options.with_body(body);
    }
    options = options.with_body_content_type(args.body_content_type.into());
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

fn input_request_list_options(
    args: InputRequestListArgs,
) -> Result<InputRequestListOptions, Box<dyn std::error::Error>> {
    let mut options = InputRequestListOptions::new(&args.repo)
        .with_status(args.status.into())
        .with_include_body(args.include_body)
        .with_trust_set(crate::cli::common::discover_trust_set(&args.repo));
    if let Some(revision) = &args.revision {
        let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
        options = options.with_revision_id(RevisionId::new(ids.rev(revision)?));
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
    Ok(options)
}

fn input_request_respond_options(
    args: InputRequestRespondArgs,
    stderr: &mut dyn Write,
) -> Result<(InputRequestRespondOptions, crate::cli::common::SigningSkip), Box<dyn std::error::Error>>
{
    let reason = read_body_input(
        args.reason.as_deref(),
        args.reason_file.as_deref(),
        args.reason_stdin,
    )?;
    let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
    let input_request_id = ids.input_request(&args.input_request_id)?;
    let mut options =
        InputRequestRespondOptions::new(&args.repo, InputRequestId::new(input_request_id))
            .with_outcome(args.outcome.into());
    if let Some(reason) = reason {
        options = options.with_reason(reason);
    }
    options = options.with_reason_content_type(args.reason_content_type.into());
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
        (None, None) => Ok(InputRequestTargetSelector::revision()),
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
