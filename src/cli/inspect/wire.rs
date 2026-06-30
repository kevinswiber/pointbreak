//! Inspect-only enriched wire DTO.
//!
//! The stored diff model never carries tokens (the content hash must not change), so the inspector
//! mirrors the stored artifact's serialized shape in a parallel `Wire*` family that additively
//! carries syntax tokens per row. Token byte offsets from the lib are translated to UTF-16 code
//! units here, so the web client can slice the raw row text directly.

use std::collections::HashMap;

use serde::Serialize;
use shoreline::highlight::{RowKey, TokenSpan};
use shoreline::model::{
    DiffFile, DiffRow, DiffRowKind, DiffSnapshot, FileId, FileMetadataRow, FileStatus, HunkId,
    ObjectId, ReviewHunk, ReviewId,
};
use shoreline::session::ObjectArtifact;

/// Per-file highlight cap. A file whose total diff rows exceed this is served plain (the
/// manually-expanded large-file case). Mirrors the inspector's large-file threshold so the
/// highlight cost stays bounded; the content-hash cache makes it a one-time pay.
const HIGHLIGHT_FILE_ROW_CAP: usize = 500;

/// Mirror of the stored `ObjectArtifact`'s serialized shape with additive per-row tokens. Leaf
/// fields reuse the model types so the wire is byte-identical to the stored artifact except for the
/// added `tokens` arrays.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireObjectArtifact {
    pub schema: String,
    pub version: u32,
    pub snapshot: WireDiffSnapshot,
    pub content_hash: String,
}

#[derive(Serialize)]
pub(super) struct WireDiffSnapshot {
    pub review_id: ReviewId,
    pub object_id: ObjectId,
    pub files: Vec<WireDiffFile>,
}

#[derive(Serialize)]
pub(super) struct WireDiffFile {
    pub id: FileId,
    pub status: FileStatus,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    pub old_oid: Option<String>,
    pub new_oid: Option<String>,
    pub similarity: Option<u16>,
    pub is_binary: bool,
    pub is_submodule: bool,
    pub is_mode_only: bool,
    pub synthetic: bool,
    pub metadata_rows: Vec<FileMetadataRow>,
    pub hunks: Vec<WireReviewHunk>,
}

#[derive(Serialize)]
pub(super) struct WireReviewHunk {
    pub id: HunkId,
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub rows: Vec<WireDiffRow>,
}

#[derive(Serialize)]
pub(super) struct WireDiffRow {
    pub kind: DiffRowKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tokens: Vec<WireTokenSpan>,
}

#[derive(Serialize)]
pub(super) struct WireTokenSpan {
    pub start: usize,
    pub end: usize,
    pub kind: &'static str,
}

impl WireObjectArtifact {
    /// Build the enriched wire DTO from a decoded, hash-validated artifact, calling `highlight` once
    /// per file to obtain its row tokens.
    pub(super) fn from_artifact(
        artifact: &ObjectArtifact,
        highlight: impl Fn(&DiffFile) -> HashMap<RowKey, Vec<TokenSpan>>,
    ) -> Self {
        WireObjectArtifact {
            schema: artifact.schema.clone(),
            version: artifact.version,
            snapshot: WireDiffSnapshot::from_snapshot(&artifact.snapshot, &highlight),
            content_hash: artifact.content_hash.clone(),
        }
    }
}

impl WireDiffSnapshot {
    fn from_snapshot(
        snapshot: &DiffSnapshot,
        highlight: &impl Fn(&DiffFile) -> HashMap<RowKey, Vec<TokenSpan>>,
    ) -> Self {
        WireDiffSnapshot {
            review_id: snapshot.review_id.clone(),
            object_id: snapshot.object_id.clone(),
            files: snapshot
                .files
                .iter()
                .map(|file| WireDiffFile::from_file(file, highlight))
                .collect(),
        }
    }
}

impl WireDiffFile {
    fn from_file(
        file: &DiffFile,
        highlight: &impl Fn(&DiffFile) -> HashMap<RowKey, Vec<TokenSpan>>,
    ) -> Self {
        let total_rows: usize = file.hunks.iter().map(|hunk| hunk.rows.len()).sum();
        // Bounded best-effort: a file past the row cap is served plain.
        let spans = if total_rows > HIGHLIGHT_FILE_ROW_CAP {
            HashMap::new()
        } else {
            highlight(file)
        };
        WireDiffFile {
            id: file.id.clone(),
            status: file.status.clone(),
            old_path: file.old_path.clone(),
            new_path: file.new_path.clone(),
            old_mode: file.old_mode.clone(),
            new_mode: file.new_mode.clone(),
            old_oid: file.old_oid.clone(),
            new_oid: file.new_oid.clone(),
            similarity: file.similarity,
            is_binary: file.is_binary,
            is_submodule: file.is_submodule,
            is_mode_only: file.is_mode_only,
            synthetic: file.synthetic,
            metadata_rows: file.metadata_rows.clone(),
            hunks: file
                .hunks
                .iter()
                .enumerate()
                .map(|(hunk_index, hunk)| WireReviewHunk::from_hunk(hunk_index, hunk, &spans))
                .collect(),
        }
    }
}

impl WireReviewHunk {
    fn from_hunk(
        hunk_index: usize,
        hunk: &ReviewHunk,
        spans: &HashMap<RowKey, Vec<TokenSpan>>,
    ) -> Self {
        WireReviewHunk {
            id: hunk.id.clone(),
            header: hunk.header.clone(),
            old_start: hunk.old_start,
            old_lines: hunk.old_lines,
            new_start: hunk.new_start,
            new_lines: hunk.new_lines,
            rows: hunk
                .rows
                .iter()
                .enumerate()
                .map(|(row_index, row)| {
                    let row_spans = spans
                        .get(&(hunk_index, row_index))
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    WireDiffRow::from_row(row, row_spans)
                })
                .collect(),
        }
    }
}

impl WireDiffRow {
    pub(super) fn from_row(row: &DiffRow, spans: &[TokenSpan]) -> Self {
        // Checked byte->UTF-16 translation: never index `text` by raw byte ranges (a malformed span
        // would panic). If any span is invalid, omit tokens for the whole row and render plain.
        let tokens = translate_spans(&row.text, spans).unwrap_or_default();
        WireDiffRow {
            kind: row.kind.clone(),
            old_line: row.old_line,
            new_line: row.new_line,
            text: row.text.clone(),
            tokens,
        }
    }
}

/// Translate byte-offset spans into UTF-16 wire spans. Returns `None` if any span is reversed, out
/// of range, or not on a char boundary, so the caller drops tokens for the whole row.
fn translate_spans(text: &str, spans: &[TokenSpan]) -> Option<Vec<WireTokenSpan>> {
    spans
        .iter()
        .map(|span| {
            if span.start > span.end {
                return None;
            }
            text.get(..span.start)?; // validates start is a char boundary <= len
            text.get(..span.end)?; // validates end likewise
            Some(WireTokenSpan {
                start: utf16_len(&text[..span.start]),
                end: utf16_len(&text[..span.end]),
                kind: span.kind.as_str(),
            })
        })
        .collect()
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

#[cfg(test)]
mod tests {
    use shoreline::highlight::{TokenKind, TokenSpan};
    use shoreline::model::{DiffRow, DiffRowKind};

    use super::*;

    fn context_row(text: &str) -> DiffRow {
        DiffRow {
            kind: DiffRowKind::Context,
            old_line: Some(1),
            new_line: Some(1),
            text: text.to_owned(),
        }
    }

    #[test]
    fn wire_row_omits_tokens_when_empty() {
        let row = WireDiffRow::from_row(&context_row("let x = 1;"), &[]); // no spans
        let json = serde_json::to_value(&row).unwrap();
        assert!(json.get("tokens").is_none()); // wire byte-identical to today when unhighlighted
    }

    #[test]
    fn wire_row_carries_utf16_token_offsets() {
        // raw text has a multibyte char before the token so byte != UTF-16 offset.
        let raw = "é let"; // 'é' = 2 bytes, 1 UTF-16 unit
        let byte_spans = vec![TokenSpan {
            start: 3,
            end: 6,
            kind: TokenKind::Keyword,
        }]; // "let" by BYTES
        let row = WireDiffRow::from_row(&context_row(raw), &byte_spans);
        let json = serde_json::to_value(&row).unwrap();
        let t = &json["tokens"][0];
        assert_eq!(t["start"], 2); // UTF-16: "é " = 2 units
        assert_eq!(t["end"], 5);
        assert_eq!(t["kind"], "keyword");
    }

    #[test]
    fn malformed_span_omits_tokens_without_panic() {
        // out-of-range end, and a non-char-boundary start in a multibyte string -> no tokens.
        let raw = "é";
        let bad = vec![TokenSpan {
            start: 1,
            end: 99,
            kind: TokenKind::Keyword,
        }]; // start splits 'é', end > len
        let row = WireDiffRow::from_row(&context_row(raw), &bad);
        let json = serde_json::to_value(&row).unwrap();
        assert!(json.get("tokens").is_none()); // invalid span set -> render plain

        // reversed range (both endpoints individually valid) must ALSO omit tokens.
        let reversed = vec![TokenSpan {
            start: 3,
            end: 0,
            kind: TokenKind::Keyword,
        }];
        let row2 = WireDiffRow::from_row(&context_row("let x"), &reversed);
        assert!(serde_json::to_value(&row2).unwrap().get("tokens").is_none());
    }
}
