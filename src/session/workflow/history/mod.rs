mod cursor;
mod options;
mod projection;
mod query;
mod result;
mod search;
mod summary;

use std::collections::BTreeSet;
use std::path::Path;

pub use self::cursor::HistoryCursor;
pub use self::options::{ReviewHistoryFilters, ReviewHistoryOptions};
pub use self::projection::{BaseEntry, BaseHistoryProjection, BaseProjectionConfig};
use self::projection::{
    entry_body_states, history_base_from_events, history_base_from_events_without_bodies,
    history_default_page_from_events,
};
pub use self::query::{
    DistinctValues, HistoryOrder, HistoryPage, HistoryQuery, QueriedHistory, apply_history_query,
    count_new_since,
};
pub use self::result::ReviewHistoryResult;
pub use self::search::{
    EVENT_QUERY_FIELDS, EventRecordExtras, KNOWN_QUERY_KEYS, ParsedQuery, QueryClause,
    QueryDiagnostic, QueryDiagnosticCode, QuerySurface, RANGE_ANCHOR_FIELD,
    REVISION_ATTENTION_VALUES, REVISION_QUERY_FIELDS, SearchRecord, build_haystack, matches_query,
    parse_search_query, parse_search_query_for,
};
pub(crate) use self::search::{enum_wire, tag_index_tokens, wrap_set};
pub use self::summary::{ReviewHistoryEntry, ReviewHistorySummary};
use crate::error::{Result, ShoreError};
use crate::session::EventStore;
use crate::session::observation::validated_track_id;
use crate::session::projection::body_content::body_content_diagnostics;
use crate::session::projection::skipped_to_diagnostics;
use crate::session::state::ProjectionDiagnostic;
use crate::session::store::backend::StoreBackend;
use crate::session::store::resolution::resolve_read_store;

/// Read review events with the caller's strictness policy, build the shared
/// hydrated base, and adapt one pure `apply_history_query` result for the public
/// history surface.
pub fn review_history(options: ReviewHistoryOptions) -> Result<ReviewHistoryResult> {
    let read_store = resolve_read_store(&options.repo)?;
    options
        .track
        .as_deref()
        .map(validated_track_id)
        .transpose()?;
    let store = EventStore::from_backend(read_store.backend());
    let (events, skip_diagnostics) = if options.read_for_display {
        let (events, skipped) = store.list_events_lenient()?;
        (events, skipped_to_diagnostics(skipped))
    } else {
        (store.list_events()?, Vec::new())
    };

    apply_review_history_options(
        options,
        &events,
        Some(read_store.backend()),
        skip_diagnostics,
    )
}

/// Adapt one already-read event set through the shared query engine. Store-read
/// strictness stays in [`review_history`]; this core owns projection config,
/// pre-resolution, query mapping, visibility, and result diagnostics.
fn apply_review_history_options(
    options: ReviewHistoryOptions,
    events: &[crate::session::event::ShoreEvent],
    backend: Option<&StoreBackend>,
    skip_diagnostics: Vec<ProjectionDiagnostic>,
) -> Result<ReviewHistoryResult> {
    if matches!(options.order, HistoryOrder::Desc) && options.cursor.is_some() {
        return Err(ShoreError::Message(
            "descending history does not support continuation cursors".to_owned(),
        ));
    }

    let track_id = options
        .track
        .as_deref()
        .map(validated_track_id)
        .transpose()?;
    let ref_matched_units = match &options.ref_filter {
        Some((name, mode)) => {
            let projection = crate::session::RevisionCommitRangeProjection::from_events(events)?;
            Some(super::revision_list::revisions_matching_ref(
                &projection,
                name,
                *mode,
                &options.repo,
            )?)
        }
        None => None,
    };

    let can_recover_without_bodies = !options.include_body
        && options
            .filter
            .as_deref()
            .is_none_or(|filter| filter.trim().is_empty());
    let config = BaseProjectionConfig {
        verification_policy: options.verification_policy,
        trust_set: options.trust_set,
        removal_policy: options.removal_policy,
        actor_attributes: options.actor_attributes,
        delegation_map: options.delegation_map,
    };
    let base = match history_base_from_events(events, &config, backend) {
        Err(_) if can_recover_without_bodies => {
            history_base_from_events_without_bodies(events, &config, backend)?
        }
        result => result?,
    };

    let query = HistoryQuery {
        q: options.filter.unwrap_or_default(),
        track: track_id
            .as_ref()
            .map(|track_id| track_id.as_str().to_owned()),
        snapshot: None,
        revision: options.revision_id.clone(),
        revisions: ref_matched_units,
        types: (!options.event_types.is_empty()).then(|| {
            options
                .event_types
                .iter()
                .map(|event_type| event_type.as_str().to_owned())
                .collect::<BTreeSet<_>>()
        }),
        order: options.order,
    };
    let page = HistoryPage {
        limit: options.limit,
        after: options.cursor,
        offset: None,
        at: None,
    };
    let queried = apply_history_query(&base, &query, &page);
    if let Some(fatal) = queried.query_notices.iter().find(|diagnostic| {
        matches!(
            diagnostic.code,
            QueryDiagnosticCode::UnsupportedQualifier | QueryDiagnosticCode::UnsupportedValue
        )
    }) {
        return Err(ShoreError::Message(fatal.message.clone()));
    }

    let filters = ReviewHistoryFilters {
        revision_id: options.revision_id,
        track_id,
        event_types: options.event_types,
        include_body: options.include_body,
    };
    let mut entries = queried.entries;
    let mut diagnostics = queried.diagnostics;
    diagnostics.retain(|diagnostic| {
        !matches!(
            diagnostic.code.as_str(),
            "body_content_suppressed_present" | "body_content_physically_removed"
        )
    });
    diagnostics.extend(body_content_diagnostics(
        entries.iter().flat_map(entry_body_states),
    ));
    diagnostics.extend(skip_diagnostics);

    if !options.include_body {
        for entry in &mut entries {
            redact_hydrated_bodies(entry);
        }
    }

    Ok(ReviewHistoryResult {
        event_set_hash: queried.event_set_hash,
        event_count: queried.event_count,
        filters,
        entries,
        next_cursor: queried.next_cursor.map(|cursor| cursor.encode()),
        query_notices: queried.query_notices,
        diagnostics,
    })
}

fn redact_hydrated_bodies(entry: &mut ReviewHistoryEntry) {
    match &mut entry.summary {
        ReviewHistorySummary::ReviewObservationRecorded { body, .. }
        | ReviewHistorySummary::InputRequestOpened { body, .. } => *body = None,
        ReviewHistorySummary::ReviewAssessmentRecorded { summary, .. }
        | ReviewHistorySummary::ValidationCheckRecorded { summary, .. } => *summary = None,
        ReviewHistorySummary::InputRequestResponded { reason, .. } => *reason = None,
        _ => {}
    }
}

/// Read the store and build the full body-hydrated base projection the inspector
/// caches (#255) and queries in memory (task 3.1). `config` (advisory
/// verification + reader enrichment) is supplied by the caller — the binary
/// builds it from `discover_*` (the library cannot reach those `pub(crate)`
/// helpers — INV-8). Reads leniently for a display surface and folds the
/// skip diagnostics in, like `review_history`.
pub fn history_base_projection(
    repo: impl AsRef<Path>,
    config: &BaseProjectionConfig,
) -> Result<BaseHistoryProjection> {
    let span = tracing::debug_span!("shore.history.base_projection");
    let _guard = span.enter();

    let read_store = {
        let span = tracing::debug_span!("shore.history.resolve_read_store");
        let _guard = span.enter();
        resolve_read_store(repo.as_ref())?
    };
    let store = EventStore::from_backend(read_store.backend());
    let (events, skipped) = {
        let span = tracing::debug_span!("shore.history.list_events_lenient");
        let _guard = span.enter();
        store.list_events_lenient()?
    };
    let mut base = history_base_from_events(&events, config, Some(read_store.backend()))?;
    let diagnostics = {
        let span = tracing::debug_span!("shore.history.skipped_to_diagnostics");
        let _guard = span.enter();
        skipped_to_diagnostics(skipped)
    };
    base.diagnostics.extend(diagnostics);
    Ok(base)
}

/// Fast inspector path for the default first history page: read the event log and
/// hydrate only the requested page, avoiding the full body-hydrated search base
/// while the background cache warm is still running.
pub fn default_history_page_projection(
    repo: impl AsRef<Path>,
    config: &BaseProjectionConfig,
    limit: usize,
    order: HistoryOrder,
) -> Result<QueriedHistory> {
    let span = tracing::debug_span!("shore.history.default_page_projection");
    let _guard = span.enter();

    let read_store = {
        let span = tracing::debug_span!("shore.history.default_page.resolve_read_store");
        let _guard = span.enter();
        resolve_read_store(repo.as_ref())?
    };
    let store = EventStore::from_backend(read_store.backend());
    let (events, skipped) = {
        let span = tracing::debug_span!("shore.history.default_page.list_events_lenient");
        let _guard = span.enter();
        store.list_events_lenient()?
    };
    let mut page = history_default_page_from_events(
        &events,
        config,
        Some(read_store.backend()),
        limit,
        order,
    )?;
    let diagnostics = {
        let span = tracing::debug_span!("shore.history.default_page.skipped_to_diagnostics");
        let _guard = span.enter();
        skipped_to_diagnostics(skipped)
    };
    page.diagnostics.extend(diagnostics);
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::projection::{
        HistoryProjectionOptions, history_base_from_events, history_entry_from_event,
    };
    use super::summary::ReviewHistorySummary;
    use super::*;
    use crate::model::{
        ActorId, AssessmentId, EngagementId, EventId, InputRequestId, InputRequestResponseId,
        JournalId, ObjectId, ObservationId, ReviewEndpoint, ReviewTargetRef, RevisionId,
        RevisionSource, TargetRef, TrackId, ValidationCheckId, ValidationStatus, ValidationTarget,
        ValidationTrigger, WorktreeCaptureMode,
    };
    use crate::session::event::{
        AssertionMode, EventTarget, EventType, GitProvenance, InputRequestOpenedPayload,
        InputRequestReasonCode, InputRequestRespondedPayload, InputRequestResponseOutcome,
        ReviewAssessment, ReviewAssessmentRecordedPayload, ReviewInitializedPayload,
        ReviewObservationRecordedPayload, Revision, ShoreEvent, ValidationCheckRecordedPayload,
        WorkObjectProposal, WorkObjectProposedPayload, Writer,
    };
    use crate::session::state::DUPLICATE_SEMANTIC_OBSERVATION_EVENT_CODE;
    use crate::session::store::backend::StoreBackend;
    use crate::session::{
        ActorAttributesMap, DelegationMap, EventVerificationPolicy, RemovalPolicy, TrustSet,
    };

    #[derive(Clone, Debug, Default)]
    struct TestHistoryOptions {
        revision_id: Option<RevisionId>,
        track_id: Option<TrackId>,
        event_types: Vec<EventType>,
        include_body: bool,
        verification_policy: Option<EventVerificationPolicy>,
        trust_set: TrustSet,
        removal_policy: RemovalPolicy,
        actor_attributes: Option<ActorAttributesMap>,
        delegation_map: Option<DelegationMap>,
    }

    #[derive(Clone, Debug, Default)]
    struct TestHistoryPage {
        limit: Option<usize>,
        after: Option<HistoryCursor>,
    }

    /// Compatibility-shaped test input routed through the real converged
    /// adapter. This keeps the long-standing projection cases focused on their
    /// original assertions without preserving a second production windower.
    fn adapt_test_history(
        events: &[ShoreEvent],
        filters: TestHistoryOptions,
        window: TestHistoryPage,
        backend: Option<&StoreBackend>,
    ) -> Result<ReviewHistoryResult> {
        let mut options = ReviewHistoryOptions::new("/repo")
            .with_include_body(filters.include_body)
            .with_trust_set(filters.trust_set)
            .with_removal_policy(filters.removal_policy)
            .with_actor_attributes(filters.actor_attributes);
        if let Some(revision_id) = filters.revision_id {
            options = options.with_revision_id(revision_id);
        }
        if let Some(track_id) = filters.track_id {
            options = options.with_track(track_id.as_str());
        }
        for event_type in filters.event_types {
            options = options.with_event_type(event_type);
        }
        if let Some(policy) = filters.verification_policy {
            options = options.with_verification_policy(policy);
        }
        if let Some(delegation_map) = filters.delegation_map {
            options = options.with_delegation_map(delegation_map);
        }
        if let Some(limit) = window.limit {
            options = options.with_limit(limit);
        }
        if let Some(cursor) = window.after {
            options = options.with_cursor(cursor);
        }
        apply_review_history_options(options, events, backend, Vec::new())
    }

    /// A content-addressed note path + normalized hash + a Local backend, with
    /// the blob optionally written (the removed-body fixtures).
    fn note_blob_fixture(
        write_blob: bool,
    ) -> (
        tempfile::TempDir,
        crate::session::store::backend::StoreBackend,
        String,
        String,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let backend = crate::session::store::backend::StoreBackend::Local(dir.path().to_path_buf());
        let stem = "b".repeat(64);
        let path = format!("artifacts/notes/{stem}.json");
        let hash = format!("sha256:{stem}");
        if write_blob {
            std::fs::create_dir_all(dir.path().join("artifacts/notes")).unwrap();
            std::fs::write(
                dir.path().join(&path),
                r#"{"schema":"shore.note-body","version":1,"body":"stored body"}"#,
            )
            .unwrap();
        }
        (dir, backend, path, hash)
    }

    /// An observation whose externalized body is payload-consistent: the
    /// writers guarantee `body_content_hash` equals the locator hash, and the
    /// diagnostics join on exactly that equality.
    fn observation_event_with_artifact(path: &str, hash: &str) -> ShoreEvent {
        let payload = ReviewObservationRecordedPayload {
            observation_id: ObservationId::new("obs:sha256:artifact"),
            target: ReviewTargetRef::Revision {
                revision_id: revision_id("one"),
            },
            title: "Observation".to_owned(),
            body: None,
            body_content_type: Default::default(),
            body_artifact_path: Some(path.to_owned()),
            body_byte_size: Some(5000),
            body_content_hash: Some(hash.to_owned()),
            tags: vec![],
            confidence: None,
            supersedes_observation_ids: vec![],
            responds_to_observation_ids: vec![],
        };
        tracked_event(
            EventType::ReviewObservationRecorded,
            "observation:artifact",
            "agent:codex",
            payload,
            "2026-05-13T10:00:01Z",
        )
    }

    fn artifact_removed_event(content_hash: &str) -> ShoreEvent {
        use crate::session::event::ArtifactRemovedPayload;
        ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_journal(JournalId::new("journal:fixture")),
            Writer::shore_local("test"),
            ArtifactRemovedPayload {
                content_hash: content_hash.to_owned(),
            },
            "2026-05-13T09:00:00Z",
        )
        .unwrap()
    }

    #[test]
    fn history_hydrates_removed_body_as_removed_state_instead_of_erroring() {
        let (_dir, backend, path, hash) = note_blob_fixture(false);
        let events = vec![
            observation_event_with_artifact(&path, &hash),
            artifact_removed_event(&hash),
        ];

        let result = adapt_test_history(
            &events,
            TestHistoryOptions {
                include_body: true,
                ..TestHistoryOptions::default()
            },
            TestHistoryPage::default(),
            Some(&backend),
        )
        .expect("swept observation body must not hard-error the history read");

        let entry = result
            .entries
            .iter()
            .find(|entry| {
                matches!(
                    entry.summary,
                    ReviewHistorySummary::ReviewObservationRecorded { .. }
                )
            })
            .expect("observation entry");
        match &entry.summary {
            ReviewHistorySummary::ReviewObservationRecorded {
                body,
                body_content_state,
                ..
            } => {
                assert_eq!(*body, None);
                assert_eq!(
                    *body_content_state,
                    crate::session::BodyContentState::PhysicallyRemoved
                );
            }
            _ => unreachable!(),
        }
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "body_content_physically_removed" && d.message.contains(&hash))
        );
    }

    #[test]
    fn history_reports_removed_response_reason_state() {
        let (_dir, backend, path, hash) = note_blob_fixture(false);
        let payload = InputRequestRespondedPayload {
            input_request_response_id: InputRequestResponseId::new("resp:sha256:one"),
            input_request_id: InputRequestId::new("input:sha256:one"),
            revision_id: Some(revision_id("one")),
            task_target: None,
            outcome: InputRequestResponseOutcome::Approved,
            reason: None,
            reason_content_type: Default::default(),
            reason_artifact_path: Some(path.clone()),
            reason_byte_size: Some(5000),
            reason_content_hash: Some(hash.clone()),
            target_fingerprint: None,
        };
        let responded = tracked_event(
            EventType::InputRequestResponded,
            "response:removed-reason",
            "agent:codex",
            payload,
            "2026-05-13T10:00:02Z",
        );
        let events = vec![responded, artifact_removed_event(&hash)];

        let result = adapt_test_history(
            &events,
            TestHistoryOptions {
                include_body: true,
                ..TestHistoryOptions::default()
            },
            TestHistoryPage::default(),
            Some(&backend),
        )
        .expect("swept response reason must not hard-error the history read");

        match &result.entries[0].summary {
            ReviewHistorySummary::InputRequestResponded {
                reason,
                reason_content_state,
                ..
            } => {
                assert_eq!(*reason, None);
                assert_eq!(
                    *reason_content_state,
                    crate::session::BodyContentState::PhysicallyRemoved
                );
            }
            other => panic!("expected a responded entry, got {other:?}"),
        }
    }

    #[test]
    fn history_base_projection_survives_a_swept_body() {
        let (_dir, backend, path, hash) = note_blob_fixture(false);
        let events = vec![
            observation_event_with_artifact(&path, &hash),
            artifact_removed_event(&hash),
        ];

        let base =
            history_base_from_events(&events, &BaseProjectionConfig::default(), Some(&backend))
                .expect("the always-hydrating base cache must survive a swept body");

        assert!(!base.entries.is_empty());
    }

    /// Body diagnostics are rendered-entry scoped: hydration and state
    /// resolution only happen for entries that survive the window, so a
    /// removed body sliced out by `limit` yields no diagnostic on that page.
    #[test]
    fn windowed_out_removed_body_yields_no_diagnostic() {
        let (_dir, backend, path, hash) = note_blob_fixture(false);
        let events = vec![
            review_initialized_event("0"),
            observation_event_with_artifact(&path, &hash),
            artifact_removed_event(&hash),
        ];

        let result = adapt_test_history(
            &events,
            TestHistoryOptions {
                include_body: true,
                ..TestHistoryOptions::default()
            },
            TestHistoryPage {
                limit: Some(1),
                after: None,
            },
            Some(&backend),
        )
        .expect("windowed read");

        assert_eq!(result.entries.len(), 1);
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| d.code.starts_with("body_content_"))
        );
    }

    #[test]
    fn history_summary_serializes_body_content_state_snake_case_and_skips_present() {
        let removed = ReviewHistorySummary::ReviewObservationRecorded {
            observation_id: ObservationId::new("obs:sha256:one"),
            target: ReviewTargetRef::Revision {
                revision_id: revision_id("one"),
            },
            title: "t".to_owned(),
            body: None,
            body_content_type: Default::default(),
            body_byte_size: None,
            body_content_hash: Some("sha256:x".to_owned()),
            body_content_state: crate::session::BodyContentState::PhysicallyRemoved,
            tags: vec![],
            confidence: None,
            supersedes: vec![],
            responds_to: vec![],
        };
        let json = serde_json::to_value(&removed).unwrap();
        assert_eq!(json["bodyContentState"], "physically_removed");

        let present = ReviewHistorySummary::ReviewObservationRecorded {
            observation_id: ObservationId::new("obs:sha256:one"),
            target: ReviewTargetRef::Revision {
                revision_id: revision_id("one"),
            },
            title: "t".to_owned(),
            body: Some("b".to_owned()),
            body_content_type: Default::default(),
            body_byte_size: None,
            body_content_hash: None,
            body_content_state: Default::default(),
            tags: vec![],
            confidence: None,
            supersedes: vec![],
            responds_to: vec![],
        };
        let json = serde_json::to_value(&present).unwrap();
        assert!(json.get("bodyContentState").is_none());
    }

    #[test]
    fn review_history_returns_empty_freshness_metadata_without_events() {
        let result = adapt_test_history(
            &[],
            TestHistoryOptions::default(),
            TestHistoryPage::default(),
            None,
        )
        .unwrap();

        assert_eq!(result.event_count, 0);
        assert_eq!(result.history_count(), 0);
        assert!(result.event_set_hash.starts_with("sha256:"));
        assert!(result.entries.is_empty());
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn review_history_metadata_uses_full_event_set() {
        let first = review_initialized_event("one");
        let second = review_initialized_event("two");

        let result = adapt_test_history(
            &[first, second],
            TestHistoryOptions {
                event_types: vec![EventType::ReviewInitialized],
                ..TestHistoryOptions::default()
            },
            TestHistoryPage::default(),
            None,
        )
        .unwrap();

        assert_eq!(result.event_count, 2);
        assert_eq!(result.history_count(), 2);
        assert!(result.event_set_hash.starts_with("sha256:"));
    }

    #[test]
    fn review_history_strict_by_default_lenient_when_opted_in() {
        let repo = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo.path())
            .output()
            .unwrap();

        // A retired-type event in the resolved store: the probe rejects it.
        let events_dir = resolve_read_store(repo.path())
            .unwrap()
            .store_dir()
            .join("events");
        std::fs::create_dir_all(&events_dir).unwrap();
        std::fs::write(
            events_dir.join(format!("{}.json", "a".repeat(64))),
            br#"{"eventType":"review_disposition_recorded"}"#,
        )
        .unwrap();

        // Default (the relay/strict case): a retired event hard-fails the read.
        assert!(review_history(ReviewHistoryOptions::new(repo.path())).is_err());

        // Opted in (CLI/inspector): the retired event is skipped and surfaced.
        let result =
            review_history(ReviewHistoryOptions::new(repo.path()).with_read_for_display(true))
                .unwrap();
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "unsupported_event_type")
        );
    }

    #[test]
    fn no_body_history_tolerates_a_missing_external_body() {
        let (_dir, backend, path, hash) = note_blob_fixture(false);
        let event = observation_event_with_artifact(&path, &hash);

        let result = apply_review_history_options(
            ReviewHistoryOptions::new("/repo"),
            std::slice::from_ref(&event),
            Some(&backend),
            Vec::new(),
        )
        .unwrap();

        assert_eq!(result.entries.len(), 1);
        let ReviewHistorySummary::ReviewObservationRecorded {
            body,
            body_content_hash,
            ..
        } = &result.entries[0].summary
        else {
            panic!("expected an observation summary");
        };
        assert!(body.is_none());
        assert_eq!(body_content_hash.as_deref(), Some(hash.as_str()));

        assert!(
            apply_review_history_options(
                ReviewHistoryOptions::new("/repo").with_include_body(true),
                &[event],
                Some(&backend),
                Vec::new(),
            )
            .is_err(),
            "requesting body content keeps missing-artifact errors visible"
        );
    }

    #[test]
    fn desc_selection_takes_the_newest_at_offset_zero() {
        let events = windowing_events(5);
        let result = apply_review_history_options(
            ReviewHistoryOptions::new("/repo")
                .with_order(HistoryOrder::Desc)
                .with_limit(2),
            &events,
            None,
            Vec::new(),
        )
        .unwrap();

        assert_eq!(result.entries.len(), 2);
        assert!(result.entries[0].occurred_at > result.entries[1].occurred_at);
        assert_eq!(result.entries[0].event_id, events[4].event_id);
        assert_eq!(result.entries[1].event_id, events[3].event_id);
        assert!(result.next_cursor.is_none());
    }

    #[test]
    fn desc_with_a_continuation_cursor_is_rejected() {
        let cursor = HistoryCursor {
            occurred_at: "2026-05-13T10:00:01Z".to_owned(),
            event_id: EventId::new("evt:sha256:cursor"),
        };

        let error = apply_review_history_options(
            ReviewHistoryOptions::new("/repo")
                .with_order(HistoryOrder::Desc)
                .with_cursor(cursor),
            &windowing_events(2),
            None,
            Vec::new(),
        )
        .unwrap_err();

        assert!(error.to_string().contains("descending history"));
    }

    #[test]
    fn filter_option_runs_the_grammar_with_fatal_diagnostics_as_errors() {
        let events = vec![
            observation_event("rev:sha256:one", "agent:codex", "observation"),
            assessment_event(),
            validation_check_recorded_event(),
        ];

        let filtered = apply_review_history_options(
            ReviewHistoryOptions::new("/repo").with_filter("type:observation"),
            &events,
            None,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(filtered.entries.len(), 1);
        assert!(matches!(
            filtered.entries[0].summary,
            ReviewHistorySummary::ReviewObservationRecorded { .. }
        ));

        let fatal = apply_review_history_options(
            ReviewHistoryOptions::new("/repo").with_filter("attention:open"),
            &events,
            None,
            Vec::new(),
        )
        .unwrap_err();
        assert!(fatal.to_string().contains("not a filter"));

        let deprecated = apply_review_history_options(
            ReviewHistoryOptions::new("/repo").with_filter("status:passed"),
            &events,
            None,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(deprecated.entries.len(), 1);
        assert_eq!(deprecated.query_notices.len(), 1);
        assert_eq!(
            deprecated.query_notices[0].code,
            QueryDiagnosticCode::DeprecatedQualifier
        );
    }

    #[test]
    fn include_body_false_strips_every_hydrated_body_variant() {
        let cases = [
            (observation_event_with_body("observation body"), "body"),
            (assessment_event(), "summary"),
            (input_request_opened_event(), "body"),
            (input_request_responded_event(), "reason"),
            (validation_check_recorded_event(), "summary"),
        ];
        for (event, text_field) in cases {
            let hydrated = history_entry_from_event(
                &event,
                &HistoryProjectionOptions {
                    include_body: true,
                    ..HistoryProjectionOptions::default()
                },
                None,
                None,
                None,
            )
            .unwrap();
            let before = serde_json::to_value(&hydrated.summary).unwrap();
            assert!(before["kind"].is_string());
            let body_term = before[text_field].as_str().unwrap();

            let result = apply_review_history_options(
                ReviewHistoryOptions::new("/repo").with_filter(body_term),
                &[event],
                None,
                Vec::new(),
            )
            .unwrap();
            assert_eq!(
                result.entries.len(),
                1,
                "body search must match {text_field}"
            );

            let mut expected = before.as_object().unwrap().clone();
            expected.remove(text_field);
            let after = serde_json::to_value(&result.entries[0].summary).unwrap();
            assert_eq!(after, serde_json::Value::Object(expected));
        }
    }

    #[test]
    fn removal_diagnostics_stay_page_scoped_in_both_directions() {
        let (_dir, backend, path, hash) = note_blob_fixture(false);
        let events = vec![
            review_initialized_event("0"),
            observation_event_with_artifact(&path, &hash),
            artifact_removed_event(&hash),
        ];

        let outside = apply_review_history_options(
            ReviewHistoryOptions::new("/repo")
                .with_include_body(true)
                .with_limit(1),
            &events,
            Some(&backend),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(outside.entries.len(), 1);
        assert!(
            !outside
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.starts_with("body_content_"))
        );

        let inside = apply_review_history_options(
            ReviewHistoryOptions::new("/repo")
                .with_include_body(true)
                .with_limit(2),
            &events,
            Some(&backend),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(inside.entries.len(), 2);
        assert!(inside.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "body_content_physically_removed"
                && diagnostic.message.contains(&hash)
        }));
    }

    #[test]
    fn history_entry_summarizes_current_event_families() {
        let cases = [
            (
                review_initialized_event("init"),
                EventType::ReviewInitialized,
                "review_initialized",
            ),
            (
                revision_captured_event(),
                EventType::WorkObjectProposed,
                "revision_captured",
            ),
            (
                observation_event_with_body("body"),
                EventType::ReviewObservationRecorded,
                "review_observation_recorded",
            ),
            (
                assessment_event(),
                EventType::ReviewAssessmentRecorded,
                "review_assessment_recorded",
            ),
            (
                input_request_opened_event(),
                EventType::InputRequestOpened,
                "input_request_opened",
            ),
            (
                input_request_responded_event(),
                EventType::InputRequestResponded,
                "input_request_responded",
            ),
            (
                review_note_imported_event(),
                EventType::ReviewNoteImported,
                "review_note_imported",
            ),
            (
                validation_check_recorded_event(),
                EventType::ValidationCheckRecorded,
                "validation_check_recorded",
            ),
        ];

        for (event, event_type, summary_kind) in cases {
            let entry = history_entry_from_event(
                &event,
                &HistoryProjectionOptions::default(),
                None,
                None,
                None,
            )
            .unwrap();
            let summary_json = serde_json::to_value(&entry.summary).unwrap();

            assert_eq!(entry.event_type, event_type);
            assert_eq!(summary_json["kind"], summary_kind);
        }
    }

    #[test]
    fn history_entry_preserves_assessment_target_for_assessment_event() {
        let target = ReviewTargetRef::Assessment {
            revision_id: revision_id("one"),
            assessment_id: AssessmentId::new("assess:sha256:prior"),
        };
        let event = assessment_event_with_target(target.clone());

        let entry = history_entry_from_event(
            &event,
            &HistoryProjectionOptions::default(),
            None,
            None,
            None,
        )
        .unwrap();
        let json = serde_json::to_value(&entry).unwrap();

        match entry.summary {
            ReviewHistorySummary::ReviewAssessmentRecorded {
                target: summary_target,
                ..
            } => assert_eq!(summary_target, target),
            other => panic!("expected assessment summary, got {other:?}"),
        }
        assert_eq!(
            json["summary"]["relatedInputRequests"],
            serde_json::json!(["input-request:sha256:one"])
        );
        assert!(json["summary"].get("relatedInterventions").is_none());
    }

    #[test]
    fn history_includes_validation_check_recorded_summary() {
        let event = validation_check_recorded_event();

        let entry = history_entry_from_event(
            &event,
            &HistoryProjectionOptions::default(),
            None,
            None,
            None,
        )
        .unwrap();
        let json = serde_json::to_value(&entry.summary).unwrap();

        assert_eq!(json["kind"], "validation_check_recorded");
        assert_eq!(json["validationCheckId"], "validation:sha256:one");
        assert_eq!(json["checkName"], "cargo test");
        assert_eq!(json["status"], "passed");
    }

    #[test]
    fn history_entry_omits_internal_artifact_paths() {
        let event = observation_event_with_artifact_path("artifacts/notes/body.json");

        let entry = history_entry_from_event(
            &event,
            &HistoryProjectionOptions::default(),
            None,
            None,
            None,
        )
        .unwrap();
        let json = serde_json::to_string(&entry).unwrap();

        assert!(!json.contains("bodyArtifactPath"));
        assert!(!json.contains("artifacts/notes"));
    }

    #[test]
    fn history_sorts_mixed_timestamps_by_instant_then_event_id() {
        let late = event_with_time_and_key("2026-05-13T10:00:02Z", "late");
        let tie_b = event_with_time_and_key("2026-05-13T10:00:01Z", "b");
        let tie_a = event_with_time_and_key("2026-05-13T10:00:01Z", "a");
        let earliest = event_with_time_and_key("unix-ms:0", "earliest");

        let result = adapt_test_history(
            &[late, tie_b, tie_a, earliest],
            TestHistoryOptions::default(),
            TestHistoryPage::default(),
            None,
        )
        .unwrap();

        assert_eq!(
            result
                .entries
                .iter()
                .map(|entry| entry.occurred_at.as_str())
                .collect::<Vec<_>>(),
            vec![
                "unix-ms:0",
                "2026-05-13T10:00:01Z",
                "2026-05-13T10:00:01Z",
                "2026-05-13T10:00:02Z",
            ]
        );
        assert!(result.entries[0].event_id.as_str() < result.entries[1].event_id.as_str());
    }

    #[test]
    fn review_history_entry_subject_round_trips_review_target_ref_after_envelope_widening() {
        let event = observation_event("review-unit:sha256:one", "agent:codex", "Pinned");

        let entry = history_entry_from_event(
            &event,
            &HistoryProjectionOptions::default(),
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            entry.subject,
            Some(ReviewTargetRef::Revision {
                revision_id: RevisionId::new("review-unit:sha256:one"),
            })
        );
    }

    #[test]
    fn review_history_entry_subject_narrows_task_target_ref_to_none() {
        // The signed envelope no longer carries an assignable subject field;
        // the subject is reconstructed from the event's payload, and a
        // `ReviewInitialized` event always reconstructs to the subject-less
        // `TargetRef::Journal` carrier, which narrows the same as a task
        // subject would.
        let event = review_initialized_event("narrow");

        let entry = history_entry_from_event(
            &event,
            &HistoryProjectionOptions::default(),
            None,
            None,
            None,
        )
        .unwrap();

        assert!(entry.subject.is_none());
    }

    #[test]
    fn review_history_filters_out_task_event_types_unconditionally() {
        let init = review_initialized_event("init");
        let task_event = ShoreEvent {
            schema: "shore.event".to_owned(),
            version: 1,
            event_id: EventId::new("evt:sha256:task-checkpoint"),
            event_type: EventType::TaskCheckpointCaptured,
            idempotency_key: "task_checkpoint_captured:filter".to_owned(),
            target: EventTarget::for_journal(JournalId::new("journal:claude:abc")),
            writer: Writer::shore_local("test"),
            occurred_at: "2026-05-18T10:00:01Z".to_owned(),
            payload_hash: "sha256:placeholder".to_owned(),
            assertion_mode: AssertionMode::Advisory,
            signer: None,
            signature: None,
            source_ref: None,
            ingest: None,
            content_encoding: Vec::new(),
            payload_version: 1,
            payload: serde_json::Value::Null,
        };

        let result = adapt_test_history(
            &[init, task_event],
            TestHistoryOptions::default(),
            TestHistoryPage::default(),
            None,
        )
        .unwrap();

        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].event_type, EventType::ReviewInitialized);
    }

    #[test]
    fn review_history_filter_excludes_all_task_event_types() {
        let init = review_initialized_event("init");
        let task_events = [
            EventType::TaskCheckpointCaptured,
            EventType::TaskObservationRecorded,
        ]
        .into_iter()
        .enumerate()
        .map(|(idx, event_type)| ShoreEvent {
            schema: "shore.event".to_owned(),
            version: 1,
            event_id: EventId::new(format!("evt:sha256:task-{idx}")),
            event_type,
            idempotency_key: format!("task_kind_{idx}"),
            target: EventTarget::for_journal(JournalId::new("journal:claude:abc")),
            writer: Writer::shore_local("test"),
            occurred_at: format!("2026-05-18T10:00:0{}Z", idx + 1),
            payload_hash: "sha256:placeholder".to_owned(),
            assertion_mode: AssertionMode::Advisory,
            signer: None,
            signature: None,
            source_ref: None,
            ingest: None,
            content_encoding: Vec::new(),
            payload_version: 1,
            payload: serde_json::Value::Null,
        });

        let mut events: Vec<ShoreEvent> = vec![init];
        events.extend(task_events);

        let result = adapt_test_history(
            &events,
            TestHistoryOptions::default(),
            TestHistoryPage::default(),
            None,
        )
        .unwrap();

        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].event_type, EventType::ReviewInitialized);
    }

    #[test]
    fn review_history_filter_excludes_task_attempt_proposals() {
        // A generative move can propose a task attempt under the shared
        // WorkObjectProposed type; its Task subject keeps it out of the
        // review-domain history rather than reaching the revision summary arm.
        let init = review_initialized_event("init");
        let task_payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new("engagement:sha256:task"),
            work_object: WorkObjectProposal::TaskAttempt {
                task_attempt_id: crate::model::WorkObjectId::new("task-attempt:sha256:t"),
                project_path: "/repo".to_owned(),
                claude_session_uuid: "uuid".to_owned(),
                initial_prompt_hash: "sha256:prompt".to_owned(),
                predecessor: None,
                base_state_fingerprint: None,
                source_speaker: None,
            },
        };
        let task_attempt = ShoreEvent::new(
            EventType::WorkObjectProposed,
            "task_attempt_proposal",
            EventTarget::for_subject(
                JournalId::new("journal:claude:abc"),
                TargetRef::Task(crate::model::TaskTargetRef::TaskAttempt {
                    task_attempt_id: crate::model::WorkObjectId::new("task-attempt:sha256:t"),
                }),
                None,
            )
            .unwrap(),
            Writer::shore_local("test"),
            task_payload,
            "2026-05-18T10:00:05Z",
        )
        .unwrap();

        let result = adapt_test_history(
            &[init, task_attempt],
            TestHistoryOptions::default(),
            TestHistoryPage::default(),
            None,
        )
        .unwrap();

        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].event_type, EventType::ReviewInitialized);
    }

    #[test]
    fn review_history_projection_rejects_task_event_with_explicit_error() {
        let event = ShoreEvent {
            schema: "shore.event".to_owned(),
            version: 1,
            event_id: EventId::new("evt:sha256:checkpoint-1"),
            event_type: EventType::TaskCheckpointCaptured,
            idempotency_key: "task_checkpoint_captured:cp-1".to_owned(),
            target: EventTarget::for_journal(JournalId::new("journal:claude:abc")),
            writer: Writer::shore_local("test"),
            occurred_at: "2026-05-18T00:00:00Z".to_owned(),
            payload_hash: "sha256:placeholder".to_owned(),
            assertion_mode: AssertionMode::Advisory,
            signer: None,
            signature: None,
            source_ref: None,
            ingest: None,
            content_encoding: Vec::new(),
            payload_version: 1,
            payload: serde_json::Value::Null,
        };

        let error = history_entry_from_event(
            &event,
            &HistoryProjectionOptions::default(),
            None,
            None,
            None,
        )
        .expect_err("task events must not project to review-history");
        let message = error.to_string();

        assert!(
            message.contains("review-domain") || message.contains("task event"),
            "error message must document the review-domain contract; got: {message}"
        );
    }

    #[test]
    fn history_filters_by_revision_track_and_event_type() {
        let keep = observation_event("review-unit:sha256:one", "agent:codex", "Keep");
        let other_track = observation_event("review-unit:sha256:one", "agent:claude", "Drop track");
        let other_unit = observation_event("review-unit:sha256:two", "agent:codex", "Drop unit");
        let capture = revision_captured_event_for("review-unit:sha256:one");

        let filters = TestHistoryOptions {
            revision_id: Some(RevisionId::new("review-unit:sha256:one")),
            track_id: Some(TrackId::new("agent:codex")),
            event_types: vec![EventType::ReviewObservationRecorded],
            include_body: false,
            ..TestHistoryOptions::default()
        };

        let result = adapt_test_history(
            &[keep, other_track, other_unit, capture],
            filters,
            TestHistoryPage::default(),
            None,
        )
        .unwrap();

        assert_eq!(result.entries.len(), 1);
        assert_eq!(
            result.entries[0].track_id.as_ref().map(TrackId::as_str),
            Some("agent:codex")
        );
        assert_eq!(
            result.entries[0].event_type,
            EventType::ReviewObservationRecorded
        );
    }

    fn claude_code_delegates(
        valid_from: &str,
        valid_until: serde_json::Value,
    ) -> crate::session::DelegationMap {
        crate::session::delegation_map_from_value(serde_json::json!({
            "delegates": {
                "actor:agent:claude-code": [{
                    "principal": "actor:git-email:kevin@swiber.dev",
                    "validFrom": valid_from,
                    "validUntil": valid_until
                }]
            }
        }))
        .unwrap()
    }

    fn agent_written_observation() -> ShoreEvent {
        let mut event =
            observation_event("review-unit:sha256:one", "agent:claude-code", "Agent obs");
        event.writer.actor_id = ActorId::new("actor:agent:claude-code");
        event
    }

    #[test]
    fn history_entries_carry_resolved_principal_for_agent_writers_when_map_supplied() {
        let agent_event = agent_written_observation();
        // A human (git-email) writer for contrast.
        let mut human_event =
            observation_event("review-unit:sha256:one", "agent:codex", "Human obs");
        human_event.writer.actor_id = ActorId::new("actor:git-email:kevin@swiber.dev");

        let filters = TestHistoryOptions {
            delegation_map: Some(claude_code_delegates(
                "2026-05-01T00:00:00Z",
                serde_json::Value::Null,
            )),
            ..TestHistoryOptions::default()
        };
        let result = adapt_test_history(
            &[agent_event, human_event],
            filters,
            TestHistoryPage::default(),
            None,
        )
        .unwrap();

        let agent_entry = result
            .entries
            .iter()
            .find(|entry| entry.writer.actor_id.as_str() == "actor:agent:claude-code")
            .unwrap();
        let agent_json = serde_json::to_value(agent_entry).unwrap();
        assert_eq!(
            agent_json["principal"]["actorId"],
            "actor:git-email:kevin@swiber.dev"
        );
        assert_eq!(agent_json["principal"]["status"], "resolved");
        assert_eq!(agent_json["principal"]["source"], "delegates");

        let human_entry = result
            .entries
            .iter()
            .find(|entry| entry.writer.actor_id.as_str() == "actor:git-email:kevin@swiber.dev")
            .unwrap();
        assert!(
            serde_json::to_value(human_entry)
                .unwrap()
                .get("principal")
                .is_none(),
            "human (git-email) writers are their own principal — no principal object"
        );
    }

    #[test]
    fn history_without_map_emits_none_principal_for_agent_writers() {
        let result = adapt_test_history(
            &[agent_written_observation()],
            TestHistoryOptions::default(),
            TestHistoryPage::default(),
            None,
        )
        .unwrap();
        let json = serde_json::to_value(&result.entries[0]).unwrap();
        assert_eq!(
            json["principal"],
            serde_json::json!({ "status": "none", "source": "none" })
        );
    }

    #[test]
    fn history_principal_is_occurred_at_scoped() {
        // observation_event's occurredAt is 2026-05-13T10:00:01Z.
        let closed = TestHistoryOptions {
            delegation_map: Some(claude_code_delegates(
                "2026-01-01T00:00:00Z",
                serde_json::json!("2026-02-01T00:00:00Z"),
            )),
            ..TestHistoryOptions::default()
        };
        let closed_result = adapt_test_history(
            &[agent_written_observation()],
            closed,
            TestHistoryPage::default(),
            None,
        )
        .unwrap();
        let closed_json = serde_json::to_value(&closed_result.entries[0]).unwrap();
        assert_eq!(
            closed_json["principal"]["status"], "none",
            "a window that closed before the event resolves none"
        );

        let covering = TestHistoryOptions {
            delegation_map: Some(claude_code_delegates(
                "2026-05-01T00:00:00Z",
                serde_json::Value::Null,
            )),
            ..TestHistoryOptions::default()
        };
        let covering_result = adapt_test_history(
            &[agent_written_observation()],
            covering,
            TestHistoryPage::default(),
            None,
        )
        .unwrap();
        let covering_json = serde_json::to_value(&covering_result.entries[0]).unwrap();
        assert_eq!(covering_json["principal"]["status"], "resolved");
    }

    #[test]
    fn history_omits_body_text_by_default() {
        let event = observation_event_with_body("inline body");

        let result = adapt_test_history(
            &[event],
            TestHistoryOptions::default(),
            TestHistoryPage::default(),
            None,
        )
        .unwrap();
        let json = serde_json::to_value(&result.entries[0]).unwrap();

        assert!(json["summary"].get("body").is_none());
        assert!(json.to_string().contains("bodyContentHash"));
    }

    #[test]
    fn history_include_body_hydrates_inline_body_like_fields() {
        let filters = TestHistoryOptions {
            include_body: true,
            ..TestHistoryOptions::default()
        };
        let events = [
            observation_event_with_body("observation body"),
            assessment_event(),
            input_request_opened_event(),
            input_request_responded_event(),
            review_note_imported_event(),
        ];

        let result =
            adapt_test_history(&events, filters, TestHistoryPage::default(), None).unwrap();
        let entries = result
            .entries
            .iter()
            .map(|entry| serde_json::to_value(entry).unwrap())
            .collect::<Vec<_>>();

        assert!(entries.iter().any(|entry| {
            entry["summary"]["kind"] == "review_observation_recorded"
                && entry["summary"]["body"] == "observation body"
        }));
        assert!(entries.iter().any(|entry| {
            entry["summary"]["kind"] == "review_assessment_recorded"
                && entry["summary"]["summary"] == "ship it"
        }));
        assert!(entries.iter().any(|entry| {
            entry["summary"]["kind"] == "input_request_opened" && entry["summary"]["body"] == "body"
        }));
        assert!(entries.iter().any(|entry| {
            entry["summary"]["kind"] == "input_request_responded"
                && entry["summary"]["reason"] == "approved"
        }));
        assert!(entries.iter().any(|entry| {
            entry["summary"]
                == serde_json::json!({
                    "kind": "review_note_imported"
                })
        }));
    }

    #[test]
    fn history_include_body_hydrates_artifact_body_without_exposing_path() {
        let dir = tempfile::tempdir().unwrap();
        let artifact_path = "artifacts/notes/body.json";
        let full_path = dir.path().join(artifact_path);
        std::fs::create_dir_all(full_path.parent().unwrap()).unwrap();
        std::fs::write(
            &full_path,
            r#"{"schema":"shore.note-body","version":1,"body":"artifact body"}"#,
        )
        .unwrap();
        let filters = TestHistoryOptions {
            include_body: true,
            ..TestHistoryOptions::default()
        };

        let backend = StoreBackend::Local(dir.path().to_path_buf());
        let result = adapt_test_history(
            &[observation_event_with_artifact_path(artifact_path)],
            filters,
            TestHistoryPage::default(),
            Some(&backend),
        )
        .unwrap();
        let json = serde_json::to_value(&result.entries[0]).unwrap();
        let serialized = serde_json::to_string(&result.entries[0]).unwrap();

        assert_eq!(json["summary"]["body"], "artifact body");
        assert!(!serialized.contains("bodyArtifactPath"));
        assert!(!serialized.contains("artifacts/notes"));
    }

    #[test]
    fn window_unset_hydrates_all_and_emits_no_cursor() {
        let events = windowing_events(5);
        let result = adapt_test_history(
            &events,
            TestHistoryOptions::default(),
            TestHistoryPage::default(),
            None,
        )
        .unwrap();

        assert_eq!(result.entries.len(), 5);
        assert!(result.next_cursor.is_none());
        assert_eq!(result.event_count, 5);
    }

    #[test]
    fn window_limit_takes_prefix_and_keeps_full_identity() {
        let events = windowing_events(5);
        let result = adapt_test_history(
            &events,
            TestHistoryOptions::default(),
            TestHistoryPage {
                limit: Some(2),
                after: None,
            },
            None,
        )
        .unwrap();

        assert_eq!(result.entries.len(), 2);
        assert!(result.next_cursor.is_some());
        // Identity always describes the full replayed set, never the window.
        assert_eq!(result.event_count, 5);
        assert_eq!(result.history_count(), 2);
    }

    #[test]
    fn window_cursor_continues_without_overlap() {
        let events = windowing_events(5);
        let page1 = adapt_test_history(
            &events,
            TestHistoryOptions::default(),
            TestHistoryPage {
                limit: Some(2),
                after: None,
            },
            None,
        )
        .unwrap();
        let token = page1.next_cursor.clone().unwrap();

        let page2 = adapt_test_history(
            &events,
            TestHistoryOptions::default(),
            TestHistoryPage {
                limit: Some(2),
                after: Some(HistoryCursor::decode(&token).unwrap()),
            },
            None,
        )
        .unwrap();

        // Page two starts strictly after page one's last entry: no overlap, no gap.
        assert_ne!(
            page1.entries.last().unwrap().event_id,
            page2.entries.first().unwrap().event_id
        );
        assert!(
            page2.entries.first().unwrap().occurred_at > page1.entries.last().unwrap().occurred_at
        );
    }

    #[test]
    fn history_includes_duplicate_semantic_diagnostics() {
        let first = observation_event_with_id_and_key("obs:sha256:same", "retry-a");
        let second = observation_event_with_id_and_key("obs:sha256:same", "retry-b");

        let result = adapt_test_history(
            &[first, second],
            TestHistoryOptions::default(),
            TestHistoryPage::default(),
            None,
        )
        .unwrap();

        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == DUPLICATE_SEMANTIC_OBSERVATION_EVENT_CODE
                && diagnostic.message.contains("obs:sha256:same")
        }));
        assert_eq!(
            result.entries.len(),
            2,
            "history preserves raw append-only facts"
        );
    }

    #[test]
    fn history_diagnostics_are_not_suppressed_by_filters() {
        let duplicate_a = observation_event_with_id_and_key("obs:sha256:same", "retry-a");
        let duplicate_b = observation_event_with_id_and_key("obs:sha256:same", "retry-b");
        let filters = TestHistoryOptions {
            event_types: vec![EventType::WorkObjectProposed],
            ..TestHistoryOptions::default()
        };

        let result = adapt_test_history(
            &[duplicate_a, duplicate_b],
            filters,
            TestHistoryPage::default(),
            None,
        )
        .unwrap();

        assert!(result.entries.is_empty());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == DUPLICATE_SEMANTIC_OBSERVATION_EVENT_CODE })
        );
    }

    #[test]
    fn base_projection_hydrates_all_bodies_and_builds_records() {
        let events = [
            observation_event_with_body("inline body"),
            assessment_event(),
        ];
        // A default config (no verification policy, no enrichment) exercises the
        // zero-cost path; bodies are inline so no backend is needed.
        let base =
            history_base_from_events(&events, &BaseProjectionConfig::default(), None).unwrap();

        assert_eq!(base.entries.len(), 2);
        // Every entry is hydrated (include_body) — the inline body is in the haystack.
        assert!(
            base.entries
                .iter()
                .any(|entry| entry.record.text.contains("inline body"))
        );
        // Identity describes the full replayed set (plan 0092 INV-5).
        assert_eq!(base.event_count, 2);
        assert!(base.event_set_hash.starts_with("sha256:"));
    }

    /// The opened entry in a base projection (there is exactly one per fixture).
    fn opened_is_field(base: &BaseHistoryProjection) -> Option<&str> {
        base.entries
            .iter()
            .find(|e| e.entry.event_type == EventType::InputRequestOpened)
            .and_then(|e| e.record.field("is"))
    }

    #[test]
    fn base_marks_a_still_open_request_is_open() {
        let events = vec![input_request_opened_event()];
        let base =
            history_base_from_events(&events, &BaseProjectionConfig::default(), None).unwrap();
        assert_eq!(opened_is_field(&base), Some(" open ")); // space-wrapped set
    }

    #[test]
    fn base_marks_a_responded_request_is_answered() {
        let events = vec![
            input_request_opened_event(),
            input_request_responded_event(),
        ];
        let base =
            history_base_from_events(&events, &BaseProjectionConfig::default(), None).unwrap();
        assert_eq!(opened_is_field(&base), Some(" answered "));
    }

    #[test]
    fn is_open_derives_from_the_lifecycle_not_from_attention() {
        // An assessment can clear/subsume attention items, but the request's `is:`
        // standing depends only on whether a response exists. The open request stays
        // `is:open` after an unrelated assessment lands.
        let events = vec![input_request_opened_event(), assessment_event()];
        let base =
            history_base_from_events(&events, &BaseProjectionConfig::default(), None).unwrap();
        assert_eq!(opened_is_field(&base), Some(" open "));
    }

    #[test]
    fn base_projection_sorts_mixed_timestamps_by_instant_then_event_id() {
        let late = event_with_time_and_key("2026-05-13T10:00:02Z", "late");
        let early = event_with_time_and_key("unix-ms:0", "early");
        let base = history_base_from_events(&[late, early], &BaseProjectionConfig::default(), None)
            .unwrap();

        assert_eq!(base.entries[0].entry.occurred_at, "unix-ms:0");
        assert_eq!(base.entries[1].entry.occurred_at, "2026-05-13T10:00:02Z");
    }

    #[test]
    fn default_page_projection_sorts_mixed_timestamp_forms_by_instant() {
        let late = event_with_time_and_key("2026-05-13T10:00:02Z", "late");
        let early = event_with_time_and_key("unix-ms:0", "early");
        let page = history_default_page_from_events(
            &[late, early],
            &BaseProjectionConfig::default(),
            None,
            2,
            HistoryOrder::Asc,
        )
        .unwrap();

        assert_eq!(page.entries[0].occurred_at, "unix-ms:0");
        assert_eq!(page.entries[1].occurred_at, "2026-05-13T10:00:02Z");
    }

    #[test]
    fn base_projection_resolves_snapshot_field_to_the_captured_object() {
        // A capture carries revision.object_id; an observation on the same revision
        // must resolve the `snapshot` grammar field to that captured object id.
        // Assert the JOIN (obs snapshot == the capture's object) rather than a
        // hard-coded string. The field key is `snapshot` (#334); the value is still
        // sourced from the shared `object_id` document field.
        let capture = revision_captured_event_for("review-unit:sha256:one");
        let obs = observation_event("review-unit:sha256:one", "agent:codex", "Keep");
        let base =
            history_base_from_events(&[capture, obs], &BaseProjectionConfig::default(), None)
                .unwrap();

        let captured_object = match &base
            .entries
            .iter()
            .find(|entry| matches!(entry.entry.event_type, EventType::WorkObjectProposed))
            .unwrap()
            .entry
            .summary
        {
            ReviewHistorySummary::RevisionCaptured { object_id, .. } => {
                object_id.as_str().to_owned()
            }
            other => panic!("expected a capture summary, got {other:?}"),
        };
        let obs_entry = base
            .entries
            .iter()
            .find(|entry| matches!(entry.entry.event_type, EventType::ReviewObservationRecorded))
            .unwrap();
        assert_eq!(
            obs_entry.record.field("snapshot"),
            Some(captured_object.as_str())
        );
        assert!(!captured_object.is_empty());
    }

    fn review_initialized_event(key: &str) -> ShoreEvent {
        let journal_id = JournalId::new("journal:default");
        ShoreEvent::new(
            EventType::ReviewInitialized,
            ReviewInitializedPayload::idempotency_key(&journal_id),
            EventTarget::for_journal(journal_id),
            Writer::shore_local("test"),
            ReviewInitializedPayload {},
            format!("2026-05-13T10:00:0{key}Z"),
        )
        .unwrap()
    }

    fn revision_captured_event() -> ShoreEvent {
        revision_captured_event_for("review-unit:sha256:one")
    }

    fn revision_captured_event_for(revision_id: &str) -> ShoreEvent {
        let revision_id = RevisionId::new(revision_id);
        let payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new(format!(
                "engagement:sha256:{}",
                crate::canonical_hash::sha256_bytes_hex(
                    (RevisionId::new(format!("rev:{}", revision_id.as_str())))
                        .as_str()
                        .as_bytes()
                )
            )),
            work_object: WorkObjectProposal::Revision {
                revision: Revision {
                    // The payload revision must match the envelope subject (and the
                    // observation's target) so the object-join resolves — the
                    // subject is now reconstructed from this payload, not the envelope.
                    id: revision_id.clone(),
                    object_id: ObjectId::new(format!("snap:{}", revision_id.as_str())),
                    git_provenance: Some(GitProvenance {
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
                    }),
                },
                summary: None,
                object_artifact_content_hash: "sha256:artifact".to_owned(),
                supersedes: vec![],
            },
        };
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            "capture:one",
            EventTarget::for_revision(JournalId::new("journal:default"), revision_id, None)
                .unwrap(),
            Writer::shore_local("test"),
            payload,
            "2026-05-13T10:00:00Z",
        )
        .unwrap()
    }

    fn event_with_time_and_key(occurred_at: &str, key: &str) -> ShoreEvent {
        let mut event = observation_event("review-unit:sha256:one", "agent:codex", key);
        event.occurred_at = occurred_at.to_owned();
        event
    }

    /// `count` observation events with strictly increasing `occurred_at` and
    /// distinct event ids, for exercising the windowed projection.
    fn windowing_events(count: usize) -> Vec<ShoreEvent> {
        (0..count)
            .map(|i| {
                event_with_time_and_key(
                    &format!("2026-05-13T10:00:{:02}Z", i + 1),
                    &format!("window-{i}"),
                )
            })
            .collect()
    }

    fn observation_event(revision_id: &str, track_id: &str, title: &str) -> ShoreEvent {
        let revision_id = RevisionId::new(revision_id);
        let payload = ReviewObservationRecordedPayload {
            observation_id: ObservationId::new(format!("obs:sha256:{title}")),
            target: ReviewTargetRef::Revision {
                revision_id: revision_id.clone(),
            },
            title: title.to_owned(),
            body: None,
            body_content_type: Default::default(),
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            tags: vec![],
            confidence: None,
            supersedes_observation_ids: vec![],
            responds_to_observation_ids: vec![],
        };
        tracked_event_for_unit(
            EventType::ReviewObservationRecorded,
            &format!("observation:{title}:{track_id}"),
            track_id,
            revision_id,
            payload,
            "2026-05-13T10:00:01Z",
        )
    }

    fn observation_event_with_body(body: &str) -> ShoreEvent {
        let payload = ReviewObservationRecordedPayload {
            observation_id: ObservationId::new("obs:sha256:one"),
            target: ReviewTargetRef::Revision {
                revision_id: revision_id("one"),
            },
            title: "Observation".to_owned(),
            body: Some(body.to_owned()),
            body_content_type: Default::default(),
            body_artifact_path: None,
            body_byte_size: Some(body.len() as u64),
            body_content_hash: Some("sha256:body".to_owned()),
            tags: vec!["correctness".to_owned()],
            confidence: Some("high".to_owned()),
            supersedes_observation_ids: vec![],
            responds_to_observation_ids: vec![],
        };
        tracked_event(
            EventType::ReviewObservationRecorded,
            "observation:one",
            "agent:codex",
            payload,
            "2026-05-13T10:00:01Z",
        )
    }

    fn observation_event_with_artifact_path(path: &str) -> ShoreEvent {
        let payload = ReviewObservationRecordedPayload {
            observation_id: ObservationId::new("obs:sha256:artifact"),
            target: ReviewTargetRef::Revision {
                revision_id: revision_id("one"),
            },
            title: "Observation".to_owned(),
            body: None,
            body_content_type: Default::default(),
            body_artifact_path: Some(path.to_owned()),
            body_byte_size: Some(5000),
            body_content_hash: Some("sha256:body".to_owned()),
            tags: vec![],
            confidence: None,
            supersedes_observation_ids: vec![],
            responds_to_observation_ids: vec![],
        };
        tracked_event(
            EventType::ReviewObservationRecorded,
            "observation:artifact",
            "agent:codex",
            payload,
            "2026-05-13T10:00:01Z",
        )
    }

    fn assessment_event() -> ShoreEvent {
        assessment_event_with_target(ReviewTargetRef::Revision {
            revision_id: revision_id("one"),
        })
    }

    fn assessment_event_with_target(target: ReviewTargetRef) -> ShoreEvent {
        let payload = ReviewAssessmentRecordedPayload {
            assessment_id: AssessmentId::new("assess:sha256:one"),
            target,
            assessment: ReviewAssessment::Accepted,
            summary: Some("ship it".to_owned()),
            summary_content_type: Default::default(),
            summary_artifact_path: None,
            summary_byte_size: Some(7),
            summary_content_hash: Some("sha256:summary".to_owned()),
            replaces_assessment_ids: vec![],
            related_observation_ids: vec![ObservationId::new("obs:sha256:one")],
            related_input_request_ids: vec![InputRequestId::new("input-request:sha256:one")],
        };
        tracked_event(
            EventType::ReviewAssessmentRecorded,
            "assessment:one",
            "human:kevin",
            payload,
            "2026-05-13T10:00:02Z",
        )
    }

    fn observation_event_with_id_and_key(
        observation_id: &str,
        idempotency_key: &str,
    ) -> ShoreEvent {
        let payload = ReviewObservationRecordedPayload {
            observation_id: ObservationId::new(observation_id),
            target: ReviewTargetRef::Revision {
                revision_id: revision_id("one"),
            },
            title: "Duplicate".to_owned(),
            body: Some("same body".to_owned()),
            body_content_type: Default::default(),
            body_artifact_path: None,
            body_byte_size: Some(9),
            body_content_hash: Some("sha256:body".to_owned()),
            tags: vec![],
            confidence: None,
            supersedes_observation_ids: vec![],
            responds_to_observation_ids: vec![],
        };
        tracked_event(
            EventType::ReviewObservationRecorded,
            idempotency_key,
            "agent:codex",
            payload,
            "2026-05-13T10:00:01Z",
        )
    }

    fn input_request_opened_event() -> ShoreEvent {
        let payload = InputRequestOpenedPayload {
            input_request_id: InputRequestId::new("input-request:sha256:one"),
            target: ReviewTargetRef::Revision {
                revision_id: revision_id("one"),
            },
            task_target: None,
            reason_code: InputRequestReasonCode::ManualDecisionRequired,
            title: "Need decision".to_owned(),
            body: Some("body".to_owned()),
            body_content_type: Default::default(),
            body_artifact_path: None,
            body_byte_size: Some(4),
            body_content_hash: Some("sha256:body".to_owned()),
            target_fingerprint: None,
        };
        tracked_event(
            EventType::InputRequestOpened,
            "input-request:open",
            "human:kevin",
            payload,
            "2026-05-13T10:00:02Z",
        )
        .with_assertion_mode(AssertionMode::Operative)
    }

    fn input_request_responded_event() -> ShoreEvent {
        let payload = InputRequestRespondedPayload {
            input_request_response_id: InputRequestResponseId::new(
                "input-request-response:sha256:one",
            ),
            input_request_id: InputRequestId::new("input-request:sha256:one"),
            revision_id: Some(revision_id("one")),
            task_target: None,
            outcome: InputRequestResponseOutcome::Approved,
            reason: Some("approved".to_owned()),
            reason_content_type: Default::default(),
            reason_artifact_path: None,
            reason_byte_size: Some(8),
            reason_content_hash: Some("sha256:reason".to_owned()),
            target_fingerprint: None,
        };
        tracked_event(
            EventType::InputRequestResponded,
            "input-request:respond",
            "human:kevin",
            payload,
            "2026-05-13T10:00:03Z",
        )
    }

    fn validation_check_recorded_event() -> ShoreEvent {
        let payload = ValidationCheckRecordedPayload {
            validation_check_id: ValidationCheckId::new("validation:sha256:one"),
            target: ValidationTarget::Revision {
                revision_id: revision_id("one"),
            },
            check_name: "cargo test".to_owned(),
            command: None,
            status: ValidationStatus::Passed,
            exit_code: Some(0),
            trigger: ValidationTrigger::Manual,
            source_fingerprint: None,
            summary: Some("tests passed".to_owned()),
            summary_content_type: Default::default(),
            summary_artifact_path: None,
            summary_byte_size: Some(12),
            summary_content_hash: Some("sha256:summary".to_owned()),
            started_at: Some("2026-05-13T09:59:00Z".to_owned()),
            completed_at: Some("2026-05-13T10:00:00Z".to_owned()),
            log_artifact_content_hashes: vec!["sha256:log".to_owned()],
        };
        tracked_event(
            EventType::ValidationCheckRecorded,
            "validation:one",
            "agent:codex",
            payload,
            "2026-05-13T10:00:04Z",
        )
    }

    fn review_note_imported_event() -> ShoreEvent {
        // The typed payload is retired (parse-level tombstone); an old store's
        // event carries a raw payload the projection never decodes, so a
        // minimal local stand-in payload is all the tombstone path needs.
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct LegacyNotePayload {
            note_id: &'static str,
        }
        impl crate::session::event::EventPayload for LegacyNotePayload {
            fn event_type(&self) -> EventType {
                EventType::ReviewNoteImported
            }
        }
        ShoreEvent::new(
            EventType::ReviewNoteImported,
            "review-note:one",
            EventTarget::for_journal(JournalId::new("journal:default")),
            Writer::shore_local("test"),
            LegacyNotePayload { note_id: "note-1" },
            "2026-05-13T10:00:05Z",
        )
        .unwrap()
    }

    fn tracked_event<P>(
        event_type: EventType,
        idempotency_key: &str,
        track_id: &str,
        payload: P,
        occurred_at: &str,
    ) -> ShoreEvent
    where
        P: crate::session::event::EventPayload,
    {
        tracked_event_for_unit(
            event_type,
            idempotency_key,
            track_id,
            revision_id("one"),
            payload,
            occurred_at,
        )
    }

    fn tracked_event_for_unit<P>(
        event_type: EventType,
        idempotency_key: &str,
        track_id: &str,
        revision_id: RevisionId,
        payload: P,
        occurred_at: &str,
    ) -> ShoreEvent
    where
        P: crate::session::event::EventPayload,
    {
        let target = EventTarget::for_subject(
            JournalId::new("journal:default"),
            TargetRef::Review(ReviewTargetRef::Revision {
                revision_id: revision_id.clone(),
            }),
            Some(TrackId::new(track_id)),
        )
        .unwrap();
        ShoreEvent::new(
            event_type,
            idempotency_key,
            target,
            Writer::shore_local("test"),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    fn revision_id(suffix: &str) -> RevisionId {
        RevisionId::new(format!("rev:sha256:{suffix}"))
    }
}
