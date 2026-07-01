//! Best-effort, read-time intraline emphasis for diff views.
//!
//! A second, view-only channel that marks the changed sub-spans within a paired removed/added line.
//! Like [`super::highlight_file`], the emphasis it produces is a projection: it is never stored on the
//! diff model and never affects the content-addressed snapshot artifact.

use std::collections::HashMap;

use super::RowKey;
use crate::model::DiffFile;

/// A changed sub-span within a diff row, as **byte offsets into the raw `DiffRow.text`** (mirrors
/// [`super::TokenSpan`] offsets). Emphasis is a boolean channel — there is no `kind`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmphSpan {
    pub start: usize,
    pub end: usize,
}

/// Intraline emphasis for a whole diff file, keyed by the same [`RowKey`] as
/// [`super::highlight_file`].
///
/// Stub: the real block-buffering/greedy-pairing algorithm arrives in a later task; until then any
/// file yields an empty map (render everything plain).
pub fn emphasis_file(_file: &DiffFile) -> HashMap<RowKey, Vec<EmphSpan>> {
    HashMap::new()
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

    #[test]
    fn emphspan_is_copy_two_field() {
        let s = EmphSpan { start: 0, end: 3 };
        let t = s; // Copy, not move
        assert_eq!((s.start, s.end), (0, 3));
        assert_eq!((t.start, t.end), (0, 3));
    }

    #[test]
    fn emphasis_file_stub_is_empty() {
        // Any DiffFile → empty map for now (algorithm arrives in a later task).
        let file = file_with(
            Some("a.rs"),
            vec![
                row(DiffRowKind::Removed, "let b = 2;"),
                row(DiffRowKind::Added, "let b = 3;"),
            ],
        );
        assert!(emphasis_file(&file).is_empty());
    }
}
