//! Best-effort, read-time syntax highlighting for diff views.
//!
//! A surface-neutral tokenizer core: it turns diff rows into [`TokenSpan`]s keyed by a token
//! [`TokenKind`], which the inspector (web) and the TUI each render in their own way. Tokens are a
//! view-only projection — they are never stored on the diff model and never affect the
//! content-addressed snapshot artifact.

mod intraline;
mod syntax;
mod tokenize;

use std::collections::HashMap;

pub use intraline::{EmphSpan, emphasis_file};
use syntax::syntax_for_paths;
use tokenize::LineTokenizer;

use crate::model::{DiffFile, DiffRowKind};

/// Positional identity of a row within a file: `(hunk_index, row_index)`.
///
/// This is the single seam between the lib tokenizer core and each consuming surface (the web
/// inspector and the TUI), which each map a `RowKey` onto their own row handle.
pub type RowKey = (usize, usize);

/// Highlight a whole diff file, returning tokens keyed by [`RowKey`].
///
/// Uses per-hunk, side-separated streams: the old side (context + removed) and the new side
/// (context + added) are each tokenized as one ordered, stateful pass, so a removed-then-added
/// change block highlights both versions independently without mixing state. Tokens map back to
/// rows positionally.
///
/// Returns an empty map (render everything plain) for binary, submodule, mode-only, and
/// unknown-language files. Best-effort throughout: it never panics.
pub fn highlight_file(file: &DiffFile) -> HashMap<RowKey, Vec<TokenSpan>> {
    let mut out = HashMap::new();
    // These carry no diff rows anyway; returning early keeps the contract explicit.
    if file.is_binary || file.is_submodule || file.is_mode_only {
        return out;
    }
    let Some(syntax) = syntax_for_paths(file.new_path.as_deref(), file.old_path.as_deref()) else {
        return out; // unknown language -> plain
    };
    for (h, hunk) in file.hunks.iter().enumerate() {
        // One stateful pass per side, in row order.
        let mut old_tk = LineTokenizer::new(syntax);
        let mut new_tk = LineTokenizer::new(syntax);
        for (r, row) in hunk.rows.iter().enumerate() {
            let spans = match row.kind {
                // Context rows exist on both sides: feed both streams to keep each side's state
                // correct, but store only one result (the text is identical on either side).
                DiffRowKind::Context => {
                    let _ = old_tk.next_line(&row.text);
                    new_tk.next_line(&row.text)
                }
                DiffRowKind::Removed => old_tk.next_line(&row.text),
                DiffRowKind::Added => new_tk.next_line(&row.text),
            };
            if !spans.is_empty() {
                out.insert((h, r), spans);
            }
        }
    }
    out
}

/// Surface-neutral classification of a token.
///
/// The lowercase [`TokenKind::as_str`] string is the single spelling shared across surfaces: the
/// web inspector's CSS class family and the TUI's style map both consume it. `Plain` denotes
/// unclassified text and is never emitted in a span (gaps between spans are implicitly `Plain`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenKind {
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Function,
    Constant,
    Operator,
    Punctuation,
    Variable,
    Plain,
}

impl TokenKind {
    /// The stable lowercase wire spelling for this kind, shared verbatim by every surface.
    pub fn as_str(self) -> &'static str {
        match self {
            TokenKind::Keyword => "keyword",
            TokenKind::String => "string",
            TokenKind::Comment => "comment",
            TokenKind::Number => "number",
            TokenKind::Type => "type",
            TokenKind::Function => "function",
            TokenKind::Constant => "constant",
            TokenKind::Operator => "operator",
            TokenKind::Punctuation => "punctuation",
            TokenKind::Variable => "variable",
            TokenKind::Plain => "plain",
        }
    }
}

/// A classified, contiguous range of a single raw diff-row text.
///
/// `start`/`end` are **byte offsets into the raw `DiffRow.text`** (UTF-8). Emitted spans are
/// sorted, non-overlapping, and **non-`Plain` only** — any byte range not covered by a span is
/// implicitly [`TokenKind::Plain`]. `Copy` so callers can collect and sort spans cheaply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TokenSpan {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiffFile, DiffRow, DiffRowKind, FileId, FileStatus, HunkId, ReviewHunk};

    fn row(kind: DiffRowKind, text: &str) -> DiffRow {
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
                header: "@@ -1,4 +1,4 @@".to_owned(),
                old_start: 1,
                old_lines: 4,
                new_start: 1,
                new_lines: 4,
                rows,
            }],
        }
    }

    fn rust_file_one_hunk() -> DiffFile {
        file_with(
            Some("a.rs"),
            vec![
                row(DiffRowKind::Context, "let a = 1;"),
                row(DiffRowKind::Removed, "let b = 2;"),
                row(DiffRowKind::Added, "let b = 3;"),
                row(DiffRowKind::Context, "use x;"),
            ],
        )
    }

    fn binary_file() -> DiffFile {
        let mut f = file_with(Some("a.rs"), Vec::new());
        f.is_binary = true;
        f
    }

    fn mode_only_file() -> DiffFile {
        let mut f = file_with(Some("a.rs"), Vec::new());
        f.is_mode_only = true;
        f
    }

    fn file_with_path(path: &str) -> DiffFile {
        file_with(
            Some(path),
            vec![row(DiffRowKind::Context, "some notes here")],
        )
    }

    fn synthetic_rs_file() -> DiffFile {
        let mut f = file_with(
            Some("new.rs"),
            vec![row(DiffRowKind::Added, "fn main() { let n = 1; }")],
        );
        f.synthetic = true;
        f.status = FileStatus::Added;
        f
    }

    #[test]
    fn maps_old_and_new_side_tokens_to_their_rows() {
        let file = rust_file_one_hunk();
        let map = highlight_file(&file);
        let ctx0 = map.get(&(0, 0)).unwrap();
        assert!(ctx0.iter().any(|s| s.kind == TokenKind::Keyword)); // "let"
        let removed = map.get(&(0, 1)).unwrap();
        assert!(removed.iter().any(|s| s.kind == TokenKind::Keyword));
        let added = map.get(&(0, 2)).unwrap();
        assert!(added.iter().any(|s| s.kind == TokenKind::Keyword));
    }

    #[test]
    fn binary_and_mode_only_files_get_no_tokens() {
        assert!(highlight_file(&binary_file()).is_empty());
        assert!(highlight_file(&mode_only_file()).is_empty());
    }

    #[test]
    fn unknown_language_file_gets_no_tokens() {
        assert!(highlight_file(&file_with_path("notes.xyzzy")).is_empty());
    }

    #[test]
    fn synthetic_untracked_file_is_highlighted() {
        // synthetic untracked files DO carry rows -> they highlight via the side-streams.
        assert!(!highlight_file(&synthetic_rs_file()).is_empty());
    }

    #[test]
    fn token_kind_renders_lowercase_wire_str() {
        assert_eq!(TokenKind::Keyword.as_str(), "keyword");
        assert_eq!(TokenKind::Punctuation.as_str(), "punctuation");
        assert_eq!(TokenKind::Plain.as_str(), "plain");
    }

    #[test]
    fn token_span_holds_byte_range_and_kind() {
        let s = TokenSpan {
            start: 0,
            end: 3,
            kind: TokenKind::Keyword,
        };
        assert_eq!((s.start, s.end), (0, 3));
        assert_eq!(s.kind, TokenKind::Keyword);
    }
}
