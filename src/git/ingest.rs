use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::{Result, ShoreError};
use crate::git::command::{git_worktree_root, run_git, run_git_allowing_statuses};
use crate::git::patch::{PatchFile, parse_patch};
use crate::git::raw::{RawFile, parse_raw};
use crate::model::{DiffFile, DiffSnapshot, FileId, FileMetadataKind, FileMetadataRow, ReviewId};
use crate::session::worktree_fingerprint_for_files;

#[derive(Clone, Debug, Default)]
pub struct IngestOptions {
    helper_paths: Vec<PathBuf>,
}

impl IngestOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn exclude_helper_path(mut self, path: impl AsRef<Path>) -> Self {
        self.helper_paths.push(path.as_ref().to_path_buf());
        self
    }
}

pub fn ingest_tracked_diff(repo: impl AsRef<Path>) -> Result<DiffSnapshot> {
    ingest_tracked_diff_with_options(repo, IngestOptions::new())
}

pub fn ingest_tracked_diff_with_options(
    repo: impl AsRef<Path>,
    options: IngestOptions,
) -> Result<DiffSnapshot> {
    let repo = repo.as_ref();
    let files = filter_helper_paths(
        capture_worktree_diff_files(repo)?,
        repo,
        &options.helper_paths,
    )?;
    let fingerprint = worktree_fingerprint_for_files(repo, &files)?;

    Ok(DiffSnapshot::new(
        ReviewId::new("working-tree"),
        fingerprint.snapshot_id,
        files,
    ))
}

fn filter_helper_paths(
    files: Vec<DiffFile>,
    repo: &Path,
    helper_paths: &[PathBuf],
) -> Result<Vec<DiffFile>> {
    let excluded_paths = excluded_git_paths(repo, helper_paths)?;
    if excluded_paths.is_empty() {
        return Ok(files);
    }

    Ok(files
        .into_iter()
        .filter(|file| {
            !file
                .old_path
                .as_deref()
                .is_some_and(|path| excluded_paths.contains(path))
                && !file
                    .new_path
                    .as_deref()
                    .is_some_and(|path| excluded_paths.contains(path))
        })
        .collect())
}

fn excluded_git_paths(repo: &Path, helper_paths: &[PathBuf]) -> Result<BTreeSet<String>> {
    let worktree_root = git_worktree_root(repo)?;
    let worktree_root = worktree_root.canonicalize().map_err(|error| {
        ShoreError::Message(format!(
            "canonicalize git worktree root {}: {error}",
            worktree_root.display()
        ))
    })?;
    let paths = helper_paths
        .iter()
        .filter_map(|path| {
            let helper_path = path.canonicalize().ok()?;
            let relative = helper_path.strip_prefix(&worktree_root).ok()?;
            Some(
                relative
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/"),
            )
        })
        .collect();
    Ok(paths)
}

/// Diff flags shared by every `git diff` pass, single-sourced so the worktree
/// and commit-range paths cannot drift apart. Mode flags (`--raw -z` vs
/// `--patch`) and the endpoint args are layered on per call.
const SHARED_DIFF_FLAGS: &[&str] = &[
    "--no-ext-diff",
    "--no-color",
    "--full-index",
    "-M",
    "-C",
    "--submodule=short",
];

/// Assemble a `git diff` argument list: `diff <mode_flags> <shared> <endpoints> --`.
fn diff_args<'a>(mode_flags: &[&'a str], endpoint_args: &[&'a str]) -> Vec<&'a str> {
    let mut args = Vec::with_capacity(2 + mode_flags.len() + SHARED_DIFF_FLAGS.len());
    args.push("diff");
    args.extend_from_slice(mode_flags);
    args.extend_from_slice(SHARED_DIFF_FLAGS);
    args.extend_from_slice(endpoint_args);
    args.push("--");
    args
}

/// Run the raw + patch passes for `endpoint_args` and merge them into the
/// `DiffFile` row inventory. Diff-source-agnostic: callers supply the endpoints
/// (`["HEAD"]` for the worktree, `[base_oid, target_oid]` for a commit range).
fn diff_files_for_args(repo: &Path, endpoint_args: &[&str]) -> Result<Vec<DiffFile>> {
    let raw_output = run_git(repo, diff_args(&["--raw", "-z"], endpoint_args))?;
    let patch_output = run_git(repo, diff_args(&["--patch"], endpoint_args))?;

    let raw_files = parse_raw(&raw_output.stdout)?;
    let patch_files =
        patch_files_by_key(parse_patch(&String::from_utf8_lossy(&patch_output.stdout))?);

    raw_files
        .into_iter()
        .map(|raw_file| {
            let key = raw_file.key();
            let patch_file = patch_files
                .get(&key)
                .ok_or_else(|| ShoreError::Message(format!("missing patch entry for {key}")))?;
            diff_file(raw_file, patch_file, false)
        })
        .collect::<Result<Vec<_>>>()
}

pub(crate) fn capture_worktree_diff_files(repo: &Path) -> Result<Vec<DiffFile>> {
    let mut files = diff_files_for_args(repo, &["HEAD"])?;
    files.extend(synthesize_untracked_files(repo)?);
    Ok(files)
}

/// Tree diff between two resolved commits. Never reads the working tree or
/// index and never synthesizes untracked rows: the inventory is exactly
/// `git diff <base_oid> <target_oid>`. Callers pass already-resolved OIDs
/// (see `git_rev_parse_commit_oid`), so there is no `--end-of-options` concern.
pub(crate) fn capture_commit_range_diff_files(
    repo: &Path,
    base_oid: &str,
    target_oid: &str,
) -> Result<Vec<DiffFile>> {
    diff_files_for_args(repo, &[base_oid, target_oid])
}

fn synthesize_untracked_files(repo: &Path) -> Result<Vec<DiffFile>> {
    discover_untracked_files(repo)?
        .into_iter()
        .map(|path| synthesize_untracked_file(repo, &path))
        .collect()
}

fn discover_untracked_files(repo: &Path) -> Result<Vec<String>> {
    let output = run_git(
        repo,
        ["ls-files", "--others", "--exclude-standard", "-z", "--"],
    )?;
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| {
            std::str::from_utf8(field)
                .map(str::to_owned)
                .map_err(|error| {
                    ShoreError::Message(format!("untracked path is not utf-8: {error}"))
                })
        })
        .collect()
}

fn synthesize_untracked_file(repo: &Path, path: &str) -> Result<DiffFile> {
    let patch = no_index_patch(repo, path)?;
    let patch_file = parse_patch(&String::from_utf8_lossy(&patch.stdout))?
        .into_iter()
        .next()
        .ok_or_else(|| ShoreError::Message(format!("missing no-index patch for {path}")))?;
    let raw_file = RawFile {
        status: crate::model::FileStatus::Added,
        old_mode: None,
        new_mode: patch_file.new_mode.clone(),
        type_change: false,
        old_oid: None,
        new_oid: None,
        similarity: None,
        old_path: None,
        new_path: Some(path.to_owned()),
    };
    diff_file(raw_file, &patch_file, true)
}

fn no_index_patch(repo: &Path, path: &str) -> Result<crate::git::command::GitOutput> {
    let args = vec![
        OsString::from("diff"),
        OsString::from("--no-index"),
        OsString::from("--no-ext-diff"),
        OsString::from("--no-color"),
        OsString::from("--full-index"),
        OsString::from("--"),
        OsString::from("/dev/null"),
        OsString::from(PathBuf::from(path)),
    ];
    run_git_allowing_statuses(repo, args, &[0, 1])
}

fn diff_file(raw_file: RawFile, patch_file: &PatchFile, synthetic: bool) -> Result<DiffFile> {
    let is_submodule = raw_file.is_submodule();
    let is_binary = patch_file.is_binary;
    let is_mode_only = is_mode_only(&raw_file, patch_file);
    let metadata_rows = metadata_rows(&raw_file, patch_file);
    let omit_hunks = is_binary || is_submodule || is_mode_only;
    let hunks = if omit_hunks {
        Vec::new()
    } else {
        patch_file.hunks.clone()
    };

    Ok(DiffFile {
        id: FileId::new(raw_file.key()),
        status: raw_file.status,
        old_path: raw_file.old_path,
        new_path: raw_file.new_path,
        old_mode: raw_file.old_mode.or_else(|| patch_file.old_mode.clone()),
        new_mode: raw_file.new_mode.or_else(|| patch_file.new_mode.clone()),
        old_oid: raw_file.old_oid,
        new_oid: raw_file.new_oid,
        similarity: raw_file.similarity.or(patch_file.similarity),
        is_binary,
        is_submodule,
        is_mode_only,
        synthetic,
        metadata_rows,
        hunks,
    })
}

fn patch_files_by_key(files: Vec<PatchFile>) -> BTreeMap<String, PatchFile> {
    let mut by_key = BTreeMap::new();
    for file in files {
        match by_key.entry(file.key()) {
            Entry::Vacant(entry) => {
                entry.insert(file);
            }
            Entry::Occupied(mut entry) => {
                merge_patch_file(entry.get_mut(), file);
            }
        }
    }
    by_key
}

fn merge_patch_file(existing: &mut PatchFile, next: PatchFile) {
    fill_missing(&mut existing.old_path, next.old_path);
    fill_missing(&mut existing.new_path, next.new_path);
    fill_missing(&mut existing.old_mode, next.old_mode);
    fill_missing(&mut existing.new_mode, next.new_mode);
    fill_missing(&mut existing.similarity, next.similarity);
    existing.is_binary |= next.is_binary;
    existing.hunks.extend(next.hunks);
}

fn fill_missing<T>(current: &mut Option<T>, next: Option<T>) {
    if current.is_none() {
        *current = next;
    }
}

fn is_mode_only(raw_file: &RawFile, patch_file: &PatchFile) -> bool {
    raw_file.has_mode_change()
        && !raw_file.type_change
        && !patch_file.is_binary
        && patch_file.hunks.is_empty()
}

fn metadata_rows(
    raw_file: &crate::git::raw::RawFile,
    patch_file: &PatchFile,
) -> Vec<FileMetadataRow> {
    let mut rows = Vec::new();
    if matches!(
        raw_file.status,
        crate::model::FileStatus::Renamed | crate::model::FileStatus::Copied
    ) {
        rows.push(FileMetadataRow {
            kind: FileMetadataKind::RenameSummary,
            text: match (&raw_file.old_path, &raw_file.new_path, raw_file.similarity) {
                (Some(old), Some(new), Some(similarity)) => {
                    format!("renamed {old} -> {new} ({similarity}%)")
                }
                (Some(old), Some(new), None) => format!("renamed {old} -> {new}"),
                _ => "renamed file".to_owned(),
            },
        });
    }
    if patch_file.is_binary {
        rows.push(FileMetadataRow {
            kind: FileMetadataKind::BinarySummary,
            text: "binary files differ".to_owned(),
        });
    }
    if raw_file.has_mode_change() {
        rows.push(FileMetadataRow {
            kind: FileMetadataKind::ModeChange,
            text: match (&raw_file.old_mode, &raw_file.new_mode) {
                (Some(old), Some(new)) => format!("mode changed {old} -> {new}"),
                _ => "mode changed".to_owned(),
            },
        });
    }
    if raw_file.is_submodule() {
        rows.push(FileMetadataRow {
            kind: FileMetadataKind::SubmoduleSummary,
            text: match (&raw_file.old_oid, &raw_file.new_oid) {
                (Some(old), Some(new)) => format!("submodule changed {old} -> {new}"),
                _ => "submodule changed".to_owned(),
            },
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use super::capture_commit_range_diff_files;
    use crate::git::command::run_git;
    use crate::model::{FileMetadataKind, FileStatus};

    #[test]
    fn commit_range_diff_files_capture_committed_change_with_clean_worktree() {
        let repo = TestRepo::new();
        repo.write("file.txt", "one\n");
        repo.commit_all("base");
        let base_oid = repo.rev_parse("HEAD");
        repo.write("file.txt", "two\n");
        repo.commit_all("change");
        let head_oid = repo.rev_parse("HEAD");

        let files = capture_commit_range_diff_files(repo.path(), &base_oid, &head_oid).unwrap();

        assert_eq!(files.len(), 1);
        let file = &files[0];
        assert_eq!(file.status, FileStatus::Modified);
        assert_eq!(file.new_path.as_deref(), Some("file.txt"));
        assert!(!file.hunks.is_empty());
        // Both sides of a committed range diff carry real blob oids.
        assert!(file.old_oid.as_deref().is_some_and(|oid| !oid.is_empty()));
        assert!(file.new_oid.as_deref().is_some_and(|oid| !oid.is_empty()));
        assert!(files.iter().all(|file| !file.synthetic));
    }

    #[test]
    fn commit_range_diff_files_ignore_worktree_and_untracked_state() {
        let repo = TestRepo::new();
        repo.write("file.txt", "one\n");
        repo.commit_all("base");
        let base_oid = repo.rev_parse("HEAD");
        repo.write("file.txt", "two\n");
        repo.commit_all("change");
        let head_oid = repo.rev_parse("HEAD");

        let clean = capture_commit_range_diff_files(repo.path(), &base_oid, &head_oid).unwrap();

        // Dirty the tracked file and add an untracked file; the tree diff must not see either.
        repo.write("file.txt", "dirty worktree edit\n");
        repo.write("untracked.txt", "untracked\n");
        let dirty = capture_commit_range_diff_files(repo.path(), &base_oid, &head_oid).unwrap();

        assert_eq!(clean, dirty);
        assert!(
            dirty
                .iter()
                .all(|file| file.new_path.as_deref() != Some("untracked.txt"))
        );
    }

    #[test]
    fn commit_range_diff_files_preserve_rename_detection() {
        let repo = TestRepo::new();
        repo.write("original.txt", "line one\nline two\nline three\n");
        repo.commit_all("base");
        let base_oid = repo.rev_parse("HEAD");
        repo.git(["mv", "original.txt", "renamed.txt"]);
        repo.commit_all("rename");
        let head_oid = repo.rev_parse("HEAD");

        let files = capture_commit_range_diff_files(repo.path(), &base_oid, &head_oid).unwrap();

        assert_eq!(files.len(), 1);
        let file = &files[0];
        assert_eq!(file.status, FileStatus::Renamed);
        assert_eq!(file.old_path.as_deref(), Some("original.txt"));
        assert_eq!(file.new_path.as_deref(), Some("renamed.txt"));
        assert!(
            file.metadata_rows
                .iter()
                .any(|row| matches!(row.kind, FileMetadataKind::RenameSummary))
        );
    }

    #[test]
    fn commit_range_diff_files_for_identical_trees_are_empty() {
        let repo = TestRepo::new();
        repo.write("file.txt", "one\n");
        repo.commit_all("base");
        let head_oid = repo.rev_parse("HEAD");

        let files = capture_commit_range_diff_files(repo.path(), &head_oid, &head_oid).unwrap();

        assert!(files.is_empty());
    }

    struct TestRepo {
        root: tempfile::TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("create temp git repository directory");
            let repo = Self { root };

            repo.git(["init"]);
            repo.git(["config", "user.name", "Shore Tests"]);
            repo.git(["config", "user.email", "shore-tests@example.com"]);
            repo.git(["config", "commit.gpgsign", "false"]);

            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
            let path = self.root.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directories");
            }
            fs::write(path, contents).expect("write test repository file");
        }

        fn commit_all(&self, message: &str) {
            self.git(["add", "--all"]);
            self.git(["commit", "-m", message]);
        }

        fn rev_parse(&self, rev: &str) -> String {
            let output = run_git(self.root.path(), ["rev-parse", rev]).unwrap();
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            let args = args
                .into_iter()
                .map(|arg| arg.as_ref().to_owned())
                .collect::<Vec<_>>();
            let output = Command::new("git")
                .args(&args)
                .current_dir(self.root.path())
                .output()
                .unwrap_or_else(|error| panic!("run git {:?}: {error}", args));

            assert!(
                output.status.success(),
                "git {:?} failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
                args,
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
