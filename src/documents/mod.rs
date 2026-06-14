//! Command-output document layer for the `shore review-*` command family.
//!
//! This module owns the serializable documents the `shore review-*` commands
//! emit: the shared envelopes ([`DiagnosticDocument`], [`EventWriteDocument`]),
//! the per-item view-document mappers, the per-command body structs, and the
//! `*_document()` builders that wrap a `shoreline::session` result into the
//! documented JSON shape.
//!
//! Consumers can produce **byte-identical** `shore review-*` JSON in-process by
//! calling a builder and serializing the returned document with `serde_json`.
//! The CLI is a thin caller over these same builders, so the documented JSON
//! contract has a single source of truth.
//!
//! Stdout serialization (`write_json`) stays in the CLI; this module exposes the
//! serializable documents, not terminal IO.

use std::collections::BTreeMap;

use crate::session::ProjectionDiagnostic;

mod assessment;
mod capture;
mod history;
mod input_request;
mod lineage;
mod observation;
mod unit;
mod validation;
mod view;

pub use assessment::{
    AssessmentAddBody, AssessmentShowBody, assessment_add_document, assessment_show_document,
};
pub use capture::{
    CaptureBody, CaptureWithLineageBody, capture_document, capture_with_lineage_document,
};
pub use history::{HistoryBody, history_document};
pub use input_request::{
    InputRequestFetchBody, InputRequestListBody, InputRequestOpenBody, InputRequestRespondBody,
    input_request_fetch_document, input_request_list_document, input_request_open_document,
    input_request_respond_document,
};
pub use lineage::{
    LineageAttachBody, LineageShowBody, lineage_attach_document, lineage_show_document,
};
pub use observation::{
    ObservationAddBody, ObservationListBody, observation_add_document, observation_list_document,
};
pub use unit::{UnitListBody, UnitShowBody, unit_list_document, unit_show_document};
pub use validation::{
    ValidationAddBody, ValidationListBody, validation_add_document, validation_list_document,
};
pub use view::{
    AssessmentViewDocument, CurrentAssessmentDocument, InputRequestAssertionModeDocument,
    InputRequestResponseViewDocument, InputRequestViewDocument, ObservationViewDocument,
    ValidationCheckViewDocument,
};

/// Envelope for a read/diagnostic document: `{ schema, version, <flattened
/// body>, diagnostics }`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticDocument<T> {
    schema: &'static str,
    version: u32,
    #[serde(flatten)]
    body: T,
    diagnostics: Vec<ProjectionDiagnostic>,
}

/// Envelope for an event-write document: the diagnostic envelope plus the
/// `eventsCreated`/`eventsExisting`/`eventsCreatedByType` write counts.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventWriteDocument<T> {
    schema: &'static str,
    version: u32,
    #[serde(flatten)]
    body: T,
    events_created: usize,
    events_existing: usize,
    events_created_by_type: BTreeMap<String, usize>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

impl<T> DiagnosticDocument<T> {
    /// Wrap `body` in the diagnostic envelope under `schema` at version 1.
    pub fn new(schema: &'static str, body: T, diagnostics: Vec<ProjectionDiagnostic>) -> Self {
        Self {
            schema,
            version: 1,
            body,
            diagnostics,
        }
    }
}

impl<T> EventWriteDocument<T> {
    /// Wrap `body` in the event-write envelope under `schema` at version 1.
    pub fn new(
        schema: &'static str,
        body: T,
        events_created: usize,
        events_existing: usize,
        events_created_by_type: BTreeMap<String, usize>,
        diagnostics: Vec<ProjectionDiagnostic>,
    ) -> Self {
        Self {
            schema,
            version: 1,
            body,
            events_created,
            events_existing,
            events_created_by_type,
            diagnostics,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    fn write_compact<T: serde::Serialize>(document: &T) -> String {
        let mut buf = Vec::new();
        serde_json::to_writer(&mut buf, document).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn event_write_document_preserves_field_order() {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body {
            review_unit_id: &'static str,
            event_id: &'static str,
        }

        let doc = super::EventWriteDocument::new(
            "shore.test-write",
            Body {
                review_unit_id: "unit:1",
                event_id: "evt:1",
            },
            1,
            2,
            BTreeMap::new(),
            Vec::new(),
        );

        assert_eq!(
            write_compact(&doc),
            "{\"schema\":\"shore.test-write\",\"version\":1,\"reviewUnitId\":\"unit:1\",\"eventId\":\"evt:1\",\"eventsCreated\":1,\"eventsExisting\":2,\"eventsCreatedByType\":{},\"diagnostics\":[]}"
        );
    }

    #[test]
    fn diagnostic_document_preserves_trailing_diagnostics() {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body {
            review_unit_id: &'static str,
            count: usize,
        }

        let doc = super::DiagnosticDocument::new(
            "shore.test-read",
            Body {
                review_unit_id: "unit:1",
                count: 3,
            },
            Vec::new(),
        );

        assert_eq!(
            write_compact(&doc),
            "{\"schema\":\"shore.test-read\",\"version\":1,\"reviewUnitId\":\"unit:1\",\"count\":3,\"diagnostics\":[]}"
        );
    }

    #[test]
    fn lineage_show_document_serializes_head_and_rounds_without_paths() {
        use crate::documents::lineage_show_document;
        use crate::model::{ReviewUnitId, ReviewUnitLineageId, ReviewUnitLineageRoundId};
        use crate::session::{LineageRoundView, LineageShowResult};

        let lineage_id = ReviewUnitLineageId::new("review-unit-lineage:sha256:abc");
        let document = lineage_show_document(LineageShowResult {
            event_set_hash: "sha256:events".to_owned(),
            event_count: 4,
            lineage_id: lineage_id.clone(),
            head_review_unit_id: Some(ReviewUnitId::new("review-unit:sha256:two")),
            rounds: vec![LineageRoundView {
                lineage_id,
                round_id: ReviewUnitLineageRoundId::new("review-unit-lineage-round:sha256:two"),
                review_unit_id: ReviewUnitId::new("review-unit:sha256:two"),
                predecessor_review_unit_id: Some(ReviewUnitId::new("review-unit:sha256:one")),
                round_index: Some(1),
                is_head: true,
            }],
            diagnostics: Vec::new(),
        });
        let json = write_compact(&document);

        assert!(json.contains("\"schema\":\"shore.review-lineage\""));
        assert!(json.contains("\"headReviewUnitId\""));
        assert!(json.contains("\"rounds\""));
        assert!(!json.contains("worktreeRoot"));
        assert!(!json.contains(".shore"));
        assert!(!json.contains(".git"));
    }

    #[test]
    fn capture_with_lineage_document_nests_attach_counts() {
        use crate::documents::capture_with_lineage_document;
        use crate::model::{
            ReviewEndpoint, ReviewUnitId, ReviewUnitLineageId, ReviewUnitSource, RevisionId,
            SessionId, SnapshotId, WorktreeCaptureMode,
        };
        use crate::session::{CaptureResult, LineageAttachResult};

        let document = capture_with_lineage_document(
            CaptureResult {
                session_id: SessionId::new("session:default"),
                review_unit_id: ReviewUnitId::new("review-unit:sha256:one"),
                revision_id: RevisionId::new("rev:sha256:one"),
                snapshot_id: SnapshotId::new("snapshot:sha256:one"),
                source: ReviewUnitSource::GitWorktree {
                    mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                    include_untracked: true,
                },
                base: ReviewEndpoint::GitCommit {
                    commit_oid: "abc".to_owned(),
                    tree_oid: "def".to_owned(),
                },
                target: ReviewEndpoint::GitWorkingTree {
                    worktree_root: "/tmp/repo".to_owned(),
                },
                snapshot_artifact_content_hash: "sha256:artifact".to_owned(),
                events_created: 1,
                events_existing: 0,
                events_created_by_type: BTreeMap::from([("review_unit_captured".to_owned(), 1)]),
                diagnostics: Vec::new(),
            },
            LineageAttachResult {
                lineage_id: ReviewUnitLineageId::new("review-unit-lineage:sha256:abc"),
                head_review_unit_id: Some(ReviewUnitId::new("review-unit:sha256:one")),
                events_created: 2,
                events_existing: 0,
                events_created_by_type: BTreeMap::from([
                    ("review_unit_lineage_declared".to_owned(), 1),
                    ("review_unit_lineage_round_recorded".to_owned(), 1),
                ]),
                diagnostics: Vec::new(),
            },
        );
        let json = write_compact(&document);

        assert!(json.contains("\"schema\":\"shore.review-capture\""));
        assert!(json.contains("\"lineageAttach\""));
        assert!(json.contains("\"review_unit_lineage_round_recorded\":1"));
    }

    #[test]
    fn validation_add_document_serializes_advisory_validation_add_schema() {
        use crate::documents::validation_add_document;
        use crate::model::{
            EventId, ReviewUnitId, TrackId, ValidationCheckId, ValidationStatus, ValidationTarget,
        };
        use crate::session::ValidationAddResult;

        let review_unit_id = ReviewUnitId::new("review-unit:sha256:one");
        let doc = validation_add_document(ValidationAddResult {
            review_unit_id: review_unit_id.clone(),
            validation_check_id: ValidationCheckId::new("validation:sha256:one"),
            event_id: EventId::new("evt:sha256:one"),
            track_id: TrackId::new("agent:codex"),
            target: ValidationTarget::ReviewUnit { review_unit_id },
            status: ValidationStatus::Passed,
            summary_content_hash: Some("sha256:summary".to_owned()),
            events_created: 1,
            events_existing: 0,
            events_created_by_type: BTreeMap::from([("validation_check_recorded".to_owned(), 1)]),
            diagnostics: Vec::new(),
        });

        let value = serde_json::to_value(&doc).unwrap();
        assert_eq!(value["schema"], "shore.review-validation-add");
        assert_eq!(value["status"], "passed");
        assert_eq!(value["summaryContentHash"], "sha256:summary");
        assert!(value.get("accepted").is_none());
        assert!(value.get("gate").is_none());
    }

    #[test]
    fn view_document_principal_is_options_gated_and_agent_scoped() {
        use crate::documents::ValidationCheckViewDocument;
        use crate::model::ActorId;
        use crate::session::delegation_map_from_value;

        let map = delegation_map_from_value(serde_json::json!({
            "delegates": {
                "actor:agent:claude-code": [{
                    "principal": "actor:git-email:kevin@swiber.dev",
                    "validFrom": "2026-05-01T00:00:00Z",
                    "validUntil": null
                }]
            }
        }))
        .unwrap();

        let agent_view = || {
            let mut view = validation_view();
            view.writer.actor_id = ActorId::new("actor:agent:claude-code");
            view
        };

        // Agent writer + map → resolved principal object beside writer.
        let resolved =
            ValidationCheckViewDocument::from(agent_view()).with_resolved_principal(Some(&map));
        let value = serde_json::to_value(&resolved).unwrap();
        assert_eq!(
            value["principal"]["actorId"],
            "actor:git-email:kevin@swiber.dev"
        );
        assert_eq!(value["principal"]["status"], "resolved");
        assert_eq!(value["principal"]["source"], "delegates");

        // Agent writer + no map → mirror posture.
        let no_map = ValidationCheckViewDocument::from(agent_view()).with_resolved_principal(None);
        assert_eq!(
            serde_json::to_value(&no_map).unwrap()["principal"],
            serde_json::json!({ "status": "none", "source": "none" })
        );

        // Human writer + map → no principal object (its own principal).
        let human = ValidationCheckViewDocument::from(validation_view())
            .with_resolved_principal(Some(&map));
        assert!(
            serde_json::to_value(&human)
                .unwrap()
                .get("principal")
                .is_none(),
            "human writers carry no principal object"
        );

        // Plain `From` (the unit-document / add path) carries no principal.
        let plain = ValidationCheckViewDocument::from(validation_view());
        assert!(
            serde_json::to_value(&plain)
                .unwrap()
                .get("principal")
                .is_none(),
            "the From path attaches no principal — unit docs stay principal-free"
        );
    }

    #[test]
    fn validation_view_document_has_expected_wire_keys() {
        use crate::documents::ValidationCheckViewDocument;

        let doc = ValidationCheckViewDocument::from(validation_view());
        let value = serde_json::to_value(&doc).unwrap();

        for key in [
            "id",
            "eventId",
            "trackId",
            "target",
            "checkName",
            "status",
            "trigger",
            "logArtifactContentHashes",
            "createdAt",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        assert!(value.get("accepted").is_none());
    }

    #[test]
    fn unit_show_document_includes_validation_checks_and_count() {
        use crate::documents::unit_show_document;
        use crate::model::ValidationStatus;
        use crate::session::{
            CaptureOptions, ReviewUnitShowOptions, ValidationAddOptions, capture_worktree_review,
            record_validation_check, show_review_unit,
        };

        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_validation_check(
            ValidationAddOptions::new(repo.path())
                .with_review_unit_id(capture.review_unit_id.clone())
                .with_track("agent:codex")
                .with_check_name("cargo test")
                .with_status(ValidationStatus::Passed),
        )
        .unwrap();

        let result = show_review_unit(
            ReviewUnitShowOptions::new(repo.path())
                .with_review_unit_id(capture.review_unit_id)
                .with_include_body(true),
        )
        .unwrap();
        let value = serde_json::to_value(unit_show_document(result)).unwrap();

        assert!(value["validationChecks"].is_array());
        assert_eq!(value["summary"]["validationCheckCount"], 1);
        let row = value["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["kind"] == "validation_evidence")
            .expect("validation row");
        assert_eq!(
            row["relatedValidationCheckIds"].as_array().unwrap().len(),
            1
        );
    }

    fn validation_view() -> crate::session::ValidationCheckView {
        use crate::model::{
            EventId, ReviewUnitId, TrackId, ValidationCheckId, ValidationStatus, ValidationTarget,
            ValidationTrigger,
        };
        use crate::session::event::Writer;

        let review_unit_id = ReviewUnitId::new("review-unit:sha256:one");
        crate::session::ValidationCheckView {
            id: ValidationCheckId::new("validation:sha256:one"),
            event_id: EventId::new("evt:sha256:one"),
            track_id: TrackId::new("agent:codex"),
            target: ValidationTarget::ReviewUnit { review_unit_id },
            check_name: "cargo test".to_owned(),
            command: Some("cargo test --all".to_owned()),
            status: ValidationStatus::Passed,
            exit_code: Some(0),
            trigger: ValidationTrigger::Manual,
            source_fingerprint: Some("rev:sha256:head".to_owned()),
            summary: Some("tests passed".to_owned()),
            summary_content_hash: Some("sha256:summary".to_owned()),
            started_at: Some("2026-05-10T00:00:00Z".to_owned()),
            completed_at: Some("2026-05-10T00:01:00Z".to_owned()),
            log_artifact_content_hashes: vec!["sha256:log".to_owned()],
            created_at: "2026-05-10T00:01:01Z".to_owned(),
            writer: Writer::shore_local(env!("CARGO_PKG_VERSION")),
        }
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo
    }

    struct TestRepo {
        root: tempfile::TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("temp repo");
            let repo = Self { root };
            repo.git(["init"]);
            repo.git(["config", "user.email", "agent@example.com"]);
            repo.git(["config", "user.name", "Agent"]);
            repo.git(["config", "commit.gpgsign", "false"]);
            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn write(&self, path: &str, contents: &str) {
            let path = self.root.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directories");
            }
            fs::write(path, contents).expect("write test fixture");
        }

        fn commit_all(&self, message: &str) {
            self.git(["add", "--all"]);
            self.git(["commit", "-m", message]);
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            let args = args
                .into_iter()
                .map(|arg| arg.as_ref().to_owned())
                .collect::<Vec<_>>();
            let output = Command::new("git")
                .args(&args)
                .current_dir(self.root.path())
                .output()
                .unwrap_or_else(|error| panic!("run git {:?}: {error}", args));

            assert!(
                output.status.success(),
                "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
                args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
