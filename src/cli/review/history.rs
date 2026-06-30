use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, ValueEnum};
use shoreline::documents::history_document;
use shoreline::model::RevisionId;
use shoreline::session::event::EventType;
use shoreline::session::{
    EventVerificationPolicy, HistoryCursor, LivenessToken, RefFilterMode, ReviewHistoryOptions,
    read_events_for_display, review_history,
};

use crate::cli::json;

#[derive(Debug, Args)]
pub(super) struct HistoryArgs {
    /// Repository root or a path inside the repository.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Filter to one captured Revision by id.
    #[arg(long)]
    revision: Option<String>,

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

    /// Return at most N entries (a forward page from the start, or from --cursor);
    /// omit for the full history. The response carries a `nextCursor` to continue.
    /// With --watch, the same page is re-rendered on each liveness change.
    #[arg(long)]
    limit: Option<usize>,

    /// Continue from a previous response's opaque `nextCursor`. Omit to start from
    /// the beginning.
    #[arg(long)]
    cursor: Option<String>,

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
    RevisionRefAssociated,
    RevisionRefWithdrawn,
    RevisionCommitAssociated,
    RevisionCommitWithdrawn,
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
    let result = review_history(history_options(args)?);
    let document = history_document(result?);
    json::write_json(stdout, &document, args.pretty)
}

/// Client-side liveness poll: re-render only when the store's liveness moves —
/// either its decodable `event_set_hash` changes or the number of skip
/// diagnostics changes — never on a bare tick. The second trigger matters because
/// a lone retired event is skipped from the event set, so it would leave the hash
/// untouched; folding the diagnostic count in surfaces it. The core emits the
/// change fact but never delivers it, so this loop owns delivery — a pure pull
/// with no daemon and no filesystem watch. Cancel with Ctrl-C.
fn watch(args: &HistoryArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let interval = Duration::from_millis(args.poll_ms);
    let mut last_seen: Option<(String, usize)> = None;
    loop {
        let fingerprint = watch_fingerprint(&args.repo)?;
        if last_seen.as_ref() != Some(&fingerprint) {
            render_once(args, stdout)?;
            stdout.flush()?;
            last_seen = Some(fingerprint);
        }
        std::thread::sleep(interval);
    }
}

/// The poll fingerprint: the decodable-event-set hash paired with the
/// skip-diagnostic count. Either component moving means the rendered history
/// would change, so `--watch` re-renders when the pair does.
fn watch_fingerprint(
    repo: &std::path::Path,
) -> Result<(String, usize), Box<dyn std::error::Error>> {
    let (events, diagnostics) = read_events_for_display(repo)?;
    let token = LivenessToken::for_journal(&events)?;
    Ok((token.event_set_hash, diagnostics.len()))
}

fn history_options(args: &HistoryArgs) -> Result<ReviewHistoryOptions, Box<dyn std::error::Error>> {
    let mut options = ReviewHistoryOptions::new(&args.repo)
        .with_include_body(args.include_body)
        .with_read_for_display(true);
    if let Some(limit) = args.limit {
        options = options.with_limit(limit);
    }
    if let Some(token) = &args.cursor {
        let cursor = HistoryCursor::decode(token)
            .map_err(|_| "invalid --cursor: pass an opaque nextCursor from a prior response")?;
        options = options.with_cursor(cursor);
    }
    if let Some(revision) = &args.revision {
        options = options.with_revision_id(RevisionId::new(revision.clone()));
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
    Ok(options)
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
            HistoryEventTypeArg::RevisionRefAssociated => Self::RevisionRefAssociated,
            HistoryEventTypeArg::RevisionRefWithdrawn => Self::RevisionRefWithdrawn,
            HistoryEventTypeArg::RevisionCommitAssociated => Self::RevisionCommitAssociated,
            HistoryEventTypeArg::RevisionCommitWithdrawn => Self::RevisionCommitWithdrawn,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_fingerprint_changes_when_only_a_retired_event_appears() {
        let repo = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo.path())
            .output()
            .unwrap();

        let before = watch_fingerprint(repo.path()).unwrap();

        // Drop a raw retired-type event into the resolved store; no valid-event
        // change, so only the skip-diagnostic count moves.
        let events_dir = shoreline::session::store_dir_for_repo(repo.path())
            .unwrap()
            .join("events");
        std::fs::create_dir_all(&events_dir).unwrap();
        std::fs::write(
            events_dir.join(format!("{}.json", "a".repeat(64))),
            br#"{"eventType":"review_disposition_recorded"}"#,
        )
        .unwrap();

        let after = watch_fingerprint(repo.path()).unwrap();

        assert_ne!(
            before, after,
            "a new retired event must move the watch fingerprint"
        );
        assert_eq!(
            after.1,
            before.1 + 1,
            "the skip-diagnostic count increments"
        );
    }
}
