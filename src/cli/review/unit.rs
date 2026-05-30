use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand};
use shoreline::model::{ReviewTargetRef, ReviewUnitId, Side};
use shoreline::session::{
    AdapterNoteView, ReviewUnitListEntry, ReviewUnitListOptions, ReviewUnitListResult,
    ReviewUnitProjectionIdentity, ReviewUnitProjectionRow, ReviewUnitProjectionSummary,
    ReviewUnitShowFilters, ReviewUnitShowOptions, ReviewUnitShowResult, list_review_units,
    show_review_unit,
};

use crate::cli::json;
use crate::cli::review::documents::{
    AssessmentViewDocument, CurrentAssessmentDocument, InputRequestViewDocument,
    ObservationViewDocument,
};

#[derive(Debug, Args)]
pub(super) struct UnitArgs {
    #[command(subcommand)]
    command: UnitCommand,
}

#[derive(Debug, Subcommand)]
enum UnitCommand {
    List(UnitListArgs),
    Show(UnitShowArgs),
}

#[derive(Debug, Args)]
pub(super) struct UnitListArgs {
    /// Repository root or a path inside the repository.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Pretty-print the JSON response.
    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    /// Emit compact JSON explicitly.
    #[arg(long)]
    compact: bool,
}

#[derive(Debug, Args)]
pub(super) struct UnitShowArgs {
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

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::cli) struct UnitShowBody {
    event_set_hash: String,
    event_count: usize,
    review_unit: UnitReviewUnitDocument,
    filters: UnitShowFiltersDocument,
    summary: UnitShowSummaryDocument,
    current_assessment: CurrentAssessmentDocument,
    observations: Vec<ObservationViewDocument>,
    input_requests: Vec<InputRequestViewDocument>,
    assessments: Vec<AssessmentViewDocument>,
    adapter_notes: Vec<AdapterNoteDocument>,
    rows: Vec<UnitProjectionRowDocument>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UnitReviewUnitDocument {
    id: String,
    session_id: String,
    revision_id: String,
    snapshot_id: String,
    source: shoreline::model::ReviewUnitSource,
    base: shoreline::model::ReviewEndpoint,
    target: shoreline::model::ReviewEndpoint,
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
    input_request_count: usize,
    assessment_count: usize,
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
    related_input_request_ids: Vec<String>,
    related_assessment_ids: Vec<String>,
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

pub(super) fn run(
    args: UnitArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        UnitCommand::List(args) => {
            let span = tracing::info_span!("shore.review.unit.list");
            let _entered = span.enter();
            tracing::debug!(command = "review.unit.list", "command_start");
            review_unit_list_command(args, stdout)
        }
        UnitCommand::Show(args) => {
            let span = tracing::info_span!("shore.review.unit.show");
            let _entered = span.enter();
            tracing::debug!(command = "review.unit.show", "command_start");
            review_unit_show_command(args, stdout)
        }
    }
}

fn review_unit_list_command(
    args: UnitListArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty;
    let result = list_review_units(ReviewUnitListOptions::new(&args.repo))?;
    let document = unit_list_document(result);
    json::write_json(stdout, &document, pretty)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UnitListBody {
    event_set_hash: String,
    event_count: usize,
    review_unit_count: usize,
    entries: Vec<ReviewUnitListEntry>,
}

fn unit_list_document(result: ReviewUnitListResult) -> json::DiagnosticDocument<UnitListBody> {
    json::DiagnosticDocument::new(
        "shore.review-unit-list",
        UnitListBody {
            event_set_hash: result.event_set_hash,
            event_count: result.event_count,
            review_unit_count: result.review_unit_count,
            entries: result.entries,
        },
        result.diagnostics,
    )
}

fn review_unit_show_command(
    args: UnitShowArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty;
    let result = show_review_unit(review_unit_show_options(&args));
    let document = unit_show_document(result?);
    json::write_json(stdout, &document, pretty)
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

pub(in crate::cli) fn unit_show_document(
    result: ReviewUnitShowResult,
) -> json::DiagnosticDocument<UnitShowBody> {
    json::DiagnosticDocument::new(
        "shore.review-unit",
        UnitShowBody {
            event_set_hash: result.event_set_hash,
            event_count: result.event_count,
            review_unit: UnitReviewUnitDocument::from(result.review_unit),
            filters: UnitShowFiltersDocument::from(result.filters),
            summary: UnitShowSummaryDocument::from(result.summary),
            current_assessment: CurrentAssessmentDocument::from(result.current_assessment),
            observations: result
                .observations
                .into_iter()
                .map(ObservationViewDocument::from)
                .collect(),
            input_requests: result
                .input_requests
                .into_iter()
                .map(InputRequestViewDocument::from)
                .collect(),
            assessments: result
                .assessments
                .into_iter()
                .map(AssessmentViewDocument::from)
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
        },
        result.diagnostics,
    )
}

impl From<ReviewUnitProjectionIdentity> for UnitReviewUnitDocument {
    fn from(identity: ReviewUnitProjectionIdentity) -> Self {
        Self {
            id: identity.id.as_str().to_owned(),
            session_id: identity.session_id.as_str().to_owned(),
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
            input_request_count: summary.input_request_count,
            assessment_count: summary.assessment_count,
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

impl From<shoreline::session::event::ImportedNoteTarget> for AdapterNoteTargetDocument {
    fn from(target: shoreline::session::event::ImportedNoteTarget) -> Self {
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
            related_input_request_ids: row
                .related_input_request_ids
                .into_iter()
                .map(|id| id.as_str().to_owned())
                .collect(),
            related_assessment_ids: row
                .related_assessment_ids
                .into_iter()
                .map(|id| id.as_str().to_owned())
                .collect(),
        }
    }
}

impl From<shoreline::session::SnapshotOrder> for SnapshotOrderDocument {
    fn from(order: shoreline::session::SnapshotOrder) -> Self {
        Self {
            file_index: order.file_index,
            metadata_index: order.metadata_index,
            hunk_index: order.hunk_index,
            row_index: order.row_index,
        }
    }
}
