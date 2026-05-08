use serde::Deserialize;

use crate::error::Result;
use crate::model::{
    Anchor, Annotation, AnnotationId, DiffFile, LineRange, ResolutionStatus, Side,
    hash_normalized_lines, rows_for_line_range,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedAgentContext {
    pub context: AgentContext,
    pub diagnostics: Vec<SidecarDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentContext {
    pub schema: Option<String>,
    pub version: u32,
    pub summary: Option<String>,
    pub ownership: Option<Ownership>,
    pub files: Vec<AgentFileContext>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ownership {
    pub kind: Option<String>,
    pub tool: Option<String>,
    pub author: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentFileContext {
    pub path: String,
    pub old_path: Option<String>,
    pub summary: Option<String>,
    pub annotations: Vec<AgentAnnotation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentAnnotation {
    pub id: Option<String>,
    pub old_range: Option<Range>,
    pub new_range: Option<Range>,
    pub summary: Option<String>,
    pub rationale: Option<String>,
    pub tags: Vec<String>,
    pub confidence: Option<String>,
    pub source: Option<String>,
    pub author: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Range {
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SidecarDiagnostic {
    pub level: DiagnosticLevel,
    pub code: DiagnosticCode,
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticLevel {
    Warning,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticCode {
    InvalidRange,
    MissingAnnotationSummary,
    MissingFilePath,
    StaleFilePath,
    UnresolvedAnnotation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderedDiffFiles {
    pub files: Vec<DiffFile>,
    pub diagnostics: Vec<SidecarDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedSidecarAnnotations {
    pub annotations: Vec<Annotation>,
    pub diagnostics: Vec<SidecarDiagnostic>,
}

impl From<Range> for LineRange {
    fn from(range: Range) -> Self {
        Self::new(range.start, range.end)
    }
}

pub fn parse_agent_context(json: &str) -> Result<ParsedAgentContext> {
    let raw = serde_json::from_str::<RawAgentContext>(json)?;
    let mut diagnostics = Vec::new();
    let files = raw
        .files
        .into_iter()
        .enumerate()
        .map(|(file_index, file)| normalize_file(file_index, file, &mut diagnostics))
        .collect();

    Ok(ParsedAgentContext {
        context: AgentContext {
            schema: raw.schema,
            version: raw.version,
            summary: raw.summary,
            ownership: raw.ownership.map(Into::into),
            files,
        },
        diagnostics,
    })
}

pub fn apply_file_order(files: Vec<DiffFile>, context: &AgentContext) -> OrderedDiffFiles {
    let mut pending = files.into_iter().map(Some).collect::<Vec<_>>();
    let mut ordered = Vec::new();
    let mut diagnostics = Vec::new();

    for (file_index, sidecar_file) in context.files.iter().enumerate() {
        if sidecar_file.path.is_empty() {
            continue;
        }

        if let Some(position) = pending.iter().position(|file| {
            file.as_ref()
                .is_some_and(|file| matches_sidecar_file(file, sidecar_file))
        }) {
            ordered.push(pending[position].take().expect("matched file is present"));
        } else {
            diagnostics.push(SidecarDiagnostic {
                level: DiagnosticLevel::Warning,
                code: DiagnosticCode::StaleFilePath,
                path: format!("files[{file_index}].path"),
                message: format!(
                    "sidecar file path does not match any diff file: {}",
                    sidecar_file.path
                ),
            });
        }
    }

    ordered.extend(pending.into_iter().flatten());

    OrderedDiffFiles {
        files: ordered,
        diagnostics,
    }
}

pub fn resolve_annotations(
    files: &[DiffFile],
    context: &AgentContext,
) -> ResolvedSidecarAnnotations {
    let mut annotations = Vec::new();
    let mut diagnostics = Vec::new();

    for (file_index, sidecar_file) in context.files.iter().enumerate() {
        if sidecar_file.path.is_empty() {
            continue;
        }

        let Some(file) = files
            .iter()
            .find(|file| matches_sidecar_file(file, sidecar_file))
        else {
            diagnostics.push(SidecarDiagnostic {
                level: DiagnosticLevel::Warning,
                code: DiagnosticCode::StaleFilePath,
                path: format!("files[{file_index}].path"),
                message: format!(
                    "sidecar file path does not match any diff file: {}",
                    sidecar_file.path
                ),
            });
            continue;
        };

        for (annotation_index, sidecar_annotation) in sidecar_file.annotations.iter().enumerate() {
            let ranges = [
                (Side::Old, sidecar_annotation.old_range),
                (Side::New, sidecar_annotation.new_range),
            ];
            let range_count = ranges.iter().filter(|(_, range)| range.is_some()).count();

            if range_count == 0 {
                diagnostics.push(unresolved_annotation_diagnostic(
                    file_index,
                    annotation_index,
                    None,
                ));
                continue;
            }

            for (side, range) in ranges
                .into_iter()
                .filter_map(|(side, range)| range.map(|range| (side, range)))
            {
                if let Some(anchor) = resolve_anchor(file, side, range) {
                    annotations.push(model_annotation(
                        sidecar_annotation,
                        file_index,
                        annotation_index,
                        side,
                        range_count > 1,
                        anchor,
                    ));
                } else {
                    diagnostics.push(unresolved_annotation_diagnostic(
                        file_index,
                        annotation_index,
                        Some(side),
                    ));
                }
            }
        }
    }

    ResolvedSidecarAnnotations {
        annotations,
        diagnostics,
    }
}

fn resolve_anchor(file: &DiffFile, side: Side, range: Range) -> Option<Anchor> {
    file.hunks.iter().find_map(|hunk| {
        let line_range = LineRange::from(range);
        let rows = rows_for_line_range(&hunk.rows, side, &line_range)?;
        Some(Anchor {
            file_id: file.id.clone(),
            side,
            line_range,
            hunk_signature: hunk.signature(),
            target_text_hash: hash_normalized_lines(rows.iter().map(|row| row.text.as_str())),
            status: ResolutionStatus::Exact,
        })
    })
}

fn model_annotation(
    sidecar_annotation: &AgentAnnotation,
    file_index: usize,
    annotation_index: usize,
    side: Side,
    multi_side: bool,
    anchor: Anchor,
) -> Annotation {
    Annotation {
        id: model_annotation_id(
            sidecar_annotation,
            file_index,
            annotation_index,
            side,
            multi_side,
        ),
        anchor,
        source: crate::model::AnnotationSource::Sidecar,
        summary: sidecar_annotation.summary.clone().unwrap_or_default(),
        rationale: sidecar_annotation.rationale.clone(),
        tags: sidecar_annotation.tags.clone(),
        confidence: sidecar_annotation.confidence.clone(),
        external_source: sidecar_annotation.source.clone(),
        author: sidecar_annotation.author.clone(),
        created_at: sidecar_annotation.created_at.clone(),
    }
}

fn model_annotation_id(
    sidecar_annotation: &AgentAnnotation,
    file_index: usize,
    annotation_index: usize,
    side: Side,
    multi_side: bool,
) -> AnnotationId {
    let mut id = sidecar_annotation
        .id
        .clone()
        .unwrap_or_else(|| format!("sidecar:{file_index}:{annotation_index}"));
    if multi_side {
        id.push(':');
        id.push_str(match side {
            Side::Old => "old",
            Side::New => "new",
        });
    }
    AnnotationId::new(id)
}

fn unresolved_annotation_diagnostic(
    file_index: usize,
    annotation_index: usize,
    side: Option<Side>,
) -> SidecarDiagnostic {
    let range_field = match side {
        Some(Side::Old) => ".oldRange",
        Some(Side::New) => ".newRange",
        None => "",
    };
    SidecarDiagnostic {
        level: DiagnosticLevel::Warning,
        code: DiagnosticCode::UnresolvedAnnotation,
        path: format!("files[{file_index}].annotations[{annotation_index}]{range_field}"),
        message: "annotation range does not resolve to diff rows".to_owned(),
    }
}

fn matches_sidecar_file(file: &DiffFile, sidecar_file: &AgentFileContext) -> bool {
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
    raw: RawAgentFileContext,
    diagnostics: &mut Vec<SidecarDiagnostic>,
) -> AgentFileContext {
    let path = raw.path.unwrap_or_default();
    if path.is_empty() {
        diagnostics.push(SidecarDiagnostic {
            level: DiagnosticLevel::Warning,
            code: DiagnosticCode::MissingFilePath,
            path: format!("files[{file_index}].path"),
            message: "file context is missing path".to_owned(),
        });
    }
    let annotations = raw
        .annotations
        .into_iter()
        .enumerate()
        .map(|(annotation_index, annotation)| {
            normalize_annotation(file_index, annotation_index, annotation, diagnostics)
        })
        .collect();

    AgentFileContext {
        path,
        old_path: raw.old_path,
        summary: raw.summary,
        annotations,
    }
}

fn normalize_annotation(
    file_index: usize,
    annotation_index: usize,
    raw: RawAgentAnnotation,
    diagnostics: &mut Vec<SidecarDiagnostic>,
) -> AgentAnnotation {
    let old_range = normalize_range(
        raw.old_range,
        format!("files[{file_index}].annotations[{annotation_index}].oldRange"),
        diagnostics,
    );
    let new_range = normalize_range(
        raw.new_range,
        format!("files[{file_index}].annotations[{annotation_index}].newRange"),
        diagnostics,
    );

    if raw
        .summary
        .as_ref()
        .is_none_or(|summary| summary.is_empty())
    {
        diagnostics.push(SidecarDiagnostic {
            level: DiagnosticLevel::Warning,
            code: DiagnosticCode::MissingAnnotationSummary,
            path: format!("files[{file_index}].annotations[{annotation_index}].summary"),
            message: "annotation is missing summary".to_owned(),
        });
    }

    AgentAnnotation {
        id: raw.id,
        old_range,
        new_range,
        summary: raw.summary,
        rationale: raw.rationale,
        tags: raw.tags,
        confidence: raw.confidence,
        source: raw.source,
        author: raw.author,
        created_at: raw.created_at,
    }
}

fn normalize_range(
    raw: Option<[u32; 2]>,
    path: String,
    diagnostics: &mut Vec<SidecarDiagnostic>,
) -> Option<Range> {
    let [start, end] = raw?;
    if start == 0 || end == 0 || end < start {
        diagnostics.push(SidecarDiagnostic {
            level: DiagnosticLevel::Warning,
            code: DiagnosticCode::InvalidRange,
            path,
            message: "range must be a 1-based inclusive tuple with end >= start".to_owned(),
        });
        return None;
    }
    Some(Range { start, end })
}

#[derive(Debug, Deserialize)]
struct RawAgentContext {
    schema: Option<String>,
    version: u32,
    summary: Option<String>,
    ownership: Option<RawOwnership>,
    #[serde(default)]
    files: Vec<RawAgentFileContext>,
}

#[derive(Debug, Deserialize)]
struct RawOwnership {
    kind: Option<String>,
    tool: Option<String>,
    author: Option<String>,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
}

impl From<RawOwnership> for Ownership {
    fn from(raw: RawOwnership) -> Self {
        Self {
            kind: raw.kind,
            tool: raw.tool,
            author: raw.author,
            created_at: raw.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawAgentFileContext {
    path: Option<String>,
    #[serde(rename = "oldPath")]
    old_path: Option<String>,
    summary: Option<String>,
    #[serde(default)]
    annotations: Vec<RawAgentAnnotation>,
}

#[derive(Debug, Deserialize)]
struct RawAgentAnnotation {
    id: Option<String>,
    #[serde(rename = "oldRange")]
    old_range: Option<[u32; 2]>,
    #[serde(rename = "newRange")]
    new_range: Option<[u32; 2]>,
    summary: Option<String>,
    rationale: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    confidence: Option<String>,
    source: Option<String>,
    author: Option<String>,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
}
