//! The revision-surface search record: a pure projection of an
//! already-projected revision into the field set the shared query grammar
//! matches (`parse_search_query_for` + `matches_query` over a `SearchRecord`).
//! The event surface builds its own record in `history::search`; the two
//! records share match-kind semantics and the set encoding, never contents.

use std::collections::{BTreeMap, BTreeSet};

use super::{RevisionOverview, RevisionProjectionSummary};
use crate::model::{RevisionId, ValidationStatus};
use crate::session::event::ReviewAssessment;
use crate::session::identity::instant::normalize_instant_to_iso_millis;
use crate::session::workflow::history::{
    RANGE_ANCHOR_FIELD, REVISION_ATTENTION_VALUES, SearchRecord, enum_wire, tag_index_tokens,
    wrap_set,
};
use crate::session::workflow::revision_list::RevisionListEntry;
use crate::session::{CurrentAssessmentStatus, InputRequestStatus};

/// A revision's search record — the same `SearchRecord` shape the event surface
/// builds, filtered by the same matchers, with revision-surface field contents.
pub struct RevisionSearchRecord(pub SearchRecord);

/// Already-projected inputs for one revision's record. The builder is pure —
/// it never re-reads the store; the caller resolves the classification via
/// `revision_supersession_classification`.
pub struct RevisionRecordInputs<'a> {
    pub entry: &'a RevisionListEntry,
    pub overview: &'a RevisionOverview,
    /// `"head"` | `"superseded"` | `"isolated"`.
    pub classification_state: &'a str,
    /// The revision belongs to a supersession component with more than one
    /// current head (all members, symmetric).
    pub competing: bool,
}

/// Build the revision-surface search record from projected inputs.
///
/// Multi-valued fields (`track`/`actor`/`tag`/`is`/`attention`) store the
/// space-wrapped token-set encoding, lowercased at build time by [`wrap_set`]
/// so an already-lowercased query value matches by whole-token membership.
pub fn build_revision_search_record(inputs: RevisionRecordInputs<'_>) -> RevisionSearchRecord {
    let entry = inputs.entry;
    let overview = inputs.overview;

    // Track/actor: the union over every review-fact family on the revision.
    let mut tracks: BTreeSet<String> = BTreeSet::new();
    let mut actors: BTreeSet<String> = BTreeSet::new();
    for observation in &overview.observations {
        tracks.insert(observation.track_id.as_str().to_owned());
        actors.insert(observation.writer.actor_id.as_str().to_owned());
    }
    for request in &overview.input_requests {
        tracks.insert(request.track_id.as_str().to_owned());
        actors.insert(request.writer.actor_id.as_str().to_owned());
    }
    for assessment in &overview.assessments {
        tracks.insert(assessment.track_id.as_str().to_owned());
        actors.insert(assessment.writer.actor_id.as_str().to_owned());
    }
    for check in &overview.validation_checks {
        tracks.insert(check.track_id.as_str().to_owned());
        actors.insert(check.writer.actor_id.as_str().to_owned());
    }

    // Tags: observations are the only tag-bearing family; each tag dual-indexes
    // its full string and its first-colon key, deduplicated across facts.
    let tag_tokens: BTreeSet<String> =
        tag_index_tokens(overview.observations.iter().flat_map(|o| o.tags.iter()))
            .into_iter()
            .collect();

    // The resolved current-assessment wire value only — unassessed and
    // ambiguous both stay empty (an ambiguous revision can carry member
    // values that must not leak through this field).
    let assessment = match &overview.current_assessment.status {
        CurrentAssessmentStatus::Resolved(assessment) => enum_wire(assessment),
        CurrentAssessmentStatus::Unassessed | CurrentAssessmentStatus::Ambiguous(_) => {
            String::new()
        }
    };

    let open_request_count = overview
        .input_requests
        .iter()
        .filter(|request| request.status == InputRequestStatus::Open)
        .count();
    let answered = overview
        .input_requests
        .iter()
        .any(|request| request.status == InputRequestStatus::Responded);
    let unassessed = overview.current_assessment.status == CurrentAssessmentStatus::Unassessed;
    let follow_up = current_assessment_includes_follow_up(&overview.current_assessment.status);
    let stale = stale_review_fact_count(&overview.superseded_by, &overview.summary) > 0;
    let validation_context = overview.validation_checks.iter().any(|check| {
        matches!(
            check.status,
            ValidationStatus::Failed | ValidationStatus::Errored
        )
    });

    let mut is_tokens: Vec<&str> = Vec::new();
    if open_request_count > 0 {
        is_tokens.push("open");
    }
    if answered {
        is_tokens.push("answered");
    }
    if unassessed {
        is_tokens.push("unassessed");
    }
    if stale {
        is_tokens.push("stale");
    }
    if follow_up {
        is_tokens.push("follow-up");
    }
    if inputs.competing {
        is_tokens.push("contested");
    }
    if inputs.classification_state == "superseded" {
        is_tokens.push("superseded");
    }

    // The attention tokens are emitted FROM the authoritative constant — the
    // flag order matches REVISION_ATTENTION_VALUES member-for-member, so a
    // vocabulary change fails loudly in one place (the membership test).
    let attention_flags = [
        open_request_count > 0,
        unassessed,
        validation_context,
        follow_up,
        stale,
    ];
    let attention_tokens: Vec<String> = REVISION_ATTENTION_VALUES
        .iter()
        .zip(attention_flags)
        .filter(|(_, on)| *on)
        .map(|(token, _)| (*token).to_owned())
        .collect();

    // The range anchor: capturedAt normalized to the one canonical fixed-width
    // ISO-8601 UTC form via the shared instant normalizer, stored under the
    // shared anchor key ("" when unparseable, so before:/after: never match).
    let anchor = normalize_instant_to_iso_millis(&entry.captured_at).unwrap_or_default();

    let mut fields = BTreeMap::new();
    // Kept for haystack/record parity with the client arm; `type:` is a
    // known-but-unsupported qualifier on this surface (the parser rejects it).
    fields.insert("type".to_owned(), "revision".to_owned());
    fields.insert("revision".to_owned(), entry.revision_id.as_str().to_owned());
    fields.insert("snapshot".to_owned(), entry.object_id.as_str().to_owned());
    fields.insert(RANGE_ANCHOR_FIELD.to_owned(), anchor);
    fields.insert("track".to_owned(), wrap_set(tracks));
    fields.insert("actor".to_owned(), wrap_set(actors));
    fields.insert("tag".to_owned(), wrap_set(tag_tokens));
    fields.insert("assessment".to_owned(), assessment.clone());
    fields.insert(
        "is".to_owned(),
        wrap_set(is_tokens.into_iter().map(str::to_owned)),
    );
    fields.insert(
        "attention".to_owned(),
        wrap_set(attention_tokens.iter().cloned()),
    );

    // The free-text haystack: ids, the current-assessment standing, every
    // fact's human title (a superset of the client's latest-activity fold —
    // deriving "latest" here would re-spell the inspector's precedence for no
    // filtering gain), the attention tokens, and the fixed affordance words
    // the client haystack carries.
    let mut parts: Vec<String> = vec![
        entry.revision_id.as_str().to_owned(),
        entry.object_id.as_str().to_owned(),
        entry.summary.clone().unwrap_or_default(),
        overview.current_assessment.status.as_str().to_owned(),
        assessment,
    ];
    parts.extend(overview.observations.iter().map(|o| o.title.clone()));
    parts.extend(overview.input_requests.iter().map(|r| r.title.clone()));
    parts.extend(
        overview
            .assessments
            .iter()
            .filter_map(|a| a.summary.clone()),
    );
    parts.extend(
        overview
            .validation_checks
            .iter()
            .map(|c| c.check_name.clone()),
    );
    parts.extend(attention_tokens);
    parts.push("review cues".to_owned());
    parts.push("attention".to_owned());
    parts.retain(|part| !part.is_empty());
    let text = parts.join(" ").to_lowercase();

    RevisionSearchRecord(SearchRecord { text, fields })
}

/// Advisory count of a revision's review facts that target a now-superseded
/// revision. Non-zero only when the revision itself is superseded; sums the
/// four review-fact families (observations, input requests, assessments,
/// validation checks). Adapter notes are excluded (ingestion provenance, not a
/// review assertion). Never gates — it feeds an attention readback only.
pub fn stale_review_fact_count(
    superseded_by: &BTreeSet<RevisionId>,
    summary: &RevisionProjectionSummary,
) -> usize {
    if superseded_by.is_empty() {
        0
    } else {
        summary.observation_count
            + summary.input_request_count
            + summary.assessment_count
            + summary.validation_check_count
    }
}

/// Whether the current assessment includes an accepted-with-follow-up verdict —
/// resolved to it outright, or ambiguous with it among the competing records.
pub fn current_assessment_includes_follow_up(status: &CurrentAssessmentStatus) -> bool {
    match status {
        CurrentAssessmentStatus::Resolved(ReviewAssessment::AcceptedWithFollowUp) => true,
        CurrentAssessmentStatus::Ambiguous(assessments) => {
            assessments.contains(&ReviewAssessment::AcceptedWithFollowUp)
        }
        CurrentAssessmentStatus::Unassessed | CurrentAssessmentStatus::Resolved(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::model::{
        ActorId, EventId, InputRequestId, ObjectId, ObservationId, ReviewEndpoint, ReviewTargetRef,
        RevisionId, RevisionSource, TrackId, ValidationCheckId, ValidationTarget,
        ValidationTrigger, WorktreeCaptureMode,
    };
    use crate::session::event::{AssertionMode, InputRequestReasonCode, ReviewAssessment, Writer};
    use crate::session::identity::instant::normalize_instant_to_iso_millis;
    use crate::session::workflow::revision_projection::RevisionProjectionSummary;
    use crate::session::{
        CurrentAssessmentStatus, CurrentAssessmentView, InputRequestStatus, InputRequestView,
        ObservationStatus, ObservationView, QuerySurface, RANGE_ANCHOR_FIELD,
        REVISION_ATTENTION_VALUES, RevisionCommitRangeView, RevisionListEntry, RevisionOverview,
        SearchRecord, ValidationCheckView, matches_query, parse_search_query_for,
    };

    fn entry(revision: &str, snapshot: &str, captured_at: &str) -> RevisionListEntry {
        RevisionListEntry {
            captured_at: captured_at.to_owned(),
            revision_id: RevisionId::new(revision),
            summary: None,
            object_id: ObjectId::new(snapshot),
            source: RevisionSource::GitWorktree {
                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                include_untracked: true,
                pathspecs: Vec::new(),
            },
            base: ReviewEndpoint::GitCommit {
                commit_oid: "base".to_owned(),
                tree_oid: "base-tree".to_owned(),
            },
            target: ReviewEndpoint::GitWorkingTree {
                worktree_root: "/repo".to_owned(),
            },
            object_artifact_content_hash: "sha256:artifact".to_owned(),
            commit_range: RevisionCommitRangeView {
                revision_id: RevisionId::new(revision),
                anchored: false,
                current_commits: Vec::new(),
                current_refs: Vec::new(),
                withdrawn_commits: Vec::new(),
                withdrawn_refs: Vec::new(),
                diagnostics: Vec::new(),
            },
            merge_status: "unknown".to_owned(),
            grouped_revision_ids: vec![RevisionId::new(revision)],
            merge_status_view: None,
        }
    }

    fn observation(track: &str, actor: &str, tags: &[&str]) -> ObservationView {
        let mut view = ObservationView {
            id: ObservationId::new("obs:sha256:x"),
            event_id: EventId::new("evt:sha256:obs"),
            track_id: TrackId::new(track),
            target: ReviewTargetRef::Revision {
                revision_id: RevisionId::new("rev:sha256:one"),
            },
            title: "obs".to_owned(),
            body: None,
            body_content_type: Default::default(),
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            confidence: None,
            status: ObservationStatus::Active,
            supersedes: vec![],
            responds_to: vec![],
            responded_by: vec![],
            body_content_hash: None,
            body_content_state: Default::default(),
            created_at: "2026-05-13T10:00:01Z".to_owned(),
            writer: Writer::shore_local("test"),
        };
        view.writer.actor_id = ActorId::new(actor);
        view
    }

    fn input_request(status: InputRequestStatus) -> InputRequestView {
        InputRequestView {
            id: InputRequestId::new("input-request:sha256:x"),
            event_id: EventId::new("evt:sha256:req"),
            track_id: TrackId::new("agent:codex"),
            target: ReviewTargetRef::Revision {
                revision_id: RevisionId::new("rev:sha256:one"),
            },
            mode: AssertionMode::Advisory,
            reason_code: InputRequestReasonCode::ManualDecisionRequired,
            title: "req".to_owned(),
            body: None,
            body_content_type: Default::default(),
            body_content_hash: None,
            body_content_state: Default::default(),
            status,
            responses: vec![],
            created_at: "2026-05-13T10:00:02Z".to_owned(),
            writer: Writer::shore_local("test"),
        }
    }

    fn validation_check(status: ValidationStatus) -> ValidationCheckView {
        ValidationCheckView {
            id: ValidationCheckId::new("validation-check:sha256:x"),
            event_id: EventId::new("evt:sha256:val"),
            track_id: TrackId::new("agent:codex"),
            target: ValidationTarget::Revision {
                revision_id: RevisionId::new("rev:sha256:one"),
            },
            check_name: "cargo test".to_owned(),
            command: None,
            status,
            exit_code: Some(1),
            trigger: ValidationTrigger::Manual,
            source_fingerprint: None,
            summary: None,
            summary_content_type: Default::default(),
            summary_content_hash: None,
            summary_content_state: Default::default(),
            started_at: None,
            completed_at: None,
            log_artifact_content_hashes: Vec::new(),
            created_at: "2026-05-13T10:00:03Z".to_owned(),
            writer: Writer::shore_local("test"),
            superseded_by_revisions: Default::default(),
        }
    }

    /// A bare overview with the given observations; other fact families empty,
    /// unassessed, not superseded.
    fn overview_with_observations(observations: Vec<ObservationView>) -> RevisionOverview {
        RevisionOverview {
            summary: RevisionProjectionSummary::default(),
            current_assessment: CurrentAssessmentView {
                status: CurrentAssessmentStatus::Unassessed,
                records: vec![],
            },
            observations,
            input_requests: vec![],
            assessments: vec![],
            validation_checks: vec![],
            diagnostics: vec![],
            superseded_by: BTreeSet::new(),
        }
    }

    fn record_of(
        entry: &RevisionListEntry,
        overview: &RevisionOverview,
        state: &str,
        competing: bool,
    ) -> SearchRecord {
        build_revision_search_record(RevisionRecordInputs {
            entry,
            overview,
            classification_state: state,
            competing,
        })
        .0
    }

    // Assert the record's field contents via the public `SearchRecord::field` —
    // the space-wrapped encoding read directly, like the `record.field("type")`
    // checks in the event-grammar history/search.rs. (The `track:`/`actor:`
    // match kind itself is that module's; end-to-end coverage goes through
    // `parse_search_query_for` + `matches_query`.)

    #[test]
    fn track_and_actor_are_space_wrapped_unions_over_facts() {
        let overview = overview_with_observations(vec![
            observation("agent:codex", "actor:agent:codex", &[]),
            observation("agent:claude", "actor:agent:claude", &[]),
        ]);
        let e = entry("rev:sha256:one", "snap:sha256:one", "2026-05-13T10:00:00Z");
        let record = record_of(&e, &overview, "head", false);
        let track = record.field("track").unwrap();
        assert!(
            track.contains(" agent:codex ") && track.contains(" agent:claude "),
            "{track}"
        );
        let actor = record.field("actor").unwrap();
        assert!(actor.contains(" actor:agent:codex ") && actor.contains(" actor:agent:claude "));
    }

    #[test]
    fn tag_dual_indexes_full_string_and_first_colon_key() {
        let overview = overview_with_observations(vec![observation(
            "agent:codex",
            "actor:agent:codex",
            &["issue:191"],
        )]);
        let e = entry("rev:sha256:one", "snap:sha256:one", "2026-05-13T10:00:00Z");
        let record = record_of(&e, &overview, "head", false);
        let tag = record.field("tag").unwrap();
        assert!(tag.contains(" issue:191 "), "full string: {tag}");
        assert!(tag.contains(" issue "), "first-colon key: {tag}");
    }

    #[test]
    fn revision_record_lowercases_tokens_for_mixed_case_sources() {
        // A mixed-case fact source must match a lowercased query value (parity
        // with the client arm's build-time lowercasing).
        let overview = overview_with_observations(vec![observation(
            "Agent:Codex",
            "Actor:Agent:Codex",
            &["Issue:191"],
        )]);
        let e = entry("rev:sha256:one", "snap:sha256:one", "2026-05-13T10:00:00Z");
        let record = record_of(&e, &overview, "head", false);
        assert!(record.field("track").unwrap().contains(" agent:codex "));
        assert!(
            record
                .field("actor")
                .unwrap()
                .contains(" actor:agent:codex ")
        );
        let tag = record.field("tag").unwrap();
        assert!(tag.contains(" issue:191 ") && tag.contains(" issue "));
        // End-to-end: lowercased query values match the lowercased record tokens.
        let by_track = parse_search_query_for("track:agent:codex", QuerySurface::Revision);
        let by_tag = parse_search_query_for("tag:issue", QuerySurface::Revision);
        assert!(matches_query(&record, &by_track.clauses));
        assert!(matches_query(&record, &by_tag.clauses));
    }

    #[test]
    fn revision_attention_tokens_are_members_of_the_constant() {
        // A bare unassessed overview emits the `unassessed` attention cue and no
        // other.
        let overview = overview_with_observations(vec![]);
        let e = entry("rev:sha256:one", "snap:sha256:one", "2026-05-13T10:00:00Z");
        let record = record_of(&e, &overview, "head", false);
        let attention = record.field("attention").unwrap();
        assert!(
            attention.contains(" unassessed "),
            "unassessed cue emitted: {attention}"
        );
        // Every emitted token is a member of the authoritative constant — a
        // future vocabulary change fails loudly in this one place.
        for token in attention.split_whitespace() {
            assert!(
                REVISION_ATTENTION_VALUES.contains(&token),
                "attention token {token:?} not in REVISION_ATTENTION_VALUES"
            );
        }
    }

    #[test]
    fn attention_set_carries_open_request_cue() {
        let mut overview = overview_with_observations(vec![]);
        overview.input_requests = vec![input_request(InputRequestStatus::Open)];
        let e = entry("rev:sha256:one", "snap:sha256:one", "2026-05-13T10:00:00Z");
        let record = record_of(&e, &overview, "head", false);
        let attention = record.field("attention").unwrap();
        assert!(attention.contains(" open-request "), "{attention}");
    }

    #[test]
    fn attention_set_carries_validation_context_follow_up_and_stale_cues() {
        let e = entry("rev:sha256:one", "snap:sha256:one", "2026-05-13T10:00:00Z");

        // A failed validation check raises the validation-context cue.
        let mut failing = overview_with_observations(vec![]);
        failing.validation_checks = vec![validation_check(ValidationStatus::Failed)];
        let record = record_of(&e, &failing, "head", false);
        assert!(
            record
                .field("attention")
                .unwrap()
                .contains(" validation-context "),
            "{:?}",
            record.field("attention")
        );

        // A resolved accepted-with-follow-up assessment raises the follow-up cue.
        let mut follow_up = overview_with_observations(vec![]);
        follow_up.current_assessment = CurrentAssessmentView {
            status: CurrentAssessmentStatus::Resolved(ReviewAssessment::AcceptedWithFollowUp),
            records: vec![],
        };
        let record = record_of(&e, &follow_up, "head", false);
        assert!(record.field("attention").unwrap().contains(" follow-up "));

        // A superseded revision with review facts raises the stale-fact cue.
        let mut stale = overview_with_observations(vec![]);
        stale.superseded_by = [RevisionId::new("rev:sha256:two")].into_iter().collect();
        stale.summary.observation_count = 1;
        let record = record_of(&e, &stale, "superseded", false);
        assert!(record.field("attention").unwrap().contains(" stale-fact "));
    }

    #[test]
    fn is_set_carries_superseded_and_contested_symmetrically() {
        let overview = overview_with_observations(vec![]);
        let e = entry("rev:sha256:one", "snap:sha256:one", "2026-05-13T10:00:00Z");
        let superseded = record_of(&e, &overview, "superseded", false);
        assert!(superseded.field("is").unwrap().contains(" superseded "));
        assert!(!superseded.field("is").unwrap().contains(" contested "));
        // is:contested marks ALL members of a competing component.
        let contested = record_of(&e, &overview, "head", true);
        assert!(contested.field("is").unwrap().contains(" contested "));
    }

    #[test]
    fn is_set_derives_open_and_answered_from_the_request_lifecycle() {
        let e = entry("rev:sha256:one", "snap:sha256:one", "2026-05-13T10:00:00Z");
        let mut open = overview_with_observations(vec![]);
        open.input_requests = vec![input_request(InputRequestStatus::Open)];
        let record = record_of(&e, &open, "head", false);
        assert!(record.field("is").unwrap().contains(" open "));
        assert!(!record.field("is").unwrap().contains(" answered "));

        let mut answered = overview_with_observations(vec![]);
        answered.input_requests = vec![input_request(InputRequestStatus::Responded)];
        let record = record_of(&e, &answered, "head", false);
        assert!(record.field("is").unwrap().contains(" answered "));
        assert!(!record.field("is").unwrap().contains(" open "));

        // Ambiguous (multiple responses) is a real projected state that is
        // neither open nor answered — no lifecycle token, on either arm.
        let mut ambiguous = overview_with_observations(vec![]);
        ambiguous.input_requests = vec![input_request(InputRequestStatus::Ambiguous)];
        let record = record_of(&e, &ambiguous, "head", false);
        assert!(!record.field("is").unwrap().contains(" answered "));
        assert!(!record.field("is").unwrap().contains(" open "));
    }

    #[test]
    fn is_set_derives_unassessed_follow_up_and_stale() {
        let e = entry("rev:sha256:one", "snap:sha256:one", "2026-05-13T10:00:00Z");
        // Unassessed by default.
        let unassessed = overview_with_observations(vec![]);
        let record = record_of(&e, &unassessed, "head", false);
        assert!(record.field("is").unwrap().contains(" unassessed "));

        // Resolved(AcceptedWithFollowUp) → follow-up, not unassessed.
        let mut follow_up = overview_with_observations(vec![]);
        follow_up.current_assessment = CurrentAssessmentView {
            status: CurrentAssessmentStatus::Resolved(ReviewAssessment::AcceptedWithFollowUp),
            records: vec![],
        };
        let record = record_of(&e, &follow_up, "head", false);
        assert!(record.field("is").unwrap().contains(" follow-up "));
        assert!(!record.field("is").unwrap().contains(" unassessed "));

        // Stale: a superseded revision with review facts (mirrors
        // stale_review_fact_count — nonzero only when superseded_by is non-empty).
        let mut stale = overview_with_observations(vec![]);
        stale.superseded_by = [RevisionId::new("rev:sha256:two")].into_iter().collect();
        stale.summary.observation_count = 1;
        let record = record_of(&e, &stale, "superseded", false);
        assert!(record.field("is").unwrap().contains(" stale "));
    }

    #[test]
    fn assessment_field_is_resolved_wire_value() {
        let overview = RevisionOverview {
            current_assessment: CurrentAssessmentView {
                status: CurrentAssessmentStatus::Resolved(ReviewAssessment::Accepted),
                records: vec![],
            },
            ..overview_with_observations(vec![])
        };
        let e = entry("rev:sha256:one", "snap:sha256:one", "2026-05-13T10:00:00Z");
        let record = record_of(&e, &overview, "head", false);
        assert_eq!(record.field("assessment"), Some("accepted"));
    }

    #[test]
    fn assessment_field_is_empty_when_unassessed_or_ambiguous() {
        let e = entry("rev:sha256:one", "snap:sha256:one", "2026-05-13T10:00:00Z");
        let unassessed = overview_with_observations(vec![]);
        let record = record_of(&e, &unassessed, "head", false);
        assert_eq!(record.field("assessment"), Some(""));

        // Ambiguous never leaks a member value through the assessment field.
        let ambiguous = RevisionOverview {
            current_assessment: CurrentAssessmentView {
                status: CurrentAssessmentStatus::Ambiguous(vec![
                    ReviewAssessment::Accepted,
                    ReviewAssessment::NeedsChanges,
                ]),
                records: vec![],
            },
            ..overview_with_observations(vec![])
        };
        let record = record_of(&e, &ambiguous, "head", false);
        assert_eq!(record.field("assessment"), Some(""));
    }

    #[test]
    fn captured_at_unix_ms_is_normalized_to_iso_and_range_matches() {
        // Real stores mint `unix-ms:<millis>` tokens; 1783303159002 ms is in 2026.
        let overview = overview_with_observations(vec![]);
        let e = entry("rev:sha256:one", "snap:sha256:one", "unix-ms:1783303159002");
        let record = record_of(&e, &overview, "head", false);
        let anchor = record.field(RANGE_ANCHOR_FIELD).unwrap();
        assert!(
            !anchor.starts_with("unix-ms:"),
            "the raw token is never stored: {anchor}"
        );
        // Couples to the shared normalizer (precision-agnostic); its canonical
        // output carries the fixed-width `.mmm` fraction and a trailing `Z`.
        assert_eq!(
            anchor,
            normalize_instant_to_iso_millis("unix-ms:1783303159002").unwrap()
        );
        assert!(
            anchor.contains('.') && anchor.ends_with('Z'),
            "canonical .mmm form: {anchor}"
        );
        // End-to-end range compare via the shared matcher (RangeAfter/RangeBefore
        // read the anchor).
        let after_ok = parse_search_query_for("after:2026-01-01", QuerySurface::Revision);
        let after_future = parse_search_query_for("after:2030-01-01", QuerySurface::Revision);
        assert!(matches_query(&record, &after_ok.clauses));
        assert!(!matches_query(&record, &after_future.clauses));
    }

    #[test]
    fn stale_review_fact_count_sums_review_facts_only_when_superseded() {
        let summary = RevisionProjectionSummary {
            observation_count: 2,
            input_request_count: 1,
            assessment_count: 1,
            validation_check_count: 3,
            ..Default::default()
        };

        // Superseded ⇒ the four review families (2 + 1 + 1 + 3 = 7).
        let superseded: BTreeSet<RevisionId> = [RevisionId::new("rev:sha256:successor")]
            .into_iter()
            .collect();
        assert_eq!(stale_review_fact_count(&superseded, &summary), 7);

        // Head (empty superseders) ⇒ zero, regardless of fact counts.
        assert_eq!(stale_review_fact_count(&BTreeSet::new(), &summary), 0);
    }

    #[test]
    fn haystack_is_lowercased_and_folds_ids_and_fact_titles() {
        let mut overview =
            overview_with_observations(vec![observation("agent:codex", "actor:agent:codex", &[])]);
        overview.input_requests = vec![input_request(InputRequestStatus::Open)];
        let mut e = entry("rev:sha256:ONE", "snap:sha256:one", "2026-05-13T10:00:00Z");
        e.summary = Some("Readable capture label".to_owned());
        let record = record_of(&e, &overview, "head", false);
        for piece in [
            "rev:sha256:one",
            "snap:sha256:one",
            "readable capture label",
            "obs",
            "req",
            "attention",
        ] {
            assert!(record.text.contains(piece), "{piece} in {}", record.text);
        }
        assert_eq!(record.text, record.text.to_lowercase());
    }
}
