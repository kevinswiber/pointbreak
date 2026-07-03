use std::collections::BTreeMap;

use super::cursor::{HistoryCursor, HistoryWindow, cmp_key};
use super::options::ResolvedHistoryFilters;
use super::result::ReviewHistoryResult;
use super::search::{SearchRecord, entry_revision_id};
use super::summary::{ReviewHistoryEntry, ReviewHistorySummary};
use crate::error::{Result, ShoreError};
use crate::model::{ReviewEndpoint, ReviewTargetRef, RevisionId, TargetRef};
use crate::session::body_artifact::load_body_artifact;
use crate::session::event::{
    EventType, InputRequestRespondedPayload, ReviewAssessmentRecordedPayload,
    ReviewInitializedPayload, ReviewNoteImportedPayload, ReviewObservationRecordedPayload,
    RevisionCommitAssociatedPayload, RevisionCommitWithdrawnPayload, RevisionRefAssociatedPayload,
    RevisionRefWithdrawnPayload, ShoreEvent, ValidationCheckRecordedPayload, WorkObjectProposal,
    WorkObjectProposedPayload, decode_input_request_opened_payload,
};
use crate::session::projection::body_content::{
    BodyRemovalLens, body_content_diagnostics, resolve_body_content,
};
use crate::session::projection::cosignature::{
    CosignatureIndex, endorsement_readbacks, enrich_endorser_attributes,
};
use crate::session::state::SessionState;
use crate::session::store::backend::StoreBackend;
use crate::session::{
    ActorAttributesMap, ArtifactRemovalProjection, BodyContentState, DelegationMap,
    EventVerificationPolicy, ProjectionDiagnostic, TrustSet, principal_view_for,
    verify_event_signature,
};

pub(super) fn history_from_events(
    events: &[ShoreEvent],
    filters: ResolvedHistoryFilters,
    window: HistoryWindow,
    backend: Option<&StoreBackend>,
) -> Result<ReviewHistoryResult> {
    let state = SessionState::from_events(events)?;
    let event_set_hash = state
        .event_set_hash
        .clone()
        .expect("SessionState::from_events sets event_set_hash");
    // Build the co-signature index once per document. It indexes the full event
    // set (correctness), independent of any window; endorsement readbacks read
    // it only when a verification policy is set, the removal lens always does.
    let cosig_index = CosignatureIndex::build(events)?;
    let readback_index = filters
        .verification_policy
        .is_some()
        .then_some(&cosig_index);
    let removal = ArtifactRemovalProjection::from_events(events)?;
    // The lens pairs with a live backend (no backend, no byte reads, no state).
    let removal_lens = backend.map(|_| {
        BodyRemovalLens::new(
            &removal,
            &filters.trust_set,
            filters.removal_policy,
            &cosig_index,
        )
    });

    // Filter to the matching event references and sort them by the envelope
    // (occurred_at, event_id) — the same ordering as before — without hydrating
    // any bodies yet.
    let mut matched: Vec<&ShoreEvent> = events
        .iter()
        .filter(|event| event_matches_filters(event, &filters))
        .collect();
    matched.sort_by(|left, right| {
        cmp_key(&left.occurred_at, left.event_id.as_str())
            .cmp(&cmp_key(&right.occurred_at, right.event_id.as_str()))
    });

    // Window over the cheap envelope keys, then hydrate full entries (bodies
    // included) only for the windowed slice. This is what cuts body hydration:
    // out-of-window bodies are never loaded.
    let keys: Vec<HistoryCursor> = matched
        .iter()
        .map(|event| HistoryCursor {
            occurred_at: event.occurred_at.clone(),
            event_id: event.event_id.clone(),
        })
        .collect();
    let slice = window.apply(&keys);
    let entries = matched[slice.range.clone()]
        .iter()
        .map(|event| {
            history_entry_from_event(
                event,
                &filters,
                readback_index,
                backend,
                removal_lens.as_ref(),
            )
        })
        .collect::<Result<Vec<_>>>()?;

    // Body-content diagnostics are rendered-entry scoped: state resolution only
    // happens for entries that survive the window, so out-of-window removals
    // yield no diagnostic on this page (read/skip diagnostics keep describing
    // the full replayed set).
    let mut diagnostics = state.diagnostics;
    diagnostics.extend(body_content_diagnostics(
        entries.iter().flat_map(entry_body_states),
    ));

    Ok(ReviewHistoryResult {
        event_set_hash,
        event_count: events.len(),
        filters: filters.into(),
        entries,
        next_cursor: slice.next_cursor,
        diagnostics,
    })
}

/// The (state, removal-key) pairs a rendered entry contributes to the
/// body-content diagnostics. Imported notes carry their key in
/// `removed_body_content_hash` (their payload has no body hash).
fn entry_body_states(entry: &ReviewHistoryEntry) -> Vec<(BodyContentState, Option<&str>)> {
    match &entry.summary {
        ReviewHistorySummary::ReviewObservationRecorded {
            body_content_state,
            body_content_hash,
            ..
        } => vec![(*body_content_state, body_content_hash.as_deref())],
        ReviewHistorySummary::ReviewAssessmentRecorded {
            summary_content_state,
            summary_content_hash,
            ..
        } => vec![(*summary_content_state, summary_content_hash.as_deref())],
        ReviewHistorySummary::InputRequestOpened {
            body_content_state,
            body_content_hash,
            ..
        } => vec![(*body_content_state, body_content_hash.as_deref())],
        ReviewHistorySummary::InputRequestResponded {
            reason_content_state,
            reason_content_hash,
            ..
        } => vec![(*reason_content_state, reason_content_hash.as_deref())],
        ReviewHistorySummary::ReviewNoteImported {
            body_content_state,
            removed_body_content_hash,
            ..
        } => vec![(*body_content_state, removed_body_content_hash.as_deref())],
        ReviewHistorySummary::ValidationCheckRecorded {
            summary_content_state,
            summary_content_hash,
            ..
        } => vec![(*summary_content_state, summary_content_hash.as_deref())],
        _ => Vec::new(),
    }
}

/// Caller-supplied advisory verification + reader enrichment for the base build.
/// Built by the binary — the library cannot reach the inspector's `pub(crate)`
/// `discover_*` helpers (INV-8). `Default` is the zero-cost path unit tests use.
#[derive(Clone, Debug, Default)]
pub struct BaseProjectionConfig {
    pub verification_policy: Option<EventVerificationPolicy>,
    pub trust_set: TrustSet,
    pub actor_attributes: Option<ActorAttributesMap>,
    pub delegation_map: Option<DelegationMap>,
    /// Render-time removal policy for body-content states (advisory,
    /// reader-relative; never gates the compact erasure sweep).
    pub removal_policy: crate::session::RemovalPolicy,
}

/// One entry of the base projection: the hydrated review-history entry plus its
/// once-built search record (haystack + structured fields incl. resolved object).
#[derive(Clone, Debug)]
pub struct BaseEntry {
    pub entry: ReviewHistoryEntry,
    pub record: SearchRecord,
}

/// The full, body-hydrated, `(occurred_at, event_id)`-sorted review-history
/// projection the inspector caches once per store version (#255) and queries in
/// memory. Identity (`event_set_hash`, `event_count`) always describes the FULL
/// replayed set (plan 0092 INV-5), never a later filtered query result.
#[derive(Clone, Debug)]
pub struct BaseHistoryProjection {
    pub entries: Vec<BaseEntry>,
    pub event_set_hash: String,
    pub event_count: usize,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

/// Build the cacheable base from an in-memory event set: filter to the review
/// domain, sort by the envelope key, hydrate every body, and attach each entry's
/// `SearchRecord` (with `object` resolved via the revision->object map). Unlike
/// `history_from_events` it never windows — one base serves all queries for a
/// store version (task 3.1 filters/windows it purely, in memory).
pub(super) fn history_base_from_events(
    events: &[ShoreEvent],
    config: &BaseProjectionConfig,
    backend: Option<&StoreBackend>,
) -> Result<BaseHistoryProjection> {
    let state = SessionState::from_events(events)?;
    let event_set_hash = state
        .event_set_hash
        .clone()
        .expect("SessionState::from_events sets event_set_hash");

    // The base carries no revision/track/type filter — those are query params
    // (task 3.1). Bodies are always hydrated (`q` search needs them); advisory
    // verification + reader enrichment come from the caller's config (the binary
    // fills it from `discover_*` — the library cannot reach those helpers, INV-8).
    let filters = ResolvedHistoryFilters {
        include_body: true,
        verification_policy: config.verification_policy,
        trust_set: config.trust_set.clone(),
        actor_attributes: config.actor_attributes.clone(),
        delegation_map: config.delegation_map.clone(),
        removal_policy: config.removal_policy,
        ..Default::default()
    };

    // One co-signature index per build: endorsement readbacks read it only when
    // a policy is set, the removal lens always does.
    let cosig_index = CosignatureIndex::build(events)?;
    let readback_index = filters
        .verification_policy
        .is_some()
        .then_some(&cosig_index);
    let removal = ArtifactRemovalProjection::from_events(events)?;
    let removal_lens = backend.map(|_| {
        BodyRemovalLens::new(
            &removal,
            &filters.trust_set,
            filters.removal_policy,
            &cosig_index,
        )
    });

    let mut matched: Vec<&ShoreEvent> = events
        .iter()
        .filter(|event| event_matches_filters(event, &filters))
        .collect();
    matched.sort_by(|left, right| {
        cmp_key(&left.occurred_at, left.event_id.as_str())
            .cmp(&cmp_key(&right.occurred_at, right.event_id.as_str()))
    });

    // Pass 1: hydrate every entry (the base never slices — the full body set is
    // what `q` search needs). Pass 2: resolve `object` against the revision map
    // and attach each entry's search record.
    let built = matched
        .iter()
        .map(|event| {
            history_entry_from_event(
                event,
                &filters,
                readback_index,
                backend,
                removal_lens.as_ref(),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let mut diagnostics = state.diagnostics;
    diagnostics.extend(body_content_diagnostics(
        built.iter().flat_map(entry_body_states),
    ));
    let object_by_revision = revision_object_map(&built);
    let entries = built
        .into_iter()
        .map(|entry| {
            let object = entry_object(&entry, &object_by_revision);
            let record = SearchRecord::from_entry(&entry, object);
            BaseEntry { entry, record }
        })
        .collect();

    Ok(BaseHistoryProjection {
        entries,
        event_set_hash,
        event_count: events.len(),
        diagnostics,
    })
}

/// The captured-object id for each revision, keyed by the capture's subject
/// revision id (the same `entry_revision_id` the record/haystack join on, and the
/// `/api/revisions` `revisionId` in production), from the `RevisionCaptured`
/// entries. Empty keys are skipped.
fn revision_object_map(entries: &[ReviewHistoryEntry]) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for entry in entries {
        if let ReviewHistorySummary::RevisionCaptured { object_id, .. } = &entry.summary {
            let revision = entry_revision_id(entry);
            if !revision.is_empty() {
                map.insert(revision, object_id.as_str().to_owned());
            }
        }
    }
    map
}

/// The content-object id an entry's revision captured, or "" — the join the
/// client did via `objectIdForRevisionIn(/api/revisions, entryRevisionId(e))`,
/// single-sourced on the same `entry_revision_id` key.
fn entry_object<'a>(entry: &ReviewHistoryEntry, map: &'a BTreeMap<String, String>) -> &'a str {
    map.get(&entry_revision_id(entry))
        .map(String::as_str)
        .unwrap_or("")
}

pub(super) fn history_entry_from_event(
    event: &ShoreEvent,
    filters: &ResolvedHistoryFilters,
    cosig_index: Option<&CosignatureIndex<'_>>,
    backend: Option<&StoreBackend>,
    removal_lens: Option<&BodyRemovalLens<'_>>,
) -> Result<ReviewHistoryEntry> {
    let summary = match event.event_type {
        EventType::ReviewInitialized => {
            let _payload: ReviewInitializedPayload = serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::ReviewInitialized {}
        }
        EventType::WorkObjectProposed => {
            let payload: WorkObjectProposedPayload = serde_json::from_value(event.payload.clone())?;
            match payload.work_object {
                WorkObjectProposal::Revision {
                    revision,
                    object_artifact_content_hash,
                    ..
                } => {
                    let (source, base, target) = match revision.git_provenance {
                        Some(provenance) => (
                            Some(provenance.source),
                            Some(provenance.base),
                            Some(provenance.target),
                        ),
                        None => (None, None, None),
                    };
                    ReviewHistorySummary::RevisionCaptured {
                        revision_id: revision.id,
                        object_id: revision.object_id,
                        engagement_id: payload.engagement_id,
                        source,
                        base,
                        target,
                        object_artifact_content_hash,
                    }
                }
                // A task-attempt proposal is a task-domain event; the upstream
                // filter keeps it out of the review-domain history stream.
                WorkObjectProposal::TaskAttempt { .. } => {
                    return Err(ShoreError::Message(
                        "review history projects review-domain content events only; a task-attempt proposal reached this match arm — upstream filter missing".to_owned(),
                    ));
                }
            }
        }
        EventType::ReviewObservationRecorded => {
            let payload: ReviewObservationRecordedPayload =
                serde_json::from_value(event.payload.clone())?;
            let (body, body_content_state, _) = resolved_text(
                backend,
                removal_lens,
                filters.include_body,
                payload.body,
                payload.body_artifact_path.as_deref(),
            )?;
            ReviewHistorySummary::ReviewObservationRecorded {
                observation_id: payload.observation_id,
                target: payload.target,
                title: payload.title,
                body,
                body_content_type: payload.body_content_type,
                body_byte_size: payload.body_byte_size,
                body_content_hash: payload.body_content_hash,
                body_content_state,
                tags: payload.tags,
                confidence: payload.confidence,
                supersedes: payload.supersedes_observation_ids,
                responds_to: payload.responds_to_observation_ids,
            }
        }
        EventType::ReviewAssessmentRecorded => {
            let payload: ReviewAssessmentRecordedPayload =
                serde_json::from_value(event.payload.clone())?;
            let (summary, summary_content_state, _) = resolved_text(
                backend,
                removal_lens,
                filters.include_body,
                payload.summary,
                payload.summary_artifact_path.as_deref(),
            )?;
            ReviewHistorySummary::ReviewAssessmentRecorded {
                assessment_id: payload.assessment_id,
                target: payload.target,
                assessment: payload.assessment,
                summary,
                summary_content_type: payload.summary_content_type,
                summary_byte_size: payload.summary_byte_size,
                summary_content_hash: payload.summary_content_hash,
                summary_content_state,
                replaces: payload.replaces_assessment_ids,
                related_observations: payload.related_observation_ids,
                related_input_requests: payload.related_input_request_ids,
            }
        }
        EventType::InputRequestOpened => {
            let payload = decode_input_request_opened_payload(event.payload.clone())?;
            let (body, body_content_state, _) = resolved_text(
                backend,
                removal_lens,
                filters.include_body,
                payload.body,
                payload.body_artifact_path.as_deref(),
            )?;
            ReviewHistorySummary::InputRequestOpened {
                input_request_id: payload.input_request_id,
                target: payload.target,
                mode: event.assertion_mode,
                reason_code: payload.reason_code,
                title: payload.title,
                body,
                body_content_type: payload.body_content_type,
                body_byte_size: payload.body_byte_size,
                body_content_hash: payload.body_content_hash,
                body_content_state,
            }
        }
        EventType::InputRequestResponded => {
            let payload: InputRequestRespondedPayload =
                serde_json::from_value(event.payload.clone())?;
            let (reason, reason_content_state, _) = resolved_text(
                backend,
                removal_lens,
                filters.include_body,
                payload.reason,
                payload.reason_artifact_path.as_deref(),
            )?;
            ReviewHistorySummary::InputRequestResponded {
                input_request_response_id: payload.input_request_response_id,
                input_request_id: payload.input_request_id,
                outcome: payload.outcome,
                reason,
                reason_content_type: payload.reason_content_type,
                reason_byte_size: payload.reason_byte_size,
                reason_content_hash: payload.reason_content_hash,
                reason_content_state,
            }
        }
        EventType::ReviewNoteImported => {
            let payload: ReviewNoteImportedPayload = serde_json::from_value(event.payload.clone())?;
            let (body, body_content_state, removed_body_content_hash) = resolved_text(
                backend,
                removal_lens,
                filters.include_body,
                payload.body,
                payload.body_artifact_path.as_deref(),
            )?;
            ReviewHistorySummary::ReviewNoteImported {
                sidecar_source: payload.sidecar_source,
                note_id: payload.note_id,
                file_path: payload.file_path,
                file_old_path: payload.file_old_path,
                target: payload.target,
                title: payload.title,
                body,
                body_byte_size: payload.body_byte_size.map(|size| size as u64),
                body_content_state,
                removed_body_content_hash,
                tags: payload.tags,
                confidence: payload.confidence,
                external_source: payload.external_source,
                author: payload.author,
                created_at: payload.created_at,
                sidecar_content_hash: payload.sidecar_content_hash,
            }
        }
        EventType::ValidationCheckRecorded => {
            let payload: ValidationCheckRecordedPayload =
                serde_json::from_value(event.payload.clone())?;
            let (summary, summary_content_state, _) = resolved_text(
                backend,
                removal_lens,
                filters.include_body,
                payload.summary,
                payload.summary_artifact_path.as_deref(),
            )?;
            ReviewHistorySummary::ValidationCheckRecorded {
                validation_check_id: payload.validation_check_id,
                target: payload.target,
                check_name: payload.check_name,
                command: payload.command,
                status: payload.status,
                exit_code: payload.exit_code,
                trigger: payload.trigger,
                source_fingerprint: payload.source_fingerprint,
                summary,
                summary_content_type: payload.summary_content_type,
                summary_content_hash: payload.summary_content_hash,
                summary_content_state,
                started_at: payload.started_at,
                completed_at: payload.completed_at,
                log_artifact_content_hashes: payload.log_artifact_content_hashes,
            }
        }
        EventType::RevisionRefAssociated => {
            let payload: RevisionRefAssociatedPayload =
                serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::RevisionRefAssociated {
                ref_association_id: payload.ref_association_id,
                ref_name: payload.ref_name,
                head_oid: payload.head_oid,
            }
        }
        EventType::RevisionRefWithdrawn => {
            let payload: RevisionRefWithdrawnPayload =
                serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::RevisionRefWithdrawn {
                ref_withdrawal_id: payload.ref_withdrawal_id,
                ref_association_id: payload.ref_association_id,
            }
        }
        EventType::RevisionCommitAssociated => {
            let payload: RevisionCommitAssociatedPayload =
                serde_json::from_value(event.payload.clone())?;
            let ReviewEndpoint::GitCommit {
                commit_oid,
                tree_oid,
            } = payload.commit
            else {
                return Err(ShoreError::Message(
                    "commit association payload must carry a git_commit endpoint".to_owned(),
                ));
            };
            ReviewHistorySummary::RevisionCommitAssociated {
                commit_association_id: payload.commit_association_id,
                commit_oid,
                tree_oid,
            }
        }
        EventType::RevisionCommitWithdrawn => {
            let payload: RevisionCommitWithdrawnPayload =
                serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::RevisionCommitWithdrawn {
                commit_withdrawal_id: payload.commit_withdrawal_id,
                commit_association_id: payload.commit_association_id,
            }
        }
        EventType::TaskCheckpointCaptured
        | EventType::TaskObservationRecorded
        | EventType::EventSignatureRecorded
        | EventType::ArtifactRemoved => {
            return Err(ShoreError::Message(
                "review history projects review-domain content events only; a task, co-signature, or content-removal event reached this match arm — upstream filter missing".to_owned(),
            ));
        }
    };

    Ok(ReviewHistoryEntry {
        event_id: event.event_id.clone(),
        event_type: event.event_type,
        occurred_at: event.occurred_at.clone(),
        payload_hash: event.payload_hash.clone(),
        journal_id: event.target.journal_id.clone(),
        track_id: event.target.track_id.clone(),
        subject: match event.reconstruct_subject()? {
            TargetRef::Review(r) => Some(r),
            TargetRef::Task(_) | TargetRef::Journal => None,
        },
        writer: event.writer.clone(),
        verification_status: filters
            .verification_policy
            .map(|_| verify_event_signature(event, &filters.trust_set))
            .transpose()?,
        endorsements: match cosig_index {
            Some(index) => {
                let mut readbacks = endorsement_readbacks(
                    &index.cosignatures_for_target(event, &filters.trust_set)?,
                );
                enrich_endorser_attributes(&mut readbacks, filters.actor_attributes.as_ref());
                readbacks
            }
            None => Vec::new(),
        },
        principal: principal_view_for(
            &event.writer.actor_id,
            filters.delegation_map.as_ref(),
            &event.occurred_at,
        ),
        summary,
    })
}

/// The seam wrapper over [`optional_text`]: with a lens (paired with a live
/// backend) the body-content seam decides removed vs present vs missing and
/// reports the state (plus the removal key for surfaces whose payload carries
/// no body hash); without one, the legacy hydration runs and the state stays
/// `Present` unresolved.
fn resolved_text(
    backend: Option<&StoreBackend>,
    removal_lens: Option<&BodyRemovalLens<'_>>,
    include_body: bool,
    inline: Option<String>,
    artifact_path: Option<&str>,
) -> Result<(Option<String>, BodyContentState, Option<String>)> {
    match (backend, removal_lens) {
        (Some(backend), Some(lens)) => {
            let content = resolve_body_content(backend, lens, include_body, inline, artifact_path)?;
            let state = content.state();
            let removed_hash = content.removed_content_hash().map(str::to_owned);
            Ok((content.into_text(), state, removed_hash))
        }
        _ => Ok((
            optional_text(backend, include_body, inline, artifact_path)?,
            BodyContentState::default(),
            None,
        )),
    }
}

fn optional_text(
    backend: Option<&StoreBackend>,
    include_body: bool,
    inline: Option<String>,
    artifact_path: Option<&str>,
) -> Result<Option<String>> {
    if !include_body {
        return Ok(None);
    }
    if inline.is_some() {
        return Ok(inline);
    }
    match artifact_path {
        Some(path) => {
            let backend = backend.ok_or_else(|| {
                ShoreError::Message("store backend is required to hydrate body artifact".to_owned())
            })?;
            load_body_artifact(backend, path)
        }
        None => Ok(None),
    }
}

fn event_matches_filters(event: &ShoreEvent, filters: &ResolvedHistoryFilters) -> bool {
    // Review history is a review-domain content projection by name and contract. Task-domain
    // events have a sibling projection; detached co-signatures are read through the dedicated
    // co-signature-set projection; content-removal facts are session-anchored store maintenance
    // rendered through the removal projection. None is summarized in this content stream.
    if matches!(
        event.event_type,
        EventType::TaskCheckpointCaptured
            | EventType::TaskObservationRecorded
            | EventType::EventSignatureRecorded
            | EventType::ArtifactRemoved
    ) {
        return false;
    }
    // A generative move can propose either a revision or a task attempt; the
    // task-domain proposal carries a Task subject and belongs to the sibling
    // task projection, so the review-domain stream skips it (the same exclusion
    // the dedicated task event types get above). The subject is reconstructed
    // from the payload (the envelope carries only the opaque subjectId); a
    // reconstruction failure excludes the event from the review stream.
    let Ok(subject) = event.reconstruct_subject() else {
        return false;
    };
    if matches!(subject, TargetRef::Task(_)) {
        return false;
    }
    let subject_revision_id = subject_revision_id(&subject);
    if filters
        .revision_id
        .as_ref()
        .is_some_and(|revision_id| subject_revision_id != Some(revision_id))
    {
        return false;
    }
    if filters
        .track_id
        .as_ref()
        .is_some_and(|track_id| event.target.track_id.as_ref() != Some(track_id))
    {
        return false;
    }
    if !filters.event_types.is_empty() && !filters.event_types.contains(&event.event_type) {
        return false;
    }
    if let Some(ref_matched_units) = filters.ref_matched_units.as_ref()
        && !subject_revision_id.is_some_and(|revision_id| ref_matched_units.contains(revision_id))
    {
        return false;
    }
    true
}

/// The revision a subject addresses, if any. Every review-domain variant keys on
/// a `revision_id`; the journal carrier and task subjects address no revision.
fn subject_revision_id(subject: &TargetRef) -> Option<&RevisionId> {
    match subject {
        TargetRef::Review(review) => match review {
            ReviewTargetRef::Revision { revision_id }
            | ReviewTargetRef::File { revision_id, .. }
            | ReviewTargetRef::Range { revision_id, .. }
            | ReviewTargetRef::Observation { revision_id, .. }
            | ReviewTargetRef::InputRequest { revision_id, .. }
            | ReviewTargetRef::Assessment { revision_id, .. }
            | ReviewTargetRef::Event { revision_id, .. } => Some(revision_id),
        },
        TargetRef::Task(_) | TargetRef::Journal => None,
    }
}
