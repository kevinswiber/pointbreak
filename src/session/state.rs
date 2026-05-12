use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::{Result, ShoreError};
use crate::model::{ReviewArtifactId, ReviewId, ReviewUnitId, RevisionId, SnapshotId, WorkUnitId};
use crate::session::event::{
    EventType, ReviewArtifactAcknowledgedPayload, ReviewArtifactPublishedPayload,
    ReviewUnitCapturedPayload, RevisionPublishedPayload, ShoreEvent, SnapshotObservedPayload,
    VerdictDecision,
};

const STATE_SCHEMA: &str = "shore.state";
const STATE_VERSION: u32 = 1;
pub const AMBIGUOUS_CURRENT_REVIEW_UNIT_CODE: &str = "ambiguous_current_review_unit";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub schema: String,
    pub version: u32,
    pub review_id: ReviewId,
    pub work_unit_id: WorkUnitId,
    pub current_revision_id: Option<RevisionId>,
    pub current_snapshot_id: Option<SnapshotId>,
    #[serde(default)]
    pub review_unit_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_review_unit_id: Option<ReviewUnitId>,
    pub event_count: usize,
    pub sidecar_count: usize,
    pub note_count: usize,
    #[serde(default)]
    pub review_artifact_count: usize,
    #[serde(default)]
    pub acknowledgement_count: usize,
    #[serde(default)]
    pub last_verdict_decision: Option<VerdictDecision>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

impl SessionState {
    pub fn from_events(events: &[ShoreEvent]) -> Result<Self> {
        let mut reducer = StateReducer::default();
        for event in events {
            reducer.apply(event)?;
        }
        reducer.finish(events.len())
    }

    pub fn validate_schema_version(&self) -> Result<()> {
        if self.schema == STATE_SCHEMA && self.version == STATE_VERSION {
            return Ok(());
        }

        Err(ShoreError::UnsupportedStateSchemaVersion {
            schema: self.schema.clone(),
            version: self.version,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionDiagnostic {
    pub code: String,
    pub message: String,
}

#[derive(Debug)]
struct StateReducer {
    review_id: ReviewId,
    work_unit_id: WorkUnitId,
    published_revision_ids: BTreeSet<RevisionId>,
    superseded_revision_ids: BTreeSet<RevisionId>,
    snapshots_by_revision_id: BTreeMap<RevisionId, SnapshotId>,
    captured_review_unit_ids: BTreeSet<ReviewUnitId>,
    sidecar_count: usize,
    note_count: usize,
    review_artifact_count: usize,
    acknowledgement_count: usize,
    published_artifacts: BTreeMap<ReviewArtifactId, (RevisionId, VerdictDecision)>,
    replaced_artifacts: BTreeSet<ReviewArtifactId>,
}

impl Default for StateReducer {
    fn default() -> Self {
        Self {
            review_id: ReviewId::new("review:default"),
            work_unit_id: WorkUnitId::new("work:default"),
            published_revision_ids: BTreeSet::new(),
            superseded_revision_ids: BTreeSet::new(),
            snapshots_by_revision_id: BTreeMap::new(),
            captured_review_unit_ids: BTreeSet::new(),
            sidecar_count: 0,
            note_count: 0,
            review_artifact_count: 0,
            acknowledgement_count: 0,
            published_artifacts: BTreeMap::new(),
            replaced_artifacts: BTreeSet::new(),
        }
    }
}

impl StateReducer {
    fn apply(&mut self, event: &ShoreEvent) -> Result<()> {
        event.validate_schema_version()?;

        if event.event_type == EventType::ReviewInitialized {
            self.review_id = event.target.review_id.clone();
            if let Some(work_unit_id) = &event.target.work_unit_id {
                self.work_unit_id = work_unit_id.clone();
            }
            return Ok(());
        }

        self.set_identity_from_event_if_default(event);

        match event.event_type {
            EventType::ReviewInitialized => {}
            EventType::ReviewUnitCaptured => self.apply_review_unit_captured(event)?,
            EventType::RevisionPublished => self.apply_revision_published(event)?,
            EventType::SnapshotObserved => self.apply_snapshot_observed(event)?,
            EventType::SidecarObserved => {
                self.sidecar_count += 1;
            }
            EventType::ReviewNoteImported => {
                self.note_count += 1;
            }
            EventType::ReviewArtifactPublished => {
                self.apply_review_artifact_published(event)?;
            }
            EventType::ReviewArtifactAcknowledged => {
                self.apply_review_artifact_acknowledged(event)?;
            }
        }

        Ok(())
    }

    fn set_identity_from_event_if_default(&mut self, event: &ShoreEvent) {
        if self.review_id.as_str() == "review:default" {
            self.review_id = event.target.review_id.clone();
        }
        if self.work_unit_id.as_str() == "work:default"
            && let Some(work_unit_id) = &event.target.work_unit_id
        {
            self.work_unit_id = work_unit_id.clone();
        }
    }

    fn apply_revision_published(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: RevisionPublishedPayload = serde_json::from_value(event.payload.clone())?;
        self.published_revision_ids.insert(payload.revision_id);
        for revision_id in payload.supersedes_revision_ids {
            self.superseded_revision_ids.insert(revision_id);
        }
        Ok(())
    }

    fn apply_snapshot_observed(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: SnapshotObservedPayload = serde_json::from_value(event.payload.clone())?;
        self.snapshots_by_revision_id
            .insert(payload.revision_id, payload.snapshot_id);
        Ok(())
    }

    fn apply_review_unit_captured(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: ReviewUnitCapturedPayload = serde_json::from_value(event.payload.clone())?;
        self.captured_review_unit_ids.insert(payload.review_unit_id);
        Ok(())
    }

    fn apply_review_artifact_published(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: ReviewArtifactPublishedPayload =
            serde_json::from_value(event.payload.clone())?;
        self.review_artifact_count += 1;
        for review_artifact_id in payload.replaces_review_artifact_ids {
            self.replaced_artifacts.insert(review_artifact_id);
        }
        self.published_artifacts.insert(
            payload.review_artifact_id,
            (payload.revision_id, payload.decision),
        );
        Ok(())
    }

    fn apply_review_artifact_acknowledged(&mut self, event: &ShoreEvent) -> Result<()> {
        let _: ReviewArtifactAcknowledgedPayload = serde_json::from_value(event.payload.clone())?;
        self.acknowledgement_count += 1;
        Ok(())
    }

    fn finish(self, event_count: usize) -> Result<SessionState> {
        let mut diagnostics = Vec::new();
        let unsuperseded_revision_ids = self
            .published_revision_ids
            .difference(&self.superseded_revision_ids)
            .cloned()
            .collect::<BTreeSet<_>>();
        let current_revision_id = match unsuperseded_revision_ids.len() {
            0 => None,
            1 => unsuperseded_revision_ids.iter().next().cloned(),
            _ => {
                diagnostics.push(ProjectionDiagnostic {
                    code: "ambiguous_current_revision".to_owned(),
                    message: "multiple unsuperseded revisions remain current".to_owned(),
                });
                None
            }
        };
        let current_snapshot_id = current_revision_id
            .as_ref()
            .and_then(|revision_id| self.snapshots_by_revision_id.get(revision_id))
            .cloned();
        let current_review_unit_id = match self.captured_review_unit_ids.len() {
            0 => None,
            1 => self.captured_review_unit_ids.iter().next().cloned(),
            _ => {
                diagnostics.push(ProjectionDiagnostic {
                    code: AMBIGUOUS_CURRENT_REVIEW_UNIT_CODE.to_owned(),
                    message: "multiple captured review units remain current".to_owned(),
                });
                None
            }
        };
        let last_verdict_decision = match current_revision_id.as_ref() {
            Some(revision_id) => {
                let candidate_ids = self
                    .published_artifacts
                    .iter()
                    .filter(|(review_artifact_id, (artifact_revision_id, _))| {
                        artifact_revision_id == revision_id
                            && !self.replaced_artifacts.contains(*review_artifact_id)
                    })
                    .map(|(review_artifact_id, (_, decision))| (review_artifact_id, *decision))
                    .collect::<Vec<_>>();
                match candidate_ids.as_slice() {
                    [] => None,
                    [(_, decision)] => Some(*decision),
                    _ => {
                        diagnostics.push(ProjectionDiagnostic {
                            code: "ambiguous_current_verdict".to_owned(),
                            message: format!(
                                "multiple unsuperseded verdicts remain current for revision {}: {}",
                                revision_id.as_str(),
                                candidate_ids
                                    .iter()
                                    .map(|(review_artifact_id, _)| review_artifact_id.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                        });
                        None
                    }
                }
            }
            None => None,
        };

        Ok(SessionState {
            schema: STATE_SCHEMA.to_owned(),
            version: STATE_VERSION,
            review_id: self.review_id,
            work_unit_id: self.work_unit_id,
            current_revision_id,
            current_snapshot_id,
            review_unit_count: self.captured_review_unit_ids.len(),
            current_review_unit_id,
            event_count,
            sidecar_count: self.sidecar_count,
            note_count: self.note_count,
            review_artifact_count: self.review_artifact_count,
            acknowledgement_count: self.acknowledgement_count,
            last_verdict_decision,
            diagnostics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        AcknowledgementId, ReviewArtifactId, ReviewEndpoint, ReviewId, ReviewUnitId,
        ReviewUnitSource, RevisionId, Side, SnapshotId, WorkUnitId, WorktreeCaptureMode,
    };
    use crate::session::event::{
        AcknowledgementNextAction, ImportedNoteTarget, ReviewArtifactAcknowledgedPayload,
        ReviewArtifactPublishedPayload, ReviewNoteImportedPayload, ReviewUnitCapturedPayload,
        VerdictDecision,
    };
    use crate::session::{
        EventTarget, EventType, ReviewInitializedPayload, RevisionPublishedPayload, ShoreEvent,
        SidecarObservedPayload, SidecarSource, SnapshotObservedPayload, Writer,
    };

    #[test]
    fn projection_tracks_current_revision_snapshot_and_sidecar_count_without_event_history() {
        let events = vec![
            review_initialized("review:default", "work:default"),
            revision_published("rev:worktree:sha256:one", vec![]),
            snapshot_observed("snap:git:sha256:one", "rev:worktree:sha256:one"),
            sidecar_observed("review_notes", "sha256:sidecar"),
        ];

        let projection = SessionState::from_events(&events).expect("projection builds");
        let json = serde_json::to_value(&projection).expect("projection serializes");

        assert_eq!(json["schema"], "shore.state");
        assert_eq!(json["version"], 1);
        assert_eq!(
            projection
                .current_revision_id
                .as_ref()
                .map(RevisionId::as_str),
            Some("rev:worktree:sha256:one")
        );
        assert_eq!(
            projection
                .current_snapshot_id
                .as_ref()
                .map(SnapshotId::as_str),
            Some("snap:git:sha256:one")
        );
        assert_eq!(projection.event_count, 4);
        assert_eq!(projection.sidecar_count, 1);
        assert_eq!(projection.note_count, 0);
        assert!(json.get("events").is_none());
    }

    #[test]
    fn projection_tracks_note_count_without_embedded_note_history() {
        let events = vec![
            review_initialized("review:default", "work:default"),
            review_note_imported("note:abc"),
            review_note_imported("note:def"),
        ];

        let projection = SessionState::from_events(&events).expect("projection builds");
        let json = serde_json::to_value(&projection).expect("projection serializes");

        assert_eq!(projection.note_count, 2);
        assert_eq!(json["noteCount"], 2);
        assert!(json.get("notes").is_none());
    }

    #[test]
    fn projection_uses_explicit_supersession_not_timestamp_ordering() {
        let events = vec![
            revision_published("rev:worktree:sha256:one", vec![]),
            revision_published("rev:worktree:sha256:two", vec!["rev:worktree:sha256:one"]),
        ];

        let projection = SessionState::from_events(&events).expect("projection builds");

        assert_eq!(
            projection
                .current_revision_id
                .as_ref()
                .map(RevisionId::as_str),
            Some("rev:worktree:sha256:two")
        );
    }

    #[test]
    fn projection_reports_ambiguous_current_revision() {
        let events = vec![
            revision_published("rev:worktree:sha256:one", vec![]),
            revision_published("rev:worktree:sha256:two", vec![]),
        ];

        let projection = SessionState::from_events(&events).expect("projection still builds");

        assert_eq!(projection.current_revision_id, None);
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "ambiguous_current_revision")
        );
    }

    #[test]
    fn state_projects_single_current_review_unit() {
        let events = vec![review_unit_captured_event("review-unit:sha256:one")];

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.review_unit_count, 1);
        assert_eq!(
            state.current_review_unit_id.as_ref().unwrap().as_str(),
            "review-unit:sha256:one"
        );
        assert!(state.diagnostics.is_empty());
    }

    #[test]
    fn state_projects_no_current_review_unit_without_captures() {
        let events = Vec::new();

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.review_unit_count, 0);
        assert!(state.current_review_unit_id.is_none());
        assert!(state.diagnostics.is_empty());
    }

    #[test]
    fn state_reports_ambiguous_current_review_units() {
        let events = vec![
            review_unit_captured_event("review-unit:sha256:one"),
            review_unit_captured_event("review-unit:sha256:two"),
        ];

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.review_unit_count, 2);
        assert!(state.current_review_unit_id.is_none());
        assert!(
            state
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == AMBIGUOUS_CURRENT_REVIEW_UNIT_CODE)
        );
    }

    #[test]
    fn review_artifact_published_increments_count_order_independently() {
        let events_a = vec![
            publish_event(),
            verdict_event("review-artifact:sha256:a"),
            verdict_event("review-artifact:sha256:b"),
        ];
        let events_b = vec![
            verdict_event("review-artifact:sha256:b"),
            publish_event(),
            verdict_event("review-artifact:sha256:a"),
        ];

        let state_a = SessionState::from_events(&events_a).expect("state builds");
        let state_b = SessionState::from_events(&events_b).expect("state builds");

        assert_eq!(state_a.review_artifact_count, 2);
        assert_eq!(state_a, state_b);
    }

    #[test]
    fn acknowledgement_count_is_order_independent() {
        let order_one = vec![
            ack_event("ack:sha256:a", "review-artifact:sha256:a"),
            ack_event("ack:sha256:b", "review-artifact:sha256:a"),
        ];
        let order_two = vec![
            ack_event("ack:sha256:b", "review-artifact:sha256:a"),
            ack_event("ack:sha256:a", "review-artifact:sha256:a"),
        ];

        let state_a = SessionState::from_events(&order_one).expect("state builds");
        let state_b = SessionState::from_events(&order_two).expect("state builds");

        assert_eq!(state_a.acknowledgement_count, 2);
        assert_eq!(state_a, state_b);
    }

    #[test]
    fn last_verdict_decision_resolves_when_one_artifact_unreplaced() {
        let events = vec![
            publish_event(),
            verdict_event_with("review-artifact:sha256:v1", VerdictDecision::Pass, vec![]),
            verdict_event_with(
                "review-artifact:sha256:v2",
                VerdictDecision::RequestChanges,
                vec!["review-artifact:sha256:v1"],
            ),
        ];

        let state = SessionState::from_events(&events).expect("state builds");

        assert_eq!(
            state.last_verdict_decision,
            Some(VerdictDecision::RequestChanges)
        );
    }

    #[test]
    fn last_verdict_decision_is_none_and_emits_diagnostic_when_ambiguous() {
        let events = vec![
            publish_event(),
            verdict_event_with("review-artifact:sha256:v1", VerdictDecision::Pass, vec![]),
            verdict_event_with("review-artifact:sha256:v2", VerdictDecision::Pass, vec![]),
        ];

        let state = SessionState::from_events(&events).expect("state builds");

        assert_eq!(state.last_verdict_decision, None);
        assert!(
            state
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "ambiguous_current_verdict")
        );
    }

    #[test]
    fn state_serialization_does_not_embed_event_lists_or_artifact_maps() {
        let events = vec![
            publish_event(),
            verdict_event("review-artifact:sha256:a"),
            ack_event("ack:sha256:a", "review-artifact:sha256:a"),
        ];

        let state = SessionState::from_events(&events).expect("state builds");
        let json = serde_json::to_value(&state).expect("state serializes");

        assert!(json.get("publishedReviewArtifactIds").is_none());
        assert!(json.get("acknowledgedReviewArtifactIds").is_none());
    }

    #[test]
    fn projection_rejects_unsupported_event_schema_version() {
        let mut event = revision_published("rev:worktree:sha256:one", vec![]);
        event.version = 2;

        let error = SessionState::from_events(&[event]).expect_err("unsupported event rejected");

        assert!(
            error
                .to_string()
                .contains("unsupported event schema/version")
        );
    }

    #[test]
    fn projection_has_typed_state_schema_version_validation() {
        let mut projection =
            SessionState::from_events(&[revision_published("rev:worktree:sha256:one", vec![])])
                .expect("projection builds");
        projection.version = 2;

        let error = projection
            .validate_schema_version()
            .expect_err("version 2 is unsupported");

        assert!(matches!(
            error,
            ShoreError::UnsupportedStateSchemaVersion { .. }
        ));
    }

    #[test]
    fn state_deserialization_defaults_new_bounded_verdict_fields() {
        let projection: SessionState = serde_json::from_value(serde_json::json!({
            "schema": "shore.state",
            "version": 1,
            "reviewId": "review:default",
            "workUnitId": "work:default",
            "currentRevisionId": null,
            "currentSnapshotId": null,
            "eventCount": 0,
            "sidecarCount": 0,
            "noteCount": 0,
            "diagnostics": []
        }))
        .expect("projection deserializes");

        assert_eq!(projection.review_artifact_count, 0);
        assert_eq!(projection.acknowledgement_count, 0);
        assert_eq!(projection.last_verdict_decision, None);
    }

    fn review_initialized(review_id: &str, work_unit_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewInitialized,
            format!("review_initialized:{review_id}:{work_unit_id}"),
            target(review_id, work_unit_id),
            Writer::shore_local_author("0.1.0"),
            ReviewInitializedPayload {},
            "2026-05-09T20:42:45Z",
        )
        .expect("review initialized event builds")
    }

    fn revision_published(revision_id: &str, supersedes: Vec<&str>) -> ShoreEvent {
        ShoreEvent::new(
            EventType::RevisionPublished,
            format!("revision_published:explicit:work:default:{revision_id}"),
            target("review:default", "work:default"),
            Writer::shore_local_author("0.1.0"),
            RevisionPublishedPayload {
                revision_id: RevisionId::new(revision_id),
                supersedes_revision_ids: supersedes.into_iter().map(RevisionId::new).collect(),
            },
            "2026-05-09T20:42:45Z",
        )
        .expect("revision published event builds")
    }

    fn snapshot_observed(snapshot_id: &str, revision_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::SnapshotObserved,
            format!("snapshot_observed:work:default:{revision_id}:{snapshot_id}"),
            target("review:default", "work:default"),
            Writer::shore_local_author("0.1.0"),
            SnapshotObservedPayload {
                snapshot_id: SnapshotId::new(snapshot_id),
                revision_id: RevisionId::new(revision_id),
            },
            "2026-05-09T20:42:45Z",
        )
        .expect("snapshot observed event builds")
    }

    fn review_unit_captured_event(review_unit_id: &str) -> ShoreEvent {
        let review_unit_id = ReviewUnitId::new(review_unit_id);
        let revision_id = RevisionId::new(format!("rev:git:sha256:{}", review_unit_id.as_str()));
        let snapshot_id = SnapshotId::new(format!("snap:git:sha256:{}", review_unit_id.as_str()));
        ShoreEvent::new(
            EventType::ReviewUnitCaptured,
            format!("review_unit_captured:{}", review_unit_id.as_str()),
            EventTarget::for_review_unit(
                ReviewId::new("review:default"),
                review_unit_id.clone(),
                revision_id.clone(),
                snapshot_id.clone(),
            ),
            Writer::shore_local_author("0.1.0"),
            ReviewUnitCapturedPayload {
                review_unit_id,
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
                revision_id,
                snapshot_id,
            },
            "2026-05-12T00:00:00Z",
        )
        .expect("review unit captured event builds")
    }

    fn sidecar_observed(source: &str, content_hash: &str) -> ShoreEvent {
        let mut diagnostic_levels = BTreeMap::new();
        diagnostic_levels.insert("warning".to_owned(), 0);

        ShoreEvent::new(
            EventType::SidecarObserved,
            format!("sidecar_observed:{source}:{content_hash}"),
            target("review:default", "work:default"),
            Writer::shore_local_author("0.1.0"),
            SidecarObservedPayload {
                source: match source {
                    "review_notes" => SidecarSource::ReviewNotes,
                    "legacy_hunk_agent_context" => SidecarSource::LegacyHunkAgentContext,
                    other => panic!("unknown sidecar source: {other}"),
                },
                path: "review-notes.json".to_owned(),
                byte_size: 2,
                content_hash: content_hash.to_owned(),
                schema: Some("shore.review-notes".to_owned()),
                imported_schema: None,
                version: Some(1),
                diagnostic_count: 0,
                diagnostic_levels,
            },
            "2026-05-09T20:42:45Z",
        )
        .expect("sidecar observed event builds")
    }

    fn review_note_imported(note_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewNoteImported,
            format!("review_note_imported:review_notes:work:default:{note_id}"),
            target("review:default", "work:default"),
            Writer::shore_local_author("0.1.0"),
            ReviewNoteImportedPayload {
                sidecar_source: SidecarSource::ReviewNotes,
                note_id: note_id.to_owned(),
                file_path: "src/lib.rs".to_owned(),
                file_old_path: None,
                target: Some(ImportedNoteTarget {
                    side: Side::New,
                    start_line: 1,
                    end_line: 1,
                }),
                title: "Imported note".to_owned(),
                body: Some("Body".to_owned()),
                body_artifact_path: None,
                body_byte_size: None,
                tags: vec![],
                confidence: None,
                external_source: Some("external".to_owned()),
                author: Some("reviewer".to_owned()),
                created_at: Some("2026-05-10T00:00:00Z".to_owned()),
                sidecar_content_hash: "sha256:sidecar".to_owned(),
            },
            "2026-05-09T20:42:45Z",
        )
        .expect("review note imported event builds")
    }

    fn publish_event() -> ShoreEvent {
        revision_published("rev:worktree:sha256:current", vec![])
    }

    fn verdict_event(review_artifact_id: &str) -> ShoreEvent {
        verdict_event_with(review_artifact_id, VerdictDecision::Pass, vec![])
    }

    fn verdict_event_with(
        review_artifact_id: &str,
        decision: VerdictDecision,
        replaces_review_artifact_ids: Vec<&str>,
    ) -> ShoreEvent {
        let review_artifact_id = ReviewArtifactId::new(review_artifact_id);
        ShoreEvent::new(
            EventType::ReviewArtifactPublished,
            ReviewArtifactPublishedPayload::idempotency_key(
                &WorkUnitId::new("work:default"),
                &review_artifact_id,
            ),
            target("review:default", "work:default"),
            Writer::shore_local_reviewer("0.1.0"),
            ReviewArtifactPublishedPayload {
                review_artifact_id,
                work_unit_id: WorkUnitId::new("work:default"),
                revision_id: RevisionId::new("rev:worktree:sha256:current"),
                decision,
                summary: Some("looks good".to_owned()),
                summary_artifact_path: None,
                summary_byte_size: Some(10),
                replaces_review_artifact_ids: replaces_review_artifact_ids
                    .into_iter()
                    .map(ReviewArtifactId::new)
                    .collect(),
                reviewer: Writer::shore_local_reviewer("0.1.0"),
            },
            "2026-05-10T00:00:00Z",
        )
        .expect("review artifact published event builds")
    }

    fn ack_event(acknowledgement_id: &str, review_artifact_id: &str) -> ShoreEvent {
        let acknowledgement_id = AcknowledgementId::new(acknowledgement_id);
        let review_artifact_id = ReviewArtifactId::new(review_artifact_id);
        ShoreEvent::new(
            EventType::ReviewArtifactAcknowledged,
            ReviewArtifactAcknowledgedPayload::idempotency_key(
                &review_artifact_id,
                &acknowledgement_id,
            ),
            target("review:default", "work:default"),
            Writer::shore_local_author("0.1.0"),
            ReviewArtifactAcknowledgedPayload {
                acknowledgement_id,
                review_artifact_id,
                next_action: AcknowledgementNextAction::Accept,
                reason: Some("accepted".to_owned()),
                reason_artifact_path: None,
                reason_byte_size: Some(8),
                acknowledger: Writer::shore_local_author("0.1.0"),
            },
            "2026-05-10T00:00:00Z",
        )
        .expect("review artifact acknowledged event builds")
    }

    fn target(review_id: &str, work_unit_id: &str) -> EventTarget {
        EventTarget::new(ReviewId::new(review_id), WorkUnitId::new(work_unit_id))
    }
}
