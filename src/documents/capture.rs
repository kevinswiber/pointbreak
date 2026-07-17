// Document builder for `pointbreak review-capture`.
use crate::documents::EventWriteDocument;
use crate::model::ReviewEndpoint;
use crate::session::CaptureResult;

/// Documented body for `pointbreak.review-capture`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureBody {
    revision: CaptureRevisionDocument,
    /// Additive per-file diff tallies (ADR-0029 soft shell; fields may be added
    /// within a document `version`). The consumed `.revision.id` path is unmoved.
    diffstat: CaptureDiffstatDocument,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureDiffstatDocument {
    file_count: usize,
    added_files: usize,
    modified_files: usize,
    deleted_files: usize,
    renamed_files: usize,
    copied_files: usize,
    binary_files: usize,
    mode_only_files: usize,
    added_lines: usize,
    removed_lines: usize,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureRevisionDocument {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    base: ReviewEndpoint,
    target: ReviewEndpoint,
    revision_id: String,
    object_id: String,
    object_artifact_content_hash: String,
}

/// Build the `pointbreak.review-capture` document from a capture result.
pub fn capture_document(result: CaptureResult) -> EventWriteDocument<CaptureBody> {
    EventWriteDocument::new(
        "pointbreak.review-capture",
        CaptureBody {
            revision: CaptureRevisionDocument {
                id: result.revision_id.as_str().to_owned(),
                summary: result.summary,
                base: result.base,
                target: result.target,
                revision_id: result.revision_id.as_str().to_owned(),
                object_id: result.object_id.as_str().to_owned(),
                object_artifact_content_hash: result.object_artifact_content_hash,
            },
            diffstat: CaptureDiffstatDocument {
                file_count: result.diffstat.file_count,
                added_files: result.diffstat.added_files,
                modified_files: result.diffstat.modified_files,
                deleted_files: result.diffstat.deleted_files,
                renamed_files: result.diffstat.renamed_files,
                copied_files: result.diffstat.copied_files,
                binary_files: result.diffstat.binary_files,
                mode_only_files: result.diffstat.mode_only_files,
                added_lines: result.diffstat.added_lines,
                removed_lines: result.diffstat.removed_lines,
            },
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::capture_document;
    use crate::model::{
        EngagementId, JournalId, ObjectId, ReviewEndpoint, RevisionId, RevisionSource,
        WorktreeCaptureMode,
    };
    use crate::session::{CaptureDiffstat, CaptureResult};

    fn sample_capture_result() -> CaptureResult {
        CaptureResult {
            journal_id: JournalId::new("journal:default"),
            revision_id: RevisionId::new("rev:sha256:abc123"),
            object_id: ObjectId::new("obj:sha256:def456"),
            engagement_id: EngagementId::new("engagement:default"),
            summary: None,
            source: RevisionSource::GitWorktree {
                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                include_untracked: false,
                pathspecs: Vec::new(),
            },
            base: ReviewEndpoint::GitCommit {
                commit_oid: "0".repeat(40),
                tree_oid: "1".repeat(40),
            },
            target: ReviewEndpoint::GitWorkingTree {
                worktree_root: "/repo".to_owned(),
            },
            object_artifact_content_hash: "sha256:cafe".to_owned(),
            events_created: 2,
            events_existing: 0,
            events_created_by_type: BTreeMap::new(),
            diagnostics: Vec::new(),
            diffstat: CaptureDiffstat {
                file_count: 6,
                added_files: 1,
                modified_files: 3,
                deleted_files: 1,
                renamed_files: 1,
                copied_files: 0,
                binary_files: 1,
                mode_only_files: 1,
                added_lines: 13,
                removed_lines: 5,
            },
        }
    }

    #[test]
    fn capture_document_carries_additive_diffstat_block() {
        let document = capture_document(sample_capture_result());
        let value = serde_json::to_value(&document).expect("serialize capture document");
        assert_eq!(value["diffstat"]["fileCount"], 6);
        assert_eq!(value["diffstat"]["addedLines"], 13);
        assert_eq!(value["diffstat"]["removedLines"], 5);
        // The consumed hard-core path stays present.
        assert!(value["revision"]["id"].is_string());
        assert!(value["revision"].get("summary").is_none());
    }
}
