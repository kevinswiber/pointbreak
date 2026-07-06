//! `shore diff` — captured-revision diff readback (ADR-0030 D2).
//!
//! Prints a captured revision's diff — base to target, from the frozen captured
//! snapshot — as a text unified diff on stdout. Text-only (no machine lane),
//! non-interactive, pipe-friendly; the output is formally disposable (nothing
//! parses it). The subject is always the frozen snapshot, never the live working
//! tree: `git diff` owns the live tree, and shore's bare verbs read against the
//! review record.

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use shoreline::highlight::{
    EmphSpan, TokenKind, TokenSpan, attributed_segments, emphasis_file, highlight_file,
};
use shoreline::model::{DiffFile, DiffRowKind, DiffSnapshot, FileStatus, ReviewHunk, RevisionId};
use shoreline::session::{
    RevisionShowOptions, RevisionShowResult, SnapshotContentState, diffstat_from_files,
    show_revision,
};

use super::theme::{self, DiffPalette};

/// Print a captured revision's diff (base to target) as a text unified diff.
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
    /// When to colorize output: auto (TTY only) | always | never.
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto)]
    color: ColorChoice,
    /// Color theme for the diff body: auto (detect the terminal background
    /// and pick the light or dark palette), light, dark, or the name of a
    /// bundled syntax theme, case-insensitively (for example "Monokai
    /// Extended" or "onehalflight"). Themes apply on truecolor terminals; the 16-color
    /// palette follows the terminal's own theme. Overrides SHORE_THEME and
    /// BAT_THEME.
    #[arg(long)]
    theme: Option<String>,
}

/// When `shore diff` colorizes its output. Resolved against the ADR-0029 D5
/// presentation precedence by [`resolve_color`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, clap::ValueEnum)]
pub(super) enum ColorChoice {
    /// Colorize only when stdout is a TTY (honoring `NO_COLOR` / `CLICOLOR_FORCE`).
    #[default]
    Auto,
    /// Always colorize, even when piped.
    Always,
    /// Never colorize.
    Never,
}

/// Pure presentation-precedence core: does `shore diff` emit ANSI color?
///
/// Implements the ADR-0029 D5 total order `--color` > `NO_COLOR` >
/// `CLICOLOR_FORCE` > `isatty(stdout)`. `no_color` is a present, non-empty
/// `NO_COLOR`; `force` is a non-zero `CLICOLOR_FORCE`. Under `Auto`, disabling
/// wins ties (`NO_COLOR` beats `CLICOLOR_FORCE`). Injected signals keep it
/// unit-testable without a real terminal.
pub(super) fn resolve_color_core(
    flag: ColorChoice,
    no_color: bool,
    force: bool,
    stdout_is_tty: bool,
) -> bool {
    match flag {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            if no_color {
                false
            } else if force {
                true
            } else {
                stdout_is_tty
            }
        }
    }
}

/// Reads the `NO_COLOR` / `CLICOLOR_FORCE` env signals once and isattys the real
/// stdout, then delegates to [`resolve_color_core`]. The single env decision point
/// for color; the resolved `bool` threads to the render (and, later, the pager).
pub(super) fn resolve_color(flag: ColorChoice) -> bool {
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let force = std::env::var("CLICOLOR_FORCE")
        .ok()
        .is_some_and(|v| v != "0" && !v.is_empty());
    resolve_color_core(flag, no_color, force, std::io::stdout().is_terminal())
}

pub(super) fn run(
    args: DiffArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::debug!(command = "diff", "command_start");

    let mut options = RevisionShowOptions::new(&args.repo).with_read_for_display(true);
    if let Some(revision) = &args.revision {
        let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
        options = options.with_revision_id(RevisionId::new(ids.rev(revision)?));
    }
    let result: RevisionShowResult = show_revision(options)?;

    // The theme decision point: resolve the colored lane here (env reads,
    // terminal detection, the stderr warning) so the render stays pure. Built
    // after `show_revision` so a store error is never preempted by a theme
    // error, and only when the colored body lane is active — `--stat` and
    // plain output never resolve themes (and never query the terminal).
    let lane: Option<ColorLane> = if resolve_color(args.color) && !args.stat {
        Some(match color_depth() {
            ColorDepth::Named => ColorLane::Named,
            ColorDepth::Truecolor => {
                let selection = theme::theme_selection_from_env(args.theme.as_deref());
                // The terminal gate; the Auto-only condition lives inside
                // resolve_truecolor_palette, which invokes the detector at
                // most once, only where Auto behavior applies.
                let gate = theme::detection_allowed(
                    true, // color already resolved on in this branch
                    std::io::stdout().is_terminal(),
                    true, // truecolor in this arm
                );
                let choice = theme::resolve_truecolor_palette(&selection, || {
                    if gate { theme::detect_mode() } else { None }
                })?;
                if let Some(warning) = &choice.warning {
                    eprintln!("warning: {warning}");
                }
                ColorLane::Truecolor(Box::new(choice.palette))
            }
        })
    } else {
        None
    };

    // `shore diff` is a filter: render the whole output, then write it. A broken
    // downstream pipe (`shore diff | head`) is a clean stop, not an error.
    write_all_filtered(stdout, &render_output(&args, &result, lane.as_ref()))
}

/// Build the full `shore diff` output as one string: the removed-content or
/// empty-diff message, the `--stat` table, or the diffstat header + blank line +
/// (plain or colored) diff body. Pure — no writes, no store, no git, no env;
/// the resolved lane (or `None` for plain) is threaded in from `run()`.
fn render_output(args: &DiffArgs, result: &RevisionShowResult, lane: Option<&ColorLane>) -> String {
    // Removed/suppressed content: an explained line, the full id one step away.
    if result.snapshot_content_state != SnapshotContentState::Present {
        let hash = result
            .removed_snapshot_content_hash
            .as_deref()
            .unwrap_or("");
        return format!(
            "captured diff content is unavailable ({}); it was removed from this store\n",
            crate::cli::output::short_ref(hash)
        );
    }

    // A genuine empty diff (present, but no files) — distinct from removed content.
    if result.snapshot.files.is_empty() {
        return "no changes in the captured revision\n".to_string();
    }

    if args.stat {
        return render_stat_table(&result.snapshot.files);
    }

    let mut out = render_diffstat(&result.snapshot.files);
    out.push('\n');
    out.push_str(&render_body(&result.snapshot, lane));
    out
}

/// The (plain or colored) diff body: the pure seam between the lane decision
/// in `run()` and the renderers. `None` renders plain.
fn render_body(snapshot: &DiffSnapshot, lane: Option<&ColorLane>) -> String {
    match lane {
        Some(lane) => render_unified_diff_colored(snapshot, lane),
        None => render_unified_diff(snapshot),
    }
}

/// Write `content` to `out` and flush, treating a broken downstream pipe as a
/// clean stop — a filter is done when its reader goes away (`shore diff | head`).
/// Any other write/flush error propagates. (Rust ignores SIGPIPE, so a closed
/// pipe surfaces here as an `io::ErrorKind::BrokenPipe` rather than a signal.)
fn write_all_filtered(
    out: &mut dyn Write,
    content: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match out.write_all(content.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Render a captured snapshot as a plain git-style unified diff. Pure: no git,
/// no live tree, no store — the subject is the frozen snapshot.
pub(super) fn render_unified_diff(snapshot: &DiffSnapshot) -> String {
    let mut out = String::new();
    for file in &snapshot.files {
        render_file_header(&mut out, file);
        push_metadata_rows(&mut out, file);
        for hunk in &file.hunks {
            push_hunk_header(&mut out, hunk);
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

/// Emit a file's metadata rows (binary/mode/rename/submodule summaries) verbatim.
/// Shared by the plain and colored renderers so both stay byte-identical here.
fn push_metadata_rows(out: &mut String, file: &DiffFile) {
    for meta in &file.metadata_rows {
        out.push_str(&meta.text);
        out.push('\n');
    }
}

/// Emit a hunk's `@@ … @@` header, normalizing its trailing newline. Shared by
/// both renderers (INV-D: the colored path only recolors code rows, never headers).
fn push_hunk_header(out: &mut String, hunk: &ReviewHunk) {
    out.push_str(&hunk.header);
    if !hunk.header.ends_with('\n') {
        out.push('\n');
    }
}

fn marker(kind: &DiffRowKind) -> char {
    match kind {
        DiffRowKind::Added => '+',
        DiffRowKind::Removed => '-',
        DiffRowKind::Context => ' ',
    }
}

/// Per-file row cap above which highlighting is skipped and rows render plain
/// (best-effort presentation, never a hard cost — INV-E).
const HIGHLIGHT_ROW_CAP: usize = 500;

const SGR_RESET: &str = "\x1b[0m";
const SGR_UNDERLINE: &str = "\x1b[4m";

/// Terminal color capability, mirroring the TUI's `color_depth` (`src/tui/render.rs`).
#[derive(Clone, Copy)]
pub(super) enum ColorDepth {
    Truecolor,
    Named,
}

/// Truecolor only when the terminal advertises it via `COLORTERM`; otherwise the
/// named-ANSI 16-color palette, which degrades cleanly on limited terminals. No new
/// dependency — just the `COLORTERM` convention the TUI already follows.
fn color_depth() -> ColorDepth {
    match std::env::var("COLORTERM").ok().as_deref() {
        Some("truecolor") | Some("24bit") => ColorDepth::Truecolor,
        _ => ColorDepth::Named,
    }
}

/// Which colored lane the render uses: the named-ANSI table (the terminal's
/// own theme colors it, byte-frozen) or a truecolor palette (theme-aware —
/// built-in light/dark or derived from an embedded theme).
pub(super) enum ColorLane {
    Named,
    // Boxed: the palette is ~12 Cow fields and the lane is built once per
    // run, so the indirection is free and keeps the enum small (clippy).
    Truecolor(Box<DiffPalette>),
}

/// `TokenKind` → named-ANSI SGR foreground, the 16-color lane's frozen table.
/// The color *policy* mirrors the TUI palette (`token_fg`, `src/tui/render.rs`);
/// only the *encoding* is new — raw SGR strings, no ratatui and no styling
/// dependency (INV-E: a new emit surface, not a parallel highlighter). The
/// truecolor tables live on [`DiffPalette`]. `Plain` carries no color.
fn named_sgr_for_kind(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::Keyword => "\x1b[35m",     // magenta
        TokenKind::String => "\x1b[32m",      // green
        TokenKind::Comment => "\x1b[90m",     // dark gray
        TokenKind::Number => "\x1b[36m",      // cyan
        TokenKind::Type => "\x1b[33m",        // yellow
        TokenKind::Function => "\x1b[34m",    // blue
        TokenKind::Constant => "\x1b[93m",    // light yellow
        TokenKind::Operator => "\x1b[97m",    // white
        TokenKind::Punctuation => "\x1b[37m", // gray
        TokenKind::Variable => "\x1b[97m",    // white
        TokenKind::Plain => "",
    }
}

/// Colored render of one diff row. `text` is the BARE row text (the marker byte is
/// stripped at ingestion); the `+`/`-`/` ` gutter is emitted here from `kind`,
/// OUTSIDE any colored segment. Code segments come from
/// `attributed_segments(text, tokens, emphasis)` (offsets into the bare text), each
/// wrapped in its lane's foreground. Intraline emphasis renders per lane: the
/// named lane underlines; the truecolor lane paints the palette's add/del
/// background tint by row kind (context rows never carry emphasis — the
/// intraline pass pairs removed/added blocks only — so they defensively get no
/// tint). Empty `tokens`/`emphasis` leaves the bare text after the gutter, so
/// stripping the SGR reproduces the plain row exactly (INV-D).
fn render_row_ansi(
    text: &str,
    kind: DiffRowKind,
    tokens: &[TokenSpan],
    emphasis: &[EmphSpan],
    lane: &ColorLane,
) -> String {
    let emph_sgr = match lane {
        ColorLane::Named => SGR_UNDERLINE,
        ColorLane::Truecolor(palette) => match kind {
            DiffRowKind::Added => palette.emph_add_bg.as_ref(),
            DiffRowKind::Removed => palette.emph_del_bg.as_ref(),
            DiffRowKind::Context => "",
        },
    };
    let mut out = String::new();
    out.push(marker(&kind));
    for seg in attributed_segments(text, tokens, emphasis) {
        let slice = &text[seg.start..seg.end];
        let fg = seg
            .kind
            .map(|k| match lane {
                ColorLane::Named => named_sgr_for_kind(k),
                ColorLane::Truecolor(palette) => palette.sgr_for(k),
            })
            .unwrap_or("");
        let emph = if seg.emphasized { emph_sgr } else { "" };
        if fg.is_empty() && emph.is_empty() {
            out.push_str(slice);
        } else {
            out.push_str(fg);
            out.push_str(emph);
            out.push_str(slice);
            out.push_str(SGR_RESET);
        }
    }
    out.push('\n');
    out
}

/// Colored sibling of [`render_unified_diff`]. File/hunk headers and metadata rows
/// are emitted identically (uncolored); only code rows carry syntax + intraline SGR,
/// so `strip_ansi(colored) == render_unified_diff` — color is pure presentation over
/// identical text (INV-D). Highlighting is best-effort (INV-E): a file over the row
/// cap or of an unknown language has empty span maps and renders plain.
pub(super) fn render_unified_diff_colored(snapshot: &DiffSnapshot, lane: &ColorLane) -> String {
    let mut out = String::new();
    for file in &snapshot.files {
        render_file_header(&mut out, file);
        push_metadata_rows(&mut out, file);
        let total_rows: usize = file.hunks.iter().map(|hunk| hunk.rows.len()).sum();
        let (tokens_map, emphasis_map) = if total_rows <= HIGHLIGHT_ROW_CAP {
            (highlight_file(file), emphasis_file(file))
        } else {
            (HashMap::new(), HashMap::new())
        };
        for (h, hunk) in file.hunks.iter().enumerate() {
            push_hunk_header(&mut out, hunk);
            for (r, row) in hunk.rows.iter().enumerate() {
                let tokens = tokens_map
                    .get(&(h, r))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let emphasis = emphasis_map
                    .get(&(h, r))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                out.push_str(&render_row_ansi(
                    &row.text,
                    row.kind.clone(),
                    tokens,
                    emphasis,
                    lane,
                ));
            }
        }
    }
    out
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
    // mode-only files carry their readable summary in `metadata_rows` instead.
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

fn plural(count: usize, singular: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {singular}s")
    }
}

/// One-line summary: `N files changed, A insertions(+), R deletions(-)`.
/// Counts come from the shared `diffstat_from_files` so diffstat semantics live
/// in one place. Rendered above the diff body by default; disposable output.
pub(super) fn render_diffstat(files: &[DiffFile]) -> String {
    let stat = diffstat_from_files(files);
    format!(
        "{} changed, {}(+), {}(-)\n",
        plural(stat.file_count, "file"),
        plural(stat.added_lines, "insertion"),
        plural(stat.removed_lines, "deletion"),
    )
}

/// Body-less per-file stat block (git `--stat`-style), ending with the summary.
/// Per-file counts reuse `diffstat_from_files` over a single-file slice.
pub(super) fn render_stat_table(files: &[DiffFile]) -> String {
    let mut out = String::new();
    for file in files {
        let stat = diffstat_from_files(std::slice::from_ref(file));
        let path = file
            .new_path
            .as_deref()
            .or(file.old_path.as_deref())
            .unwrap_or("");
        out.push_str(&format!(
            " {path} | {} +{} -{}\n",
            stat.added_lines + stat.removed_lines,
            stat.added_lines,
            stat.removed_lines,
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

    // Color-resolution precedence: (flag, no_color, clicolor_force, stdout_is_tty) -> emit ANSI?

    #[test]
    fn flag_always_and_never_beat_everything() {
        // An explicit flag wins outright, over every env signal and isatty.
        assert!(resolve_color_core(ColorChoice::Always, true, false, false));
        assert!(!resolve_color_core(ColorChoice::Never, false, true, true));
    }

    #[test]
    fn no_color_beats_clicolor_force_under_auto() {
        // Both set -> disabling wins ties.
        assert!(!resolve_color_core(ColorChoice::Auto, true, true, true));
    }

    #[test]
    fn clicolor_force_enables_color_when_piped_under_auto() {
        assert!(resolve_color_core(ColorChoice::Auto, false, true, false));
    }

    #[test]
    fn auto_falls_through_to_isatty() {
        assert!(resolve_color_core(ColorChoice::Auto, false, false, true));
        assert!(!resolve_color_core(ColorChoice::Auto, false, false, false));
    }

    /// Strip ANSI SGR sequences (`ESC [ … m`); test-local sibling of the
    /// integration harness's copy (`tests/cli_diff.rs`).
    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                let mut lookahead = chars.clone();
                if lookahead.next() == Some('[') {
                    chars = lookahead;
                    for cc in chars.by_ref() {
                        if cc == 'm' {
                            break;
                        }
                    }
                    continue;
                }
            }
            out.push(c);
        }
        out
    }

    #[test]
    fn token_kind_maps_to_an_ansi_sgr_sequence() {
        let seq = named_sgr_for_kind(TokenKind::Keyword);
        assert!(seq.starts_with("\x1b[")); // CSI
    }

    #[test]
    fn theme_flag_parses_into_diff_args() {
        use clap::Parser;
        #[derive(clap::Parser)]
        struct TestCli {
            #[command(flatten)]
            args: DiffArgs,
        }
        let cli = TestCli::parse_from(["test", "--theme", "OneHalfLight"]);
        assert_eq!(cli.args.theme.as_deref(), Some("OneHalfLight"));
        let cli = TestCli::parse_from(["test"]);
        assert!(cli.args.theme.is_none());
    }

    #[test]
    fn render_body_uses_the_given_lane() {
        // The pure body seam: a truecolor light lane surfaces light bytes,
        // no lane renders plain.
        let snapshot = snapshot_with(vec![modified_file(
            "src/lib.rs",
            vec![hunk(
                "@@ -1 +1 @@",
                vec![row(DiffRowKind::Added, "let x = 2;")],
            )],
        )]);
        let lane = ColorLane::Truecolor(Box::new(DiffPalette::builtin_light()));
        let colored = render_body(&snapshot, Some(&lane));
        assert!(colored.contains("\x1b[38;2;122;68;212m")); // light keyword
        let plain = render_body(&snapshot, None);
        assert!(!plain.contains('\x1b'));
        assert_eq!(strip_ansi(&colored), plain);
    }

    #[test]
    fn truecolor_emphasis_paints_background_by_row_kind() {
        let palette = Box::new(DiffPalette::builtin_dark());
        let emphasis = vec![EmphSpan { start: 0, end: 3 }];
        let added = render_row_ansi(
            "let x",
            DiffRowKind::Added,
            &[],
            &emphasis,
            &ColorLane::Truecolor(Box::new(DiffPalette::builtin_dark())),
        );
        assert!(added.contains("\x1b[48;2;0;96;0m")); // dark add tint
        assert!(!added.contains("\x1b[4m")); // underline retired on truecolor
        let removed = render_row_ansi(
            "let x",
            DiffRowKind::Removed,
            &[],
            &emphasis,
            &ColorLane::Truecolor(palette),
        );
        assert!(removed.contains("\x1b[48;2;144;16;17m")); // dark del tint
    }

    #[test]
    fn named_lane_keeps_underline_emphasis_and_named_fg() {
        let tokens = vec![TokenSpan {
            start: 0,
            end: 3,
            kind: TokenKind::Keyword,
        }];
        let emphasis = vec![EmphSpan { start: 0, end: 3 }];
        let out = render_row_ansi(
            "let x",
            DiffRowKind::Added,
            &tokens,
            &emphasis,
            &ColorLane::Named,
        );
        assert!(out.contains("\x1b[35m")); // named keyword magenta, unchanged
        assert!(out.contains("\x1b[4m")); // underline emphasis, unchanged
        assert!(!out.contains("48;2")); // no background on the named lane
    }

    #[test]
    fn truecolor_colored_render_strips_to_the_plain_render() {
        // Presentation purity with emphasis present: background SGR strips
        // like any SGR (INV-D).
        let snapshot = snapshot_with(vec![modified_file(
            "src/lib.rs",
            vec![hunk(
                "@@ -1 +1 @@",
                vec![
                    row(DiffRowKind::Removed, "let x = 1;"),
                    row(DiffRowKind::Added, "let x = 2;"),
                ],
            )],
        )]);
        let colored = render_unified_diff_colored(
            &snapshot,
            &ColorLane::Truecolor(Box::new(DiffPalette::builtin_light())),
        );
        assert_eq!(strip_ansi(&colored), render_unified_diff(&snapshot));
    }

    #[test]
    fn colored_row_wraps_a_token_segment_and_resets() {
        // `render_row_ansi` takes the BARE row text and emits the `+` gutter itself;
        // spans are byte offsets into the bare text. Keyword "let" = [0,3).
        let tokens = vec![TokenSpan {
            start: 0,
            end: 3,
            kind: TokenKind::Keyword,
        }];
        let out = render_row_ansi("let x", DiffRowKind::Added, &tokens, &[], &ColorLane::Named);
        assert!(out.starts_with('+')); // gutter from the kind, outside any colored segment
        assert!(out.contains("\x1b[")); // an SGR wraps the keyword
        assert!(out.contains("\x1b[0m")); // reset
        assert!(out.ends_with('\n'));
    }

    /// A `Write` that always fails with a chosen `ErrorKind` — models a downstream
    /// reader that has gone away (`shore diff | head`).
    struct FailingWriter(std::io::ErrorKind);
    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(self.0, "write failed"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(self.0, "flush failed"))
        }
    }

    #[test]
    fn write_all_filtered_treats_a_broken_pipe_as_a_clean_stop() {
        // `shore diff | head` closes the pipe early — a filter is done when its
        // reader goes away, so this is a clean exit, not an error.
        let mut out = FailingWriter(std::io::ErrorKind::BrokenPipe);
        assert!(write_all_filtered(&mut out, "diff --git a/x b/x\n").is_ok());
    }

    #[test]
    fn write_all_filtered_propagates_non_pipe_errors() {
        // A real write failure (not a closed pipe) is still an error.
        let mut out = FailingWriter(std::io::ErrorKind::PermissionDenied);
        assert!(write_all_filtered(&mut out, "x").is_err());
    }
}
