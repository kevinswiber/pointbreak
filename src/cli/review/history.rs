use std::io::Write;
use std::path::PathBuf;

use clap::{Args, ValueEnum};
use shore::model::ReviewUnitId;
use shore::session::event::EventType;
use shore::session::{
    ReviewHistoryEntry, ReviewHistoryFilters, ReviewHistoryOptions, ReviewHistoryResult,
    review_history,
};

use crate::cli::json;

#[derive(Debug, Args)]
pub(super) struct HistoryArgs {
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

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryBody {
    event_set_hash: String,
    event_count: usize,
    history_count: usize,
    filters: ReviewHistoryFilters,
    entries: Vec<ReviewHistoryEntry>,
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

pub(super) fn run(
    args: HistoryArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.review.history");
    let _entered = span.enter();
    tracing::debug!(command = "review.history", "command_start");
    let pretty = args.pretty;
    let result = review_history(history_options(&args));
    let document = history_document(result?);
    json::write_json(stdout, &document, pretty)
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

fn history_document(result: ReviewHistoryResult) -> json::DiagnosticDocument<HistoryBody> {
    let history_count = result.history_count();
    json::DiagnosticDocument::new(
        "shore.review-history",
        HistoryBody {
            event_set_hash: result.event_set_hash,
            event_count: result.event_count,
            history_count,
            filters: result.filters,
            entries: result.entries,
        },
        result.diagnostics,
    )
}

impl From<HistoryEventTypeArg> for EventType {
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
