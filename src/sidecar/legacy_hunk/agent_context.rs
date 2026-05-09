use serde::Deserialize;

use super::super::review_notes::DiagnosticLevel;
use crate::error::Result;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedAgentContext {
    pub context: AgentContext,
    pub diagnostics: Vec<SidecarDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentContext {
    pub summary: Option<String>,
    pub files: Vec<AgentFileContext>,
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
pub enum DiagnosticCode {
    InvalidRange,
    MissingAnnotationSummary,
    MissingFilePath,
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
            summary: raw.summary,
            files,
        },
        diagnostics,
    })
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
    summary: Option<String>,
    #[serde(default)]
    files: Vec<RawAgentFileContext>,
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
