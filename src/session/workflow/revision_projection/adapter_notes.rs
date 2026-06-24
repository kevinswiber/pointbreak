use std::collections::BTreeMap;

use crate::error::Result;
use crate::model::{DiffSnapshot, ResolutionStatus};
use crate::session::body_artifact::load_body_artifact;
use crate::session::event::{EventType, ImportedNoteTarget, ReviewNoteImportedPayload, ShoreEvent};
use crate::session::store::backend::StoreBackend;
use crate::sidecar::{
    ReviewNoteEntry, ReviewNoteTarget, ReviewNotesFile, ReviewNotesSidecar, resolve_notes,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterNoteView {
    pub id: String,
    pub title: String,
    pub body: Option<String>,
    pub target: Option<ImportedNoteTarget>,
    pub status: AdapterNoteStatus,
    pub file_path: String,
    pub file_old_path: Option<String>,
    pub tags: Vec<String>,
    pub confidence: Option<String>,
    pub external_source: Option<String>,
    pub author: Option<String>,
    pub created_at: Option<String>,
    pub sidecar_content_hash: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterNoteStatus {
    Exact,
    Relocated,
    FileLevel,
    Stale,
    Orphaned,
    Unresolved,
}

impl AdapterNoteStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Relocated => "relocated",
            Self::FileLevel => "file_level",
            Self::Stale => "stale",
            Self::Orphaned => "orphaned",
            Self::Unresolved => "unresolved",
        }
    }
}

pub(super) fn project_adapter_notes(
    events: &[ShoreEvent],
    backend: &StoreBackend,
    snapshot: &DiffSnapshot,
    include_body: bool,
) -> Result<Vec<AdapterNoteView>> {
    let mut payloads = Vec::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewNoteImported)
    {
        payloads.push(serde_json::from_value::<ReviewNoteImportedPayload>(
            event.payload.clone(),
        )?);
    }

    let statuses = adapter_note_statuses(snapshot, &payloads);
    let mut views = payloads
        .iter()
        .map(|payload| {
            let body = if include_body {
                adapter_note_body(backend, payload)?
            } else {
                None
            };
            Ok(AdapterNoteView {
                id: payload.note_id.clone(),
                title: payload.title.clone(),
                body,
                target: payload.target.clone(),
                status: statuses
                    .get(&payload.note_id)
                    .copied()
                    .unwrap_or(AdapterNoteStatus::Unresolved),
                file_path: payload.file_path.clone(),
                file_old_path: payload.file_old_path.clone(),
                tags: payload.tags.clone(),
                confidence: payload.confidence.clone(),
                external_source: payload.external_source.clone(),
                author: payload.author.clone(),
                created_at: payload.created_at.clone(),
                sidecar_content_hash: payload.sidecar_content_hash.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    views.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| {
                left.target
                    .as_ref()
                    .map(|target| target.start_line)
                    .cmp(&right.target.as_ref().map(|target| target.start_line))
            })
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(views)
}

fn adapter_note_statuses(
    snapshot: &DiffSnapshot,
    payloads: &[ReviewNoteImportedPayload],
) -> BTreeMap<String, AdapterNoteStatus> {
    let sidecar = ReviewNotesSidecar {
        schema: Some("shore.review-notes".to_owned()),
        version: 1,
        summary: None,
        files: payloads
            .iter()
            .map(|payload| ReviewNotesFile {
                path: payload.file_path.clone(),
                old_path: payload.file_old_path.clone(),
                summary: None,
                notes: vec![review_note_entry_from_payload(payload, None)],
            })
            .collect(),
    };
    resolve_notes(&snapshot.files, &sidecar)
        .notes
        .into_iter()
        .map(|note| {
            (
                note.id.as_str().to_owned(),
                adapter_note_status(&note.anchor.status),
            )
        })
        .collect()
}

fn review_note_entry_from_payload(
    payload: &ReviewNoteImportedPayload,
    body: Option<String>,
) -> ReviewNoteEntry {
    ReviewNoteEntry {
        id: Some(payload.note_id.clone()),
        title: Some(payload.title.clone()),
        body,
        target: payload.target.as_ref().map(imported_note_target),
        tags: payload.tags.clone(),
        confidence: payload.confidence.clone(),
        source: payload.external_source.clone(),
        author: payload.author.clone(),
        created_at: payload.created_at.clone(),
    }
}

fn imported_note_target(target: &ImportedNoteTarget) -> ReviewNoteTarget {
    ReviewNoteTarget {
        side: target.side,
        start_line: target.start_line,
        end_line: target.end_line,
    }
}

pub(super) fn adapter_note_status(status: &ResolutionStatus) -> AdapterNoteStatus {
    match status {
        ResolutionStatus::Exact => AdapterNoteStatus::Exact,
        ResolutionStatus::Relocated => AdapterNoteStatus::Relocated,
        ResolutionStatus::FileLevel => AdapterNoteStatus::FileLevel,
        ResolutionStatus::Stale => AdapterNoteStatus::Stale,
        ResolutionStatus::Orphaned => AdapterNoteStatus::Orphaned,
        ResolutionStatus::Unresolved => AdapterNoteStatus::Unresolved,
    }
}

fn adapter_note_body(
    backend: &StoreBackend,
    payload: &ReviewNoteImportedPayload,
) -> Result<Option<String>> {
    if payload.body.is_some() {
        return Ok(payload.body.clone());
    }
    match payload.body_artifact_path.as_deref() {
        Some(path) => load_body_artifact(backend, path),
        None => Ok(None),
    }
}
