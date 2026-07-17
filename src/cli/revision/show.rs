use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use pointbreak::documents::revision_show_document;
use pointbreak::model::{EventId, RevisionId};
use pointbreak::session::event::AssertionMode;
use pointbreak::session::{
    CurrentAssessmentStatus, EventVerificationPolicy, EventVerificationStatus, InputRequestStatus,
    InputRequestView, RemovalPolicy, RevisionShowOptions, RevisionShowResult,
    diagnose_ref_continuity, effective_integration_ref, enrich_liveness, show_revision,
};

use crate::cli::common::{count_label, endpoint_label};
use crate::cli::output;

/// Show the composite view of a captured revision.
#[derive(Debug, Args)]
pub(super) struct ShowArgs {
    /// The revision to show (a head seed): a current head resolves exactly; a
    /// superseded revision resolves its thread's current head, erroring when that
    /// thread has competing heads. Omitted shows the current capture.
    #[arg(value_name = "REVISION")]
    revision: Option<String>,

    /// Repository root or a path inside the repository.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Filter narrative facts to one review track.
    #[arg(long)]
    track: Option<String>,

    /// Hydrate body-like text from inline payloads or body artifacts.
    #[arg(long)]
    include_body: bool,

    /// Reachability target for the liveness block's "merged" condition: the commit
    /// is merged only when an ancestor of this ref (equality counts). Defaults to
    /// the repository's detected default branch (`origin/HEAD`, else local
    /// `main`/`master`), so liveness answers "did this land on the default
    /// branch?"; when no default branch is detected it falls back to broad
    /// reachability (any live tip).
    #[arg(long = "integration-ref")]
    integration_ref: Option<String>,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

pub(super) fn run(
    mut args: ShowArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.revision.show");
    let _entered = span.enter();
    tracing::debug!(command = "revision.show", "command_start");

    let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
    let resolved = match &args.revision {
        Some(revision) => Some(ids.rev(revision)?),
        None => None,
    };
    args.revision = resolved;

    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    let result = show_revision(show_options(&args))?;

    // Liveness (merged/live/unreachable/missing per OID + headline) is layered
    // here, outside the git-free document workflow: best-effort, omitted when
    // reachability is unknown. Narrow by default against the repository's
    // detected default branch so the block answers "did this land on the
    // default branch?" (#445); an explicit `--integration-ref` overrides, and
    // an undetectable default falls back to broad reachability.
    let integration_ref = effective_integration_ref(&args.repo, args.integration_ref.as_deref());
    let mut liveness =
        enrich_liveness(&result.commit_range, &args.repo, integration_ref.as_deref()).ok();
    // Ref continuity (current/advanced/rewritten/moved/deleted per recorded
    // ref, with best-effort reflog rewrite evidence) rides the same block; its
    // `ref_rewritten` diagnostics join the enrichment's and surface in the
    // document's top-level diagnostics below.
    if let Some(liveness) = liveness.as_mut() {
        let continuity = diagnose_ref_continuity(&result.commit_range, &args.repo);
        liveness.ref_continuity = continuity.refs;
        liveness.diagnostics.extend(continuity.diagnostics);
    }
    // `revision_show_document` consumes `result` by value; the text digest reads
    // the same result, so clone it only when the text lane will actually render
    // (the machine lanes never pay for the clone — this is the #96 heavy command).
    let digest_source = matches!(format.format, output::OutputFormat::Text).then(|| result.clone());
    let document = revision_show_document(result);
    let mut value = serde_json::to_value(&document)?;
    if let Some(mut liveness) = liveness {
        // Enrichment-level diagnostics (divergence needs ancestry, so it is
        // liveness-derived) surface in the document's top-level diagnostics —
        // the same array the fold's per-unit diagnostics land in — rather than
        // duplicated inside the embedded liveness blob.
        let enrichment_diagnostics = std::mem::take(&mut liveness.diagnostics);
        if let Some(commit_range) = value
            .get_mut("commitRange")
            .and_then(|cr| cr.as_object_mut())
        {
            commit_range.insert("liveness".to_owned(), serde_json::to_value(liveness)?);
        }
        if !enrichment_diagnostics.is_empty()
            && let Some(diagnostics) = value.get_mut("diagnostics").and_then(|d| d.as_array_mut())
        {
            for diagnostic in enrichment_diagnostics {
                diagnostics.push(serde_json::to_value(diagnostic)?);
            }
        }
    }
    output::write_document(stdout, format, &value, || {
        render_revision_digest(
            digest_source
                .as_ref()
                .expect("text lane resolves the digest source"),
        )
    })
}

/// The #96 text digest for `revision show`: a bounded per-track summary mirroring
/// the inspector revision-page header — identity line, current call,
/// signed-by-enrolled-key, summary counts, per-track fact counts, and open input
/// requests. Never the snapshot rows (INV-6); reads only the public
/// `RevisionShowResult` (INV-12); ids truncate via `output::short_ref` (INV-7).
fn render_revision_digest(result: &RevisionShowResult) -> String {
    let mut lines: Vec<String> = Vec::new();
    let identity = &result.revision;

    lines.push(format!(
        "{} · base {} → {}",
        output::short_ref(identity.revision_id.as_str()),
        endpoint_label(&identity.base),
        endpoint_label(&identity.target),
    ));
    if let Some(summary) = &identity.summary {
        lines.push(format!(
            "summary: {}",
            crate::cli::common::clamp_title(summary)
        ));
    }

    lines.push(crate::cli::common::current_call_line(
        &result.current_assessment.status,
    ));
    // Signed-by-enrolled-key keys on the resolved call's own event id; a resolved
    // status always carries exactly one current record.
    if matches!(
        result.current_assessment.status,
        CurrentAssessmentStatus::Resolved(_)
    ) && let Some(record) = result.current_assessment.records.first()
    {
        lines.push(format!(
            "signed by enrolled key: {}",
            signed_by_enrolled_key(result, &record.event_id),
        ));
    }

    let summary = &result.summary;
    lines.push(
        [
            count_label(summary.file_count, "file", "files"),
            count_label(summary.observation_count, "observation", "observations"),
            count_label(
                summary.input_request_count,
                "input request",
                "input requests",
            ),
            count_label(summary.assessment_count, "assessment", "assessments"),
            count_label(
                summary.validation_check_count,
                "validation check",
                "validation checks",
            ),
        ]
        .join(" · "),
    );

    let tracks = group_fact_counts_by_track(result);
    if !tracks.is_empty() {
        lines.push("tracks:".to_owned());
        for (track, counts) in &tracks {
            lines.push(format!("  {track} — {counts}"));
        }
    }

    let open: Vec<&InputRequestView> = result
        .input_requests
        .iter()
        .filter(|request| request.status == InputRequestStatus::Open)
        .collect();
    if !open.is_empty() {
        lines.push("open input requests:".to_owned());
        for request in open {
            lines.push(format!(
                "  {} — \"{}\" ({})",
                output::short_ref(request.id.as_str()),
                crate::cli::common::clamp_title(&request.title),
                mode_label(request.mode),
            ));
        }
    }

    lines.join("\n")
}

/// `yes` when the resolved call's event verifies under the reader's trust set,
/// otherwise `no (<status>)` naming why (unsigned / untrusted_key / invalid) or a
/// bare `no` when no readback exists.
fn signed_by_enrolled_key(result: &RevisionShowResult, event_id: &EventId) -> String {
    match result
        .member_readbacks
        .get(event_id)
        .and_then(|readback| readback.verification_status)
    {
        Some(EventVerificationStatus::Valid) => "yes".to_owned(),
        Some(status) => format!("no ({})", status.as_str()),
        None => "no".to_owned(),
    }
}

fn mode_label(mode: AssertionMode) -> &'static str {
    match mode {
        AssertionMode::Advisory => "advisory",
        AssertionMode::Operative => "operative",
    }
}

/// Per-track fact tallies, rendered as `N observations · N validation checks`,
/// only the non-zero fact types. `BTreeMap` keeps the track order deterministic.
#[derive(Default)]
struct TrackFactCounts {
    observations: usize,
    input_requests: usize,
    assessments: usize,
    validation_checks: usize,
}

impl TrackFactCounts {
    fn render(&self) -> String {
        [
            (self.observations, "observation", "observations"),
            (self.input_requests, "input request", "input requests"),
            (self.assessments, "assessment", "assessments"),
            (
                self.validation_checks,
                "validation check",
                "validation checks",
            ),
        ]
        .into_iter()
        .filter(|(count, _, _)| *count > 0)
        .map(|(count, singular, plural)| count_label(count, singular, plural))
        .collect::<Vec<_>>()
        .join(" · ")
    }
}

fn group_fact_counts_by_track(result: &RevisionShowResult) -> BTreeMap<&str, String> {
    let mut counts: BTreeMap<&str, TrackFactCounts> = BTreeMap::new();
    for observation in &result.observations {
        counts
            .entry(observation.track_id.as_str())
            .or_default()
            .observations += 1;
    }
    for request in &result.input_requests {
        counts
            .entry(request.track_id.as_str())
            .or_default()
            .input_requests += 1;
    }
    for assessment in &result.assessments {
        counts
            .entry(assessment.track_id.as_str())
            .or_default()
            .assessments += 1;
    }
    for check in &result.validation_checks {
        counts
            .entry(check.track_id.as_str())
            .or_default()
            .validation_checks += 1;
    }
    counts
        .into_iter()
        .map(|(track, tally)| (track, tally.render()))
        .collect()
}

fn show_options(args: &ShowArgs) -> RevisionShowOptions {
    let mut options = RevisionShowOptions::new(&args.repo)
        .with_include_body(args.include_body)
        .with_read_for_display(true);
    if let Some(revision) = &args.revision {
        options = options.with_revision_id(RevisionId::new(revision.clone()));
    }
    if let Some(track) = &args.track {
        options = options.with_track(track.clone());
    }
    if let Some(map) = crate::cli::common::discover_delegation_map(&args.repo) {
        options = options.with_delegation_map(map);
    }
    // Advisory policy + reader trust: enable the per-event verificationStatus +
    // endorsement readback, reader-relative; render-only, never a gate.
    options = options
        .with_trust_set(crate::cli::common::discover_trust_set(&args.repo))
        .with_verification_policy(EventVerificationPolicy::advisory())
        .with_removal_policy(RemovalPolicy::default())
        .with_actor_attributes(crate::cli::common::discover_actor_attributes(&args.repo));
    options
}
