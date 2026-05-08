use crate::error::{Result, ShoreError};
use crate::model::{DiffRow, DiffRowKind, HunkId, ReviewHunk};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PatchFile {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    pub similarity: Option<u16>,
    pub is_binary: bool,
    pub hunks: Vec<ReviewHunk>,
}

impl PatchFile {
    pub fn key(&self) -> String {
        self.new_path
            .as_ref()
            .or(self.old_path.as_ref())
            .expect("patch file has at least one path")
            .clone()
    }
}

pub(crate) fn parse_patch(patch: &str) -> Result<Vec<PatchFile>> {
    let mut files = Vec::new();
    let mut current_file: Option<PatchFile> = None;
    let mut current_hunk: Option<ReviewHunk> = None;
    let mut old_line = 0;
    let mut new_line = 0;

    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            finish_hunk(&mut current_file, &mut current_hunk);
            if let Some(file) = current_file.take() {
                files.push(file);
            }
            current_file = Some(PatchFile {
                old_path: diff_git_path(line, 'a'),
                new_path: diff_git_path(line, 'b'),
                old_mode: None,
                new_mode: None,
                similarity: None,
                is_binary: false,
                hunks: Vec::new(),
            });
            continue;
        }

        if current_file.is_none() {
            continue;
        }

        if let Some(path) = line.strip_prefix("--- ") {
            current_file.as_mut().expect("current file").old_path = parse_patch_path(path);
            continue;
        }
        if let Some(path) = line.strip_prefix("+++ ") {
            current_file.as_mut().expect("current file").new_path = parse_patch_path(path);
            continue;
        }
        if let Some(mode) = line.strip_prefix("old mode ") {
            current_file.as_mut().expect("current file").old_mode = Some(mode.to_owned());
            continue;
        }
        if let Some(mode) = line.strip_prefix("new mode ") {
            current_file.as_mut().expect("current file").new_mode = Some(mode.to_owned());
            continue;
        }
        if let Some(mode) = line.strip_prefix("new file mode ") {
            current_file.as_mut().expect("current file").new_mode = Some(mode.to_owned());
            continue;
        }
        if let Some(mode) = line.strip_prefix("deleted file mode ") {
            current_file.as_mut().expect("current file").old_mode = Some(mode.to_owned());
            continue;
        }
        if let Some(index) = line.strip_prefix("similarity index ") {
            let similarity = index.trim_end_matches('%').parse().map_err(|error| {
                ShoreError::Message(format!("invalid similarity index {index}: {error}"))
            })?;
            current_file.as_mut().expect("current file").similarity = Some(similarity);
            continue;
        }
        if let Some(path) = line.strip_prefix("rename from ") {
            current_file.as_mut().expect("current file").old_path = Some(path.to_owned());
            continue;
        }
        if let Some(path) = line.strip_prefix("rename to ") {
            current_file.as_mut().expect("current file").new_path = Some(path.to_owned());
            continue;
        }
        if line.starts_with("Binary files ") || line == "GIT binary patch" {
            current_file.as_mut().expect("current file").is_binary = true;
            continue;
        }
        if line.starts_with("@@ ") {
            finish_hunk(&mut current_file, &mut current_hunk);
            let file_key = current_file.as_ref().expect("current file").key();
            let range = parse_hunk_header(line)?;
            old_line = range.old_start;
            new_line = range.new_start;
            current_hunk = Some(ReviewHunk {
                id: HunkId::new(format!(
                    "{}:{}:{}",
                    file_key, range.old_start, range.new_start
                )),
                header: line.to_owned(),
                old_start: range.old_start,
                old_lines: range.old_lines,
                new_start: range.new_start,
                new_lines: range.new_lines,
                rows: Vec::new(),
            });
            continue;
        }

        let Some(hunk) = current_hunk.as_mut() else {
            continue;
        };
        if line.starts_with('\\') {
            continue;
        }

        let Some(marker) = line.as_bytes().first().copied() else {
            continue;
        };
        let text = line[1..].to_owned();
        match marker {
            b' ' => {
                hunk.rows.push(DiffRow {
                    kind: DiffRowKind::Context,
                    old_line: Some(old_line),
                    new_line: Some(new_line),
                    text,
                });
                old_line += 1;
                new_line += 1;
            }
            b'+' => {
                hunk.rows.push(DiffRow {
                    kind: DiffRowKind::Added,
                    old_line: None,
                    new_line: Some(new_line),
                    text,
                });
                new_line += 1;
            }
            b'-' => {
                hunk.rows.push(DiffRow {
                    kind: DiffRowKind::Removed,
                    old_line: Some(old_line),
                    new_line: None,
                    text,
                });
                old_line += 1;
            }
            _ => {}
        }
    }

    finish_hunk(&mut current_file, &mut current_hunk);
    if let Some(file) = current_file {
        files.push(file);
    }

    Ok(files)
}

fn finish_hunk(file: &mut Option<PatchFile>, hunk: &mut Option<ReviewHunk>) {
    if let (Some(file), Some(hunk)) = (file.as_mut(), hunk.take()) {
        file.hunks.push(hunk);
    }
}

fn parse_patch_path(path: &str) -> Option<String> {
    match path {
        "/dev/null" => None,
        path => path
            .strip_prefix("a/")
            .or_else(|| path.strip_prefix("b/"))
            .or(Some(path))
            .map(str::to_owned),
    }
}

fn diff_git_path(line: &str, prefix: char) -> Option<String> {
    match prefix {
        'a' => line
            .strip_prefix("diff --git a/")
            .and_then(|tail| tail.split_once(" b/").map(|(path, _)| path.to_owned())),
        'b' => line.split_once(" b/").map(|(_, path)| path.to_owned()),
        _ => None,
    }
}

#[derive(Debug)]
struct HunkRange {
    old_start: u32,
    old_lines: u32,
    new_start: u32,
    new_lines: u32,
}

fn parse_hunk_header(header: &str) -> Result<HunkRange> {
    let mut parts = header.split_whitespace();
    let _at = parts.next();
    let old = parts
        .next()
        .ok_or_else(|| ShoreError::Message(format!("missing old range in hunk header {header}")))?;
    let new = parts
        .next()
        .ok_or_else(|| ShoreError::Message(format!("missing new range in hunk header {header}")))?;
    let (old_start, old_lines) = parse_range(old, '-')?;
    let (new_start, new_lines) = parse_range(new, '+')?;

    Ok(HunkRange {
        old_start,
        old_lines,
        new_start,
        new_lines,
    })
}

fn parse_range(range: &str, prefix: char) -> Result<(u32, u32)> {
    let range = range
        .strip_prefix(prefix)
        .ok_or_else(|| ShoreError::Message(format!("range {range} missing prefix {prefix}")))?;
    let (start, lines) = match range.split_once(',') {
        Some((start, lines)) => (start, lines),
        None => (range, "1"),
    };
    Ok((
        start.parse().map_err(|error| {
            ShoreError::Message(format!("invalid range start {start}: {error}"))
        })?,
        lines.parse().map_err(|error| {
            ShoreError::Message(format!("invalid range length {lines}: {error}"))
        })?,
    ))
}
