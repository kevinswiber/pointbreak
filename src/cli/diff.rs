//! `shore diff` — captured-revision human diff readback (ADR-0030 D2).
//!
//! Prints a captured revision's diff — base to target, from the frozen captured
//! snapshot — as a human unified diff on stdout. Human-only (no machine lane),
//! non-interactive, pipe-friendly; the output is formally disposable (nothing
//! parses it). The subject is always the frozen snapshot, never the live working
//! tree: `git diff` owns the live tree, and shore's bare verbs read against the
//! review record.

use std::io::Write;
use std::path::PathBuf;

use shoreline::model::{DiffFile, DiffRowKind, DiffSnapshot, FileStatus, RevisionId};
use shoreline::session::{
    RevisionShowOptions, RevisionShowResult, SnapshotContentState, show_revision,
};

#[derive(Debug, clap::Args)]
pub(super) struct DiffArgs {
    /// Repository path (defaults to the current directory).
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    /// The captured revision to diff (a head seed): a current head resolves
    /// exactly; a superseded revision resolves its thread's current head. Omit to
    /// diff the current capture; required when the store holds more than one.
    #[arg(long)]
    revision: Option<String>,
    /// Print only the diffstat, not the diff body.
    #[arg(long)]
    stat: bool,
}

pub(super) fn run(
    args: DiffArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::debug!(command = "diff", "command_start");

    let mut options = RevisionShowOptions::new(&args.repo).with_read_for_display(true);
    if let Some(revision) = &args.revision {
        options = options.with_revision_id(RevisionId::new(revision.clone()));
    }
    let result: RevisionShowResult = show_revision(options)?;

    // Removed/suppressed content: an explained line, the full id one step away.
    if result.snapshot_content_state != SnapshotContentState::Present {
        let hash = result
            .removed_snapshot_content_hash
            .as_deref()
            .unwrap_or("");
        writeln!(
            stdout,
            "captured diff content is unavailable ({}); it was removed from this store",
            crate::cli::output::short_ref(hash)
        )?;
        return Ok(());
    }

    // A genuine empty diff (present, but no files) — distinct from removed content.
    if result.snapshot.files.is_empty() {
        writeln!(stdout, "no changes in the captured revision")?;
        return Ok(());
    }

    if args.stat {
        write!(stdout, "{}", render_stat_table(&result.snapshot.files))?;
        return Ok(());
    }

    write!(stdout, "{}", render_diffstat(&result.snapshot.files))?;
    writeln!(stdout)?;
    write!(stdout, "{}", render_unified_diff(&result.snapshot))?;
    Ok(())
}

/// Render a captured snapshot as a plain git-style unified diff. Pure: no git,
/// no live tree, no store — the subject is the frozen snapshot.
pub(super) fn render_unified_diff(snapshot: &DiffSnapshot) -> String {
    let mut out = String::new();
    for file in &snapshot.files {
        render_file_header(&mut out, file);
        for meta in &file.metadata_rows {
            out.push_str(&meta.text);
            out.push('\n');
        }
        for hunk in &file.hunks {
            out.push_str(&hunk.header);
            if !hunk.header.ends_with('\n') {
                out.push('\n');
            }
            for row in &hunk.rows {
                // `row.text` is bare (the marker byte is stripped at ingestion,
                // src/git/patch.rs); emit exactly one marker from `row.kind`.
                out.push(marker(&row.kind));
                out.push_str(&row.text);
                out.push('\n');
            }
        }
    }
    out
}

fn marker(kind: &DiffRowKind) -> char {
    match kind {
        DiffRowKind::Added => '+',
        DiffRowKind::Removed => '-',
        DiffRowKind::Context => ' ',
    }
}

fn render_file_header(out: &mut String, file: &DiffFile) {
    let old = file.old_path.as_deref();
    let new = file.new_path.as_deref();
    let a = old.or(new).unwrap_or("");
    let b = new.or(old).unwrap_or("");
    out.push_str(&format!("diff --git a/{a} b/{b}\n"));

    match file.status {
        FileStatus::Renamed => {
            if let Some(similarity) = file.similarity {
                out.push_str(&format!("similarity index {similarity}%\n"));
            }
            out.push_str(&format!("rename from {}\n", old.unwrap_or(a)));
            out.push_str(&format!("rename to {}\n", new.unwrap_or(b)));
        }
        FileStatus::Copied => {
            if let Some(similarity) = file.similarity {
                out.push_str(&format!("similarity index {similarity}%\n"));
            }
            out.push_str(&format!("copy from {}\n", old.unwrap_or(a)));
            out.push_str(&format!("copy to {}\n", new.unwrap_or(b)));
        }
        _ => {}
    }

    if let (Some(old_mode), Some(new_mode)) = (&file.old_mode, &file.new_mode)
        && old_mode != new_mode
    {
        out.push_str(&format!("old mode {old_mode}\n"));
        out.push_str(&format!("new mode {new_mode}\n"));
    }

    // The `--- / +++` pair belongs to textual files with hunks; binary and
    // mode-only files carry their human summary in `metadata_rows` instead.
    if !file.hunks.is_empty() {
        let minus = if matches!(file.status, FileStatus::Added) {
            "/dev/null".to_string()
        } else {
            format!("a/{}", old.unwrap_or(a))
        };
        let plus = if matches!(file.status, FileStatus::Deleted) {
            "/dev/null".to_string()
        } else {
            format!("b/{}", new.unwrap_or(b))
        };
        out.push_str(&format!("--- {minus}\n"));
        out.push_str(&format!("+++ {plus}\n"));
    }
}

/// Insertion/deletion line counts for one file (from the row kinds).
fn file_line_counts(file: &DiffFile) -> (usize, usize) {
    let mut insertions = 0;
    let mut deletions = 0;
    for hunk in &file.hunks {
        for row in &hunk.rows {
            match row.kind {
                DiffRowKind::Added => insertions += 1,
                DiffRowKind::Removed => deletions += 1,
                DiffRowKind::Context => {}
            }
        }
    }
    (insertions, deletions)
}

fn plural(count: usize, singular: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {singular}s")
    }
}

/// One-line summary: `N files changed, A insertions(+), R deletions(-)`.
/// Rendered above the diff body by default. Disposable — nothing parses it.
pub(super) fn render_diffstat(files: &[DiffFile]) -> String {
    let (insertions, deletions) = files
        .iter()
        .map(file_line_counts)
        .fold((0, 0), |(i, d), (fi, fd)| (i + fi, d + fd));
    format!(
        "{} changed, {}(+), {}(-)\n",
        plural(files.len(), "file"),
        plural(insertions, "insertion"),
        plural(deletions, "deletion"),
    )
}

/// Body-less per-file stat block (git `--stat`-style), ending with the summary.
pub(super) fn render_stat_table(files: &[DiffFile]) -> String {
    let mut out = String::new();
    for file in files {
        let (insertions, deletions) = file_line_counts(file);
        let path = file
            .new_path
            .as_deref()
            .or(file.old_path.as_deref())
            .unwrap_or("");
        out.push_str(&format!(
            " {path} | {} +{insertions} -{deletions}\n",
            insertions + deletions
        ));
    }
    out.push_str(&render_diffstat(files));
    out
}

#[cfg(test)]
mod tests {
    use shoreline::model::{
        DiffRow, FileId, FileMetadataKind, FileMetadataRow, HunkId, ObjectId, ReviewHunk, ReviewId,
    };

    use super::*;

    fn row(kind: DiffRowKind, text: &str) -> DiffRow {
        DiffRow {
            kind,
            old_line: None,
            new_line: None,
            text: text.to_owned(),
        }
    }

    fn hunk(header: &str, rows: Vec<DiffRow>) -> ReviewHunk {
        ReviewHunk {
            id: HunkId::new("hunk:test"),
            header: header.to_owned(),
            old_start: 1,
            old_lines: 1,
            new_start: 1,
            new_lines: 1,
            rows,
        }
    }

    fn base_file(status: FileStatus) -> DiffFile {
        DiffFile {
            id: FileId::new("file:test"),
            status,
            old_path: None,
            new_path: None,
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
            hunks: Vec::new(),
        }
    }

    fn modified_file(path: &str, hunks: Vec<ReviewHunk>) -> DiffFile {
        DiffFile {
            old_path: Some(path.to_owned()),
            new_path: Some(path.to_owned()),
            hunks,
            ..base_file(FileStatus::Modified)
        }
    }

    fn added_file(path: &str) -> DiffFile {
        DiffFile {
            new_path: Some(path.to_owned()),
            hunks: vec![hunk(
                "@@ -0,0 +1 @@",
                vec![row(DiffRowKind::Added, "new content")],
            )],
            ..base_file(FileStatus::Added)
        }
    }

    fn deleted_file(path: &str) -> DiffFile {
        DiffFile {
            old_path: Some(path.to_owned()),
            hunks: vec![hunk(
                "@@ -1 +0,0 @@",
                vec![row(DiffRowKind::Removed, "old content")],
            )],
            ..base_file(FileStatus::Deleted)
        }
    }

    fn renamed_file(old: &str, new: &str, similarity: u16) -> DiffFile {
        DiffFile {
            old_path: Some(old.to_owned()),
            new_path: Some(new.to_owned()),
            similarity: Some(similarity),
            ..base_file(FileStatus::Renamed)
        }
    }

    fn copied_file(old: &str, new: &str, similarity: u16) -> DiffFile {
        DiffFile {
            old_path: Some(old.to_owned()),
            new_path: Some(new.to_owned()),
            similarity: Some(similarity),
            ..base_file(FileStatus::Copied)
        }
    }

    fn binary_file(path: &str, summary: &str) -> DiffFile {
        DiffFile {
            old_path: Some(path.to_owned()),
            new_path: Some(path.to_owned()),
            is_binary: true,
            metadata_rows: vec![FileMetadataRow {
                kind: FileMetadataKind::BinarySummary,
                text: summary.to_owned(),
            }],
            ..base_file(FileStatus::Modified)
        }
    }

    fn snapshot_with(files: Vec<DiffFile>) -> DiffSnapshot {
        DiffSnapshot::new(
            ReviewId::new("review:test"),
            ObjectId::new("obj:test"),
            files,
        )
    }

    #[test]
    fn renders_a_modified_file_as_unified_diff() {
        // DiffRow.text is BARE — the renderer prefixes exactly one marker from the kind.
        let snapshot = snapshot_with(vec![modified_file(
            "src/lib.rs",
            vec![hunk(
                "@@ -1,2 +1,2 @@",
                vec![
                    row(DiffRowKind::Context, "unchanged"),
                    row(DiffRowKind::Removed, "old line"),
                    row(DiffRowKind::Added, "new line"),
                ],
            )],
        )]);
        let out = render_unified_diff(&snapshot);
        assert!(out.contains("diff --git a/src/lib.rs b/src/lib.rs"));
        assert!(out.contains("--- a/src/lib.rs"));
        assert!(out.contains("+++ b/src/lib.rs"));
        assert!(out.contains("@@ -1,2 +1,2 @@"));
        assert!(out.contains("\n-old line"));
        assert!(out.contains("\n+new line"));
        assert!(out.contains("\n unchanged"));
    }

    #[test]
    fn renders_added_and_deleted_files_with_devnull_headers() {
        let added = render_unified_diff(&snapshot_with(vec![added_file("new.txt")]));
        assert!(added.contains("--- /dev/null"));
        assert!(added.contains("+++ b/new.txt"));

        let deleted = render_unified_diff(&snapshot_with(vec![deleted_file("gone.txt")]));
        assert!(deleted.contains("--- a/gone.txt"));
        assert!(deleted.contains("+++ /dev/null"));
    }

    #[test]
    fn renders_rename_header_from_status_and_paths() {
        let out = render_unified_diff(&snapshot_with(vec![renamed_file(
            "old/a.rs", "new/a.rs", 98,
        )]));
        assert!(out.contains("rename from old/a.rs"));
        assert!(out.contains("rename to new/a.rs"));
        assert!(out.contains("similarity index 98%"));
    }

    #[test]
    fn renders_copy_header_distinct_from_rename() {
        let out = render_unified_diff(&snapshot_with(vec![copied_file(
            "src/a.rs", "src/b.rs", 95,
        )]));
        assert!(out.contains("copy from src/a.rs"));
        assert!(out.contains("copy to src/b.rs"));
        assert!(out.contains("similarity index 95%"));
        assert!(!out.contains("rename"));
    }

    #[test]
    fn renders_binary_from_metadata_rows_without_hunks() {
        let out = render_unified_diff(&snapshot_with(vec![binary_file(
            "img.png",
            "Binary files a/img.png and b/img.png differ",
        )]));
        assert!(out.contains("img.png"));
        assert!(out.contains("Binary files"));
        assert!(!out.contains("@@"));
        assert!(!out.contains("--- ")); // no ---/+++ pair for a binary file
    }

    #[test]
    fn empty_snapshot_renders_nothing_substantive() {
        assert!(
            render_unified_diff(&snapshot_with(vec![]))
                .trim()
                .is_empty()
        );
    }

    #[test]
    fn diffstat_summary_counts_files_and_lines() {
        let snapshot = snapshot_with(vec![
            modified_file(
                "a.rs",
                vec![hunk(
                    "@@ -1 +1,2 @@",
                    vec![
                        row(DiffRowKind::Context, "keep"),
                        row(DiffRowKind::Added, "added one"),
                        row(DiffRowKind::Added, "added two"),
                        row(DiffRowKind::Removed, "removed one"),
                    ],
                )],
            ),
            added_file("b.txt"),
        ]);
        let summary = render_diffstat(&snapshot.files);
        assert!(summary.contains("2 files changed"));
        assert!(summary.contains("insertion"));
        assert!(summary.contains("(+)"));
        assert!(summary.contains("deletion"));
        assert!(summary.contains("(-)"));
    }

    #[test]
    fn diffstat_summary_uses_singular_for_one_file() {
        let summary = render_diffstat(&snapshot_with(vec![added_file("only.txt")]).files);
        assert!(summary.contains("1 file changed"));
        assert!(!summary.contains("1 files changed"));
    }

    #[test]
    fn stat_table_lists_each_file_and_summary() {
        let snapshot = snapshot_with(vec![modified_file(
            "src/lib.rs",
            vec![hunk("@@ -1 +1 @@", vec![row(DiffRowKind::Added, "x")])],
        )]);
        let table = render_stat_table(&snapshot.files);
        assert!(table.contains("src/lib.rs"));
        assert!(table.contains("1 file changed"));
        assert!(!table.contains("@@")); // stat table carries no hunk body
    }
}
