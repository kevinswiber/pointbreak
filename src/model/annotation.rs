use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{AnnotationId, DiffFile, DiffRow, FileId, ReviewHunk};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub id: AnnotationId,
    pub anchor: Anchor,
    pub source: AnnotationSource,
    pub summary: String,
    pub rationale: Option<String>,
    pub tags: Vec<String>,
    pub confidence: Option<String>,
    pub external_source: Option<String>,
    pub author: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationSource {
    Sidecar,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Old,
    New,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

impl LineRange {
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionStatus {
    Exact,
    Relocated,
    FileLevel,
    Stale,
    Orphaned,
    Unresolved,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub file_id: FileId,
    pub side: Side,
    pub line_range: LineRange,
    pub hunk_signature: String,
    pub target_text_hash: String,
    pub status: ResolutionStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnchorResolution {
    pub anchor: Anchor,
    pub reason: AnchorResolutionReason,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorResolutionReason {
    ExactMatch,
    SameHunkTargetText,
    SameFileTargetText,
    FileLevelTargetMissing,
    StaleTargetMissing,
    OrphanedFileMissing,
    AmbiguousTargetText,
}

pub fn re_resolve_annotations(annotations: &[Annotation], files: &[DiffFile]) -> Vec<Annotation> {
    annotations
        .iter()
        .map(|annotation| annotation.re_resolve(files))
        .collect()
}

impl Annotation {
    pub fn re_resolve(&self, files: &[DiffFile]) -> Self {
        let mut annotation = self.clone();
        annotation.anchor = self.anchor.re_resolve(files).anchor;
        annotation
    }
}

impl Anchor {
    pub fn re_resolve(&self, files: &[DiffFile]) -> AnchorResolution {
        let Some(file) = files
            .iter()
            .find(|file| matches_anchor_file(file, &self.file_id))
        else {
            return self.resolved_as(
                self.clone(),
                ResolutionStatus::Orphaned,
                AnchorResolutionReason::OrphanedFileMissing,
            );
        };

        let same_hunk_matches = file
            .hunks
            .iter()
            .filter(|hunk| hunk.signature() == self.hunk_signature)
            .collect::<Vec<_>>();

        for hunk in &same_hunk_matches {
            if let Some(rows) = rows_for_line_range(&hunk.rows, self.side, &self.line_range)
                && hash_rows(&rows) == self.target_text_hash
            {
                return self.resolved_as(
                    self.anchor_for_match(
                        file,
                        AnchorMatch {
                            line_range: self.line_range.clone(),
                            hunk_signature: hunk.signature(),
                        },
                        ResolutionStatus::Exact,
                    ),
                    ResolutionStatus::Exact,
                    AnchorResolutionReason::ExactMatch,
                );
            }
        }

        let same_hunk_target_matches = same_hunk_matches
            .iter()
            .flat_map(|hunk| target_matches_in_hunk(hunk, self))
            .collect::<Vec<_>>();
        match same_hunk_target_matches.as_slice() {
            [target_match] => {
                return self.resolved_as(
                    self.anchor_for_match(file, target_match.clone(), ResolutionStatus::Relocated),
                    ResolutionStatus::Relocated,
                    AnchorResolutionReason::SameHunkTargetText,
                );
            }
            [] => {}
            _ => {
                return self.resolved_as(
                    self.anchor_in_file(file, ResolutionStatus::Unresolved),
                    ResolutionStatus::Unresolved,
                    AnchorResolutionReason::AmbiguousTargetText,
                );
            }
        }

        let same_file_target_matches = file
            .hunks
            .iter()
            .flat_map(|hunk| target_matches_in_hunk(hunk, self))
            .collect::<Vec<_>>();
        match same_file_target_matches.as_slice() {
            [target_match] => {
                return self.resolved_as(
                    self.anchor_for_match(file, target_match.clone(), ResolutionStatus::Relocated),
                    ResolutionStatus::Relocated,
                    AnchorResolutionReason::SameFileTargetText,
                );
            }
            [] => {}
            _ => {
                return self.resolved_as(
                    self.anchor_in_file(file, ResolutionStatus::Unresolved),
                    ResolutionStatus::Unresolved,
                    AnchorResolutionReason::AmbiguousTargetText,
                );
            }
        }

        if same_hunk_matches.is_empty() {
            self.resolved_as(
                self.anchor_in_file(file, ResolutionStatus::FileLevel),
                ResolutionStatus::FileLevel,
                AnchorResolutionReason::FileLevelTargetMissing,
            )
        } else {
            self.resolved_as(
                self.anchor_in_file(file, ResolutionStatus::Stale),
                ResolutionStatus::Stale,
                AnchorResolutionReason::StaleTargetMissing,
            )
        }
    }

    fn anchor_for_match(
        &self,
        file: &DiffFile,
        target_match: AnchorMatch,
        status: ResolutionStatus,
    ) -> Anchor {
        Anchor {
            file_id: file.id.clone(),
            side: self.side,
            line_range: target_match.line_range,
            hunk_signature: target_match.hunk_signature,
            target_text_hash: self.target_text_hash.clone(),
            status,
        }
    }

    fn anchor_in_file(&self, file: &DiffFile, status: ResolutionStatus) -> Anchor {
        let mut anchor = self.clone();
        anchor.file_id = file.id.clone();
        anchor.status = status;
        anchor
    }

    fn resolved_as(
        &self,
        mut anchor: Anchor,
        status: ResolutionStatus,
        reason: AnchorResolutionReason,
    ) -> AnchorResolution {
        anchor.status = status;
        AnchorResolution { anchor, reason }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AnchorMatch {
    line_range: LineRange,
    hunk_signature: String,
}

fn matches_anchor_file(file: &DiffFile, file_id: &FileId) -> bool {
    let path = file_id.as_str();
    file.id == *file_id
        || file.new_path.as_deref() == Some(path)
        || file.old_path.as_deref() == Some(path)
}

fn target_matches_in_hunk(hunk: &ReviewHunk, anchor: &Anchor) -> Vec<AnchorMatch> {
    matching_line_ranges(
        &hunk.rows,
        anchor.side,
        &anchor.line_range,
        &anchor.target_text_hash,
    )
    .into_iter()
    .map(|line_range| AnchorMatch {
        line_range,
        hunk_signature: hunk.signature(),
    })
    .collect()
}

fn matching_line_ranges(
    rows: &[DiffRow],
    side: Side,
    original_range: &LineRange,
    target_text_hash: &str,
) -> Vec<LineRange> {
    let Some(line_count) = line_count(original_range) else {
        return Vec::new();
    };
    let side_rows = rows
        .iter()
        .filter_map(|row| row.line_on_side(side).map(|line| (line, row)))
        .collect::<Vec<_>>();

    side_rows
        .windows(line_count)
        .filter(|window| has_contiguous_lines(window))
        .filter_map(|window| {
            let rows = window.iter().map(|(_, row)| *row).collect::<Vec<_>>();
            if hash_rows(&rows) == target_text_hash {
                let start = window.first().map(|(line, _)| *line)?;
                let end = window.last().map(|(line, _)| *line)?;
                Some(LineRange::new(start, end))
            } else {
                None
            }
        })
        .collect()
}

pub(crate) fn rows_for_line_range<'a>(
    rows: &'a [DiffRow],
    side: Side,
    range: &LineRange,
) -> Option<Vec<&'a DiffRow>> {
    line_count(range)?;

    let rows = rows
        .iter()
        .filter(|row| {
            row.line_on_side(side)
                .is_some_and(|line| range.start <= line && line <= range.end)
        })
        .collect::<Vec<_>>();
    let lines = rows
        .iter()
        .filter_map(|row| row.line_on_side(side))
        .collect::<Vec<_>>();
    if lines == (range.start..=range.end).collect::<Vec<_>>() {
        Some(rows)
    } else {
        None
    }
}

fn line_count(range: &LineRange) -> Option<usize> {
    if range.start == 0 || range.end < range.start {
        None
    } else {
        Some((range.end - range.start + 1) as usize)
    }
}

fn has_contiguous_lines(window: &[(u32, &DiffRow)]) -> bool {
    window.windows(2).all(|lines| lines[1].0 == lines[0].0 + 1)
}

fn hash_rows(rows: &[&DiffRow]) -> String {
    hash_normalized_lines(rows.iter().map(|row| row.text.as_str()))
}

pub(crate) fn hash_normalized_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> String {
    let mut payload = String::new();
    for line in lines {
        push_normalized_line(&mut payload, line);
    }
    sha256_prefixed(&payload)
}

pub(crate) fn push_normalized_line(payload: &mut String, line: &str) {
    payload.push_str(&line.replace("\r\n", "\n").replace('\r', "\n"));
    payload.push('\n');
}

pub(crate) fn sha256_prefixed(payload: &str) -> String {
    let digest = Sha256::digest(payload.as_bytes());
    format!("sha256:{digest:x}")
}
