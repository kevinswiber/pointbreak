use std::collections::BTreeMap;
use std::path::Path;

use crate::error::{Result, ShoreError};
use crate::git::command::run_git;
use crate::git::patch::{PatchFile, parse_patch};
use crate::git::raw::parse_raw;
use crate::model::{
    DiffFile, DiffSnapshot, FileId, FileMetadataKind, FileMetadataRow, ReviewId, SnapshotId,
};

pub fn ingest_tracked_diff(repo: impl AsRef<Path>) -> Result<DiffSnapshot> {
    let repo = repo.as_ref();
    let raw_output = run_git(
        repo,
        [
            "diff",
            "--raw",
            "-z",
            "--no-ext-diff",
            "--no-color",
            "--full-index",
            "-M",
            "-C",
            "--submodule=short",
            "HEAD",
            "--",
        ],
    )?;
    let patch_output = run_git(
        repo,
        [
            "diff",
            "--patch",
            "--no-ext-diff",
            "--no-color",
            "--full-index",
            "-M",
            "-C",
            "--submodule=short",
            "HEAD",
            "--",
        ],
    )?;

    let raw_files = parse_raw(&raw_output.stdout)?;
    let patch_files = parse_patch(&String::from_utf8_lossy(&patch_output.stdout))?
        .into_iter()
        .map(|file| (file.key(), file))
        .collect::<BTreeMap<_, _>>();

    let files = raw_files
        .into_iter()
        .map(|raw_file| {
            let key = raw_file.key();
            let patch_file = patch_files
                .get(&key)
                .ok_or_else(|| ShoreError::Message(format!("missing patch entry for {key}")))?;
            diff_file(raw_file, patch_file)
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(DiffSnapshot::new(
        ReviewId::new("working-tree"),
        SnapshotId::new("git-diff-head"),
        files,
    ))
}

fn diff_file(raw_file: crate::git::raw::RawFile, patch_file: &PatchFile) -> Result<DiffFile> {
    let is_submodule = raw_file.is_submodule();
    let is_mode_only = raw_file.is_mode_only();
    let is_binary = patch_file.is_binary;
    let metadata_rows = metadata_rows(&raw_file, patch_file);
    let hunks = if metadata_rows.is_empty() {
        patch_file.hunks.clone()
    } else {
        Vec::new()
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
        metadata_rows,
        hunks,
    })
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
    if raw_file.is_mode_only() {
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
