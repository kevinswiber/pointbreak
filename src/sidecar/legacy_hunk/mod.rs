use agent_context::{
    AgentAnnotation, AgentContext, DiagnosticCode, Range, SidecarDiagnostic, parse_agent_context,
};

use super::review_notes::{
    ParsedReviewNotes, ReviewNoteEntry, ReviewNoteTarget, ReviewNotesDiagnostic,
    ReviewNotesDiagnosticCode, ReviewNotesFile, ReviewNotesSidecar,
};
use crate::error::Result;
use crate::model::Side;

mod agent_context;

pub fn parse_hunk_agent_context(json: &str) -> Result<ParsedReviewNotes> {
    let parsed = parse_agent_context(json)?;
    Ok(ParsedReviewNotes {
        sidecar: convert_context(parsed.context),
        diagnostics: parsed
            .diagnostics
            .into_iter()
            .map(convert_diagnostic)
            .collect(),
    })
}

fn convert_context(context: AgentContext) -> ReviewNotesSidecar {
    ReviewNotesSidecar {
        schema: Some("shore.review-notes".to_owned()),
        version: 1,
        summary: context.summary,
        files: context.files.into_iter().map(Into::into).collect(),
    }
}

impl From<agent_context::AgentFileContext> for ReviewNotesFile {
    fn from(file: agent_context::AgentFileContext) -> Self {
        Self {
            path: file.path,
            old_path: file.old_path,
            summary: file.summary,
            notes: file
                .annotations
                .into_iter()
                .flat_map(convert_annotation)
                .collect(),
        }
    }
}

fn convert_annotation(annotation: AgentAnnotation) -> Vec<ReviewNoteEntry> {
    let ranges = [
        (Side::Old, annotation.old_range),
        (Side::New, annotation.new_range),
    ];
    let range_count = ranges.iter().filter(|(_, range)| range.is_some()).count();
    let notes = ranges
        .into_iter()
        .filter_map(|(side, range)| range.map(|range| (side, range)))
        .map(|(side, range)| note_for_range(&annotation, side, range, range_count > 1))
        .collect::<Vec<_>>();

    if notes.is_empty() {
        vec![note_without_target(annotation)]
    } else {
        notes
    }
}

fn note_for_range(
    annotation: &AgentAnnotation,
    side: Side,
    range: Range,
    multi_side: bool,
) -> ReviewNoteEntry {
    let mut id = annotation.id.clone();
    if multi_side && let Some(id) = id.as_mut() {
        id.push(':');
        id.push_str(match side {
            Side::Old => "old",
            Side::New => "new",
        });
    }

    ReviewNoteEntry {
        id,
        title: annotation.summary.clone(),
        body: annotation.rationale.clone(),
        target: Some(ReviewNoteTarget {
            side,
            start_line: range.start,
            end_line: range.end,
        }),
        tags: annotation.tags.clone(),
        confidence: annotation.confidence.clone(),
        source: annotation.source.clone(),
        author: annotation.author.clone(),
        created_at: annotation.created_at.clone(),
    }
}

fn note_without_target(annotation: AgentAnnotation) -> ReviewNoteEntry {
    ReviewNoteEntry {
        id: annotation.id,
        title: annotation.summary,
        body: annotation.rationale,
        target: None,
        tags: annotation.tags,
        confidence: annotation.confidence,
        source: annotation.source,
        author: annotation.author,
        created_at: annotation.created_at,
    }
}

fn convert_diagnostic(diagnostic: SidecarDiagnostic) -> ReviewNotesDiagnostic {
    ReviewNotesDiagnostic {
        level: diagnostic.level,
        code: match diagnostic.code {
            DiagnosticCode::InvalidRange => ReviewNotesDiagnosticCode::InvalidRange,
            DiagnosticCode::MissingAnnotationSummary => ReviewNotesDiagnosticCode::MissingNoteTitle,
            DiagnosticCode::MissingFilePath => ReviewNotesDiagnosticCode::MissingFilePath,
        },
        path: convert_path(&diagnostic.path),
        message: diagnostic.message,
    }
}

fn convert_path(path: &str) -> String {
    // This remap is tied to the finite diagnostic path shapes emitted by
    // `legacy_hunk::agent_context`: file paths and annotation oldRange/newRange/summary fields.
    // If new legacy diagnostics are added, update this mapping with the new path shape.
    path.replace(".annotations[", ".notes[")
        .replace("annotations[", "notes[")
        .replace(".newRange", ".target")
        .replace(".oldRange", ".target")
        .replace(".summary", ".title")
}
