use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, ValueEnum};
use shoreline::documents::history_document;
use shoreline::model::RevisionId;
use shoreline::session::event::EventType;
use shoreline::session::{
    EventVerificationPolicy, LivenessToken, RefFilterMode, ReviewHistoryOptions, read_events,
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

    /// Filter to events of units associated with this ref; a short branch name is
    /// normalized to its full ref before matching.
    #[arg(long = "ref", alias = "branch")]
    ref_name: Option<String>,

    /// How `--ref` matches: by the recorded label (offline) or by reachability
    /// from the ref's live tip.
    #[arg(long, value_enum, default_value = "label")]
    by: HistoryRefByArg,

    /// Hydrate body-like text from inline payloads or body artifacts.
    #[arg(long)]
    include_body: bool,

    /// Pretty-print the JSON response.
    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    /// Emit compact JSON explicitly.
    #[arg(long)]
    compact: bool,

    /// Re-render whenever the store's liveness changes, polling client-side.
    /// Pull-only: no daemon and no filesystem watch. Cancel with Ctrl-C.
    #[arg(long)]
    watch: bool,

    /// Poll interval in milliseconds for `--watch`.
    #[arg(long, default_value_t = 3000)]
    poll_ms: u64,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum HistoryEventTypeArg {
    ReviewInitialized,
    RevisionCaptured,
    ReviewObservationRecorded,
    ReviewAssessmentRecorded,
    ValidationCheckRecorded,
    InputRequestOpened,
    InputRequestResponded,
    ReviewNoteImported,
    ReviewUnitLineageDeclared,
    ReviewUnitLineageRoundRecorded,
    ReviewUnitRefAssociated,
    ReviewUnitRefWithdrawn,
    ReviewUnitCommitAssociated,
    ReviewUnitCommitWithdrawn,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum HistoryRefByArg {
    #[default]
    Label,
    Liveness,
}

impl From<HistoryRefByArg> for RefFilterMode {
    fn from(by: HistoryRefByArg) -> Self {
        match by {
            HistoryRefByArg::Label => RefFilterMode::Label,
            HistoryRefByArg::Liveness => RefFilterMode::Liveness,
        }
    }
}

pub(super) fn run(
    args: HistoryArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.review.history");
    let _entered = span.enter();
    tracing::debug!(command = "review.history", "command_start");
    if args.watch {
        return watch(&args, stdout);
    }
    render_once(&args, stdout)
}

fn render_once(
    args: &HistoryArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = review_history(history_options(args));
    let document = history_document(result?);
    json::write_json(stdout, &document, args.pretty)
}

/// Client-side liveness poll: re-render only when the store's `event_set_hash`
/// moves, never on a bare tick. The core emits the change fact through the
/// liveness token but never delivers it, so this loop owns delivery — a pure
/// pull with no daemon and no filesystem watch. Cancel with Ctrl-C.
fn watch(args: &HistoryArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let interval = Duration::from_millis(args.poll_ms);
    let mut last_seen: Option<String> = None;
    loop {
        let token = LivenessToken::for_ledger(&read_events(&args.repo)?)?;
        if last_seen.as_deref() != Some(token.event_set_hash.as_str()) {
            render_once(args, stdout)?;
            stdout.flush()?;
            last_seen = Some(token.event_set_hash);
        }
        std::thread::sleep(interval);
    }
}

fn history_options(args: &HistoryArgs) -> ReviewHistoryOptions {
    let mut options = ReviewHistoryOptions::new(&args.repo).with_include_body(args.include_body);
    if let Some(review_unit) = &args.review_unit {
        options = options.with_review_unit_id(RevisionId::new(review_unit.clone()));
    }
    if let Some(track) = &args.track {
        options = options.with_track(track.clone());
    }
    for event_type in args.event_types.iter().copied() {
        options = options.with_event_type(event_type.into());
    }
    if let Some(ref_name) = &args.ref_name {
        options = options.with_ref_filter(ref_name.clone(), args.by.into());
    }
    if let Some(map) = super::common::discover_delegation_map(&args.repo) {
        options = options.with_delegation_map(map);
    }
    // Advisory policy: presence enables the verificationStatus render, never gates a write.
    options = options.with_trust_set(super::common::discover_trust_set(&args.repo));
    options = options.with_verification_policy(EventVerificationPolicy::advisory());
    // Sibling enrichment for endorsement readbacks (endorser kind/roles), reader-relative.
    options = options.with_actor_attributes(super::common::discover_actor_attributes(&args.repo));
    options
}

impl From<HistoryEventTypeArg> for EventType {
    fn from(value: HistoryEventTypeArg) -> Self {
        match value {
            HistoryEventTypeArg::ReviewInitialized => Self::ReviewInitialized,
            HistoryEventTypeArg::RevisionCaptured => Self::WorkObjectProposed,
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
            HistoryEventTypeArg::ReviewUnitRefAssociated => Self::ReviewUnitRefAssociated,
            HistoryEventTypeArg::ReviewUnitRefWithdrawn => Self::ReviewUnitRefWithdrawn,
            HistoryEventTypeArg::ReviewUnitCommitAssociated => Self::ReviewUnitCommitAssociated,
            HistoryEventTypeArg::ReviewUnitCommitWithdrawn => Self::ReviewUnitCommitWithdrawn,
        }
    }
}
