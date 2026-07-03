use serde::{Deserialize, Serialize};

use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::crypto::SignerId;
use crate::error::{Result, ShoreError};
use crate::model::{EventId, id_prefix};

mod artifact_removal;
mod assertion;
mod assessment;
mod association;
mod event_signature;
mod input_request;
mod kind;
mod observation;
mod payload;
mod provenance;
mod record_hash;
mod review;
mod signature;
mod source;
mod subject_id;
mod subject_reconstruction;
mod target;
mod task;
mod tbs;
mod type_code;
mod validation;
mod work_object_proposed;
mod writer;

pub use artifact_removal::ArtifactRemovedPayload;
pub use assertion::AssertionMode;
pub use assessment::{ReviewAssessment, ReviewAssessmentRecordedPayload};
pub use association::{
    RevisionCommitAssociatedPayload, RevisionCommitWithdrawnPayload, RevisionRefAssociatedPayload,
    RevisionRefWithdrawnPayload,
};
pub(crate) use association::{
    build_commit_association_id, build_commit_withdrawal_id, build_ref_association_id,
    build_ref_withdrawal_id,
};
pub use event_signature::{EventSignatureRecordedPayload, InclusionProof};
pub(crate) use input_request::decode_input_request_opened_payload;
pub use input_request::{
    InputRequestOpenedPayload, InputRequestReasonCode, InputRequestRespondedPayload,
    InputRequestResponseOutcome,
};
pub use kind::EventType;
pub use observation::ReviewObservationRecordedPayload;
pub use payload::{BodyContentType, EventPayload};
pub(crate) use provenance::stamp_ingest_provenance;
pub use provenance::{IngestProvenance, IngestVia};
pub use record_hash::EventRecordView;
pub use review::{
    ImportedNoteTarget, ReviewInitializedPayload, ReviewNoteImportedPayload, SidecarSource,
};
pub use signature::{EffectiveSignerError, EventSignature, resolve_effective_signer};
pub use source::SourceRef;
pub(crate) use subject_id::{review_subject_id, subject_id};
pub use target::EventTarget;
pub use task::{SourceSpeaker, TaskCheckpointCapturedPayload, TaskObservationRecordedPayload};
pub use tbs::{
    EVENT_TO_BE_SIGNED_V1_PAYLOAD_TYPE, EventToBeSigned,
    event_signature_pre_authentication_encoding, event_to_be_signed, pre_authentication_encoding,
};
pub(crate) use type_code::type_code;
pub use validation::ValidationCheckRecordedPayload;
pub use work_object_proposed::{
    GitProvenance, Revision, WorkObjectProposal, WorkObjectProposedPayload,
};
pub use writer::{Writer, WriterProducer};

const EVENT_SCHEMA: &str = "shore.event";
const EVENT_VERSION: u32 = 1;

fn default_assertion_mode(event_type: EventType) -> AssertionMode {
    match event_type {
        EventType::ReviewAssessmentRecorded => AssertionMode::Operative,
        _ => AssertionMode::Advisory,
    }
}

fn default_payload_version() -> u32 {
    1
}

fn is_default_payload_version(version: &u32) -> bool {
    *version == default_payload_version()
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShoreEvent {
    pub schema: String,
    pub version: u32,
    pub event_id: EventId,
    #[serde(with = "type_code::serde_code")]
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
    pub signer: Option<SignerId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<EventSignature>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<SourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingest: Option<IngestProvenance>,
    /// Ordered content-coding tokens (compression/encryption) applied to the
    /// stored record in list order at write and reversed on read; default `[]`
    /// is the identity encoding. Hash-excluded storage metadata: identity is
    /// computed over the decoded content, never the stored encoded bytes.
    /// Reserved — no codec populates it yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_encoding: Vec<String>,
    /// The decoded payload's view version — the key a read-time view upcast
    /// dispatches on. Hash-excluded, so a bump is signature-neutral. Reserved
    /// at its default of `1`.
    #[serde(
        default = "default_payload_version",
        skip_serializing_if = "is_default_payload_version"
    )]
    pub payload_version: u32,
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
            "{}:sha256:{}",
            id_prefix::EVENT,
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
            signer: None,
            signature: None,
            source_ref: None,
            ingest: None,
            content_encoding: Vec::new(),
            payload_version: default_payload_version(),
            payload,
        })
    }

    pub fn with_assertion_mode(mut self, mode: AssertionMode) -> Self {
        self.assertion_mode = mode;
        self
    }

    /// Computes the signature- and hop-exclusive `eventRecordHash` (ADR-0008
    /// §Event-Set Root): the stored record excluding `signer`, `signature`,
    /// `sourceRef`, `ingest`, `contentEncoding`, and `payloadVersion`. It is
    /// the content-identity the detached co-signature carrier binds as
    /// `targetEventRecordHash`.
    pub fn event_record_hash(&self) -> Result<String> {
        record_hash::EventRecordView::from_event(self).event_record_hash()
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
        AssessmentId, EngagementId, InputRequestId, InputRequestResponseId, JournalId, ObjectId,
        ObservationId, ReviewEndpoint, ReviewTargetRef, RevisionId, RevisionSource, Side,
        TargetRef, TrackId, WorktreeCaptureMode,
    };

    #[test]
    fn event_envelope_serializes_with_required_idempotency_key_and_payload_hash() {
        let event = valid_revision_captured_event();

        let json = serde_json::to_value(&event).expect("event serializes");

        assert_eq!(json["schema"], "shore.event");
        assert_eq!(json["version"], 1);
        assert_eq!(json["eventType"], "t:02");
        assert_eq!(
            json["idempotencyKey"],
            "work_object_proposed:review-unit:sha256:abc"
        );
        assert!(json["eventId"].as_str().unwrap().starts_with("evt:sha256:"));
        assert!(json["payloadHash"].as_str().unwrap().starts_with("sha256:"));
    }

    #[test]
    fn event_envelope_rejects_empty_idempotency_key() {
        let error = ShoreEvent::new(
            EventType::ReviewInitialized,
            "",
            EventTarget::for_journal(JournalId::new("journal:default")),
            Writer::shore_local("0.1.0"),
            ReviewInitializedPayload {},
            FixedClock::at("2026-05-09T20:42:45Z"),
        )
        .expect_err("empty idempotency key is invalid");

        assert!(error.to_string().contains("idempotency"));
    }

    #[test]
    fn event_envelope_rejects_empty_idempotency_key_on_decode() {
        let mut json = serde_json::to_value(valid_revision_captured_event()).unwrap();
        json["idempotencyKey"] = json!("");

        let error = serde_json::from_value::<ShoreEvent>(json)
            .expect_err("empty idempotency key cannot decode");

        assert!(error.to_string().contains("idempotencyKey"));
    }

    #[test]
    fn event_id_is_deterministic_from_idempotency_key() {
        let first = valid_revision_captured_event();
        let second = revision_captured_event(
            "sha256:different-artifact",
            "work_object_proposed:review-unit:sha256:abc",
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
    fn writer_shore_local_stamps_shore_producer() {
        let writer = Writer::shore_local("0.0.1");

        assert_eq!(writer.actor_id.as_str(), "actor:local");
        assert_eq!(writer.producer.name, "shore");
        assert_eq!(writer.producer.version, "0.0.1");
    }

    #[test]
    fn event_envelope_allows_unknown_optional_fields_for_same_version() {
        let mut json = serde_json::to_value(valid_revision_captured_event()).unwrap();
        json["futureOptionalField"] = json!("kept-compatible");

        let event: ShoreEvent =
            serde_json::from_value(json).expect("unknown optional field is ignored");

        assert_eq!(event.version, 1);
    }

    #[test]
    fn event_envelope_round_trips_through_serde() {
        let event = valid_revision_captured_event();

        let json = serde_json::to_string(&event).expect("event serializes");
        let decoded: ShoreEvent = serde_json::from_str(&json).expect("event deserializes");

        assert_eq!(decoded, event);
    }

    #[test]
    fn stored_envelope_binds_opaque_type_code_not_snake_case() {
        let event = valid_revision_captured_event();
        let json = serde_json::to_value(&event).expect("event serializes");

        // The stored envelope carries the frozen opaque code, not the renamable
        // display name, so a future rename of the family is projection-only.
        assert_eq!(json["eventType"], "t:02");
        assert_ne!(json["eventType"], "work_object_proposed");

        let decoded: ShoreEvent = serde_json::from_value(json).expect("event deserializes");
        assert_eq!(decoded.event_type, EventType::WorkObjectProposed);
    }

    #[test]
    fn stored_envelope_rejects_snake_case_event_type_on_decode() {
        let mut json = serde_json::to_value(valid_revision_captured_event()).unwrap();
        json["eventType"] = json!("work_object_proposed");

        let error = serde_json::from_value::<ShoreEvent>(json)
            .expect_err("a snake_case eventType no longer decodes as the opaque envelope");

        assert!(
            error.to_string().contains("unknown event type code"),
            "got {error}"
        );
    }

    #[test]
    fn revision_captured_event_serializes_target_and_payload() {
        let target = EventTarget::for_revision(
            JournalId::new("journal:default"),
            RevisionId::new("review-unit:sha256:abc"),
            None,
        )
        .unwrap();
        let payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new(format!(
                "engagement:sha256:{}",
                crate::canonical_hash::sha256_bytes_hex(
                    (RevisionId::new("rev:git:sha256:def")).as_str().as_bytes()
                )
            )),
            work_object: WorkObjectProposal::Revision {
                revision: Revision {
                    id: RevisionId::new("rev:git:sha256:def"),
                    object_id: ObjectId::new("snap:git:sha256:ghi"),
                    git_provenance: Some(GitProvenance {
                        source: RevisionSource::GitWorktree {
                            mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                            include_untracked: true,
                            pathspecs: Vec::new(),
                        },
                        base: ReviewEndpoint::GitCommit {
                            commit_oid: "abc".to_owned(),
                            tree_oid: "def".to_owned(),
                        },
                        target: ReviewEndpoint::GitWorkingTree {
                            worktree_root: "/repo".to_owned(),
                        },
                    }),
                },
                object_artifact_content_hash: "sha256:artifact".to_owned(),
                supersedes: vec![],
            },
        };

        let event = ShoreEvent::new(
            EventType::WorkObjectProposed,
            "work_object_proposed:review-unit:sha256:abc",
            target,
            Writer::shore_local("test"),
            payload,
            FixedClock::at("2026-05-12T00:00:00Z"),
        )
        .unwrap();

        let json = serde_json::to_value(event).unwrap();
        assert_eq!(json["eventType"], "work_object_proposed");
        assert_eq!(json["target"]["journalId"], "journal:default");
        assert_eq!(json["target"]["subject"]["review"]["kind"], "revision");
        assert_eq!(
            json["target"]["subject"]["review"]["revisionId"],
            "review-unit:sha256:abc"
        );
        assert!(json["target"].get("trackId").is_none());
        assert!(json["target"].get("workUnitId").is_none());
        assert!(json["target"].get("snapshotId").is_none());
        assert!(json["target"].get("reviewUnitId").is_none());
        // The revision's content object id and git provenance ride the payload,
        // not the envelope.
        assert_eq!(
            json["payload"]["workObject"]["revision"]["id"],
            "rev:git:sha256:def"
        );
        assert_eq!(
            json["payload"]["workObject"]["revision"]["objectId"],
            "snap:git:sha256:ghi"
        );
        assert_eq!(
            json["payload"]["workObject"]["revision"]["gitProvenance"]["base"]["commitOid"],
            "abc"
        );
        assert_eq!(
            json["payload"]["workObject"]["revision"]["gitProvenance"]["target"]["worktreeRoot"],
            "/repo"
        );
        assert_eq!(
            json["payload"]["workObject"]["objectArtifactContentHash"],
            "sha256:artifact"
        );
    }

    #[test]
    fn revision_captured_payload_hash_changes_with_artifact_binding() {
        let first = revision_captured_event_with_artifact_hash("sha256:first");
        let second = revision_captured_event_with_artifact_hash("sha256:second");

        assert_ne!(first.payload_hash, second.payload_hash);
    }

    #[test]
    fn input_request_event_types_serialize_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&EventType::InputRequestOpened).unwrap(),
            "\"input_request_opened\""
        );
        assert_eq!(
            serde_json::to_string(&EventType::InputRequestResponded).unwrap(),
            "\"input_request_responded\""
        );
    }

    #[test]
    fn input_request_opened_payload_round_trips_and_has_stable_key() {
        let payload = InputRequestOpenedPayload {
            input_request_id: InputRequestId::new("input-request:sha256:abc"),
            target: ReviewTargetRef::Revision {
                revision_id: RevisionId::new("review-unit:sha256:unit"),
            },
            task_target: None,
            reason_code: InputRequestReasonCode::ManualDecisionRequired,
            title: "Need a decision".to_owned(),
            body: Some("Which path should win?".to_owned()),
            body_content_type: Default::default(),
            body_artifact_path: None,
            body_byte_size: Some(22),
            body_content_hash: Some("sha256:body".to_owned()),
            target_fingerprint: None,
        };

        let json = serde_json::to_value(&payload).unwrap();
        let round: InputRequestOpenedPayload = serde_json::from_value(json).unwrap();

        assert_eq!(round, payload);
        assert_eq!(
            InputRequestOpenedPayload::idempotency_key(
                &RevisionId::new("review-unit:sha256:unit"),
                &TrackId::new("agent:codex"),
                "input-request:sha256:abc"
            ),
            format!(
                "{}:review-unit:sha256:unit:agent:codex:input-request:sha256:abc",
                type_code(EventType::InputRequestOpened)
            )
        );
    }

    #[test]
    fn input_request_responded_payload_round_trips_and_has_stable_key() {
        let payload = InputRequestRespondedPayload {
            input_request_response_id: InputRequestResponseId::new(
                "input-request-response:sha256:def",
            ),
            input_request_id: InputRequestId::new("input-request:sha256:abc"),
            revision_id: None,
            task_target: None,
            outcome: InputRequestResponseOutcome::Approved,
            reason: Some("Approved locally".to_owned()),
            reason_content_type: Default::default(),
            reason_artifact_path: None,
            reason_byte_size: Some(16),
            reason_content_hash: Some("sha256:reason".to_owned()),
            target_fingerprint: None,
        };

        let json = serde_json::to_value(&payload).unwrap();
        let round: InputRequestRespondedPayload = serde_json::from_value(json).unwrap();

        assert_eq!(round.outcome, InputRequestResponseOutcome::Approved);
        assert_eq!(
            InputRequestRespondedPayload::idempotency_key(
                &InputRequestId::new("input-request:sha256:abc"),
                "input-request-response:sha256:def"
            ),
            format!(
                "{}:input-request:sha256:abc:input-request-response:sha256:def",
                type_code(EventType::InputRequestResponded)
            )
        );
    }

    #[test]
    fn input_request_opened_payload_uses_expected_wire_keys() {
        let payload = InputRequestOpenedPayload {
            input_request_id: InputRequestId::new("input-request:sha256:abc"),
            target: ReviewTargetRef::Revision {
                revision_id: RevisionId::new("review-unit:sha256:unit"),
            },
            task_target: None,
            reason_code: InputRequestReasonCode::ManualDecisionRequired,
            title: "Need a decision".to_owned(),
            body: Some("Which path should win?".to_owned()),
            body_content_type: Default::default(),
            body_artifact_path: None,
            body_byte_size: Some(22),
            body_content_hash: Some("sha256:body".to_owned()),
            target_fingerprint: None,
        };

        let json = serde_json::to_value(&payload).unwrap();

        assert_eq!(json["inputRequestId"], "input-request:sha256:abc");
        assert_eq!(json["target"]["kind"], "revision");
        assert!(json.get("mode").is_none());
        assert_eq!(json["reasonCode"], "manual_decision_required");
        assert!(json.get("interventionId").is_none());
    }

    fn revision_captured_event_with_artifact_hash(
        object_artifact_content_hash: &str,
    ) -> ShoreEvent {
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:review-unit:sha256:abc:{object_artifact_content_hash}"),
            EventTarget::for_revision(
                JournalId::new("journal:default"),
                RevisionId::new("review-unit:sha256:abc"),
                None,
            )
            .unwrap(),
            Writer::shore_local("test"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new(format!(
                    "engagement:sha256:{}",
                    crate::canonical_hash::sha256_bytes_hex(
                        (RevisionId::new("rev:git:sha256:def")).as_str().as_bytes()
                    )
                )),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: RevisionId::new("rev:git:sha256:def"),
                        object_id: ObjectId::new("snap:git:sha256:ghi"),
                        git_provenance: Some(GitProvenance {
                            source: RevisionSource::GitWorktree {
                                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                                include_untracked: true,
                                pathspecs: Vec::new(),
                            },
                            base: ReviewEndpoint::GitCommit {
                                commit_oid: "abc".to_owned(),
                                tree_oid: "def".to_owned(),
                            },
                            target: ReviewEndpoint::GitWorkingTree {
                                worktree_root: "/repo".to_owned(),
                            },
                        }),
                    },
                    object_artifact_content_hash: object_artifact_content_hash.to_owned(),
                    supersedes: vec![],
                },
            },
            FixedClock::at("2026-05-12T00:00:00Z"),
        )
        .expect("review unit captured event builds")
    }

    #[test]
    fn review_observation_recorded_event_serializes_target_track_and_payload() {
        let revision_id = RevisionId::new("review-unit:sha256:abc");
        let target_ref = ReviewTargetRef::Range {
            revision_id: revision_id.clone(),
            file_path: "src/lib.rs".to_owned(),
            side: Side::New,
            start_line: 4,
            end_line: 5,
        };
        let target = EventTarget::for_subject(
            JournalId::new("journal:default"),
            TargetRef::Review(target_ref.clone()),
            Some(TrackId::new("agent:codex")),
        )
        .unwrap();

        let event = ShoreEvent::new(
            EventType::ReviewObservationRecorded,
            "review_observation_recorded:review-unit:sha256:abc:agent:codex:obs:sha256:one",
            target,
            Writer::shore_local("test"),
            ReviewObservationRecordedPayload {
                observation_id: ObservationId::new("obs:sha256:one"),
                target: target_ref,
                title: "Check this branch".to_owned(),
                body: Some("Body".to_owned()),
                body_content_type: Default::default(),
                body_artifact_path: None,
                body_byte_size: Some(4),
                body_content_hash: Some("sha256:body".to_owned()),
                tags: vec!["correctness".to_owned()],
                confidence: Some("high".to_owned()),
                supersedes_observation_ids: vec![],
                responds_to_observation_ids: vec![],
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
        let revision_id = RevisionId::new("review-unit:sha256:abc");
        let target_ref = ReviewTargetRef::Revision {
            revision_id: revision_id.clone(),
        };
        ShoreEvent::new(
            EventType::ReviewObservationRecorded,
            format!(
                "review_observation_recorded:{}:agent:codex:obs:sha256:abc",
                revision_id.as_str()
            ),
            EventTarget::for_subject(
                JournalId::new("journal:default"),
                TargetRef::Review(target_ref.clone()),
                Some(TrackId::new("agent:codex")),
            )
            .unwrap(),
            Writer::shore_local("test"),
            ReviewObservationRecordedPayload {
                observation_id: ObservationId::new("obs:sha256:abc"),
                target: target_ref,
                title: "Title".to_owned(),
                body: None,
                body_content_type: Default::default(),
                body_artifact_path: Some("artifacts/notes/body.json".to_owned()),
                body_byte_size: Some(4097),
                body_content_hash: Some(body_content_hash.to_owned()),
                tags: vec![],
                confidence: None,
                supersedes_observation_ids: vec![],
                responds_to_observation_ids: vec![],
            },
            "2026-05-12T00:00:00Z",
        )
        .unwrap()
    }

    fn valid_review_assessment_recorded_event() -> ShoreEvent {
        let revision_id = RevisionId::new("review-unit:sha256:assessment");
        let track_id = TrackId::new("human:kevin");
        let assessment_id = AssessmentId::new("assess:sha256:one");
        let target_ref = ReviewTargetRef::Revision {
            revision_id: revision_id.clone(),
        };

        ShoreEvent::new(
            EventType::ReviewAssessmentRecorded,
            ReviewAssessmentRecordedPayload::idempotency_key(
                &revision_id,
                &track_id,
                assessment_id.as_str(),
            ),
            EventTarget::for_subject(
                JournalId::new("journal:default"),
                TargetRef::Review(target_ref.clone()),
                Some(track_id),
            )
            .unwrap(),
            Writer::shore_local("test"),
            ReviewAssessmentRecordedPayload {
                assessment_id,
                target: target_ref,
                assessment: ReviewAssessment::Accepted,
                summary: Some("Ship it".to_owned()),
                summary_content_type: Default::default(),
                summary_artifact_path: None,
                summary_byte_size: Some(7),
                summary_content_hash: Some("sha256:summary".to_owned()),
                replaces_assessment_ids: vec![],
                related_observation_ids: vec![],
                related_input_request_ids: vec![],
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
            EventTarget::for_journal(JournalId::new("journal:default")),
            Writer::shore_local("0.1.0"),
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
            EventTarget::for_journal(JournalId::new("journal:default")),
            Writer::shore_local("0.1.0"),
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
        let mut event = valid_revision_captured_event();
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
        let mut json = serde_json::to_value(valid_revision_captured_event()).unwrap();
        json.as_object_mut().unwrap().remove("assertionMode");

        let event: ShoreEvent =
            serde_json::from_value(json).expect("missing assertionMode is accepted");

        assert_eq!(event.assertion_mode, AssertionMode::Advisory);
    }

    #[test]
    fn event_envelope_skip_serializes_default_advisory_assertion_mode() {
        let event = valid_revision_captured_event();
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
        let mut event = valid_revision_captured_event();
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
        let mut json = serde_json::to_value(valid_revision_captured_event()).unwrap();
        json.as_object_mut().unwrap().remove("sourceRef");

        let event: ShoreEvent =
            serde_json::from_value(json).expect("missing sourceRef is accepted");

        assert!(event.source_ref.is_none());
    }

    #[test]
    fn event_envelope_skip_serializes_absent_source_ref() {
        let event = valid_revision_captured_event();
        assert!(event.source_ref.is_none());

        let json = serde_json::to_value(&event).unwrap();

        assert!(
            json.get("sourceRef").is_none(),
            "default None must skip-serialize, got {}",
            json
        );
    }

    #[test]
    fn unsigned_event_serialization_omits_signature_fields() {
        let event = valid_revision_captured_event();

        let json = serde_json::to_value(&event).unwrap();

        assert!(json.get("signer").is_none());
        assert!(json.get("signature").is_none());
        assert!(json.get("signatures").is_none());
    }

    #[test]
    fn signed_event_round_trips_top_level_signer_and_signature() {
        const FRIENDLY_SIGNER: &str = "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd";
        const FIXTURE_SIG: &str = "EzOVlqmX/g3nHametOmU067NsuvweZEwo73/cOypvT2KfCtNK6BfxsWJQ7Ox9E/MtunGEkJGEMSfn/qdmKSFAg==";

        let mut event = valid_revision_captured_event();
        event.signer = Some(crate::crypto::SignerId::parse(FRIENDLY_SIGNER).unwrap());
        event.signature = Some(EventSignature::new_ed25519_v1(FIXTURE_SIG).unwrap());

        let json = serde_json::to_value(&event).unwrap();

        assert_eq!(json["signer"], FRIENDLY_SIGNER);
        assert_eq!(json["signature"]["alg"], "ed25519");
        assert_eq!(json["signature"]["sigVersion"], 1);
        assert_eq!(json["signature"]["sig"], FIXTURE_SIG);
        assert!(json["signature"].get("publicKey").is_none());
        assert!(json["signature"].get("keyId").is_none());
        assert!(json.get("signatures").is_none());

        let round: ShoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(round.signer, event.signer);
        assert_eq!(round.signature, event.signature);
    }

    #[test]
    fn event_envelope_serializes_source_ref_with_system_and_id() {
        let mut event = valid_revision_captured_event();
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
        let mut event = valid_revision_captured_event();
        event.source_ref = Some(SourceRef::new("claude_code", "tool_result:7"));

        let json = serde_json::to_value(&event).unwrap();

        let source_ref = json["sourceRef"].as_object().expect("sourceRef is object");
        let keys: Vec<&str> = source_ref.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["sourceId", "sourceSystem"]);

        // Writer.producer keeps its identity at the envelope level.
        assert_eq!(json["writer"]["producer"]["name"], "shore");
    }

    #[test]
    fn event_envelope_round_trips_ingest_provenance() {
        let mut event = valid_revision_captured_event();
        event.ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "unix-ms:1760000000000".to_owned(),
        });

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["ingest"]["via"], "ingest-events");
        assert_eq!(json["ingest"]["receivedAt"], "unix-ms:1760000000000");

        let round: ShoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(round.ingest, event.ingest);
    }

    #[test]
    fn event_envelope_skip_serializes_absent_ingest_and_defaults_missing_to_none() {
        let event = valid_revision_captured_event();
        assert!(event.ingest.is_none());
        let mut json = serde_json::to_value(&event).unwrap();
        assert!(
            json.get("ingest").is_none(),
            "absent ingest must skip-serialize, got {json}"
        );

        json.as_object_mut().unwrap().remove("ingest"); // no-op; pins decode default
        let round: ShoreEvent = serde_json::from_value(json).unwrap();
        assert!(round.ingest.is_none());
    }

    #[test]
    fn ingest_via_vocabulary_is_bounded() {
        assert_eq!(
            serde_json::to_string(&IngestVia::IngestEvents).unwrap(),
            "\"ingest-events\""
        );
        assert_eq!(
            serde_json::to_string(&IngestVia::BundleApply).unwrap(),
            "\"bundle-apply\""
        );
        assert!(serde_json::from_str::<IngestVia>("\"relay-forward\"").is_err());
    }

    #[test]
    fn ingest_stamp_does_not_change_event_id_or_payload_hash() {
        let unstamped = valid_revision_captured_event();
        let mut stamped = unstamped.clone();
        stamped.ingest = Some(IngestProvenance {
            via: IngestVia::BundleApply,
            received_at: "unix-ms:1760000000000".to_owned(),
        });

        assert_eq!(stamped.event_id, unstamped.event_id);
        assert_eq!(stamped.payload_hash, unstamped.payload_hash);
        assert_eq!(stamped.idempotency_key, unstamped.idempotency_key);
    }

    #[test]
    fn content_encoding_and_payload_version_do_not_change_event_id_or_payload_hash() {
        // Like the ingest stamp, the storage-encoding descriptor and the payload
        // view version are envelope-adjacent metadata: setting them leaves the
        // event's identity (eventId, payloadHash, idempotencyKey) untouched.
        let baseline = valid_revision_captured_event();
        let mut described = baseline.clone();
        described.content_encoding = vec!["zstd".to_owned()];
        described.payload_version = 7;

        assert_eq!(described.event_id, baseline.event_id);
        assert_eq!(described.payload_hash, baseline.payload_hash);
        assert_eq!(described.idempotency_key, baseline.idempotency_key);
    }

    #[test]
    fn event_envelope_skip_serializes_default_content_encoding_and_payload_version() {
        // Defaults (identity encoding, view version 1) are skip-serialized so a
        // reserved-field envelope is byte-identical to one stored before the
        // fields existed; non-defaults round-trip through serde.
        let event = valid_revision_captured_event();
        let json = serde_json::to_value(&event).unwrap();
        assert!(json.get("contentEncoding").is_none());
        assert!(json.get("payloadVersion").is_none());

        let decoded: ShoreEvent = serde_json::from_value(json).unwrap();
        assert!(decoded.content_encoding.is_empty());
        assert_eq!(decoded.payload_version, 1);

        let mut described = valid_revision_captured_event();
        described.content_encoding = vec!["zstd".to_owned()];
        described.payload_version = 2;
        let json = serde_json::to_value(&described).unwrap();
        assert_eq!(json["contentEncoding"], serde_json::json!(["zstd"]));
        assert_eq!(json["payloadVersion"], serde_json::json!(2));
        let decoded: ShoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, described);
    }

    fn valid_revision_captured_event() -> ShoreEvent {
        revision_captured_event(
            "sha256:artifact",
            "work_object_proposed:review-unit:sha256:abc",
        )
    }

    fn revision_captured_event(
        object_artifact_content_hash: &str,
        idempotency_key: &str,
    ) -> ShoreEvent {
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            idempotency_key,
            EventTarget::for_revision(
                JournalId::new("journal:default"),
                RevisionId::new("review-unit:sha256:abc"),
                None,
            )
            .unwrap(),
            Writer::shore_local("0.1.0"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new(format!(
                    "engagement:sha256:{}",
                    crate::canonical_hash::sha256_bytes_hex(
                        (RevisionId::new("rev:git:sha256:def")).as_str().as_bytes()
                    )
                )),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: RevisionId::new("rev:git:sha256:def"),
                        object_id: ObjectId::new("snap:git:sha256:ghi"),
                        git_provenance: Some(GitProvenance {
                            source: RevisionSource::GitWorktree {
                                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                                include_untracked: true,
                                pathspecs: Vec::new(),
                            },
                            base: ReviewEndpoint::GitCommit {
                                commit_oid: "abc".to_owned(),
                                tree_oid: "def".to_owned(),
                            },
                            target: ReviewEndpoint::GitWorkingTree {
                                worktree_root: "/repo".to_owned(),
                            },
                        }),
                    },
                    object_artifact_content_hash: object_artifact_content_hash.to_owned(),
                    supersedes: vec![],
                },
            },
            FixedClock::at("2026-05-09T20:42:45Z"),
        )
        .expect("event builds")
    }
}
