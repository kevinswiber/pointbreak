use super::identity::RevisionProjectionIdentity;
use crate::error::{Result, ShoreError};
use crate::session::event::{
    EventType, GitProvenance, ShoreEvent, WorkObjectProposal, WorkObjectProposedPayload,
};
use crate::session::observation::ResolvedRevision;

/// The current view version of the `work_object_proposed` payload, matching
/// the envelope's default `payloadVersion`. A payload marked with an older
/// view version is routed through [`upcast_work_object_proposed`] before the
/// projection decodes it.
const CURRENT_WORK_OBJECT_PROPOSED_VIEW: u32 = 1;

/// Read-time payload-view upcast for the `work_object_proposed` family, keyed
/// on the hash-excluded per-payload `payloadVersion` (see
/// `docs/store-migration.md`, the bounded exception to the fail-loud strict
/// reader). A pure map from a stored payload value to the current in-memory
/// model: it never re-serializes the upcast view back to stored bytes, never
/// re-derives a digest, and never writes — the stored event and all four
/// digests (`payloadHash`, the to-be-signed bytes, `eventRecordHash`,
/// `eventSetHash`) stay untouched.
///
/// This is the upcast's single attach point today. The sibling decoders
/// (`revision_identity_from_capture_event` below, `revision_list.rs`, the
/// other projection decoders) are deliberately not gated: generalizing to
/// per-family upcast hooks is deferred until a second family needs one.
fn upcast_work_object_proposed(
    value: serde_json::Value,
    payload_version: u32,
) -> Result<WorkObjectProposedPayload> {
    if payload_version < CURRENT_WORK_OBJECT_PROPOSED_VIEW {
        upcast_legacy_work_object_proposed(value)
    } else {
        Ok(serde_json::from_value(value)?)
    }
}

/// Map a legacy-view `work_object_proposed` payload value to the current
/// model: a revision proposal's artifact binding that rides under the retired
/// `snapshotArtifactContentHash` wire key is re-presented under the current
/// `objectArtifactContentHash` field. Pure re-presentation of the projected
/// view only; the stored bytes are never rewritten.
fn upcast_legacy_work_object_proposed(
    mut value: serde_json::Value,
) -> Result<WorkObjectProposedPayload> {
    if let Some(work_object) = value.get_mut("workObject").and_then(|v| v.as_object_mut())
        && !work_object.contains_key("objectArtifactContentHash")
        && let Some(hash) = work_object.remove("snapshotArtifactContentHash")
    {
        work_object.insert("objectArtifactContentHash".to_owned(), hash);
    }
    Ok(serde_json::from_value(value)?)
}

pub(super) fn selected_revision_capture(
    events: &[ShoreEvent],
    resolved: &ResolvedRevision,
) -> Result<RevisionProjectionIdentity> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::WorkObjectProposed)
    {
        let payload = upcast_work_object_proposed(event.payload.clone(), event.payload_version)?;
        let WorkObjectProposal::Revision {
            revision,
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
///
/// Deliberately not routed through [`upcast_work_object_proposed`]: the view
/// upcast attaches at the selected-revision decode only, and gating every
/// decode site is deferred until a second payload family needs an upcast.
fn revision_identity_from_capture_event(
    event: &ShoreEvent,
) -> Result<Option<RevisionProjectionIdentity>> {
    let payload: WorkObjectProposedPayload = serde_json::from_value(event.payload.clone())?;
    let WorkObjectProposal::Revision {
        revision,
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
    use crate::crypto::EventVerificationStatus;
    use crate::model::{EngagementId, ObjectId, RevisionId};
    use crate::session::event::{Revision, event_to_be_signed};
    use crate::session::projection::freshness::event_set_hash_for_events;
    use crate::session::signing::{event_signature_trust_set, verify_event_signature};

    fn legacy_view_fixture_event() -> ShoreEvent {
        serde_json::from_str(include_str!(
            "../../../../tests/fixtures/event_signatures/legacy-view-work-object-proposed-event.json"
        ))
        .expect("legacy-view fixture decodes")
    }

    fn legacy_artifact_hash() -> String {
        format!("sha256:{}", "a".repeat(64))
    }

    #[test]
    fn view_upcast_leaves_all_stored_digests_and_signature_intact() {
        // A read-time payload-view upcast may re-present the payload in the
        // projected view; it may never perturb the stored event's digests or
        // its signature (see docs/store-migration.md, the bounded exception to
        // the fail-loud strict reader). This computes all four digests through
        // the same builders the real verify/dedup/freshness paths use, runs the
        // upcast against the projected view only, and proves byte-identity.
        let event = legacy_view_fixture_event();
        assert_eq!(
            event.payload_version, 0,
            "fixture is pinned at the legacy payload view"
        );

        let baseline_payload_hash = event.payload_hash.clone();
        let baseline_tbs_bytes = event_to_be_signed(&event)
            .expect("build to-be-signed view")
            .canonical_bytes()
            .expect("canonical to-be-signed bytes");
        let baseline_record_hash = event.event_record_hash().expect("event record hash");
        let baseline_set_hash =
            event_set_hash_for_events(std::slice::from_ref(&event)).expect("event set hash");

        // The upcast operates on the projected view only; the stored event is
        // held unchanged and never written back.
        let view = upcast_work_object_proposed(event.payload.clone(), event.payload_version)
            .expect("legacy payload upcasts to the current model");

        // The upcast did observable work: the stored legacy payload does not
        // decode as the current model directly (the artifact binding rides
        // under the retired wire key), while the upcast view carries it under
        // the current field.
        assert!(
            serde_json::from_value::<WorkObjectProposedPayload>(event.payload.clone()).is_err(),
            "the legacy payload must not decode as the current model without the upcast"
        );
        let WorkObjectProposal::Revision {
            object_artifact_content_hash,
            ..
        } = view.work_object
        else {
            panic!("legacy fixture proposes a revision");
        };
        assert_eq!(object_artifact_content_hash, legacy_artifact_hash());

        // Every digest recomputed from the unchanged stored event is
        // byte-identical to its baseline.
        assert_eq!(event.payload_hash, baseline_payload_hash);
        assert_eq!(
            event_to_be_signed(&event)
                .expect("rebuild to-be-signed view")
                .canonical_bytes()
                .expect("recompute canonical bytes"),
            baseline_tbs_bytes
        );
        assert_eq!(
            event.event_record_hash().expect("recompute record hash"),
            baseline_record_hash
        );
        assert_eq!(
            event_set_hash_for_events(std::slice::from_ref(&event)).expect("recompute set hash"),
            baseline_set_hash
        );

        // Capstone: the stored signature over the legacy payload still
        // verifies Valid after the upcast ran.
        let trust = event_signature_trust_set(
            serde_json::from_str(include_str!(
                "../../../../tests/fixtures/event_signatures/did-key-ed25519.json"
            ))
            .expect("trust fixture decodes"),
        )
        .expect("build trust set");
        assert_eq!(
            verify_event_signature(&event, &trust).expect("verify fixture event"),
            EventVerificationStatus::Valid
        );
    }

    #[test]
    fn upcast_maps_legacy_payload_and_passes_current_payload_through() {
        // Legacy view: the retired artifact-hash wire key is re-presented
        // under the current model field.
        let legacy = serde_json::json!({
            "engagementId": "engagement:sha256:e",
            "workObject": {
                "kind": "revision",
                "revision": { "id": "rev:sha256:r", "objectId": "obj:sha256:o" },
                "snapshotArtifactContentHash": "sha256:legacy",
            },
        });
        let upcast = upcast_work_object_proposed(legacy, 0).expect("legacy value upcasts");
        let WorkObjectProposal::Revision {
            object_artifact_content_hash,
            ..
        } = upcast.work_object
        else {
            panic!("legacy value proposes a revision");
        };
        assert_eq!(object_artifact_content_hash, "sha256:legacy");

        // Current view: a current-model value passes through unchanged.
        let current = WorkObjectProposedPayload {
            engagement_id: EngagementId::new("engagement:sha256:e"),
            work_object: WorkObjectProposal::Revision {
                revision: Revision {
                    id: RevisionId::new("rev:sha256:r"),
                    object_id: ObjectId::new("obj:sha256:o"),
                    git_provenance: None,
                },
                object_artifact_content_hash: "sha256:current".to_owned(),
                supersedes: vec![],
            },
        };
        let value = serde_json::to_value(&current).expect("current payload serializes");
        let passed = upcast_work_object_proposed(value, CURRENT_WORK_OBJECT_PROPOSED_VIEW)
            .expect("current value decodes");
        assert_eq!(passed, current);
    }
}
