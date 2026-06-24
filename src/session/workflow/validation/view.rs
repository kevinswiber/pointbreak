use std::collections::BTreeMap;

use serde::Serialize;

use crate::error::{Result, ShoreError};
use crate::model::{
    EventId, RevisionId, TrackId, ValidationCheckId, ValidationStatus, ValidationTarget,
    ValidationTrigger,
};
use crate::session::body_artifact::load_body_artifact;
use crate::session::event::{EventType, ShoreEvent, ValidationCheckRecordedPayload, Writer};
use crate::session::store::backend::StoreBackend;

struct ValidationEventRecord<'a> {
    event: &'a ShoreEvent,
    payload: ValidationCheckRecordedPayload,
    track_id: TrackId,
}

pub(crate) struct ValidationCheckProjectionOptions<'a> {
    pub backend: &'a StoreBackend,
    pub events: &'a [ShoreEvent],
    pub revision_id: &'a RevisionId,
    pub track_filter: Option<TrackId>,
    pub status_filter: Option<ValidationStatus>,
    pub include_body: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationCheckView {
    pub id: ValidationCheckId,
    pub event_id: EventId,
    pub track_id: TrackId,
    pub target: ValidationTarget,
    pub check_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub status: ValidationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    pub trigger: ValidationTrigger,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_content_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub log_artifact_content_hashes: Vec<String>,
    pub created_at: String,
    pub writer: Writer,
}

pub fn project_validation_checks(
    options: ValidationCheckProjectionOptions<'_>,
) -> Result<Vec<ValidationCheckView>> {
    let mut validation_records: BTreeMap<ValidationCheckId, ValidationEventRecord<'_>> =
        BTreeMap::new();

    for event in options
        .events
        .iter()
        .filter(|event| event.event_type == EventType::ValidationCheckRecorded)
    {
        if crate::model::subject_revision_id(&event.target.subject) != Some(options.revision_id) {
            continue;
        }

        let payload: ValidationCheckRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        if options
            .status_filter
            .is_some_and(|filter| filter != payload.status)
        {
            continue;
        }

        let track_id =
            event.target.track_id.clone().ok_or_else(|| {
                ShoreError::Message("validation event missing track id".to_owned())
            })?;
        if options
            .track_filter
            .as_ref()
            .is_some_and(|filter| filter != &track_id)
        {
            continue;
        }

        let validation_check_id = payload.validation_check_id.clone();
        let replace_record = validation_records
            .get(&validation_check_id)
            .is_none_or(|record| {
                // Event IDs are deterministic storage addresses, not causal order. Pick the
                // lowest one only as a stable representative for duplicate semantic facts.
                event.event_id.as_str() < record.event.event_id.as_str()
            });
        if replace_record {
            validation_records.insert(
                validation_check_id,
                ValidationEventRecord {
                    event,
                    payload,
                    track_id,
                },
            );
        }
    }

    let mut validations = Vec::new();
    for (_, record) in validation_records {
        let summary = if options.include_body {
            validation_summary(options.backend, &record.payload)?
        } else {
            None
        };

        validations.push(ValidationCheckView {
            id: record.payload.validation_check_id,
            event_id: record.event.event_id.clone(),
            track_id: record.track_id,
            target: record.payload.target,
            check_name: record.payload.check_name,
            command: record.payload.command,
            status: record.payload.status,
            exit_code: record.payload.exit_code,
            trigger: record.payload.trigger,
            source_fingerprint: record.payload.source_fingerprint,
            summary,
            summary_content_hash: record.payload.summary_content_hash,
            started_at: record.payload.started_at,
            completed_at: record.payload.completed_at,
            log_artifact_content_hashes: record.payload.log_artifact_content_hashes,
            created_at: record.event.occurred_at.clone(),
            writer: record.event.writer.clone(),
        });
    }

    sort_validation_check_views(&mut validations);
    Ok(validations)
}

fn validation_summary(
    backend: &StoreBackend,
    payload: &ValidationCheckRecordedPayload,
) -> Result<Option<String>> {
    if payload.summary.is_some() {
        return Ok(payload.summary.clone());
    }
    match payload.summary_artifact_path.as_deref() {
        Some(path) => load_body_artifact(backend, path),
        None => Ok(None),
    }
}

fn sort_validation_check_views(validations: &mut [ValidationCheckView]) {
    validations.sort_by(|left, right| {
        validation_sort_time(left)
            .cmp(validation_sort_time(right))
            .then_with(|| left.event_id.as_str().cmp(right.event_id.as_str()))
    });
}

fn validation_sort_time(view: &ValidationCheckView) -> &str {
    view.completed_at.as_deref().unwrap_or(&view.created_at)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::model::{
        EventId, JournalId, RevisionId, TrackId, ValidationCheckId, ValidationStatus,
        ValidationTarget, ValidationTrigger,
    };
    use crate::session::event::{
        EventTarget, EventType, ShoreEvent, ValidationCheckRecordedPayload, Writer,
    };

    #[test]
    fn project_validation_checks_filters_by_revision() {
        let events = vec![
            validation_event(
                "evt:sha256:0001",
                "review-unit:sha256:one",
                "validation:sha256:one",
                "2026-05-10T00:00:00Z",
                None,
            ),
            validation_event(
                "evt:sha256:0002",
                "review-unit:sha256:two",
                "validation:sha256:two",
                "2026-05-10T00:00:01Z",
                None,
            ),
        ];
        let dir = tempfile::tempdir().unwrap();
        let backend = StoreBackend::Local(dir.path().to_path_buf());

        let views = project_validation_checks(ValidationCheckProjectionOptions {
            backend: &backend,
            events: &events,
            revision_id: &RevisionId::new("review-unit:sha256:one"),
            track_filter: None,
            status_filter: None,
            include_body: false,
        })
        .unwrap();

        assert_eq!(views.len(), 1);
        assert_eq!(views[0].id.as_str(), "validation:sha256:one");
    }

    #[test]
    fn project_validation_checks_selects_stable_representative_for_duplicate_ids() {
        let events = vec![
            validation_event(
                "evt:sha256:0002",
                "review-unit:sha256:one",
                "validation:sha256:same",
                "2026-05-10T00:00:00Z",
                None,
            ),
            validation_event(
                "evt:sha256:0001",
                "review-unit:sha256:one",
                "validation:sha256:same",
                "2026-05-10T00:00:00Z",
                None,
            ),
        ];
        let dir = tempfile::tempdir().unwrap();
        let backend = StoreBackend::Local(dir.path().to_path_buf());

        let views = project_validation_checks(ValidationCheckProjectionOptions {
            backend: &backend,
            events: &events,
            revision_id: &RevisionId::new("review-unit:sha256:one"),
            track_filter: None,
            status_filter: None,
            include_body: false,
        })
        .unwrap();

        assert_eq!(views.len(), 1);
        assert_eq!(views[0].event_id.as_str(), "evt:sha256:0001");
    }

    #[test]
    fn project_validation_checks_sorts_by_completed_then_event_id() {
        let events = vec![
            validation_event(
                "evt:sha256:0003",
                "review-unit:sha256:one",
                "validation:sha256:third",
                "2026-05-10T00:00:03Z",
                Some("2026-05-10T00:00:03Z"),
            ),
            validation_event(
                "evt:sha256:0001",
                "review-unit:sha256:one",
                "validation:sha256:first",
                "2026-05-10T00:00:02Z",
                Some("2026-05-10T00:00:01Z"),
            ),
            validation_event(
                "evt:sha256:0002",
                "review-unit:sha256:one",
                "validation:sha256:second",
                "2026-05-10T00:00:00Z",
                None,
            ),
        ];
        let dir = tempfile::tempdir().unwrap();
        let backend = StoreBackend::Local(dir.path().to_path_buf());

        let views = project_validation_checks(ValidationCheckProjectionOptions {
            backend: &backend,
            events: &events,
            revision_id: &RevisionId::new("review-unit:sha256:one"),
            track_filter: None,
            status_filter: None,
            include_body: false,
        })
        .unwrap();

        let ids = views
            .iter()
            .map(|view| view.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                "validation:sha256:second",
                "validation:sha256:first",
                "validation:sha256:third"
            ]
        );
    }

    #[test]
    fn project_validation_checks_hydrates_summary_only_when_requested() {
        let dir = tempfile::tempdir().unwrap();
        let backend = StoreBackend::Local(dir.path().to_path_buf());
        let artifact_path = "artifacts/notes/abc.json";
        fs::create_dir_all(dir.path().join("artifacts/notes")).unwrap();
        fs::write(
            dir.path().join(artifact_path),
            r#"{"schema":"shore.note-body","version":1,"body":"artifact summary"}"#,
        )
        .unwrap();
        let events = vec![validation_event_with_summary_artifact(
            "evt:sha256:0001",
            "review-unit:sha256:one",
            "validation:sha256:one",
            artifact_path,
        )];
        let revision_id = RevisionId::new("review-unit:sha256:one");
        let options = |include_body| ValidationCheckProjectionOptions {
            backend: &backend,
            events: &events,
            revision_id: &revision_id,
            track_filter: None,
            status_filter: None,
            include_body,
        };

        let omitted = project_validation_checks(options(false)).unwrap();
        let hydrated = project_validation_checks(options(true)).unwrap();

        assert_eq!(omitted[0].summary, None);
        assert_eq!(hydrated[0].summary.as_deref(), Some("artifact summary"));
    }

    fn validation_event(
        event_id: &str,
        revision_id: &str,
        validation_check_id: &str,
        occurred_at: &str,
        completed_at: Option<&str>,
    ) -> ShoreEvent {
        validation_event_with_payload(
            event_id,
            revision_id,
            ValidationCheckRecordedPayload {
                validation_check_id: ValidationCheckId::new(validation_check_id),
                target: ValidationTarget::Revision {
                    revision_id: RevisionId::new(revision_id),
                },
                check_name: "cargo test".to_owned(),
                command: None,
                status: ValidationStatus::Passed,
                exit_code: Some(0),
                trigger: ValidationTrigger::Manual,
                source_fingerprint: None,
                summary: None,
                summary_artifact_path: None,
                summary_byte_size: None,
                summary_content_hash: None,
                started_at: None,
                completed_at: completed_at.map(str::to_owned),
                log_artifact_content_hashes: Vec::new(),
            },
            occurred_at,
        )
    }

    fn validation_event_with_summary_artifact(
        event_id: &str,
        revision_id: &str,
        validation_check_id: &str,
        artifact_path: &str,
    ) -> ShoreEvent {
        validation_event_with_payload(
            event_id,
            revision_id,
            ValidationCheckRecordedPayload {
                validation_check_id: ValidationCheckId::new(validation_check_id),
                target: ValidationTarget::Revision {
                    revision_id: RevisionId::new(revision_id),
                },
                check_name: "cargo test".to_owned(),
                command: None,
                status: ValidationStatus::Passed,
                exit_code: Some(0),
                trigger: ValidationTrigger::Manual,
                source_fingerprint: None,
                summary: None,
                summary_artifact_path: Some(artifact_path.to_owned()),
                summary_byte_size: Some(16),
                summary_content_hash: Some("sha256:artifact-summary".to_owned()),
                started_at: None,
                completed_at: None,
                log_artifact_content_hashes: Vec::new(),
            },
            "2026-05-10T00:00:00Z",
        )
    }

    fn validation_event_with_payload(
        event_id: &str,
        revision_id: &str,
        payload: ValidationCheckRecordedPayload,
        occurred_at: &str,
    ) -> ShoreEvent {
        let mut target = EventTarget::for_revision(
            JournalId::new("journal:default"),
            RevisionId::new(revision_id),
            None,
        );
        target.track_id = Some(TrackId::new("agent:codex"));
        let mut event = ShoreEvent::new(
            EventType::ValidationCheckRecorded,
            format!(
                "validation_check_recorded:{}:{}",
                payload.validation_check_id.as_str(),
                event_id
            ),
            target,
            Writer::shore_local("0.1.0"),
            payload,
            occurred_at,
        )
        .unwrap();
        event.event_id = EventId::new(event_id);
        event
    }
}
