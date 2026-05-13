use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand, ValueEnum};
use shore::model::{
    DispositionId, InterventionId, ObservationId, ReviewEndpoint, ReviewTargetRef, ReviewUnitId,
    ReviewUnitSource, Side,
};
use shore::session::{
    AdapterNoteView, CaptureOptions, CaptureResult, CurrentDispositionStatus,
    DispositionAddOptions, DispositionAddResult, DispositionShowFilters, DispositionShowOptions,
    DispositionShowResult, DispositionTargetSelector, DispositionView, InterventionFetchOptions,
    InterventionFetchResult, InterventionListOptions, InterventionListResult, InterventionMode,
    InterventionReasonCode, InterventionRequestOptions, InterventionRequestResult,
    InterventionResolutionOutcome, InterventionResolveOptions, InterventionResolveResult,
    InterventionStatusFilter, InterventionTargetSelector, InterventionView, ObservationAddOptions,
    ObservationAddResult, ObservationListOptions, ObservationListResult, ObservationTargetSelector,
    ObservationView, ProjectionDiagnostic, ReviewDisposition, ReviewHistoryEntry,
    ReviewHistoryFilters, ReviewHistoryOptions, ReviewHistoryResult, ReviewUnitProjectionIdentity,
    ReviewUnitProjectionRow, ReviewUnitProjectionSummary, ReviewUnitShowFilters,
    ReviewUnitShowOptions, ReviewUnitShowResult, capture_worktree_review, fetch_intervention,
    list_interventions, list_observations, record_disposition, record_observation,
    request_intervention, resolve_intervention, review_history, show_dispositions,
    show_review_unit,
};

use crate::cli_tracing::TracingArgs;

mod dump;
mod input;
mod json;
mod notes;
mod review;
mod show;

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
    Dump(dump::DumpArgs),
    Notes(notes::NotesArgs),
    // Boxed because the review subcommands carry much larger argument structs
    // than the other top-level commands.
    Review(Box<ReviewArgs>),
    Show(show::ShowArgs),
}

#[derive(Debug, Args)]
struct ReviewArgs {
    #[command(subcommand)]
    command: ReviewCommand,
}

#[derive(Debug, Subcommand)]
enum ReviewCommand {
    Capture(CaptureArgs),
    Disposition(DispositionArgs),
    History(HistoryArgs),
    Intervention(InterventionArgs),
    Observation(ObservationArgs),
    Unit(UnitArgs),
}

#[derive(Debug, Args)]
struct CaptureArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,
}

#[derive(Debug, Args)]
struct ObservationArgs {
    #[command(subcommand)]
    command: ObservationCommand,
}

#[derive(Debug, Args)]
struct InterventionArgs {
    #[command(subcommand)]
    command: InterventionCommand,
}

#[derive(Debug, Args)]
struct DispositionArgs {
    #[command(subcommand)]
    command: DispositionCommand,
}

#[derive(Debug, Args)]
struct UnitArgs {
    #[command(subcommand)]
    command: UnitCommand,
}

#[derive(Debug, Args)]
struct HistoryArgs {
    /// Repository root or a path inside the repository.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Filter to one captured ReviewUnit by id.
    #[arg(long)]
    review_unit: Option<String>,

    /// Filter to one review track, such as agent:codex.
    #[arg(long)]
    track: Option<String>,

    /// Filter to one or more durable event types.
    #[arg(long = "event-type")]
    event_types: Vec<HistoryEventTypeArg>,

    /// Hydrate body-like text from inline payloads or body artifacts.
    #[arg(long)]
    include_body: bool,

    /// Pretty-print the JSON response.
    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    /// Emit compact JSON explicitly.
    #[arg(long)]
    compact: bool,
}

#[derive(Debug, Subcommand)]
enum DispositionCommand {
    Add(Box<DispositionAddArgs>),
    Show(DispositionShowArgs),
}

#[derive(Debug, Subcommand)]
enum InterventionCommand {
    Request(InterventionRequestArgs),
    List(InterventionListArgs),
    Fetch(InterventionFetchArgs),
    Resolve(InterventionResolveArgs),
}

#[derive(Debug, Subcommand)]
enum ObservationCommand {
    Add(ObservationAddArgs),
    List(ObservationListArgs),
}

#[derive(Debug, Subcommand)]
enum UnitCommand {
    Show(UnitShowArgs),
}

#[derive(Debug, Args)]
struct UnitShowArgs {
    /// Repository root or a path inside the repository.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Select one ReviewUnit by id.
    #[arg(long)]
    review_unit: Option<String>,

    /// Filter narrative facts to one review track.
    #[arg(long)]
    track: Option<String>,

    /// Hydrate body-like text from inline payloads or body artifacts.
    #[arg(long)]
    include_body: bool,

    /// Pretty-print the JSON response.
    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    /// Emit compact JSON explicitly.
    #[arg(long)]
    compact: bool,
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
    mode: InterventionModeArg,

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
    mode: Option<InterventionModeArg>,

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
    intervention_id: String,

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
    intervention_id: String,

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

    #[arg(long)]
    include_body: bool,

    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    #[arg(long)]
    compact: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureDocument {
    schema: &'static str,
    version: u32,
    review_unit: CaptureReviewUnitDocument,
    events_created: usize,
    events_existing: usize,
    events_created_by_type: std::collections::BTreeMap<String, usize>,
    diagnostics: Vec<shore::session::ProjectionDiagnostic>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureReviewUnitDocument {
    id: String,
    base: ReviewEndpoint,
    target: ReviewEndpoint,
    revision_id: String,
    snapshot_id: String,
    snapshot_artifact_content_hash: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationAddDocument {
    schema: &'static str,
    version: u32,
    review_unit_id: String,
    observation_id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    body_content_hash: Option<String>,
    events_created: usize,
    events_existing: usize,
    events_created_by_type: BTreeMap<String, usize>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationListDocument {
    schema: &'static str,
    version: u32,
    review_unit_id: String,
    filters: ObservationListFiltersDocument,
    observations: Vec<ObservationViewDocument>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationListFiltersDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    track_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    include_body: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationViewDocument {
    id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    confidence: Option<String>,
    status: shore::session::ObservationStatus,
    supersedes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_content_hash: Option<String>,
    created_at: String,
    writer: shore::session::Writer,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InterventionRequestDocument {
    schema: &'static str,
    version: u32,
    review_unit_id: String,
    intervention_id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    mode: InterventionMode,
    reason_code: InterventionReasonCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_content_hash: Option<String>,
    events_created: usize,
    events_existing: usize,
    events_created_by_type: BTreeMap<String, usize>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InterventionListDocument {
    schema: &'static str,
    version: u32,
    review_unit_id: String,
    filters: InterventionListFiltersDocument,
    interventions: Vec<InterventionViewDocument>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InterventionFetchDocument {
    schema: &'static str,
    version: u32,
    intervention: InterventionViewDocument,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InterventionResolveDocument {
    schema: &'static str,
    version: u32,
    intervention_id: String,
    intervention_resolution_id: String,
    event_id: String,
    outcome: InterventionResolutionOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_content_hash: Option<String>,
    events_created: usize,
    events_existing: usize,
    events_created_by_type: BTreeMap<String, usize>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InterventionListFiltersDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    track_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<InterventionMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    status: &'static str,
    include_body: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InterventionViewDocument {
    id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    mode: InterventionMode,
    reason_code: InterventionReasonCode,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_content_hash: Option<String>,
    status: &'static str,
    resolutions: Vec<InterventionResolutionViewDocument>,
    created_at: String,
    writer: shore::session::Writer,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InterventionResolutionViewDocument {
    id: String,
    event_id: String,
    outcome: InterventionResolutionOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_content_hash: Option<String>,
    created_at: String,
    writer: shore::session::Writer,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DispositionAddDocument {
    schema: &'static str,
    version: u32,
    review_unit_id: String,
    disposition_id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    disposition: ReviewDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_content_hash: Option<String>,
    events_created: usize,
    events_existing: usize,
    events_created_by_type: BTreeMap<String, usize>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DispositionShowDocument {
    schema: &'static str,
    version: u32,
    review_unit_id: String,
    filters: DispositionShowFiltersDocument,
    current: CurrentDispositionDocument,
    dispositions: Vec<DispositionViewDocument>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DispositionShowFiltersDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    track_id: Option<String>,
    all: bool,
    include_summary: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CurrentDispositionDocument {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    disposition_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disposition: Option<ReviewDisposition>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    candidates: Vec<DispositionViewDocument>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DispositionViewDocument {
    id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    disposition: ReviewDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_content_hash: Option<String>,
    status: &'static str,
    replaces: Vec<String>,
    related_observations: Vec<String>,
    related_interventions: Vec<String>,
    overrides: Vec<ReviewTargetRef>,
    created_at: String,
    writer: shore::session::Writer,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryDocument {
    schema: &'static str,
    version: u32,
    event_set_hash: String,
    event_count: usize,
    history_count: usize,
    filters: ReviewHistoryFilters,
    entries: Vec<ReviewHistoryEntry>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UnitShowDocument {
    schema: &'static str,
    version: u32,
    event_set_hash: String,
    event_count: usize,
    review_unit: UnitReviewUnitDocument,
    filters: UnitShowFiltersDocument,
    summary: UnitShowSummaryDocument,
    current_disposition: CurrentDispositionDocument,
    observations: Vec<ObservationViewDocument>,
    interventions: Vec<InterventionViewDocument>,
    dispositions: Vec<DispositionViewDocument>,
    adapter_notes: Vec<AdapterNoteDocument>,
    rows: Vec<UnitProjectionRowDocument>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UnitReviewUnitDocument {
    id: String,
    review_id: String,
    revision_id: String,
    snapshot_id: String,
    source: ReviewUnitSource,
    base: ReviewEndpoint,
    target: ReviewEndpoint,
    snapshot_artifact_content_hash: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UnitShowFiltersDocument {
    review_unit_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    track_id: Option<String>,
    include_body: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UnitShowSummaryDocument {
    file_count: usize,
    row_count: usize,
    narrative_row_count: usize,
    snapshot_row_count: usize,
    snapshot_remainder_row_count: usize,
    observation_count: usize,
    intervention_count: usize,
    disposition_count: usize,
    adapter_note_count: usize,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AdapterNoteDocument {
    id: String,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<AdapterNoteTargetDocument>,
    status: &'static str,
    file_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_old_path: Option<String>,
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    confidence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    external_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
    sidecar_content_hash: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AdapterNoteTargetDocument {
    side: Side,
    start_line: u32,
    end_line: u32,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UnitProjectionRowDocument {
    id: String,
    kind: &'static str,
    projection_phase: &'static str,
    projection_order: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot_order: Option<SnapshotOrderDocument>,
    coverage: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<ReviewTargetRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_path: Option<String>,
    related_observation_ids: Vec<String>,
    related_intervention_ids: Vec<String>,
    related_disposition_ids: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotOrderDocument {
    file_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hunk_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    row_index: Option<usize>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum SideArg {
    Old,
    New,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum ConfidenceArg {
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum InterventionModeArg {
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

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum HistoryEventTypeArg {
    ReviewInitialized,
    ReviewUnitCaptured,
    ReviewObservationRecorded,
    ReviewDispositionRecorded,
    InterventionRequested,
    InterventionResolved,
    ReviewNoteImported,
}

pub(crate) fn run_main() -> ExitCode {
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
        && crate::cli_tracing::tracing_enabled(&cli.tracing)
        && cli.tracing.log_file.is_none()
    {
        return Err("shore show requires --log-file when tracing is enabled".into());
    }

    crate::cli_tracing::init_tracing(&cli.tracing)?;

    match cli.command {
        Command::Dump(args) => {
            tracing::debug!(command = "dump", "command_start");
            dump::run(args, &cli.tracing, stdout)
        }
        Command::Notes(args) => notes::run(args, stdout),
        Command::Review(args) => review_command(*args, &cli.tracing, stdout),
        Command::Show(args) => {
            tracing::debug!(command = "show", "command_start");
            show::run(args, &cli.tracing)
        }
    }
}

fn review_command(
    args: ReviewArgs,
    tracing: &TracingArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        ReviewCommand::Capture(args) => {
            tracing::debug!(command = "review.capture", "command_start");
            review_capture(args, tracing, stdout)
        }
        ReviewCommand::Disposition(args) => review_disposition(args, stdout),
        ReviewCommand::History(args) => {
            tracing::debug!(command = "review.history", "command_start");
            review_history_command(args, stdout)
        }
        ReviewCommand::Intervention(args) => review_intervention(args, stdout),
        ReviewCommand::Observation(args) => review_observation(args, stdout),
        ReviewCommand::Unit(args) => review_unit(args, stdout),
    }
}

fn review_history_command(
    args: HistoryArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty;
    let result = review_history(history_options(&args));
    let document = HistoryDocument::from(result?);
    json::write_json(stdout, &document, pretty)
}

fn review_unit(args: UnitArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        UnitCommand::Show(args) => {
            tracing::debug!(command = "review.unit.show", "command_start");
            review_unit_show_command(args, stdout)
        }
    }
}

fn review_unit_show_command(
    args: UnitShowArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty;
    let result = show_review_unit(review_unit_show_options(&args));
    let document = UnitShowDocument::from(result?);
    json::write_json(stdout, &document, pretty)
}

fn review_disposition(
    args: DispositionArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        DispositionCommand::Add(args) => {
            tracing::debug!(command = "review.disposition.add", "command_start");
            review_disposition_add(*args, stdout)
        }
        DispositionCommand::Show(args) => {
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
    let document = DispositionAddDocument::from(result);
    json::write_json(stdout, &document, false)
}

fn review_disposition_show(
    args: DispositionShowArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty && !args.compact;
    let result = show_dispositions(disposition_show_options(args));
    let document = DispositionShowDocument::from(result?);
    json::write_json(stdout, &document, pretty)
}

fn review_intervention(
    args: InterventionArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        InterventionCommand::Request(args) => {
            tracing::debug!(command = "review.intervention.request", "command_start");
            review_intervention_request(args, stdout)
        }
        InterventionCommand::List(args) => {
            tracing::debug!(command = "review.intervention.list", "command_start");
            review_intervention_list(args, stdout)
        }
        InterventionCommand::Fetch(args) => {
            tracing::debug!(command = "review.intervention.fetch", "command_start");
            review_intervention_fetch(args, stdout)
        }
        InterventionCommand::Resolve(args) => {
            tracing::debug!(command = "review.intervention.resolve", "command_start");
            review_intervention_resolve(args, stdout)
        }
    }
}

fn review_intervention_request(
    args: InterventionRequestArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = request_intervention(intervention_request_options(args)?)?;
    let document = InterventionRequestDocument::from(result);
    json::write_json(stdout, &document, false)
}

fn review_intervention_list(
    args: InterventionListArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty && !args.compact;
    let result = list_interventions(intervention_list_options(args));
    let document = InterventionListDocument::from(result?);
    json::write_json(stdout, &document, pretty)
}

fn review_intervention_fetch(
    args: InterventionFetchArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty && !args.compact;
    let result = fetch_intervention(
        InterventionFetchOptions::new(&args.repo, InterventionId::new(args.intervention_id))
            .with_include_body(args.include_body),
    );
    let document = InterventionFetchDocument::from(result?);
    json::write_json(stdout, &document, pretty)
}

fn review_intervention_resolve(
    args: InterventionResolveArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = resolve_intervention(intervention_resolve_options(args)?)?;
    let document = InterventionResolveDocument::from(result);
    json::write_json(stdout, &document, false)
}

fn review_capture(
    args: CaptureArgs,
    tracing: &TracingArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = capture_worktree_review(capture_options(&args, tracing));
    let document = CaptureDocument::from(result?);
    json::write_json(stdout, &document, false)
}

fn review_observation(
    args: ObservationArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        ObservationCommand::Add(args) => {
            tracing::debug!(command = "review.observation.add", "command_start");
            review_observation_add(args, stdout)
        }
        ObservationCommand::List(args) => {
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
    let document = ObservationAddDocument::from(result);
    json::write_json(stdout, &document, false)
}

fn review_observation_list(
    args: ObservationListArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty && !args.compact;
    let result = list_observations(observation_list_options(args));
    let document = ObservationListDocument::from(result?);
    json::write_json(stdout, &document, pretty)
}

fn capture_options(args: &CaptureArgs, tracing: &TracingArgs) -> CaptureOptions {
    let mut options = CaptureOptions::new(&args.repo);
    if let Some(log_file) = &tracing.log_file {
        options = options.with_excluded_helper_path(log_file);
    }
    options
}

fn disposition_add_options(
    args: DispositionAddArgs,
) -> Result<DispositionAddOptions, Box<dyn std::error::Error>> {
    let target = disposition_target(&args)?;
    let summary = review::common::read_body_input(
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

fn intervention_request_options(
    args: InterventionRequestArgs,
) -> Result<InterventionRequestOptions, Box<dyn std::error::Error>> {
    let target = intervention_target(&args)?;
    let body = review::common::read_body_input(
        args.body.as_deref(),
        args.body_file.as_deref(),
        args.body_stdin,
    )?;
    let mut options = InterventionRequestOptions::new(&args.repo)
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

fn intervention_list_options(args: InterventionListArgs) -> InterventionListOptions {
    let mut options = InterventionListOptions::new(&args.repo)
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
) -> Result<InterventionResolveOptions, Box<dyn std::error::Error>> {
    let reason = review::common::read_body_input(
        args.reason.as_deref(),
        args.reason_file.as_deref(),
        args.reason_stdin,
    )?;
    let mut options =
        InterventionResolveOptions::new(&args.repo, InterventionId::new(args.intervention_id))
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
) -> Result<InterventionTargetSelector, Box<dyn std::error::Error>> {
    if let Some(observation_id) = &args.observation {
        if args.file.is_some() || args.start_line.is_some() || args.end_line.is_some() {
            return Err("observation target cannot be combined with file or line target".into());
        }
        return Ok(InterventionTargetSelector::observation(ObservationId::new(
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
        (Some(file), Some(start_line)) => Ok(InterventionTargetSelector::range(
            file.clone(),
            args.side.into(),
            start_line,
            args.end_line,
        )),
        (Some(file), None) => Ok(InterventionTargetSelector::file(file.clone())),
        (None, Some(_)) => Err("file is required when selecting intervention lines".into()),
        (None, None) => Ok(InterventionTargetSelector::review_unit()),
    }
}

fn observation_add_options(
    args: ObservationAddArgs,
) -> Result<ObservationAddOptions, Box<dyn std::error::Error>> {
    let target = observation_target(&args);
    let body = read_observation_body(&args)?;
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
    options
}

fn history_options(args: &HistoryArgs) -> ReviewHistoryOptions {
    let mut options = ReviewHistoryOptions::new(&args.repo).with_include_body(args.include_body);
    if let Some(review_unit) = &args.review_unit {
        options = options.with_review_unit_id(ReviewUnitId::new(review_unit.clone()));
    }
    if let Some(track) = &args.track {
        options = options.with_track(track.clone());
    }
    for event_type in args.event_types.iter().copied() {
        options = options.with_event_type(event_type.into());
    }
    options
}

fn review_unit_show_options(args: &UnitShowArgs) -> ReviewUnitShowOptions {
    let mut options = ReviewUnitShowOptions::new(&args.repo).with_include_body(args.include_body);
    if let Some(review_unit) = &args.review_unit {
        options = options.with_review_unit_id(ReviewUnitId::new(review_unit.clone()));
    }
    if let Some(track) = &args.track {
        options = options.with_track(track.clone());
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

fn read_observation_body(
    args: &ObservationAddArgs,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    if let Some(body) = &args.body {
        return Ok(Some(body.clone()));
    }
    if let Some(path) = &args.body_file {
        return Ok(Some(std::fs::read_to_string(path)?));
    }
    if args.body_stdin {
        let mut body = String::new();
        std::io::stdin().read_to_string(&mut body)?;
        return Ok(Some(body));
    }
    Ok(None)
}

impl From<CaptureResult> for CaptureDocument {
    fn from(result: CaptureResult) -> Self {
        Self {
            schema: "shore.review-capture",
            version: 1,
            review_unit: CaptureReviewUnitDocument {
                id: result.review_unit_id.as_str().to_owned(),
                base: result.base,
                target: result.target,
                revision_id: result.revision_id.as_str().to_owned(),
                snapshot_id: result.snapshot_id.as_str().to_owned(),
                snapshot_artifact_content_hash: result.snapshot_artifact_content_hash,
            },
            events_created: result.events_created,
            events_existing: result.events_existing,
            events_created_by_type: result.events_created_by_type,
            diagnostics: result.diagnostics,
        }
    }
}

impl From<ObservationAddResult> for ObservationAddDocument {
    fn from(result: ObservationAddResult) -> Self {
        Self {
            schema: "shore.review-observation-add",
            version: 1,
            review_unit_id: result.review_unit_id.as_str().to_owned(),
            observation_id: result.observation_id.as_str().to_owned(),
            event_id: result.event_id.as_str().to_owned(),
            track_id: result.track_id.as_str().to_owned(),
            target: result.target,
            body_content_hash: result.body_content_hash,
            events_created: result.events_created,
            events_existing: result.events_existing,
            events_created_by_type: result.events_created_by_type,
            diagnostics: result.diagnostics,
        }
    }
}

impl From<ObservationListResult> for ObservationListDocument {
    fn from(result: ObservationListResult) -> Self {
        Self {
            schema: "shore.review-observation-list",
            version: 1,
            review_unit_id: result.review_unit_id.as_str().to_owned(),
            filters: ObservationListFiltersDocument {
                track_id: result
                    .filters
                    .track_id
                    .map(|track_id| track_id.as_str().to_owned()),
                file: result.filters.file,
                include_body: result.filters.include_body,
            },
            observations: result
                .observations
                .into_iter()
                .map(ObservationViewDocument::from)
                .collect(),
            diagnostics: result.diagnostics,
        }
    }
}

impl From<ObservationView> for ObservationViewDocument {
    fn from(view: ObservationView) -> Self {
        Self {
            id: view.id.as_str().to_owned(),
            event_id: view.event_id.as_str().to_owned(),
            track_id: view.track_id.as_str().to_owned(),
            target: view.target,
            title: view.title,
            body: view.body,
            tags: view.tags,
            confidence: view.confidence,
            status: view.status,
            supersedes: view
                .supersedes
                .into_iter()
                .map(|observation_id| observation_id.as_str().to_owned())
                .collect(),
            body_content_hash: view.body_content_hash,
            created_at: view.created_at,
            writer: view.writer,
        }
    }
}

impl From<InterventionRequestResult> for InterventionRequestDocument {
    fn from(result: InterventionRequestResult) -> Self {
        Self {
            schema: "shore.review-intervention-request",
            version: 1,
            review_unit_id: result.review_unit_id.as_str().to_owned(),
            intervention_id: result.intervention_id.as_str().to_owned(),
            event_id: result.event_id.as_str().to_owned(),
            track_id: result.track_id.as_str().to_owned(),
            target: result.target,
            mode: result.mode,
            reason_code: result.reason_code,
            body_content_hash: result.body_content_hash,
            events_created: result.events_created,
            events_existing: result.events_existing,
            events_created_by_type: result.events_created_by_type,
            diagnostics: result.diagnostics,
        }
    }
}

impl From<InterventionListResult> for InterventionListDocument {
    fn from(result: InterventionListResult) -> Self {
        Self {
            schema: "shore.review-intervention-list",
            version: 1,
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
                .interventions
                .into_iter()
                .map(InterventionViewDocument::from)
                .collect(),
            diagnostics: result.diagnostics,
        }
    }
}

impl From<InterventionFetchResult> for InterventionFetchDocument {
    fn from(result: InterventionFetchResult) -> Self {
        Self {
            schema: "shore.review-intervention-fetch",
            version: 1,
            intervention: InterventionViewDocument::from(result.intervention),
            diagnostics: result.diagnostics,
        }
    }
}

impl From<InterventionResolveResult> for InterventionResolveDocument {
    fn from(result: InterventionResolveResult) -> Self {
        Self {
            schema: "shore.review-intervention-resolve",
            version: 1,
            intervention_id: result.intervention_id.as_str().to_owned(),
            intervention_resolution_id: result.intervention_resolution_id.as_str().to_owned(),
            event_id: result.event_id.as_str().to_owned(),
            outcome: result.outcome,
            reason_content_hash: result.reason_content_hash,
            events_created: result.events_created,
            events_existing: result.events_existing,
            events_created_by_type: result.events_created_by_type,
            diagnostics: result.diagnostics,
        }
    }
}

impl From<InterventionView> for InterventionViewDocument {
    fn from(view: InterventionView) -> Self {
        Self {
            id: view.id.as_str().to_owned(),
            event_id: view.event_id.as_str().to_owned(),
            track_id: view.track_id.as_str().to_owned(),
            target: view.target,
            mode: view.mode,
            reason_code: view.reason_code,
            title: view.title,
            body: view.body,
            body_content_hash: view.body_content_hash,
            status: view.status.as_str(),
            resolutions: view
                .resolutions
                .into_iter()
                .map(InterventionResolutionViewDocument::from)
                .collect(),
            created_at: view.created_at,
            writer: view.writer,
        }
    }
}

impl From<shore::session::InterventionResolutionView> for InterventionResolutionViewDocument {
    fn from(view: shore::session::InterventionResolutionView) -> Self {
        Self {
            id: view.id.as_str().to_owned(),
            event_id: view.event_id.as_str().to_owned(),
            outcome: view.outcome,
            reason: view.reason,
            reason_content_hash: view.reason_content_hash,
            created_at: view.created_at,
            writer: view.writer,
        }
    }
}

impl From<DispositionAddResult> for DispositionAddDocument {
    fn from(result: DispositionAddResult) -> Self {
        Self {
            schema: "shore.review-disposition-add",
            version: 1,
            review_unit_id: result.review_unit_id.as_str().to_owned(),
            disposition_id: result.disposition_id.as_str().to_owned(),
            event_id: result.event_id.as_str().to_owned(),
            track_id: result.track_id.as_str().to_owned(),
            target: result.target,
            disposition: result.disposition,
            summary_content_hash: result.summary_content_hash,
            events_created: result.events_created,
            events_existing: result.events_existing,
            events_created_by_type: result.events_created_by_type,
            diagnostics: result.diagnostics,
        }
    }
}

impl From<DispositionShowResult> for DispositionShowDocument {
    fn from(result: DispositionShowResult) -> Self {
        Self {
            schema: "shore.review-disposition-show",
            version: 1,
            review_unit_id: result.review_unit_id.as_str().to_owned(),
            filters: DispositionShowFiltersDocument::from(result.filters),
            current: CurrentDispositionDocument::from(result.current),
            dispositions: result
                .dispositions
                .into_iter()
                .map(DispositionViewDocument::from)
                .collect(),
            diagnostics: result.diagnostics,
        }
    }
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

impl From<shore::session::CurrentDispositionView> for CurrentDispositionDocument {
    fn from(current: shore::session::CurrentDispositionView) -> Self {
        let status = current.status;
        let mut dispositions = current.dispositions.into_iter();
        match status {
            CurrentDispositionStatus::None => Self {
                status: status.as_str(),
                disposition_id: None,
                disposition: None,
                candidates: Vec::new(),
            },
            CurrentDispositionStatus::Resolved => {
                let disposition = dispositions
                    .next()
                    .expect("resolved current disposition has one record");
                Self {
                    status: status.as_str(),
                    disposition_id: Some(disposition.id.as_str().to_owned()),
                    disposition: Some(disposition.disposition),
                    candidates: Vec::new(),
                }
            }
            CurrentDispositionStatus::Ambiguous => Self {
                status: status.as_str(),
                disposition_id: None,
                disposition: None,
                candidates: dispositions.map(DispositionViewDocument::from).collect(),
            },
        }
    }
}

impl From<DispositionView> for DispositionViewDocument {
    fn from(view: DispositionView) -> Self {
        Self {
            id: view.id.as_str().to_owned(),
            event_id: view.event_id.as_str().to_owned(),
            track_id: view.track_id.as_str().to_owned(),
            target: view.target,
            disposition: view.disposition,
            summary: view.summary,
            summary_content_hash: view.summary_content_hash,
            status: view.status.as_str(),
            replaces: view
                .replaces
                .into_iter()
                .map(|disposition_id| disposition_id.as_str().to_owned())
                .collect(),
            related_observations: view
                .related_observations
                .into_iter()
                .map(|observation_id| observation_id.as_str().to_owned())
                .collect(),
            related_interventions: view
                .related_interventions
                .into_iter()
                .map(|intervention_id| intervention_id.as_str().to_owned())
                .collect(),
            overrides: view.overrides,
            created_at: view.created_at,
            writer: view.writer,
        }
    }
}

impl From<ReviewHistoryResult> for HistoryDocument {
    fn from(result: ReviewHistoryResult) -> Self {
        let history_count = result.history_count();
        Self {
            schema: "shore.review-history",
            version: 1,
            event_set_hash: result.event_set_hash,
            event_count: result.event_count,
            history_count,
            filters: result.filters,
            entries: result.entries,
            diagnostics: result.diagnostics,
        }
    }
}

impl From<ReviewUnitShowResult> for UnitShowDocument {
    fn from(result: ReviewUnitShowResult) -> Self {
        Self {
            schema: "shore.review-unit",
            version: 1,
            event_set_hash: result.event_set_hash,
            event_count: result.event_count,
            review_unit: UnitReviewUnitDocument::from(result.review_unit),
            filters: UnitShowFiltersDocument::from(result.filters),
            summary: UnitShowSummaryDocument::from(result.summary),
            current_disposition: CurrentDispositionDocument::from(result.current_disposition),
            observations: result
                .observations
                .into_iter()
                .map(ObservationViewDocument::from)
                .collect(),
            interventions: result
                .interventions
                .into_iter()
                .map(InterventionViewDocument::from)
                .collect(),
            dispositions: result
                .dispositions
                .into_iter()
                .map(DispositionViewDocument::from)
                .collect(),
            adapter_notes: result
                .adapter_notes
                .into_iter()
                .map(AdapterNoteDocument::from)
                .collect(),
            rows: result
                .rows
                .into_iter()
                .map(UnitProjectionRowDocument::from)
                .collect(),
            diagnostics: result.diagnostics,
        }
    }
}

impl From<ReviewUnitProjectionIdentity> for UnitReviewUnitDocument {
    fn from(identity: ReviewUnitProjectionIdentity) -> Self {
        Self {
            id: identity.id.as_str().to_owned(),
            review_id: identity.review_id.as_str().to_owned(),
            revision_id: identity.revision_id.as_str().to_owned(),
            snapshot_id: identity.snapshot_id.as_str().to_owned(),
            source: identity.source,
            base: identity.base,
            target: identity.target,
            snapshot_artifact_content_hash: identity.snapshot_artifact_content_hash,
        }
    }
}

impl From<ReviewUnitShowFilters> for UnitShowFiltersDocument {
    fn from(filters: ReviewUnitShowFilters) -> Self {
        Self {
            review_unit_id: filters.review_unit_id.as_str().to_owned(),
            track_id: filters
                .track_id
                .map(|track_id| track_id.as_str().to_owned()),
            include_body: filters.include_body,
        }
    }
}

impl From<ReviewUnitProjectionSummary> for UnitShowSummaryDocument {
    fn from(summary: ReviewUnitProjectionSummary) -> Self {
        Self {
            file_count: summary.file_count,
            row_count: summary.row_count,
            narrative_row_count: summary.narrative_row_count,
            snapshot_row_count: summary.snapshot_row_count,
            snapshot_remainder_row_count: summary.snapshot_remainder_row_count,
            observation_count: summary.observation_count,
            intervention_count: summary.intervention_count,
            disposition_count: summary.disposition_count,
            adapter_note_count: summary.adapter_note_count,
        }
    }
}

impl From<AdapterNoteView> for AdapterNoteDocument {
    fn from(view: AdapterNoteView) -> Self {
        Self {
            id: view.id,
            title: view.title,
            body: view.body,
            target: view.target.map(AdapterNoteTargetDocument::from),
            status: view.status.as_str(),
            file_path: view.file_path,
            file_old_path: view.file_old_path,
            tags: view.tags,
            confidence: view.confidence,
            external_source: view.external_source,
            author: view.author,
            created_at: view.created_at,
            sidecar_content_hash: view.sidecar_content_hash,
        }
    }
}

impl From<shore::session::event::ImportedNoteTarget> for AdapterNoteTargetDocument {
    fn from(target: shore::session::event::ImportedNoteTarget) -> Self {
        Self {
            side: target.side,
            start_line: target.start_line,
            end_line: target.end_line,
        }
    }
}

impl From<ReviewUnitProjectionRow> for UnitProjectionRowDocument {
    fn from(row: ReviewUnitProjectionRow) -> Self {
        Self {
            id: row.id.as_str().to_owned(),
            kind: row.kind.as_str(),
            projection_phase: row.projection_phase.as_str(),
            projection_order: row.projection_order,
            snapshot_order: row.snapshot_order.map(SnapshotOrderDocument::from),
            coverage: row.coverage.as_str(),
            target: row.target,
            file_path: row.file_path,
            old_path: row.old_path,
            related_observation_ids: row
                .related_observation_ids
                .into_iter()
                .map(|id| id.as_str().to_owned())
                .collect(),
            related_intervention_ids: row
                .related_intervention_ids
                .into_iter()
                .map(|id| id.as_str().to_owned())
                .collect(),
            related_disposition_ids: row
                .related_disposition_ids
                .into_iter()
                .map(|id| id.as_str().to_owned())
                .collect(),
        }
    }
}

impl From<shore::session::SnapshotOrder> for SnapshotOrderDocument {
    fn from(order: shore::session::SnapshotOrder) -> Self {
        Self {
            file_index: order.file_index,
            metadata_index: order.metadata_index,
            hunk_index: order.hunk_index,
            row_index: order.row_index,
        }
    }
}

impl From<SideArg> for Side {
    fn from(value: SideArg) -> Self {
        match value {
            SideArg::Old => Side::Old,
            SideArg::New => Side::New,
        }
    }
}

impl From<InterventionModeArg> for InterventionMode {
    fn from(value: InterventionModeArg) -> Self {
        match value {
            InterventionModeArg::Blocking => InterventionMode::Blocking,
            InterventionModeArg::Advisory => InterventionMode::Advisory,
        }
    }
}

impl From<InterventionReasonArg> for InterventionReasonCode {
    fn from(value: InterventionReasonArg) -> Self {
        match value {
            InterventionReasonArg::AmbiguousState => InterventionReasonCode::AmbiguousState,
            InterventionReasonArg::UnsafeAction => InterventionReasonCode::UnsafeAction,
            InterventionReasonArg::StaleRevision => InterventionReasonCode::StaleRevision,
            InterventionReasonArg::FailedGate => InterventionReasonCode::FailedGate,
            InterventionReasonArg::ExternalSideEffect => InterventionReasonCode::ExternalSideEffect,
            InterventionReasonArg::ConflictingEvent => InterventionReasonCode::ConflictingEvent,
            InterventionReasonArg::MissingPermission => InterventionReasonCode::MissingPermission,
            InterventionReasonArg::ManualDecisionRequired => {
                InterventionReasonCode::ManualDecisionRequired
            }
        }
    }
}

impl From<InterventionStatusArg> for InterventionStatusFilter {
    fn from(value: InterventionStatusArg) -> Self {
        match value {
            InterventionStatusArg::Open => InterventionStatusFilter::Open,
            InterventionStatusArg::Resolved => InterventionStatusFilter::Resolved,
            InterventionStatusArg::Ambiguous => InterventionStatusFilter::Ambiguous,
            InterventionStatusArg::All => InterventionStatusFilter::All,
        }
    }
}

impl From<InterventionOutcomeArg> for InterventionResolutionOutcome {
    fn from(value: InterventionOutcomeArg) -> Self {
        match value {
            InterventionOutcomeArg::Approved => InterventionResolutionOutcome::Approved,
            InterventionOutcomeArg::Rejected => InterventionResolutionOutcome::Rejected,
            InterventionOutcomeArg::Dismissed => InterventionResolutionOutcome::Dismissed,
            InterventionOutcomeArg::Superseded => InterventionResolutionOutcome::Superseded,
            InterventionOutcomeArg::Abandoned => InterventionResolutionOutcome::Abandoned,
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

impl From<HistoryEventTypeArg> for shore::session::EventType {
    fn from(value: HistoryEventTypeArg) -> Self {
        match value {
            HistoryEventTypeArg::ReviewInitialized => Self::ReviewInitialized,
            HistoryEventTypeArg::ReviewUnitCaptured => Self::ReviewUnitCaptured,
            HistoryEventTypeArg::ReviewObservationRecorded => Self::ReviewObservationRecorded,
            HistoryEventTypeArg::ReviewDispositionRecorded => Self::ReviewDispositionRecorded,
            HistoryEventTypeArg::InterventionRequested => Self::InterventionRequested,
            HistoryEventTypeArg::InterventionResolved => Self::InterventionResolved,
            HistoryEventTypeArg::ReviewNoteImported => Self::ReviewNoteImported,
        }
    }
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

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::process::{Command, ExitCode};

    use shore::dump::DumpInputSource;
    use shore::session::ImportNotesOptions;

    use super::dump::{DumpArgs, document_for_dump};
    use super::input::ReviewInputArgs;
    use super::run_with_io;
    use super::show::{ShowArgs, document_for_show};
    use crate::cli_tracing::{LogFormatArg, TracingArgs};

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
        shore::session::import_notes(
            ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar_path),
        )
        .expect("notes import succeeds");

        let input = ReviewInputArgs {
            repo: repo.path().to_owned(),
            review_notes: None,
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
        };

        document_for_show(&ShowArgs { input }, &tracing_args(None)).expect("show document builds");

        assert!(!repo.path().join(".shore").exists());
    }

    #[test]
    fn dump_and_show_prefer_explicit_review_notes_over_durable_notes() {
        let repo = dump_repo();
        let durable_sidecar = write_native_review_notes(&repo);
        shore::session::import_notes(
            ImportNotesOptions::new(repo.path()).with_review_notes(&durable_sidecar),
        )
        .unwrap();

        let explicit_path = repo.path().join("override-review-notes.json");
        fs::write(&explicit_path, explicit_review_notes_json()).expect("write explicit notes");

        let input = ReviewInputArgs {
            repo: repo.path().to_owned(),
            review_notes: Some(explicit_path),
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
