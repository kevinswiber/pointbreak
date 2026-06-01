use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::Result;
use crate::model::{
    ReviewEndpoint, ReviewUnitId, ReviewUnitSource, RevisionId, SessionId, SnapshotId,
};
use crate::session::EventStore;
use crate::session::event::{EventType, ReviewUnitCapturedPayload, ShoreEvent};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::resolve_store;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitListOptions {
    repo: PathBuf,
}

impl ReviewUnitListOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitListEntry {
    pub review_unit_id: ReviewUnitId,
    pub session_id: SessionId,
    pub captured_at: String,
    pub revision_id: RevisionId,
    pub snapshot_id: SnapshotId,
    pub source: ReviewUnitSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
    pub snapshot_artifact_content_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitListResult {
    pub event_set_hash: String,
    pub event_count: usize,
    pub review_unit_count: usize,
    pub entries: Vec<ReviewUnitListEntry>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn list_review_units(options: ReviewUnitListOptions) -> Result<ReviewUnitListResult> {
    let resolution = resolve_store(&options.repo)?;
    let events = EventStore::open(resolution.store_dir()).list_events()?;
    list_from_events(&events)
}

fn list_from_events(events: &[ShoreEvent]) -> Result<ReviewUnitListResult> {
    let state = SessionState::from_events(events)?;
    let event_set_hash = state
        .event_set_hash
        .clone()
        .expect("SessionState::from_events sets event_set_hash");

    let mut entries = events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewUnitCaptured)
        .map(entry_from_event)
        .collect::<Result<Vec<_>>>()?;

    entries.sort_by(|left, right| {
        left.captured_at.cmp(&right.captured_at).then_with(|| {
            left.review_unit_id
                .as_str()
                .cmp(right.review_unit_id.as_str())
        })
    });

    Ok(ReviewUnitListResult {
        event_set_hash,
        event_count: events.len(),
        review_unit_count: entries.len(),
        entries,
        diagnostics: state.diagnostics,
    })
}

fn entry_from_event(event: &ShoreEvent) -> Result<ReviewUnitListEntry> {
    let payload: ReviewUnitCapturedPayload = serde_json::from_value(event.payload.clone())?;
    Ok(ReviewUnitListEntry {
        review_unit_id: payload.review_unit_id,
        session_id: event.target.session_id.clone(),
        captured_at: event.occurred_at.clone(),
        revision_id: payload.revision_id,
        snapshot_id: payload.snapshot_id,
        source: payload.source,
        base: payload.base,
        target: payload.target,
        snapshot_artifact_content_hash: payload.snapshot_artifact_content_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ReviewEndpoint, ReviewUnitSource, WorktreeCaptureMode};
    use crate::session::event::{EventTarget, Writer};

    #[test]
    fn empty_event_set_returns_no_entries() {
        let result = list_from_events(&[]).unwrap();

        assert_eq!(result.event_count, 0);
        assert_eq!(result.review_unit_count, 0);
        assert!(result.entries.is_empty());
        assert!(result.event_set_hash.starts_with("sha256:"));
    }

    #[test]
    fn includes_only_review_unit_captured_events() {
        let capture = captured_event("a", "2026-05-13T10:00:00Z");
        let result = list_from_events(&[capture]).unwrap();

        assert_eq!(result.event_count, 1);
        assert_eq!(result.review_unit_count, 1);
        assert_eq!(
            result.entries[0].review_unit_id.as_str(),
            "review-unit:sha256:a"
        );
        assert_eq!(result.entries[0].captured_at, "2026-05-13T10:00:00Z");
        assert_eq!(
            result.entries[0].snapshot_artifact_content_hash,
            "sha256:artifact:a"
        );
    }

    #[test]
    fn sorts_entries_by_captured_at_then_review_unit_id() {
        let later = captured_event("z-later", "2026-05-13T10:00:05Z");
        let tie_b = captured_event("b-tie", "2026-05-13T10:00:01Z");
        let tie_a = captured_event("a-tie", "2026-05-13T10:00:01Z");

        let result = list_from_events(&[later, tie_b, tie_a]).unwrap();

        let order: Vec<&str> = result
            .entries
            .iter()
            .map(|entry| entry.review_unit_id.as_str())
            .collect();
        assert_eq!(
            order,
            vec![
                "review-unit:sha256:a-tie",
                "review-unit:sha256:b-tie",
                "review-unit:sha256:z-later",
            ]
        );
    }

    #[test]
    fn entry_serializes_with_camel_case_and_no_internal_paths() {
        let result = list_from_events(&[captured_event("one", "2026-05-13T10:00:00Z")]).unwrap();
        let json = serde_json::to_string(&result.entries[0]).unwrap();

        assert!(json.contains("reviewUnitId"));
        assert!(json.contains("capturedAt"));
        assert!(json.contains("snapshotArtifactContentHash"));
        assert!(!json.contains("artifacts/"));
        assert!(!json.contains("statePath"));
        assert!(!json.contains("payloadHash"));
    }

    fn captured_event(suffix: &str, occurred_at: &str) -> ShoreEvent {
        let review_unit_id = ReviewUnitId::new(format!("review-unit:sha256:{suffix}"));
        let revision_id = RevisionId::new(format!("rev:sha256:{suffix}"));
        let snapshot_id = SnapshotId::new(format!("snap:sha256:{suffix}"));
        let payload = ReviewUnitCapturedPayload {
            review_unit_id: review_unit_id.clone(),
            source: ReviewUnitSource::GitWorktree {
                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                include_untracked: true,
            },
            base: ReviewEndpoint::GitCommit {
                commit_oid: format!("base:{suffix}"),
                tree_oid: format!("base-tree:{suffix}"),
            },
            target: ReviewEndpoint::GitWorkingTree {
                worktree_root: "/repo".to_owned(),
            },
            revision_id: revision_id.clone(),
            snapshot_id: snapshot_id.clone(),
            snapshot_artifact_content_hash: format!("sha256:artifact:{suffix}"),
        };
        ShoreEvent::new(
            EventType::ReviewUnitCaptured,
            format!("capture:{suffix}"),
            EventTarget::for_review_unit(
                SessionId::new("session:default"),
                review_unit_id,
                revision_id,
                snapshot_id,
            ),
            Writer::shore_local_author("test"),
            payload,
            occurred_at,
        )
        .unwrap()
    }
}
