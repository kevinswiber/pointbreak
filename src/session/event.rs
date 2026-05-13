use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::error::{Result, ShoreError};
use crate::model::{
    AcknowledgementId, ActorId, DispositionId, EventId, InterventionId, InterventionResolutionId,
    ObservationId, ReviewArtifactId, ReviewEndpoint, ReviewId, ReviewTargetRef, ReviewUnitId,
    ReviewUnitSource, RevisionId, Side, SnapshotId, TrackId, WorkUnitId,
};

const EVENT_SCHEMA: &str = "shore.event";
const EVENT_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShoreEvent {
    pub schema: String,
    pub version: u32,
    pub event_id: EventId,
    pub event_type: EventType,
    #[serde(deserialize_with = "deserialize_non_empty_idempotency_key")]
    pub idempotency_key: String,
    pub target: EventTarget,
    pub writer: Writer,
    pub occurred_at: String,
    pub payload_hash: String,
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
            payload,
        })
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    ReviewInitialized,
    ReviewUnitCaptured,
    ReviewObservationRecorded,
    ReviewDispositionRecorded,
    InterventionRequested,
    InterventionResolved,
    RevisionPublished,
    SnapshotObserved,
    SidecarObserved,
    ReviewNoteImported,
    ReviewArtifactPublished,
    ReviewArtifactAcknowledged,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventTarget {
    pub review_id: ReviewId,
    /// Legacy work-unit target used by publish/verdict/ack scaffolding.
    ///
    /// New ReviewUnit-native events should target `review_unit_id` and may omit
    /// this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_unit_id: Option<WorkUnitId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_unit_id: Option<ReviewUnitId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<RevisionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<SnapshotId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<ReviewTargetRef>,
}

impl EventTarget {
    pub fn new(review_id: ReviewId, work_unit_id: WorkUnitId) -> Self {
        Self {
            review_id,
            work_unit_id: Some(work_unit_id),
            review_unit_id: None,
            revision_id: None,
            snapshot_id: None,
            track_id: None,
            subject: None,
        }
    }

    pub fn for_review_unit(
        review_id: ReviewId,
        review_unit_id: ReviewUnitId,
        revision_id: RevisionId,
        snapshot_id: SnapshotId,
    ) -> Self {
        Self {
            review_id,
            work_unit_id: None,
            review_unit_id: Some(review_unit_id.clone()),
            revision_id: Some(revision_id),
            snapshot_id: Some(snapshot_id),
            track_id: None,
            subject: Some(ReviewTargetRef::ReviewUnit { review_unit_id }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Writer {
    pub actor_id: ActorId,
    pub role: WriterRole,
    pub tool: WriterTool,
}

impl Writer {
    pub fn shore_local_author(version: impl Into<String>) -> Self {
        Self {
            actor_id: ActorId::new("actor:local"),
            role: WriterRole::Author,
            tool: WriterTool {
                name: "shore".to_owned(),
                version: version.into(),
            },
        }
    }

    pub fn shore_local_reviewer(version: impl Into<String>) -> Self {
        Self {
            actor_id: ActorId::new("actor:local"),
            role: WriterRole::Reviewer,
            tool: WriterTool {
                name: "shore".to_owned(),
                version: version.into(),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriterRole {
    Author,
    Reviewer,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterTool {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictDecision {
    Pass,
    PassMinorNit,
    RequestChanges,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcknowledgementNextAction {
    Accept,
    Address,
    Defer,
    Obsolete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDisposition {
    Accepted,
    AcceptedWithFollowUp,
    NeedsChanges,
    NeedsClarification,
    Overridden,
    Deferred,
    SplitOut,
    Superseded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionMode {
    Blocking,
    Advisory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionReasonCode {
    AmbiguousState,
    UnsafeAction,
    StaleRevision,
    FailedGate,
    ExternalSideEffect,
    ConflictingEvent,
    MissingPermission,
    ManualDecisionRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionResolutionOutcome {
    Approved,
    Rejected,
    Dismissed,
    Superseded,
    Abandoned,
}

pub trait EventPayload: Serialize {
    fn event_type(&self) -> EventType;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewInitializedPayload {}

impl ReviewInitializedPayload {
    pub fn idempotency_key(review_id: &ReviewId, work_unit_id: &WorkUnitId) -> String {
        format!(
            "review_initialized:{}:{}",
            review_id.as_str(),
            work_unit_id.as_str()
        )
    }
}

impl EventPayload for ReviewInitializedPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewInitialized
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitCapturedPayload {
    pub review_unit_id: ReviewUnitId,
    pub source: ReviewUnitSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
    pub revision_id: RevisionId,
    pub snapshot_id: SnapshotId,
    pub snapshot_artifact_content_hash: String,
}

impl EventPayload for ReviewUnitCapturedPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewUnitCaptured
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewObservationRecordedPayload {
    pub observation_id: ObservationId,
    pub target: ReviewTargetRef,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_artifact_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_byte_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes_observation_ids: Vec<ObservationId>,
}

impl ReviewObservationRecordedPayload {
    pub fn idempotency_key(
        review_unit_id: &ReviewUnitId,
        track_id: &TrackId,
        source_key: &str,
    ) -> String {
        format!(
            "review_observation_recorded:{}:{}:{}",
            review_unit_id.as_str(),
            track_id.as_str(),
            source_key
        )
    }
}

impl EventPayload for ReviewObservationRecordedPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewObservationRecorded
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDispositionRecordedPayload {
    pub disposition_id: DispositionId,
    pub target: ReviewTargetRef,
    pub disposition: ReviewDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_artifact_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_byte_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaces_disposition_ids: Vec<DispositionId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_observation_ids: Vec<ObservationId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_intervention_ids: Vec<InterventionId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<ReviewTargetRef>,
}

impl ReviewDispositionRecordedPayload {
    pub fn idempotency_key(
        review_unit_id: &ReviewUnitId,
        track_id: &TrackId,
        source_key: &str,
    ) -> String {
        format!(
            "review_disposition_recorded:{}:{}:{}",
            review_unit_id.as_str(),
            track_id.as_str(),
            source_key
        )
    }
}

impl EventPayload for ReviewDispositionRecordedPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewDispositionRecorded
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterventionRequestedPayload {
    pub intervention_id: InterventionId,
    pub target: ReviewTargetRef,
    pub mode: InterventionMode,
    pub reason_code: InterventionReasonCode,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_artifact_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_byte_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_content_hash: Option<String>,
}

impl InterventionRequestedPayload {
    pub fn idempotency_key(
        review_unit_id: &ReviewUnitId,
        track_id: &TrackId,
        source_key: &str,
    ) -> String {
        format!(
            "intervention_requested:{}:{}:{}",
            review_unit_id.as_str(),
            track_id.as_str(),
            source_key
        )
    }
}

impl EventPayload for InterventionRequestedPayload {
    fn event_type(&self) -> EventType {
        EventType::InterventionRequested
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterventionResolvedPayload {
    pub intervention_resolution_id: InterventionResolutionId,
    pub intervention_id: InterventionId,
    pub outcome: InterventionResolutionOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_artifact_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_byte_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_content_hash: Option<String>,
}

impl InterventionResolvedPayload {
    pub fn idempotency_key(intervention_id: &InterventionId, source_key: &str) -> String {
        format!(
            "intervention_resolved:{}:{}",
            intervention_id.as_str(),
            source_key
        )
    }
}

impl EventPayload for InterventionResolvedPayload {
    fn event_type(&self) -> EventType {
        EventType::InterventionResolved
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionPublishedPayload {
    pub revision_id: RevisionId,
    pub supersedes_revision_ids: Vec<RevisionId>,
}

impl EventPayload for RevisionPublishedPayload {
    fn event_type(&self) -> EventType {
        EventType::RevisionPublished
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotObservedPayload {
    pub snapshot_id: SnapshotId,
    pub revision_id: RevisionId,
}

impl EventPayload for SnapshotObservedPayload {
    fn event_type(&self) -> EventType {
        EventType::SnapshotObserved
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarObservedPayload {
    pub source: SidecarSource,
    pub path: String,
    pub byte_size: usize,
    pub content_hash: String,
    pub schema: Option<String>,
    pub imported_schema: Option<String>,
    pub version: Option<u32>,
    pub diagnostic_count: usize,
    pub diagnostic_levels: BTreeMap<String, usize>,
}

impl EventPayload for SidecarObservedPayload {
    fn event_type(&self) -> EventType {
        EventType::SidecarObserved
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidecarSource {
    ReviewNotes,
    LegacyHunkAgentContext,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewNoteImportedPayload {
    pub sidecar_source: SidecarSource,
    pub note_id: String,
    pub file_path: String,
    pub file_old_path: Option<String>,
    pub target: Option<ImportedNoteTarget>,
    pub title: String,
    pub body: Option<String>,
    pub body_artifact_path: Option<String>,
    pub body_byte_size: Option<usize>,
    pub tags: Vec<String>,
    pub confidence: Option<String>,
    pub external_source: Option<String>,
    pub author: Option<String>,
    pub created_at: Option<String>,
    pub sidecar_content_hash: String,
}

impl EventPayload for ReviewNoteImportedPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewNoteImported
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewArtifactPublishedPayload {
    pub review_artifact_id: ReviewArtifactId,
    pub work_unit_id: WorkUnitId,
    pub revision_id: RevisionId,
    pub decision: VerdictDecision,
    pub summary: Option<String>,
    pub summary_artifact_path: Option<String>,
    pub summary_byte_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaces_review_artifact_ids: Vec<ReviewArtifactId>,
    pub reviewer: Writer,
}

impl ReviewArtifactPublishedPayload {
    pub fn idempotency_key(
        work_unit_id: &WorkUnitId,
        review_artifact_id: &ReviewArtifactId,
    ) -> String {
        format!(
            "review_artifact_published:{}:{}",
            work_unit_id.as_str(),
            review_artifact_id.as_str()
        )
    }
}

impl EventPayload for ReviewArtifactPublishedPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewArtifactPublished
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewArtifactAcknowledgedPayload {
    pub acknowledgement_id: AcknowledgementId,
    pub review_artifact_id: ReviewArtifactId,
    pub next_action: AcknowledgementNextAction,
    pub reason: Option<String>,
    pub reason_artifact_path: Option<String>,
    pub reason_byte_size: Option<u64>,
    pub acknowledger: Writer,
}

impl ReviewArtifactAcknowledgedPayload {
    pub fn idempotency_key(
        review_artifact_id: &ReviewArtifactId,
        acknowledgement_id: &AcknowledgementId,
    ) -> String {
        format!(
            "review_artifact_acknowledged:{}:{}",
            review_artifact_id.as_str(),
            acknowledgement_id.as_str()
        )
    }
}

impl EventPayload for ReviewArtifactAcknowledgedPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewArtifactAcknowledged
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedNoteTarget {
    pub side: Side,
    pub start_line: u32,
    pub end_line: u32,
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

fn deserialize_non_empty_idempotency_key<'de, D>(
    deserializer: D,
) -> std::result::Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.trim().is_empty() {
        return Err(serde::de::Error::custom("idempotencyKey cannot be empty"));
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::error::ShoreError;
    use crate::model::{
        DispositionId, InterventionId, InterventionResolutionId, ReviewEndpoint, ReviewId,
        ReviewTargetRef, ReviewUnitId, ReviewUnitSource, TrackId, WorktreeCaptureMode,
    };

    #[test]
    fn event_envelope_serializes_with_required_idempotency_key_and_payload_hash() {
        let event = ShoreEvent::new(
            EventType::RevisionPublished,
            "revision_published:explicit:work:default:rev:worktree:sha256:abc",
            EventTarget::new(
                ReviewId::new("review:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local_author("0.1.0"),
            RevisionPublishedPayload {
                revision_id: RevisionId::new("rev:worktree:sha256:abc"),
                supersedes_revision_ids: vec![],
            },
            FixedClock::at("2026-05-09T20:42:45Z"),
        )
        .expect("event builds");

        let json = serde_json::to_value(&event).expect("event serializes");

        assert_eq!(json["schema"], "shore.event");
        assert_eq!(json["version"], 1);
        assert_eq!(json["eventType"], "revision_published");
        assert_eq!(
            json["idempotencyKey"],
            "revision_published:explicit:work:default:rev:worktree:sha256:abc"
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
                ReviewId::new("review:default"),
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
        let mut json = serde_json::to_value(valid_revision_published_event()).unwrap();
        json["idempotencyKey"] = json!("");

        let error = serde_json::from_value::<ShoreEvent>(json)
            .expect_err("empty idempotency key cannot decode");

        assert!(error.to_string().contains("idempotencyKey"));
    }

    #[test]
    fn event_id_is_deterministic_from_idempotency_key() {
        let first = valid_revision_published_event();
        let second = ShoreEvent::new(
            EventType::RevisionPublished,
            "revision_published:explicit:work:default:rev:worktree:sha256:abc",
            EventTarget::new(
                ReviewId::new("review:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local_author("0.1.0"),
            RevisionPublishedPayload {
                revision_id: RevisionId::new("rev:worktree:sha256:different-payload"),
                supersedes_revision_ids: vec![],
            },
            FixedClock::at("2026-05-09T21:00:00Z"),
        )
        .expect("event builds");

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
    fn verdict_decision_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&VerdictDecision::Pass).unwrap(),
            "\"pass\""
        );
        assert_eq!(
            serde_json::to_string(&VerdictDecision::PassMinorNit).unwrap(),
            "\"pass_minor_nit\""
        );
        assert_eq!(
            serde_json::to_string(&VerdictDecision::RequestChanges).unwrap(),
            "\"request_changes\""
        );
    }

    #[test]
    fn review_disposition_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ReviewDisposition::AcceptedWithFollowUp).unwrap(),
            "\"accepted_with_follow_up\""
        );
        assert_eq!(
            serde_json::to_string(&ReviewDisposition::NeedsClarification).unwrap(),
            "\"needs_clarification\""
        );
        assert_eq!(
            serde_json::to_string(&ReviewDisposition::SplitOut).unwrap(),
            "\"split_out\""
        );
    }

    #[test]
    fn acknowledgement_next_action_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&AcknowledgementNextAction::Accept).unwrap(),
            "\"accept\""
        );
        assert_eq!(
            serde_json::to_string(&AcknowledgementNextAction::Address).unwrap(),
            "\"address\""
        );
        assert_eq!(
            serde_json::to_string(&AcknowledgementNextAction::Defer).unwrap(),
            "\"defer\""
        );
        assert_eq!(
            serde_json::to_string(&AcknowledgementNextAction::Obsolete).unwrap(),
            "\"obsolete\""
        );
    }

    #[test]
    fn review_artifact_published_payload_round_trips_and_hashes_deterministically() {
        let payload = ReviewArtifactPublishedPayload {
            review_artifact_id: ReviewArtifactId::new("review-artifact:sha256:abc"),
            work_unit_id: WorkUnitId::new("wu:1"),
            revision_id: RevisionId::new("rev:sha256:r1"),
            decision: VerdictDecision::Pass,
            summary: Some("Looks good.".to_owned()),
            summary_artifact_path: None,
            summary_byte_size: Some(11),
            replaces_review_artifact_ids: vec![],
            reviewer: Writer::shore_local_reviewer("0.0.1"),
        };
        let hash_a = crate::canonical_hash::sha256_json_hex(&payload).unwrap();
        let hash_b = crate::canonical_hash::sha256_json_hex(&payload.clone()).unwrap();
        assert_eq!(hash_a, hash_b);

        let json = serde_json::to_value(&payload).unwrap();
        let round: ReviewArtifactPublishedPayload = serde_json::from_value(json).unwrap();
        assert_eq!(round.decision, VerdictDecision::Pass);
    }

    #[test]
    fn review_artifact_published_idempotency_key_is_stable() {
        let key = ReviewArtifactPublishedPayload::idempotency_key(
            &WorkUnitId::new("wu:1"),
            &ReviewArtifactId::new("review-artifact:sha256:abc"),
        );
        assert_eq!(
            key,
            "review_artifact_published:wu:1:review-artifact:sha256:abc"
        );
    }

    #[test]
    fn review_artifact_acknowledged_idempotency_key_is_stable() {
        let key = ReviewArtifactAcknowledgedPayload::idempotency_key(
            &ReviewArtifactId::new("review-artifact:sha256:abc"),
            &AcknowledgementId::new("ack:sha256:def"),
        );
        assert_eq!(
            key,
            "review_artifact_acknowledged:review-artifact:sha256:abc:ack:sha256:def"
        );
    }

    #[test]
    fn event_envelope_allows_unknown_optional_fields_for_same_version() {
        let mut json = serde_json::to_value(valid_revision_published_event()).unwrap();
        json["futureOptionalField"] = json!("kept-compatible");

        let event: ShoreEvent =
            serde_json::from_value(json).expect("unknown optional field is ignored");

        assert_eq!(event.version, 1);
    }

    #[test]
    fn event_envelope_round_trips_through_serde() {
        let event = valid_revision_published_event();

        let json = serde_json::to_string(&event).expect("event serializes");
        let decoded: ShoreEvent = serde_json::from_str(&json).expect("event deserializes");

        assert_eq!(decoded, event);
    }

    #[test]
    fn review_unit_captured_event_serializes_target_and_payload() {
        let target = EventTarget::for_review_unit(
            ReviewId::new("review:default"),
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
    fn disposition_recorded_event_serializes_target_and_payload() {
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:one");
        let track_id = TrackId::new("human:kevin");
        let disposition_id = DispositionId::new("disp:sha256:one");
        let target_ref = ReviewTargetRef::ReviewUnit {
            review_unit_id: review_unit_id.clone(),
        };

        let event = ShoreEvent::new(
            EventType::ReviewDispositionRecorded,
            ReviewDispositionRecordedPayload::idempotency_key(
                &review_unit_id,
                &track_id,
                disposition_id.as_str(),
            ),
            EventTarget {
                review_id: ReviewId::new("review:default"),
                work_unit_id: None,
                review_unit_id: Some(review_unit_id.clone()),
                revision_id: Some(RevisionId::new("rev:git:sha256:one")),
                snapshot_id: Some(SnapshotId::new("snap:git:sha256:one")),
                track_id: Some(track_id.clone()),
                subject: Some(target_ref.clone()),
            },
            Writer::shore_local_reviewer("test"),
            ReviewDispositionRecordedPayload {
                disposition_id: disposition_id.clone(),
                target: target_ref,
                disposition: ReviewDisposition::Accepted,
                summary: Some("Ship it".to_owned()),
                summary_artifact_path: None,
                summary_byte_size: Some(7),
                summary_content_hash: Some("sha256:summary".to_owned()),
                replaces_disposition_ids: vec![],
                related_observation_ids: vec![],
                related_intervention_ids: vec![],
                overrides: vec![],
            },
            "2026-05-12T00:00:00Z",
        )
        .unwrap();

        let json = serde_json::to_value(&event).unwrap();

        assert_eq!(json["eventType"], "review_disposition_recorded");
        assert_eq!(json["target"]["reviewUnitId"], "review-unit:sha256:one");
        assert_eq!(json["target"]["trackId"], "human:kevin");
        assert_eq!(json["payload"]["dispositionId"], "disp:sha256:one");
        assert_eq!(json["payload"]["disposition"], "accepted");
        assert_eq!(json["payload"]["summaryContentHash"], "sha256:summary");
        assert!(json["payload"].get("replacesDispositionIds").is_none());
    }

    #[test]
    fn disposition_recorded_rejects_payload_event_type_mismatch() {
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:one");
        let error = ShoreEvent::new(
            EventType::ReviewObservationRecorded,
            "review_disposition_recorded:review-unit:sha256:one:human:kevin:disp:sha256:one",
            EventTarget::for_review_unit(
                ReviewId::new("review:default"),
                review_unit_id.clone(),
                RevisionId::new("rev:git:sha256:one"),
                SnapshotId::new("snap:git:sha256:one"),
            ),
            Writer::shore_local_reviewer("test"),
            ReviewDispositionRecordedPayload {
                disposition_id: DispositionId::new("disp:sha256:one"),
                target: ReviewTargetRef::ReviewUnit { review_unit_id },
                disposition: ReviewDisposition::Accepted,
                summary: None,
                summary_artifact_path: None,
                summary_byte_size: None,
                summary_content_hash: None,
                replaces_disposition_ids: vec![],
                related_observation_ids: vec![],
                related_intervention_ids: vec![],
                overrides: vec![],
            },
            "2026-05-12T00:00:00Z",
        )
        .expect_err("payload mismatch rejected");

        assert!(matches!(error, ShoreError::InvalidEvent { .. }));
        assert!(error.to_string().contains("payload type"));
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
                ReviewId::new("review:default"),
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
            review_id: ReviewId::new("review:default"),
            work_unit_id: None,
            review_unit_id: Some(review_unit_id.clone()),
            revision_id: Some(RevisionId::new("rev:git:sha256:def")),
            snapshot_id: Some(SnapshotId::new("snap:git:sha256:ghi")),
            track_id: Some(TrackId::new("agent:codex")),
            subject: Some(target_ref.clone()),
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
        assert_eq!(json["target"]["subject"]["kind"], "range");
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
                review_id: ReviewId::new("review:default"),
                work_unit_id: None,
                review_unit_id: Some(review_unit_id.clone()),
                revision_id: Some(RevisionId::new("rev:git:sha256:def")),
                snapshot_id: Some(SnapshotId::new("snap:git:sha256:ghi")),
                track_id: Some(TrackId::new("agent:codex")),
                subject: Some(target_ref.clone()),
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

    #[test]
    fn review_note_imported_event_serializes_with_payload_hash() {
        let event = ShoreEvent::new(
            EventType::ReviewNoteImported,
            "review_note_imported:review_notes:work:default:note:abc",
            EventTarget::new(
                ReviewId::new("review:default"),
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
                ReviewId::new("review:default"),
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
        let mut event = valid_revision_published_event();
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

    fn valid_revision_published_event() -> ShoreEvent {
        ShoreEvent::new(
            EventType::RevisionPublished,
            "revision_published:explicit:work:default:rev:worktree:sha256:abc",
            EventTarget::new(
                ReviewId::new("review:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local_author("0.1.0"),
            RevisionPublishedPayload {
                revision_id: RevisionId::new("rev:worktree:sha256:abc"),
                supersedes_revision_ids: vec![],
            },
            FixedClock::at("2026-05-09T20:42:45Z"),
        )
        .expect("event builds")
    }
}
