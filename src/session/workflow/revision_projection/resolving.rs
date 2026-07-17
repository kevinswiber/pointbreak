use super::identity::RevisionProjectionIdentity;
use crate::error::{Result, ShoreError};
use crate::session::event::{
    EventType, GitProvenance, ShoreEvent, WorkObjectProposal, WorkObjectProposedPayload,
};
use crate::session::observation::ResolvedRevision;

/// Decode a `work_object_proposed` payload into the current model, rejecting the
/// retired pre-1.0 legacy view. A legacy revision capture bound its object
/// artifact under the `snapshotArtifactContentHash` wire key instead of the
/// current `objectArtifactContentHash`; 1.0 is the store-format floor, so such a
/// capture is no longer upcast at read time — it is refused with a clear error.
fn decode_work_object_proposed(value: &serde_json::Value) -> Result<WorkObjectProposedPayload> {
    let rides_retired_view = value
        .get("workObject")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|work_object| {
            work_object.contains_key("snapshotArtifactContentHash")
                && !work_object.contains_key("objectArtifactContentHash")
        });
    if rides_retired_view {
        return Err(ShoreError::Message(
            "work_object_proposed capture uses the retired snapshotArtifactContentHash view; \
             this pre-1.0 store format is no longer supported"
                .to_owned(),
        ));
    }
    Ok(serde_json::from_value(value.clone())?)
}

pub(super) fn selected_revision_capture(
    events: &[ShoreEvent],
    resolved: &ResolvedRevision,
) -> Result<RevisionProjectionIdentity> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::WorkObjectProposed)
    {
        let payload = decode_work_object_proposed(&event.payload)?;
        let WorkObjectProposal::Revision {
            revision,
            summary,
            object_artifact_content_hash,
            ..
        } = payload.work_object
        else {
            continue;
        };
        if revision.id == resolved.revision_id {
            // Provenance is enforced only for the matching capture, so a malformed
            // sibling (e.g. a fabricated identity-reuse capture with no provenance)
            // never masks the target the caller asked for.
            let Some(GitProvenance {
                source,
                base,
                target,
            }) = revision.git_provenance
            else {
                return Err(ShoreError::Message(format!(
                    "captured revision {} has no git provenance",
                    revision.id.as_str()
                )));
            };
            return Ok(RevisionProjectionIdentity {
                id: revision.id.clone(),
                summary,
                journal_id: event.target.journal_id.clone(),
                source,
                base,
                target,
                revision_id: revision.id,
                object_id: revision.object_id,
                object_artifact_content_hash,
                capture_event_id: event.event_id.clone(),
            });
        }
    }

    Err(ShoreError::Message(format!(
        "captured review unit event missing for {}",
        resolved.revision_id.as_str()
    )))
}

/// Every captured revision identity in the event set, in event order — the
/// single-pass enumeration the overview batch folds over. Mirrors
/// `list_from_events`' `WorkObjectProposed` scan and shares its provenance
/// requirement: a captured revision without git provenance is an error here, the
/// same way `entry_from_event` rejects it on the list path (so the batch and the
/// `/api/revisions` list it serves agree on which captures are listable). Task
/// proposals are skipped, exactly as the review listing skips them.
pub(super) fn enumerate_revision_identities(
    events: &[ShoreEvent],
) -> Result<Vec<RevisionProjectionIdentity>> {
    events
        .iter()
        .filter(|event| event.event_type == EventType::WorkObjectProposed)
        .filter_map(|event| revision_identity_from_capture_event(event).transpose())
        .collect()
}

/// Decode one `WorkObjectProposed` event into a [`RevisionProjectionIdentity`],
/// or `None` when the move proposes a task attempt rather than a review revision.
/// Errors when a captured revision lacks git provenance — matching the list
/// path's `entry_from_event`. Used by [`enumerate_revision_identities`].
fn revision_identity_from_capture_event(
    event: &ShoreEvent,
) -> Result<Option<RevisionProjectionIdentity>> {
    let payload = decode_work_object_proposed(&event.payload)?;
    let WorkObjectProposal::Revision {
        revision,
        summary,
        object_artifact_content_hash,
        ..
    } = payload.work_object
    else {
        return Ok(None);
    };
    let Some(GitProvenance {
        source,
        base,
        target,
    }) = revision.git_provenance
    else {
        return Err(ShoreError::Message(format!(
            "captured revision {} has no git provenance",
            revision.id.as_str()
        )));
    };
    Ok(Some(RevisionProjectionIdentity {
        id: revision.id.clone(),
        summary,
        journal_id: event.target.journal_id.clone(),
        source,
        base,
        target,
        revision_id: revision.id,
        object_id: revision.object_id,
        object_artifact_content_hash,
        capture_event_id: event.event_id.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EngagementId, ObjectId, RevisionId};
    use crate::session::event::Revision;

    fn legacy_view_fixture_event() -> ShoreEvent {
        serde_json::from_str(include_str!(
            "../../../../tests/fixtures/event_signatures/legacy-view-work-object-proposed-event.json"
        ))
        .expect("legacy-view fixture decodes")
    }

    #[test]
    fn legacy_view_capture_is_rejected_at_the_format_floor() {
        // The retired pre-1.0 view bound the object artifact under
        // `snapshotArtifactContentHash`. 1.0 is the store-format floor: it is no
        // longer upcast at read time; the capture is refused with a clear error.
        let event = legacy_view_fixture_event();
        assert_eq!(
            event.payload_version, 0,
            "fixture is pinned at the legacy payload view"
        );
        let err = decode_work_object_proposed(&event.payload)
            .expect_err("a legacy-view capture is rejected at the format floor");
        assert!(
            err.to_string().contains("no longer supported"),
            "reads as a retired format; got: {err}"
        );
    }

    #[test]
    fn decode_accepts_the_current_view() {
        let current = WorkObjectProposedPayload {
            engagement_id: EngagementId::new("engagement:sha256:e"),
            work_object: WorkObjectProposal::Revision {
                revision: Revision {
                    id: RevisionId::new("rev:sha256:r"),
                    object_id: ObjectId::new("obj:sha256:o"),
                    git_provenance: None,
                },
                summary: None,
                object_artifact_content_hash: "sha256:current".to_owned(),
                supersedes: vec![],
            },
        };
        let value = serde_json::to_value(&current).expect("current payload serializes");
        let decoded = decode_work_object_proposed(&value).expect("current value decodes");
        assert_eq!(decoded, current);
    }
}
