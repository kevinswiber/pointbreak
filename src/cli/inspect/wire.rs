//! Inspect-only enriched wire DTO.
//!
//! The stored diff model never carries tokens (the content hash must not change), so the inspector
//! mirrors the stored artifact's serialized shape in a parallel `Wire*` family that additively
//! carries syntax tokens per row. Token byte offsets from the lib are translated to UTF-16 code
//! units here, so the web client can slice the raw row text directly.

use std::collections::HashMap;

use serde::Serialize;
use shoreline::highlight::{EmphSpan, RowKey, TokenSpan};
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub emphasis: Vec<WireEmphSpan>,
}

#[derive(Serialize)]
pub(super) struct WireTokenSpan {
    pub start: usize,
    pub end: usize,
    pub kind: &'static str,
}

/// Intraline emphasis span on the wire (UTF-16 offsets). Two fields only — unlike [`WireTokenSpan`]
/// there is no `kind`; emphasis is a boolean decoration channel (INV-H).
#[derive(Serialize)]
pub(super) struct WireEmphSpan {
    pub start: usize,
    pub end: usize,
}

impl WireObjectArtifact {
    /// Build the enriched wire DTO from a decoded, hash-validated artifact, calling `highlight` and
    /// `emphasis` once per file to obtain its row tokens and intraline emphasis. The two channels are
    /// threaded as parallel producers (mirroring the syntax path) so they share one row-cap gate and
    /// never touch the stored bytes.
    pub(super) fn from_artifact(
        artifact: &ObjectArtifact,
        highlight: impl Fn(&DiffFile) -> HashMap<RowKey, Vec<TokenSpan>>,
        emphasis: impl Fn(&DiffFile) -> HashMap<RowKey, Vec<EmphSpan>>,
    ) -> Self {
        WireObjectArtifact {
            schema: artifact.schema.clone(),
            version: artifact.version,
            snapshot: WireDiffSnapshot::from_snapshot(&artifact.snapshot, &highlight, &emphasis),
            content_hash: artifact.content_hash.clone(),
        }
    }
}

impl WireDiffSnapshot {
    fn from_snapshot(
        snapshot: &DiffSnapshot,
        highlight: &impl Fn(&DiffFile) -> HashMap<RowKey, Vec<TokenSpan>>,
        emphasis: &impl Fn(&DiffFile) -> HashMap<RowKey, Vec<EmphSpan>>,
    ) -> Self {
        WireDiffSnapshot {
            review_id: snapshot.review_id.clone(),
            object_id: snapshot.object_id.clone(),
            files: snapshot
                .files
                .iter()
                .map(|file| WireDiffFile::from_file(file, highlight, emphasis))
                .collect(),
        }
    }
}

impl WireDiffFile {
    fn from_file(
        file: &DiffFile,
        highlight: &impl Fn(&DiffFile) -> HashMap<RowKey, Vec<TokenSpan>>,
        emphasis: &impl Fn(&DiffFile) -> HashMap<RowKey, Vec<EmphSpan>>,
    ) -> Self {
        let total_rows: usize = file.hunks.iter().map(|hunk| hunk.rows.len()).sum();
        // Bounded best-effort: a file past the row cap is served fully plain — the one
        // `HIGHLIGHT_FILE_ROW_CAP` gate covers BOTH channels (syntax and intraline emphasis).
        let (spans, emph) = if total_rows > HIGHLIGHT_FILE_ROW_CAP {
            (HashMap::new(), HashMap::new())
        } else {
            (highlight(file), emphasis(file))
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
                .map(|(hunk_index, hunk)| {
                    WireReviewHunk::from_hunk(hunk_index, hunk, &spans, &emph)
                })
                .collect(),
        }
    }
}

impl WireReviewHunk {
    fn from_hunk(
        hunk_index: usize,
        hunk: &ReviewHunk,
        spans: &HashMap<RowKey, Vec<TokenSpan>>,
        emph: &HashMap<RowKey, Vec<EmphSpan>>,
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
                    // Both channels key by the identical (hunk_index, row_index) walk (INV-C).
                    let row_spans = spans
                        .get(&(hunk_index, row_index))
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    let row_emph = emph
                        .get(&(hunk_index, row_index))
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    WireDiffRow::from_row(row, row_spans, row_emph)
                })
                .collect(),
        }
    }
}

impl WireDiffRow {
    pub(super) fn from_row(row: &DiffRow, spans: &[TokenSpan], emph: &[EmphSpan]) -> Self {
        // Checked byte->UTF-16 translation per channel: never index `text` by raw byte ranges (a
        // malformed span would panic). Each channel is validated independently, so an invalid
        // emphasis set drops emphasis only and vice-versa (INV-F); an invalid set renders plain.
        let tokens = translate_spans(&row.text, spans).unwrap_or_default();
        let emphasis = translate_emphasis(&row.text, emph).unwrap_or_default();
        WireDiffRow {
            kind: row.kind.clone(),
            old_line: row.old_line,
            new_line: row.new_line,
            text: row.text.clone(),
            tokens,
            emphasis,
        }
    }
}

/// Shared byte→UTF-16 offset mapper (single source of the UTF-16 rule, INV-G). Returns `None` if the
/// span is reversed, out of range, or not on a char boundary, so the caller drops the whole channel's
/// spans for the row.
fn to_utf16_span(text: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    if start > end
        || end > text.len()
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
    {
        return None;
    }
    Some((utf16_len(&text[..start]), utf16_len(&text[..end])))
}

/// Translate byte-offset token spans into UTF-16 wire spans (carrying `kind`). Returns `None` if any
/// span is invalid, so the caller drops tokens for the whole row.
fn translate_spans(text: &str, spans: &[TokenSpan]) -> Option<Vec<WireTokenSpan>> {
    spans
        .iter()
        .map(|span| {
            let (start, end) = to_utf16_span(text, span.start, span.end)?;
            Some(WireTokenSpan {
                start,
                end,
                kind: span.kind.as_str(),
            })
        })
        .collect()
}

/// Translate byte-offset emphasis spans into UTF-16 wire spans. Returns `None` if any span is
/// invalid, so the caller drops emphasis for the whole row (INV-F).
fn translate_emphasis(text: &str, spans: &[EmphSpan]) -> Option<Vec<WireEmphSpan>> {
    spans
        .iter()
        .map(|span| {
            let (start, end) = to_utf16_span(text, span.start, span.end)?;
            Some(WireEmphSpan { start, end })
        })
        .collect()
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

#[cfg(test)]
mod tests {
    use shoreline::highlight::{EmphSpan, TokenKind, TokenSpan, emphasis_file, highlight_file};
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

    fn row_with_text(text: &str) -> DiffRow {
        context_row(text)
    }

    fn row_json(row: &DiffRow, tokens: &[TokenSpan], emphasis: &[EmphSpan]) -> serde_json::Value {
        serde_json::to_value(WireDiffRow::from_row(row, tokens, emphasis)).unwrap()
    }

    fn diff_row(kind: DiffRowKind, text: &str) -> DiffRow {
        DiffRow {
            kind,
            old_line: None,
            new_line: None,
            text: text.to_owned(),
        }
    }

    fn file_with(new_path: Option<&str>, rows: Vec<DiffRow>) -> DiffFile {
        DiffFile {
            id: FileId::new("file:a"),
            status: FileStatus::Modified,
            old_path: new_path.map(str::to_owned),
            new_path: new_path.map(str::to_owned),
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
                id: HunkId::new("hunk:1"),
                header: "@@ -1,2 +1,2 @@".to_owned(),
                old_start: 1,
                old_lines: 2,
                new_start: 1,
                new_lines: 2,
                rows,
            }],
        }
    }

    fn modified_rs_file() -> DiffFile {
        file_with(
            Some("m.rs"),
            vec![
                diff_row(DiffRowKind::Removed, "let b = 2;"),
                diff_row(DiffRowKind::Added, "let b = 3;"),
            ],
        )
    }

    fn file_with_n_rows(n: usize) -> DiffFile {
        let rows: Vec<DiffRow> = (0..n)
            .map(|i| {
                if i % 2 == 0 {
                    diff_row(DiffRowKind::Removed, "let a = 1;")
                } else {
                    diff_row(DiffRowKind::Added, "let a = 2;")
                }
            })
            .collect();
        file_with(Some("big.rs"), rows)
    }

    #[test]
    fn from_file_serializes_emphasis_on_changed_rows() {
        // highlight: none, to isolate emphasis; emphasis: real intraline.
        let no_highlight = |_: &DiffFile| HashMap::<RowKey, Vec<TokenSpan>>::new();
        let wire = WireDiffFile::from_file(&modified_rs_file(), &no_highlight, &emphasis_file);
        let json = serde_json::to_value(&wire).unwrap();
        let added_row = &json["hunks"][0]["rows"][1]; // the Added row
        assert!(!added_row["emphasis"].as_array().unwrap().is_empty());
    }

    #[test]
    fn from_file_over_cap_skips_both_channels() {
        let file = file_with_n_rows(HIGHLIGHT_FILE_ROW_CAP + 1);
        let wire = WireDiffFile::from_file(&file, &highlight_file, &emphasis_file);
        let json = serde_json::to_value(&wire).unwrap();
        for row in json["hunks"][0]["rows"].as_array().unwrap() {
            assert!(
                row.get("tokens").is_none(),
                "over-cap file serves no tokens"
            );
            assert!(
                row.get("emphasis").is_none(),
                "over-cap file serves no emphasis"
            );
        }
    }

    #[test]
    fn wire_row_omits_emphasis_when_empty() {
        let json = row_json(&context_row("let x"), &[], &[]);
        assert!(json.get("emphasis").is_none()); // byte-identical to today
    }

    #[test]
    fn wire_row_carries_utf16_emphasis_offsets() {
        // raw "é let": byte span [3,6) = "let" → utf16 [2,5)
        let json = row_json(
            &row_with_text("é let"),
            &[],
            &[EmphSpan { start: 3, end: 6 }],
        );
        assert_eq!(json["emphasis"][0]["start"], 2);
        assert_eq!(json["emphasis"][0]["end"], 5);
        assert!(json["emphasis"][0].get("kind").is_none()); // no kind (INV-H)
    }

    #[test]
    fn malformed_emphasis_drops_emphasis_but_keeps_tokens() {
        let tokens = vec![TokenSpan {
            start: 0,
            end: 3,
            kind: TokenKind::Keyword,
        }];
        let bad = vec![EmphSpan { start: 1, end: 99 }]; // out of range
        let json = row_json(&row_with_text("let x"), &tokens, &bad);
        assert_eq!(json["tokens"].as_array().unwrap().len(), 1); // syntax survives
        assert!(json.get("emphasis").is_none()); // emphasis dropped (INV-F)
    }

    #[test]
    fn wire_row_omits_tokens_when_empty() {
        let row = WireDiffRow::from_row(&context_row("let x = 1;"), &[], &[]); // no spans
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
        let row = WireDiffRow::from_row(&context_row(raw), &byte_spans, &[]);
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
        let row = WireDiffRow::from_row(&context_row(raw), &bad, &[]);
        let json = serde_json::to_value(&row).unwrap();
        assert!(json.get("tokens").is_none()); // invalid span set -> render plain

        // reversed range (both endpoints individually valid) must ALSO omit tokens.
        let reversed = vec![TokenSpan {
            start: 3,
            end: 0,
            kind: TokenKind::Keyword,
        }];
        let row2 = WireDiffRow::from_row(&context_row("let x"), &reversed, &[]);
        assert!(serde_json::to_value(&row2).unwrap().get("tokens").is_none());
    }
}
