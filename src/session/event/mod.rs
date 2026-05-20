use serde::{Deserialize, Serialize};

use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::error::{Result, ShoreError};
use crate::model::EventId;

mod assertion;
mod assessment;
mod intervention;
mod kind;
mod observation;
mod payload;
mod review;
mod source;
mod target;
mod task;
mod writer;

pub use assertion::AssertionMode;
pub use assessment::{ReviewAssessment, ReviewAssessmentRecordedPayload};
pub use intervention::{
    InterventionMode, InterventionReasonCode, InterventionRequestedPayload,
    InterventionResolutionOutcome, InterventionResolvedPayload,
};
pub use kind::EventType;
pub use observation::ReviewObservationRecordedPayload;
pub use payload::EventPayload;
pub use review::{
    ImportedNoteTarget, ReviewInitializedPayload, ReviewNoteImportedPayload,
    ReviewUnitCapturedPayload, SidecarSource,
};
pub use source::SourceRef;
pub use target::EventTarget;
pub use task::{
    TaskAttemptCapturedPayload, TaskCheckpointCapturedPayload, TaskObservationRecordedPayload,
};
pub use writer::{Writer, WriterRole, WriterTool};

const EVENT_SCHEMA: &str = "shore.event";
const EVENT_VERSION: u32 = 1;

fn default_assertion_mode(event_type: EventType) -> AssertionMode {
    match event_type {
        EventType::ReviewAssessmentRecorded => AssertionMode::Operative,
        _ => AssertionMode::Advisory,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShoreEvent {
    pub schema: String,
    pub version: u32,
    pub event_id: EventId,
    pub event_type: EventType,
    #[serde(deserialize_with = "payload::deserialize_non_empty_idempotency_key")]
    pub idempotency_key: String,
    pub target: EventTarget,
    pub writer: Writer,
    pub occurred_at: String,
    pub payload_hash: String,
    #[serde(default, skip_serializing_if = "assertion::is_default_advisory")]
    pub assertion_mode: AssertionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<SourceRef>,
    pub payload: serde_json::Value,
}

impl ShoreEvent {
    pub fn new<P>(
        event_type: EventType,
        idempotency_key: impl Into<String>,
        target: EventTarget,
        writer: Writer,
        payload: P,
        occurred_at: impl Into<String>,
    ) -> Result<Self>
    where
        P: EventPayload,
    {
        if event_type != payload.event_type() {
            return Err(ShoreError::InvalidEvent {
                message: format!(
                    "payload type {:?} does not match event type {:?}",
                    payload.event_type(),
                    event_type
                ),
            });
        }

        let idempotency_key = idempotency_key.into();
        if idempotency_key.trim().is_empty() {
            return Err(ShoreError::InvalidEvent {
                message: "idempotencyKey cannot be empty".to_owned(),
            });
        }

        let payload = serde_json::to_value(payload)?;
        let payload_hash = sha256_json_prefixed(&payload)?;
        let event_id = EventId::new(format!(
            "evt:sha256:{}",
            sha256_bytes_hex(idempotency_key.as_bytes())
        ));

        Ok(Self {
            schema: EVENT_SCHEMA.to_owned(),
            version: EVENT_VERSION,
            event_id,
            event_type,
            idempotency_key,
            target,
            writer,
            occurred_at: occurred_at.into(),
            payload_hash,
            assertion_mode: default_assertion_mode(event_type),
            source_ref: None,
            payload,
        })
    }

    pub fn with_assertion_mode(mut self, mode: AssertionMode) -> Self {
        self.assertion_mode = mode;
        self
    }

    pub fn validate_schema_version(&self) -> Result<()> {
        if self.schema == EVENT_SCHEMA && self.version == EVENT_VERSION {
            return Ok(());
        }

        Err(ShoreError::UnsupportedEventSchemaVersion {
            schema: self.schema.clone(),
            version: self.version,
        })
    }
}

#[cfg(test)]
struct FixedClock(String);

#[cfg(test)]
impl FixedClock {
    fn at(timestamp: impl Into<String>) -> Self {
        Self(timestamp.into())
    }
}

#[cfg(test)]
impl From<FixedClock> for String {
    fn from(clock: FixedClock) -> Self {
        clock.0
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::error::ShoreError;
    use crate::model::{
        AssessmentId, InterventionId, InterventionResolutionId, ObservationId, ReviewEndpoint,
        ReviewTargetRef, ReviewUnitId, ReviewUnitSource, RevisionId, SessionId, Side, SnapshotId,
        TargetRef, TrackId, WorkUnitId, WorktreeCaptureMode,
    };

    #[test]
    fn event_envelope_serializes_with_required_idempotency_key_and_payload_hash() {
        let event = valid_review_unit_captured_event();

        let json = serde_json::to_value(&event).expect("event serializes");

        assert_eq!(json["schema"], "shore.event");
        assert_eq!(json["version"], 1);
        assert_eq!(json["eventType"], "review_unit_captured");
        assert_eq!(
            json["idempotencyKey"],
            "review_unit_captured:review-unit:sha256:abc"
        );
        assert!(json["eventId"].as_str().unwrap().starts_with("evt:sha256:"));
        assert!(json["payloadHash"].as_str().unwrap().starts_with("sha256:"));
    }

    #[test]
    fn event_envelope_rejects_empty_idempotency_key() {
        let error = ShoreEvent::new(
            EventType::ReviewInitialized,
            "",
            EventTarget::new(
                SessionId::new("session:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local_author("0.1.0"),
            ReviewInitializedPayload {},
            FixedClock::at("2026-05-09T20:42:45Z"),
        )
        .expect_err("empty idempotency key is invalid");

        assert!(error.to_string().contains("idempotency"));
    }

    #[test]
    fn event_envelope_rejects_empty_idempotency_key_on_decode() {
        let mut json = serde_json::to_value(valid_review_unit_captured_event()).unwrap();
        json["idempotencyKey"] = json!("");

        let error = serde_json::from_value::<ShoreEvent>(json)
            .expect_err("empty idempotency key cannot decode");

        assert!(error.to_string().contains("idempotencyKey"));
    }

    #[test]
    fn event_id_is_deterministic_from_idempotency_key() {
        let first = valid_review_unit_captured_event();
        let second = review_unit_captured_event(
            "sha256:different-artifact",
            "review_unit_captured:review-unit:sha256:abc",
        );

        assert_eq!(second.event_id, first.event_id);
        assert_ne!(second.payload_hash, first.payload_hash);
    }

    #[test]
    fn payload_hash_uses_canonical_object_key_order() {
        let first: serde_json::Value =
            serde_json::from_str(r#"{"outer":{"b":2,"a":1},"items":[{"d":4,"c":3}]}"#)
                .expect("json parses");
        let second: serde_json::Value =
            serde_json::from_str(r#"{"items":[{"c":3,"d":4}],"outer":{"a":1,"b":2}}"#)
                .expect("json parses");

        assert_eq!(
            sha256_json_prefixed(&second).unwrap(),
            sha256_json_prefixed(&first).unwrap()
        );
    }

    #[test]
    fn writer_shore_local_reviewer_stamps_reviewer_role() {
        let writer = Writer::shore_local_reviewer("0.0.1");

        assert_eq!(writer.role, WriterRole::Reviewer);
        assert_eq!(writer.tool.name, "shore");
        assert_eq!(writer.tool.version, "0.0.1");
    }

    #[test]
    fn event_envelope_allows_unknown_optional_fields_for_same_version() {
        let mut json = serde_json::to_value(valid_review_unit_captured_event()).unwrap();
        json["futureOptionalField"] = json!("kept-compatible");

        let event: ShoreEvent =
            serde_json::from_value(json).expect("unknown optional field is ignored");

        assert_eq!(event.version, 1);
    }

    #[test]
    fn event_envelope_round_trips_through_serde() {
        let event = valid_review_unit_captured_event();

        let json = serde_json::to_string(&event).expect("event serializes");
        let decoded: ShoreEvent = serde_json::from_str(&json).expect("event deserializes");

        assert_eq!(decoded, event);
    }

    #[test]
    fn review_unit_captured_event_serializes_target_and_payload() {
        let target = EventTarget::for_review_unit(
            SessionId::new("session:default"),
            ReviewUnitId::new("review-unit:sha256:abc"),
            RevisionId::new("rev:git:sha256:def"),
            SnapshotId::new("snap:git:sha256:ghi"),
        );
        let payload = ReviewUnitCapturedPayload {
            review_unit_id: ReviewUnitId::new("review-unit:sha256:abc"),
            source: ReviewUnitSource::GitWorktree {
                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                include_untracked: true,
            },
            base: ReviewEndpoint::GitCommit {
                commit_oid: "abc".to_owned(),
                tree_oid: "def".to_owned(),
            },
            target: ReviewEndpoint::GitWorkingTree {
                worktree_root: "/repo".to_owned(),
            },
            revision_id: RevisionId::new("rev:git:sha256:def"),
            snapshot_id: SnapshotId::new("snap:git:sha256:ghi"),
            snapshot_artifact_content_hash: "sha256:artifact".to_owned(),
        };

        let event = ShoreEvent::new(
            EventType::ReviewUnitCaptured,
            "review_unit_captured:review-unit:sha256:abc",
            target,
            Writer::shore_local_author("test"),
            payload,
            FixedClock::at("2026-05-12T00:00:00Z"),
        )
        .unwrap();

        let json = serde_json::to_value(event).unwrap();
        assert_eq!(json["eventType"], "review_unit_captured");
        assert_eq!(json["target"]["sessionId"], "session:default");
        assert!(json["target"].get("reviewId").is_none());
        assert_eq!(json["target"]["reviewUnitId"], "review-unit:sha256:abc");
        assert_eq!(json["target"]["revisionId"], "rev:git:sha256:def");
        assert_eq!(json["target"]["snapshotId"], "snap:git:sha256:ghi");
        assert!(json["target"].get("trackId").is_none());
        assert!(json["target"].get("workUnitId").is_none());
        assert_eq!(json["payload"]["base"]["commitOid"], "abc");
        assert_eq!(json["payload"]["target"]["worktreeRoot"], "/repo");
        assert_eq!(
            json["payload"]["snapshotArtifactContentHash"],
            "sha256:artifact"
        );
    }

    #[test]
    fn review_unit_captured_payload_hash_changes_with_artifact_binding() {
        let first = review_unit_captured_event_with_artifact_hash("sha256:first");
        let second = review_unit_captured_event_with_artifact_hash("sha256:second");

        assert_ne!(first.payload_hash, second.payload_hash);
    }

    #[test]
    fn intervention_event_types_serialize_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&EventType::InterventionRequested).unwrap(),
            "\"intervention_requested\""
        );
        assert_eq!(
            serde_json::to_string(&EventType::InterventionResolved).unwrap(),
            "\"intervention_resolved\""
        );
    }

    #[test]
    fn intervention_requested_payload_round_trips_and_has_stable_key() {
        let payload = InterventionRequestedPayload {
            intervention_id: InterventionId::new("intervention:sha256:abc"),
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: ReviewUnitId::new("review-unit:sha256:unit"),
            },
            mode: InterventionMode::Blocking,
            reason_code: InterventionReasonCode::ManualDecisionRequired,
            title: "Need a decision".to_owned(),
            body: Some("Which path should win?".to_owned()),
            body_artifact_path: None,
            body_byte_size: Some(22),
            body_content_hash: Some("sha256:body".to_owned()),
            target_fingerprint: None,
        };

        let json = serde_json::to_value(&payload).unwrap();
        let round: InterventionRequestedPayload = serde_json::from_value(json).unwrap();

        assert_eq!(round.mode, InterventionMode::Blocking);
        assert_eq!(
            InterventionRequestedPayload::idempotency_key(
                &ReviewUnitId::new("review-unit:sha256:unit"),
                &TrackId::new("agent:codex"),
                "intervention:sha256:abc"
            ),
            "intervention_requested:review-unit:sha256:unit:agent:codex:intervention:sha256:abc"
        );
    }

    #[test]
    fn intervention_resolved_payload_round_trips_and_has_stable_key() {
        let payload = InterventionResolvedPayload {
            intervention_resolution_id: InterventionResolutionId::new(
                "intervention-resolution:sha256:def",
            ),
            intervention_id: InterventionId::new("intervention:sha256:abc"),
            outcome: InterventionResolutionOutcome::Approved,
            reason: Some("Approved locally".to_owned()),
            reason_artifact_path: None,
            reason_byte_size: Some(16),
            reason_content_hash: Some("sha256:reason".to_owned()),
            target_fingerprint: None,
        };

        let json = serde_json::to_value(&payload).unwrap();
        let round: InterventionResolvedPayload = serde_json::from_value(json).unwrap();

        assert_eq!(round.outcome, InterventionResolutionOutcome::Approved);
        assert_eq!(
            InterventionResolvedPayload::idempotency_key(
                &InterventionId::new("intervention:sha256:abc"),
                "intervention-resolution:sha256:def"
            ),
            "intervention_resolved:intervention:sha256:abc:intervention-resolution:sha256:def"
        );
    }

    fn review_unit_captured_event_with_artifact_hash(
        snapshot_artifact_content_hash: &str,
    ) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewUnitCaptured,
            format!("review_unit_captured:review-unit:sha256:abc:{snapshot_artifact_content_hash}"),
            EventTarget::for_review_unit(
                SessionId::new("session:default"),
                ReviewUnitId::new("review-unit:sha256:abc"),
                RevisionId::new("rev:git:sha256:def"),
                SnapshotId::new("snap:git:sha256:ghi"),
            ),
            Writer::shore_local_author("test"),
            ReviewUnitCapturedPayload {
                review_unit_id: ReviewUnitId::new("review-unit:sha256:abc"),
                source: ReviewUnitSource::GitWorktree {
                    mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                    include_untracked: true,
                },
                base: ReviewEndpoint::GitCommit {
                    commit_oid: "abc".to_owned(),
                    tree_oid: "def".to_owned(),
                },
                target: ReviewEndpoint::GitWorkingTree {
                    worktree_root: "/repo".to_owned(),
                },
                revision_id: RevisionId::new("rev:git:sha256:def"),
                snapshot_id: SnapshotId::new("snap:git:sha256:ghi"),
                snapshot_artifact_content_hash: snapshot_artifact_content_hash.to_owned(),
            },
            FixedClock::at("2026-05-12T00:00:00Z"),
        )
        .expect("review unit captured event builds")
    }

    #[test]
    fn review_observation_recorded_event_serializes_target_track_and_payload() {
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:abc");
        let target_ref = ReviewTargetRef::Range {
            review_unit_id: review_unit_id.clone(),
            file_path: "src/lib.rs".to_owned(),
            side: Side::New,
            start_line: 4,
            end_line: 5,
        };
        let target = EventTarget {
            session_id: SessionId::new("session:default"),
            work_unit_id: None,
            work_object_id: None,
            work_object_type: None,
            review_unit_id: Some(review_unit_id.clone()),
            revision_id: Some(RevisionId::new("rev:git:sha256:def")),
            snapshot_id: Some(SnapshotId::new("snap:git:sha256:ghi")),
            track_id: Some(TrackId::new("agent:codex")),
            subject: Some(TargetRef::Review(target_ref.clone())),
        };

        let event = ShoreEvent::new(
            EventType::ReviewObservationRecorded,
            "review_observation_recorded:review-unit:sha256:abc:agent:codex:obs:sha256:one",
            target,
            Writer::shore_local_reviewer("test"),
            ReviewObservationRecordedPayload {
                observation_id: ObservationId::new("obs:sha256:one"),
                target: target_ref,
                title: "Check this branch".to_owned(),
                body: Some("Body".to_owned()),
                body_artifact_path: None,
                body_byte_size: Some(4),
                body_content_hash: Some("sha256:body".to_owned()),
                tags: vec!["correctness".to_owned()],
                confidence: Some("high".to_owned()),
                supersedes_observation_ids: vec![],
            },
            "2026-05-12T00:00:00Z",
        )
        .unwrap();

        let json = serde_json::to_value(event).unwrap();

        assert_eq!(json["eventType"], "review_observation_recorded");
        assert_eq!(json["target"]["trackId"], "agent:codex");
        assert_eq!(json["target"]["subject"]["review"]["kind"], "range");
        assert_eq!(json["payload"]["observationId"], "obs:sha256:one");
        assert_eq!(json["payload"]["target"]["kind"], "range");
        assert_eq!(json["payload"]["bodyContentHash"], "sha256:body");
        assert!(json["target"].get("workUnitId").is_none());
    }

    #[test]
    fn review_observation_recorded_payload_hash_changes_with_body_binding() {
        let first = review_observation_recorded_event_with_body_hash("sha256:first");
        let second = review_observation_recorded_event_with_body_hash("sha256:second");

        assert_ne!(first.payload_hash, second.payload_hash);
    }

    fn review_observation_recorded_event_with_body_hash(body_content_hash: &str) -> ShoreEvent {
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:abc");
        let target_ref = ReviewTargetRef::ReviewUnit {
            review_unit_id: review_unit_id.clone(),
        };
        ShoreEvent::new(
            EventType::ReviewObservationRecorded,
            format!(
                "review_observation_recorded:{}:agent:codex:obs:sha256:abc",
                review_unit_id.as_str()
            ),
            EventTarget {
                session_id: SessionId::new("session:default"),
                work_unit_id: None,
                work_object_id: None,
                work_object_type: None,
                review_unit_id: Some(review_unit_id.clone()),
                revision_id: Some(RevisionId::new("rev:git:sha256:def")),
                snapshot_id: Some(SnapshotId::new("snap:git:sha256:ghi")),
                track_id: Some(TrackId::new("agent:codex")),
                subject: Some(TargetRef::Review(target_ref.clone())),
            },
            Writer::shore_local_reviewer("test"),
            ReviewObservationRecordedPayload {
                observation_id: ObservationId::new("obs:sha256:abc"),
                target: target_ref,
                title: "Title".to_owned(),
                body: None,
                body_artifact_path: Some("artifacts/notes/body.json".to_owned()),
                body_byte_size: Some(4097),
                body_content_hash: Some(body_content_hash.to_owned()),
                tags: vec![],
                confidence: None,
                supersedes_observation_ids: vec![],
            },
            "2026-05-12T00:00:00Z",
        )
        .unwrap()
    }

    fn valid_review_assessment_recorded_event() -> ShoreEvent {
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:assessment");
        let track_id = TrackId::new("human:kevin");
        let assessment_id = AssessmentId::new("assess:sha256:one");
        let target_ref = ReviewTargetRef::ReviewUnit {
            review_unit_id: review_unit_id.clone(),
        };

        ShoreEvent::new(
            EventType::ReviewAssessmentRecorded,
            ReviewAssessmentRecordedPayload::idempotency_key(
                &review_unit_id,
                &track_id,
                assessment_id.as_str(),
            ),
            EventTarget {
                session_id: SessionId::new("session:default"),
                work_unit_id: None,
                work_object_id: None,
                work_object_type: None,
                review_unit_id: Some(review_unit_id.clone()),
                revision_id: Some(RevisionId::new("rev:git:sha256:one")),
                snapshot_id: Some(SnapshotId::new("snap:git:sha256:one")),
                track_id: Some(track_id),
                subject: Some(TargetRef::Review(target_ref.clone())),
            },
            Writer::shore_local_reviewer("test"),
            ReviewAssessmentRecordedPayload {
                assessment_id,
                target: target_ref,
                assessment: ReviewAssessment::Accepted,
                summary: Some("Ship it".to_owned()),
                summary_artifact_path: None,
                summary_byte_size: Some(7),
                summary_content_hash: Some("sha256:summary".to_owned()),
                replaces_assessment_ids: vec![],
                related_observation_ids: vec![],
                related_intervention_ids: vec![],
            },
            FixedClock::at("2026-05-12T00:00:00Z"),
        )
        .unwrap()
    }

    #[test]
    fn review_note_imported_event_serializes_with_payload_hash() {
        let event = ShoreEvent::new(
            EventType::ReviewNoteImported,
            "review_note_imported:review_notes:work:default:note:abc",
            EventTarget::new(
                SessionId::new("session:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local_author("0.1.0"),
            ReviewNoteImportedPayload {
                sidecar_source: SidecarSource::ReviewNotes,
                note_id: "note:abc".to_owned(),
                file_path: "src/lib.rs".to_owned(),
                file_old_path: None,
                target: Some(ImportedNoteTarget {
                    side: Side::New,
                    start_line: 1,
                    end_line: 1,
                }),
                title: "Changed return value".to_owned(),
                body: Some("Body".to_owned()),
                body_artifact_path: None,
                body_byte_size: None,
                tags: vec!["parser".to_owned()],
                confidence: Some("high".to_owned()),
                external_source: Some("external".to_owned()),
                author: Some("reviewer".to_owned()),
                created_at: Some("2026-05-10T00:00:00Z".to_owned()),
                sidecar_content_hash: "sha256:sidecar".to_owned(),
            },
            FixedClock::at("2026-05-10T00:00:00Z"),
        )
        .expect("event builds");

        let json = serde_json::to_value(&event).expect("event serializes");

        assert_eq!(json["eventType"], "review_note_imported");
        assert_eq!(json["payload"]["noteId"], "note:abc");
        assert!(json["payloadHash"].as_str().unwrap().starts_with("sha256:"));
    }

    #[test]
    fn review_note_imported_event_round_trips_through_serde() {
        let event = ShoreEvent::new(
            EventType::ReviewNoteImported,
            "review_note_imported:review_notes:work:default:note:abc",
            EventTarget::new(
                SessionId::new("session:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local_author("0.1.0"),
            ReviewNoteImportedPayload {
                sidecar_source: SidecarSource::ReviewNotes,
                note_id: "note:abc".to_owned(),
                file_path: "src/lib.rs".to_owned(),
                file_old_path: None,
                target: Some(ImportedNoteTarget {
                    side: Side::New,
                    start_line: 1,
                    end_line: 3,
                }),
                title: "Changed return value".to_owned(),
                body: Some("Body".to_owned()),
                body_artifact_path: None,
                body_byte_size: None,
                tags: vec!["parser".to_owned()],
                confidence: Some("high".to_owned()),
                external_source: Some("external".to_owned()),
                author: Some("reviewer".to_owned()),
                created_at: Some("2026-05-10T00:00:00Z".to_owned()),
                sidecar_content_hash: "sha256:sidecar".to_owned(),
            },
            FixedClock::at("2026-05-10T00:00:00Z"),
        )
        .expect("event builds");

        let json = serde_json::to_string(&event).expect("event serializes");
        let decoded: ShoreEvent = serde_json::from_str(&json).expect("event deserializes");

        assert_eq!(decoded, event);
    }

    #[test]
    fn event_envelope_has_typed_unsupported_schema_version_validation() {
        let mut event = valid_review_unit_captured_event();
        event.schema = "shore.event".to_owned();
        event.version = 2;

        let error = event
            .validate_schema_version()
            .expect_err("version 2 is unsupported");

        assert!(matches!(
            error,
            ShoreError::UnsupportedEventSchemaVersion { .. }
        ));
    }

    #[test]
    fn event_envelope_defaults_missing_assertion_mode_to_advisory() {
        let mut json = serde_json::to_value(valid_review_unit_captured_event()).unwrap();
        json.as_object_mut().unwrap().remove("assertionMode");

        let event: ShoreEvent =
            serde_json::from_value(json).expect("missing assertionMode is accepted");

        assert_eq!(event.assertion_mode, AssertionMode::Advisory);
    }

    #[test]
    fn event_envelope_skip_serializes_default_advisory_assertion_mode() {
        let event = valid_review_unit_captured_event();
        assert_eq!(event.assertion_mode, AssertionMode::Advisory);

        let json = serde_json::to_value(&event).unwrap();

        assert!(
            json.get("assertionMode").is_none(),
            "default Advisory must skip-serialize, got {}",
            json
        );
    }

    #[test]
    fn event_envelope_serializes_explicit_operative_assertion_mode() {
        let mut event = valid_review_unit_captured_event();
        event.assertion_mode = AssertionMode::Operative;

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["assertionMode"], "operative");

        let round: ShoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(round.assertion_mode, AssertionMode::Operative);
    }

    #[test]
    fn assessment_recorded_events_default_to_operative_assertion_mode() {
        let event = valid_review_assessment_recorded_event();

        assert_eq!(event.assertion_mode, AssertionMode::Operative);
    }

    #[test]
    fn event_envelope_defaults_missing_source_ref_to_none() {
        let mut json = serde_json::to_value(valid_review_unit_captured_event()).unwrap();
        json.as_object_mut().unwrap().remove("sourceRef");

        let event: ShoreEvent =
            serde_json::from_value(json).expect("missing sourceRef is accepted");

        assert!(event.source_ref.is_none());
    }

    #[test]
    fn event_envelope_skip_serializes_absent_source_ref() {
        let event = valid_review_unit_captured_event();
        assert!(event.source_ref.is_none());

        let json = serde_json::to_value(&event).unwrap();

        assert!(
            json.get("sourceRef").is_none(),
            "default None must skip-serialize, got {}",
            json
        );
    }

    #[test]
    fn event_envelope_serializes_source_ref_with_system_and_id() {
        let mut event = valid_review_unit_captured_event();
        event.source_ref = Some(SourceRef::new("claude_code", "session:abc/tool_result:1"));

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["sourceRef"]["sourceSystem"], "claude_code");
        assert_eq!(json["sourceRef"]["sourceId"], "session:abc/tool_result:1");

        let round: ShoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(round.source_ref, event.source_ref);
    }

    #[test]
    fn source_ref_shape_does_not_duplicate_writer_tool() {
        // Pin the OQ-G decision: actor/tool identity stays in Writer; source_ref
        // carries only source_system and source_id.
        let mut event = valid_review_unit_captured_event();
        event.source_ref = Some(SourceRef::new("claude_code", "tool_result:7"));

        let json = serde_json::to_value(&event).unwrap();

        let source_ref = json["sourceRef"].as_object().expect("sourceRef is object");
        let keys: Vec<&str> = source_ref.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["sourceId", "sourceSystem"]);

        // Writer.tool keeps its identity at the envelope level.
        assert_eq!(json["writer"]["tool"]["name"], "shore");
    }

    fn valid_review_unit_captured_event() -> ShoreEvent {
        review_unit_captured_event(
            "sha256:artifact",
            "review_unit_captured:review-unit:sha256:abc",
        )
    }

    fn review_unit_captured_event(
        snapshot_artifact_content_hash: &str,
        idempotency_key: &str,
    ) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewUnitCaptured,
            idempotency_key,
            EventTarget::for_review_unit(
                SessionId::new("session:default"),
                ReviewUnitId::new("review-unit:sha256:abc"),
                RevisionId::new("rev:git:sha256:def"),
                SnapshotId::new("snap:git:sha256:ghi"),
            ),
            Writer::shore_local_author("0.1.0"),
            ReviewUnitCapturedPayload {
                review_unit_id: ReviewUnitId::new("review-unit:sha256:abc"),
                source: ReviewUnitSource::GitWorktree {
                    mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                    include_untracked: true,
                },
                base: ReviewEndpoint::GitCommit {
                    commit_oid: "abc".to_owned(),
                    tree_oid: "def".to_owned(),
                },
                target: ReviewEndpoint::GitWorkingTree {
                    worktree_root: "/repo".to_owned(),
                },
                revision_id: RevisionId::new("rev:git:sha256:def"),
                snapshot_id: SnapshotId::new("snap:git:sha256:ghi"),
                snapshot_artifact_content_hash: snapshot_artifact_content_hash.to_owned(),
            },
            FixedClock::at("2026-05-09T20:42:45Z"),
        )
        .expect("event builds")
    }
}
