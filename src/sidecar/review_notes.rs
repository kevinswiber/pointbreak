use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::model::{
    Anchor, DiffFile, FileId, LineRange, ResolutionStatus, ReviewNote, ReviewNoteId,
    ReviewNoteSource, Side, hash_normalized_lines, rows_for_line_range,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedReviewNotes {
    pub sidecar: ReviewNotesSidecar,
    pub diagnostics: Vec<ReviewNotesDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewNotesSidecar {
    pub schema: Option<String>,
    pub version: u32,
    pub summary: Option<String>,
    pub files: Vec<ReviewNotesFile>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewNotesFile {
    pub path: String,
    pub old_path: Option<String>,
    pub summary: Option<String>,
    pub notes: Vec<ReviewNoteEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewNoteEntry {
    pub id: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub target: Option<ReviewNoteTarget>,
    pub tags: Vec<String>,
    pub confidence: Option<String>,
    pub source: Option<String>,
    pub author: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReviewNoteTarget {
    pub side: Side,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLevel {
    Warning,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewNotesDiagnostic {
    pub level: DiagnosticLevel,
    pub code: ReviewNotesDiagnosticCode,
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewNotesDiagnosticCode {
    InvalidSchema,
    InvalidRange,
    MissingFilePath,
    MissingNoteTarget,
    MissingNoteTitle,
    MissingNotes,
    MissingVersion,
    StaleFilePath,
    UnresolvedNote,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedReviewNotes {
    pub notes: Vec<ReviewNote>,
    pub diagnostics: Vec<ReviewNotesDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderedReviewNoteFiles {
    pub files: Vec<DiffFile>,
    pub diagnostics: Vec<ReviewNotesDiagnostic>,
}

pub fn parse_review_notes_sidecar(json: &str) -> Result<ParsedReviewNotes> {
    let raw = serde_json::from_str::<RawReviewNotesSidecar>(json)?;
    let mut diagnostics = Vec::new();
    if raw
        .schema
        .as_deref()
        .is_some_and(|schema| schema != "shore.review-notes")
    {
        diagnostics.push(ReviewNotesDiagnostic {
            level: DiagnosticLevel::Warning,
            code: ReviewNotesDiagnosticCode::InvalidSchema,
            path: "schema".to_owned(),
            message: "review notes sidecar schema must be shore.review-notes".to_owned(),
        });
    }
    if raw.version.is_none() {
        diagnostics.push(ReviewNotesDiagnostic {
            level: DiagnosticLevel::Warning,
            code: ReviewNotesDiagnosticCode::MissingVersion,
            path: "version".to_owned(),
            message: "review notes sidecar is missing version".to_owned(),
        });
    }
    let files = raw
        .files
        .into_iter()
        .enumerate()
        .map(|(file_index, file)| normalize_file(file_index, file, &mut diagnostics))
        .collect();

    Ok(ParsedReviewNotes {
        sidecar: ReviewNotesSidecar {
            schema: raw.schema,
            version: raw.version.unwrap_or_default(),
            summary: raw.summary,
            files,
        },
        diagnostics,
    })
}

pub fn resolve_notes(files: &[DiffFile], sidecar: &ReviewNotesSidecar) -> ResolvedReviewNotes {
    let mut notes = Vec::new();
    let mut diagnostics = Vec::new();

    for (file_index, sidecar_file) in sidecar.files.iter().enumerate() {
        if sidecar_file.path.is_empty() {
            continue;
        }

        let Some(file) = files
            .iter()
            .find(|file| matches_review_notes_file(file, sidecar_file))
        else {
            if sidecar_file.notes.is_empty() {
                diagnostics.push(review_notes_stale_file_diagnostic(
                    file_index,
                    &sidecar_file.path,
                ));
                continue;
            }

            for (note_index, note) in sidecar_file.notes.iter().enumerate() {
                let Some(target) = note.target else {
                    continue;
                };

                notes.push(model_note(
                    note,
                    file_index,
                    note_index,
                    synthesize_orphaned_anchor(sidecar_file, target),
                ));
            }
            continue;
        };

        for (note_index, note) in sidecar_file.notes.iter().enumerate() {
            let Some(target) = note.target else {
                continue;
            };

            let anchor = resolve_anchor(file, target)
                .unwrap_or_else(|| synthesize_stale_anchor(file, target));
            notes.push(model_note(note, file_index, note_index, anchor));
        }
    }

    ResolvedReviewNotes { notes, diagnostics }
}

pub fn apply_review_notes_file_order(
    files: Vec<DiffFile>,
    sidecar: &ReviewNotesSidecar,
) -> OrderedReviewNoteFiles {
    let mut pending = files.into_iter().map(Some).collect::<Vec<_>>();
    let mut ordered = Vec::new();
    let mut diagnostics = Vec::new();

    for (file_index, sidecar_file) in sidecar.files.iter().enumerate() {
        if sidecar_file.path.is_empty() {
            continue;
        }

        if let Some(position) = pending.iter().position(|file| {
            file.as_ref()
                .is_some_and(|file| matches_review_notes_file(file, sidecar_file))
        }) {
            ordered.push(pending[position].take().expect("matched file is present"));
        } else {
            diagnostics.push(review_notes_stale_file_diagnostic(
                file_index,
                &sidecar_file.path,
            ));
        }
    }

    ordered.extend(pending.into_iter().flatten());

    OrderedReviewNoteFiles {
        files: ordered,
        diagnostics,
    }
}

fn resolve_anchor(file: &DiffFile, target: ReviewNoteTarget) -> Option<Anchor> {
    file.hunks.iter().find_map(|hunk| {
        let line_range = LineRange::new(target.start_line, target.end_line);
        let rows = rows_for_line_range(&hunk.rows, target.side, &line_range)?;
        Some(Anchor {
            file_id: file.id.clone(),
            side: target.side,
            line_range,
            hunk_signature: hunk.signature(),
            target_text_hash: hash_normalized_lines(rows.iter().map(|row| row.text.as_str())),
            status: ResolutionStatus::Exact,
        })
    })
}

fn model_note(
    note: &ReviewNoteEntry,
    file_index: usize,
    note_index: usize,
    anchor: Anchor,
) -> ReviewNote {
    ReviewNote {
        id: note
            .id
            .clone()
            .map(ReviewNoteId::new)
            .unwrap_or_else(|| ReviewNoteId::new(format!("note:{file_index}:{note_index}"))),
        anchor,
        source: ReviewNoteSource::Sidecar,
        title: note.title.clone().unwrap_or_default(),
        body: note.body.clone(),
        tags: note.tags.clone(),
        confidence: note.confidence.clone(),
        external_source: note.source.clone(),
        author: note.author.clone(),
        created_at: note.created_at.clone(),
    }
}

fn synthesize_stale_anchor(file: &DiffFile, target: ReviewNoteTarget) -> Anchor {
    Anchor {
        file_id: file.id.clone(),
        side: target.side,
        line_range: LineRange::new(target.start_line, target.end_line),
        hunk_signature: "hunk:stale".to_owned(),
        target_text_hash: hash_normalized_lines(std::iter::empty::<&str>()),
        status: ResolutionStatus::Stale,
    }
}

fn synthesize_orphaned_anchor(sidecar_file: &ReviewNotesFile, target: ReviewNoteTarget) -> Anchor {
    Anchor {
        file_id: FileId::new(sidecar_file.path.clone()),
        side: target.side,
        line_range: LineRange::new(target.start_line, target.end_line),
        hunk_signature: "hunk:orphaned".to_owned(),
        target_text_hash: hash_normalized_lines(std::iter::empty::<&str>()),
        status: ResolutionStatus::Orphaned,
    }
}

fn review_notes_stale_file_diagnostic(file_index: usize, path: &str) -> ReviewNotesDiagnostic {
    ReviewNotesDiagnostic {
        level: DiagnosticLevel::Warning,
        code: ReviewNotesDiagnosticCode::StaleFilePath,
        path: format!("files[{file_index}].path"),
        message: format!("review notes file path does not match any diff file: {path}"),
    }
}

fn matches_review_notes_file(file: &DiffFile, sidecar_file: &ReviewNotesFile) -> bool {
    matches_diff_path(file, &sidecar_file.path)
        || sidecar_file
            .old_path
            .as_deref()
            .is_some_and(|old_path| matches_diff_path(file, old_path))
}

fn matches_diff_path(file: &DiffFile, path: &str) -> bool {
    file.new_path.as_deref() == Some(path) || file.old_path.as_deref() == Some(path)
}

fn normalize_file(
    file_index: usize,
    raw: RawReviewNotesFile,
    diagnostics: &mut Vec<ReviewNotesDiagnostic>,
) -> ReviewNotesFile {
    let path = raw.path.unwrap_or_default();
    if path.is_empty() {
        diagnostics.push(ReviewNotesDiagnostic {
            level: DiagnosticLevel::Warning,
            code: ReviewNotesDiagnosticCode::MissingFilePath,
            path: format!("files[{file_index}].path"),
            message: "review notes file is missing path".to_owned(),
        });
    }

    let notes = match raw.notes {
        Some(notes) => notes
            .into_iter()
            .enumerate()
            .map(|(note_index, note)| normalize_note(file_index, note_index, note, diagnostics))
            .collect(),
        None => {
            diagnostics.push(ReviewNotesDiagnostic {
                level: DiagnosticLevel::Warning,
                code: ReviewNotesDiagnosticCode::MissingNotes,
                path: format!("files[{file_index}].notes"),
                message: "review notes file is missing notes".to_owned(),
            });
            Vec::new()
        }
    };

    ReviewNotesFile {
        path,
        old_path: raw.old_path,
        summary: raw.summary,
        notes,
    }
}

fn normalize_note(
    file_index: usize,
    note_index: usize,
    raw: RawReviewNoteEntry,
    diagnostics: &mut Vec<ReviewNotesDiagnostic>,
) -> ReviewNoteEntry {
    if raw.title.as_ref().is_none_or(|title| title.is_empty()) {
        diagnostics.push(ReviewNotesDiagnostic {
            level: DiagnosticLevel::Warning,
            code: ReviewNotesDiagnosticCode::MissingNoteTitle,
            path: format!("files[{file_index}].notes[{note_index}].title"),
            message: "review note is missing title".to_owned(),
        });
    }

    let target = normalize_target(
        file_index,
        note_index,
        raw.target,
        format!("files[{file_index}].notes[{note_index}].target"),
        diagnostics,
    );

    ReviewNoteEntry {
        id: raw.id,
        title: raw.title,
        body: raw.body,
        target,
        tags: raw.tags,
        confidence: raw.confidence,
        source: raw.source,
        author: raw.author,
        created_at: raw.created_at,
    }
}

fn normalize_target(
    file_index: usize,
    note_index: usize,
    raw: Option<RawReviewNoteTarget>,
    path: String,
    diagnostics: &mut Vec<ReviewNotesDiagnostic>,
) -> Option<ReviewNoteTarget> {
    let Some(raw) = raw else {
        diagnostics.push(ReviewNotesDiagnostic {
            level: DiagnosticLevel::Warning,
            code: ReviewNotesDiagnosticCode::MissingNoteTarget,
            path,
            message: "review note is missing target".to_owned(),
        });
        return None;
    };

    let Some(side) = raw.side else {
        diagnostics.push(ReviewNotesDiagnostic {
            level: DiagnosticLevel::Warning,
            code: ReviewNotesDiagnosticCode::MissingNoteTarget,
            path: format!("files[{file_index}].notes[{note_index}].target.side"),
            message: "review note target is missing side".to_owned(),
        });
        return None;
    };

    let Some(start_line) = raw.start_line else {
        diagnostics.push(ReviewNotesDiagnostic {
            level: DiagnosticLevel::Warning,
            code: ReviewNotesDiagnosticCode::MissingNoteTarget,
            path: format!("files[{file_index}].notes[{note_index}].target.startLine"),
            message: "review note target is missing startLine".to_owned(),
        });
        return None;
    };

    let Some(end_line) = raw.end_line else {
        diagnostics.push(ReviewNotesDiagnostic {
            level: DiagnosticLevel::Warning,
            code: ReviewNotesDiagnosticCode::MissingNoteTarget,
            path: format!("files[{file_index}].notes[{note_index}].target.endLine"),
            message: "review note target is missing endLine".to_owned(),
        });
        return None;
    };

    if start_line == 0 || end_line == 0 || end_line < start_line {
        diagnostics.push(ReviewNotesDiagnostic {
            level: DiagnosticLevel::Warning,
            code: ReviewNotesDiagnosticCode::InvalidRange,
            path,
            message: "target must be a 1-based inclusive range with endLine >= startLine"
                .to_owned(),
        });
        return None;
    }

    Some(ReviewNoteTarget {
        side,
        start_line,
        end_line,
    })
}

#[derive(Debug, Deserialize)]
struct RawReviewNotesSidecar {
    schema: Option<String>,
    version: Option<u32>,
    summary: Option<String>,
    #[serde(default)]
    files: Vec<RawReviewNotesFile>,
}

#[derive(Debug, Deserialize)]
struct RawReviewNotesFile {
    path: Option<String>,
    #[serde(rename = "oldPath")]
    old_path: Option<String>,
    summary: Option<String>,
    notes: Option<Vec<RawReviewNoteEntry>>,
}

#[derive(Debug, Deserialize)]
struct RawReviewNoteEntry {
    id: Option<String>,
    title: Option<String>,
    body: Option<String>,
    target: Option<RawReviewNoteTarget>,
    #[serde(default)]
    tags: Vec<String>,
    confidence: Option<String>,
    source: Option<String>,
    author: Option<String>,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawReviewNoteTarget {
    side: Option<Side>,
    #[serde(rename = "startLine")]
    start_line: Option<u32>,
    #[serde(rename = "endLine")]
    end_line: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiffRow, DiffRowKind, FileId, FileStatus, HunkId, ReviewHunk};

    #[test]
    fn resolve_notes_emits_stale_anchor_when_file_exists_but_line_range_misses() {
        let files = vec![file_with_one_hunk_at_new_line(1, "line one")];
        let sidecar = ReviewNotesSidecar {
            schema: Some("shore.review-notes".to_owned()),
            version: 1,
            summary: None,
            files: vec![ReviewNotesFile {
                path: "src/lib.rs".to_owned(),
                old_path: None,
                summary: None,
                notes: vec![review_note("note:stale", "Stale", Side::New, 99, 99)],
            }],
        };

        let resolved = resolve_notes(&files, &sidecar);

        assert_eq!(
            resolved.notes.len(),
            1,
            "stale notes are preserved, not dropped"
        );
        assert_eq!(resolved.notes[0].anchor.status, ResolutionStatus::Stale);
        assert_eq!(resolved.notes[0].anchor.hunk_signature, "hunk:stale");
        assert!(
            resolved.diagnostics.is_empty(),
            "stale anchors should not also emit unresolved diagnostics; got {:?}",
            resolved.diagnostics
        );
    }

    #[test]
    fn resolve_notes_emits_orphaned_anchor_when_file_no_longer_in_diff() {
        let files: Vec<DiffFile> = Vec::new();
        let sidecar = ReviewNotesSidecar {
            schema: Some("shore.review-notes".to_owned()),
            version: 1,
            summary: None,
            files: vec![ReviewNotesFile {
                path: "src/gone.rs".to_owned(),
                old_path: None,
                summary: None,
                notes: vec![review_note("note:orphan", "Orphan", Side::New, 1, 1)],
            }],
        };

        let resolved = resolve_notes(&files, &sidecar);

        assert_eq!(resolved.notes.len(), 1);
        assert_eq!(resolved.notes[0].anchor.status, ResolutionStatus::Orphaned);
        assert!(
            resolved.diagnostics.is_empty(),
            "orphaned anchors should not also emit stale path diagnostics; got {:?}",
            resolved.diagnostics
        );
    }

    #[test]
    fn resolve_notes_keeps_existing_exact_match_behavior() {
        let files = vec![file_with_one_hunk_at_new_line(1, "line one")];
        let sidecar = ReviewNotesSidecar {
            schema: Some("shore.review-notes".to_owned()),
            version: 1,
            summary: None,
            files: vec![ReviewNotesFile {
                path: "src/lib.rs".to_owned(),
                old_path: None,
                summary: None,
                notes: vec![review_note("note:exact", "Exact", Side::New, 1, 1)],
            }],
        };

        let resolved = resolve_notes(&files, &sidecar);

        assert_eq!(resolved.notes.len(), 1);
        assert_eq!(resolved.notes[0].anchor.status, ResolutionStatus::Exact);
    }

    fn review_note(
        id: &str,
        title: &str,
        side: Side,
        start_line: u32,
        end_line: u32,
    ) -> ReviewNoteEntry {
        ReviewNoteEntry {
            id: Some(id.to_owned()),
            title: Some(title.to_owned()),
            body: None,
            target: Some(ReviewNoteTarget {
                side,
                start_line,
                end_line,
            }),
            tags: Vec::new(),
            confidence: None,
            source: None,
            author: None,
            created_at: None,
        }
    }

    fn file_with_one_hunk_at_new_line(new_line: u32, text: &str) -> DiffFile {
        DiffFile {
            id: FileId::new("src/lib.rs"),
            status: FileStatus::Modified,
            old_path: Some("src/lib.rs".to_owned()),
            new_path: Some("src/lib.rs".to_owned()),
            old_mode: None,
            new_mode: None,
            old_oid: None,
            new_oid: None,
            similarity: None,
            is_binary: false,
            is_submodule: false,
            is_mode_only: false,
            synthetic: false,
            metadata_rows: Vec::new(),
            hunks: vec![ReviewHunk {
                id: HunkId::new("src/lib.rs:1:1"),
                header: "@@ -1,0 +1,1 @@".to_owned(),
                old_start: 1,
                old_lines: 0,
                new_start: 1,
                new_lines: 1,
                rows: vec![DiffRow {
                    kind: DiffRowKind::Added,
                    old_line: None,
                    new_line: Some(new_line),
                    text: text.to_owned(),
                }],
            }],
        }
    }
}
