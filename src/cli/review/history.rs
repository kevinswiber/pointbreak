use std::io::Write;
use std::path::PathBuf;

use clap::{Args, ValueEnum};
use shoreline::documents::history_document;
use shoreline::model::ReviewUnitId;
use shoreline::session::event::EventType;
use shoreline::session::{ReviewHistoryOptions, review_history};

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

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum HistoryEventTypeArg {
    ReviewInitialized,
    ReviewUnitCaptured,
    ReviewObservationRecorded,
    ReviewAssessmentRecorded,
    ValidationCheckRecorded,
    InputRequestOpened,
    InputRequestResponded,
    ReviewNoteImported,
    ReviewUnitLineageDeclared,
    ReviewUnitLineageRoundRecorded,
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
    if let Some(map) = super::common::discover_delegation_map(&args.repo) {
        options = options.with_delegation_map(map);
    }
    options
}

impl From<HistoryEventTypeArg> for EventType {
    fn from(value: HistoryEventTypeArg) -> Self {
        match value {
            HistoryEventTypeArg::ReviewInitialized => Self::ReviewInitialized,
            HistoryEventTypeArg::ReviewUnitCaptured => Self::ReviewUnitCaptured,
            HistoryEventTypeArg::ReviewObservationRecorded => Self::ReviewObservationRecorded,
            HistoryEventTypeArg::ReviewAssessmentRecorded => Self::ReviewAssessmentRecorded,
            HistoryEventTypeArg::ValidationCheckRecorded => Self::ValidationCheckRecorded,
            HistoryEventTypeArg::InputRequestOpened => Self::InputRequestOpened,
            HistoryEventTypeArg::InputRequestResponded => Self::InputRequestResponded,
            HistoryEventTypeArg::ReviewNoteImported => Self::ReviewNoteImported,
            HistoryEventTypeArg::ReviewUnitLineageDeclared => Self::ReviewUnitLineageDeclared,
            HistoryEventTypeArg::ReviewUnitLineageRoundRecorded => {
                Self::ReviewUnitLineageRoundRecorded
            }
        }
    }
}
