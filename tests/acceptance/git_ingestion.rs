use shore::git::ingest_tracked_diff;
use shore::model::{DiffFile, DiffRowKind, FileMetadataKind, FileStatus};

use crate::support::git_repo::GitRepo;
use crate::support::snapshots::normalize_path;

#[test]
fn scratch_repo_can_create_commit_modify_and_report_status() {
    let repo = GitRepo::new();

    repo.write("src/lib.rs", "pub fn original() {}\n");
    repo.commit_all("initial commit");
    repo.write("src/lib.rs", "pub fn changed() {}\n");
    repo.write("obsolete.rs", "remove me\n");
    repo.remove("obsolete.rs");

    let status = repo.git(["status", "--porcelain=v2", "--untracked-files=all"]);

    assert!(repo.path().join("src/lib.rs").exists());
    assert!(!repo.path().join("obsolete.rs").exists());
    assert!(!normalize_path(repo.path()).is_empty());
    assert!(
        status.stderr.is_empty(),
        "status command should not write stderr:\n{}",
        status.stderr
    );
    assert!(
        status.stdout.contains("src/lib.rs"),
        "status output should mention modified file:\n{}",
        status.stdout
    );
}

#[test]
fn tracked_text_diff_ingests_modified_added_and_deleted_files() {
    let repo = GitRepo::new();
    repo.write(
        "src/modified.rs",
        "fn keep() {}\nfn old_name() {}\nfn tail() {}\n",
    );
    repo.write("src/deleted.rs", "fn gone() {}\nfn deleted_tail() {}\n");
    repo.commit_all("initial commit");

    repo.write(
        "src/modified.rs",
        "fn keep() {}\nfn new_name() {}\nfn tail() {}\n",
    );
    repo.write("src/added.rs", "fn added() {}\nfn added_tail() {}\n");
    repo.remove("src/deleted.rs");
    repo.git(["add", "--all"]);

    let snapshot = ingest_tracked_diff(repo.path()).expect("tracked diff ingests");

    assert_eq!(snapshot.files.len(), 3);
    assert!(!snapshot.snapshot_id.as_str().is_empty());

    let added = file_by_path(&snapshot.files, "src/added.rs");
    assert_eq!(added.status, FileStatus::Added);
    assert_eq!(added.old_path, None);
    assert_eq!(added.new_path.as_deref(), Some("src/added.rs"));
    assert_eq!(added.hunks.len(), 1);
    assert_eq!(added.hunks[0].old_start, 0);
    assert_eq!(added.hunks[0].old_lines, 0);
    assert_eq!(added.hunks[0].new_start, 1);
    assert_eq!(added.hunks[0].new_lines, 2);
    assert_eq!(added.hunks[0].rows.len(), 2);
    assert_eq!(added.hunks[0].rows[0].kind, DiffRowKind::Added);
    assert_eq!(added.hunks[0].rows[0].old_line, None);
    assert_eq!(added.hunks[0].rows[0].new_line, Some(1));
    assert_eq!(added.hunks[0].rows[1].new_line, Some(2));

    let deleted = file_by_path(&snapshot.files, "src/deleted.rs");
    assert_eq!(deleted.status, FileStatus::Deleted);
    assert_eq!(deleted.old_path.as_deref(), Some("src/deleted.rs"));
    assert_eq!(deleted.new_path, None);
    assert_eq!(deleted.hunks.len(), 1);
    assert_eq!(deleted.hunks[0].old_start, 1);
    assert_eq!(deleted.hunks[0].old_lines, 2);
    assert_eq!(deleted.hunks[0].new_start, 0);
    assert_eq!(deleted.hunks[0].new_lines, 0);
    assert_eq!(deleted.hunks[0].rows[0].kind, DiffRowKind::Removed);
    assert_eq!(deleted.hunks[0].rows[0].old_line, Some(1));
    assert_eq!(deleted.hunks[0].rows[0].new_line, None);
    assert_eq!(deleted.hunks[0].rows[1].old_line, Some(2));

    let modified = file_by_path(&snapshot.files, "src/modified.rs");
    assert_eq!(modified.status, FileStatus::Modified);
    assert_eq!(modified.old_path.as_deref(), Some("src/modified.rs"));
    assert_eq!(modified.new_path.as_deref(), Some("src/modified.rs"));
    assert_eq!(modified.hunks.len(), 1);
    assert_eq!(modified.hunks[0].old_start, 1);
    assert_eq!(modified.hunks[0].old_lines, 3);
    assert_eq!(modified.hunks[0].new_start, 1);
    assert_eq!(modified.hunks[0].new_lines, 3);

    let rows = &modified.hunks[0].rows;
    assert_eq!(rows[0].kind, DiffRowKind::Context);
    assert_eq!(rows[0].old_line, Some(1));
    assert_eq!(rows[0].new_line, Some(1));
    assert_eq!(rows[1].kind, DiffRowKind::Removed);
    assert_eq!(rows[1].old_line, Some(2));
    assert_eq!(rows[1].new_line, None);
    assert_eq!(rows[2].kind, DiffRowKind::Added);
    assert_eq!(rows[2].old_line, None);
    assert_eq!(rows[2].new_line, Some(2));
    assert_eq!(rows[3].kind, DiffRowKind::Context);
    assert_eq!(rows[3].old_line, Some(3));
    assert_eq!(rows[3].new_line, Some(3));
}

#[test]
fn file_level_git_entries_are_preserved_as_metadata_rows() {
    let repo = GitRepo::new();
    let submodule_source = GitRepo::new();
    submodule_source.write("lib.rs", "pub fn submodule() -> u8 { 1 }\n");
    submodule_source.commit_all("submodule initial");

    repo.write("src/old_name.rs", "pub fn renamed() {}\n");
    repo.write("assets/data.bin", [0, 159, 146, 150]);
    repo.write("scripts/run.sh", "#!/bin/sh\necho shore\n");
    repo.git(vec![
        "-c".to_owned(),
        "protocol.file.allow=always".to_owned(),
        "submodule".to_owned(),
        "add".to_owned(),
        submodule_source.path().to_string_lossy().into_owned(),
        "deps/sub".to_owned(),
    ]);
    repo.commit_all("initial file-level fixtures");

    repo.git(["mv", "src/old_name.rs", "src/new_name.rs"]);
    repo.write("assets/data.bin", [0, 159, 146, 151]);
    make_executable(repo.path().join("scripts/run.sh"));

    submodule_source.write("lib.rs", "pub fn submodule() -> u8 { 2 }\n");
    submodule_source.commit_all("submodule update");
    let submodule_branch = submodule_source.git(["branch", "--show-current"]).stdout;
    let submodule_branch = submodule_branch.trim();
    let submodule_path = repo.path().join("deps/sub").to_string_lossy().into_owned();
    repo.git(vec![
        "-C".to_owned(),
        submodule_path.clone(),
        "fetch".to_owned(),
    ]);
    repo.git(vec![
        "-C".to_owned(),
        submodule_path.clone(),
        "checkout".to_owned(),
        submodule_branch.to_owned(),
    ]);
    repo.git(vec!["-C".to_owned(), submodule_path, "pull".to_owned()]);

    let snapshot = ingest_tracked_diff(repo.path()).expect("tracked diff ingests");

    let renamed = file_by_path(&snapshot.files, "src/new_name.rs");
    assert_eq!(renamed.status, FileStatus::Renamed);
    assert_eq!(renamed.old_path.as_deref(), Some("src/old_name.rs"));
    assert_eq!(renamed.new_path.as_deref(), Some("src/new_name.rs"));
    assert_eq!(renamed.similarity, Some(100));
    assert_eq!(
        metadata_kinds(renamed),
        vec![FileMetadataKind::RenameSummary]
    );
    assert!(renamed.hunks.is_empty());

    let binary = file_by_path(&snapshot.files, "assets/data.bin");
    assert_eq!(binary.status, FileStatus::Modified);
    assert!(binary.is_binary);
    assert_eq!(
        metadata_kinds(binary),
        vec![FileMetadataKind::BinarySummary]
    );
    assert!(binary.hunks.is_empty());

    let mode_only = file_by_path(&snapshot.files, "scripts/run.sh");
    assert_eq!(mode_only.status, FileStatus::Modified);
    assert!(mode_only.is_mode_only);
    assert_eq!(mode_only.old_mode.as_deref(), Some("100644"));
    assert_eq!(mode_only.new_mode.as_deref(), Some("100755"));
    assert_eq!(
        metadata_kinds(mode_only),
        vec![FileMetadataKind::ModeChange]
    );
    assert!(mode_only.hunks.is_empty());

    let submodule = file_by_path(&snapshot.files, "deps/sub");
    assert_eq!(submodule.status, FileStatus::Modified);
    assert!(submodule.is_submodule);
    assert_eq!(
        metadata_kinds(submodule),
        vec![FileMetadataKind::SubmoduleSummary]
    );
    assert!(submodule.hunks.is_empty());
}

#[test]
fn untracked_files_are_synthesized_without_staging_them() {
    let repo = GitRepo::new();
    repo.write("src/tracked.rs", "pub fn tracked() -> u8 { 1 }\n");
    repo.commit_all("initial tracked file");

    repo.write("src/tracked.rs", "pub fn tracked() -> u8 { 2 }\n");
    repo.write("src/untracked.rs", "pub fn untracked() {}\n");
    repo.write("assets/untracked.bin", [0, 159, 146, 150]);

    let snapshot = ingest_tracked_diff(repo.path()).expect("working tree diff ingests");

    assert_eq!(
        snapshot
            .files
            .iter()
            .map(|file| file
                .new_path
                .as_deref()
                .or(file.old_path.as_deref())
                .unwrap())
            .collect::<Vec<_>>(),
        vec!["src/tracked.rs", "assets/untracked.bin", "src/untracked.rs"]
    );

    let tracked = file_by_path(&snapshot.files, "src/tracked.rs");
    assert_eq!(tracked.status, FileStatus::Modified);
    assert!(!tracked.synthetic);

    let untracked_text = file_by_path(&snapshot.files, "src/untracked.rs");
    assert_eq!(untracked_text.status, FileStatus::Added);
    assert!(untracked_text.synthetic);
    assert_eq!(untracked_text.old_path, None);
    assert_eq!(untracked_text.new_path.as_deref(), Some("src/untracked.rs"));
    assert_eq!(untracked_text.hunks.len(), 1);
    assert_eq!(untracked_text.hunks[0].old_start, 0);
    assert_eq!(untracked_text.hunks[0].old_lines, 0);
    assert_eq!(untracked_text.hunks[0].new_start, 1);
    assert_eq!(untracked_text.hunks[0].new_lines, 1);
    assert_eq!(untracked_text.hunks[0].rows[0].kind, DiffRowKind::Added);
    assert_eq!(untracked_text.hunks[0].rows[0].old_line, None);
    assert_eq!(untracked_text.hunks[0].rows[0].new_line, Some(1));

    let untracked_binary = file_by_path(&snapshot.files, "assets/untracked.bin");
    assert_eq!(untracked_binary.status, FileStatus::Added);
    assert!(untracked_binary.synthetic);
    assert!(untracked_binary.is_binary);
    assert_eq!(
        metadata_kinds(untracked_binary),
        vec![FileMetadataKind::BinarySummary]
    );
    assert!(untracked_binary.hunks.is_empty());

    let status = repo.git(["status", "--porcelain=v2", "--untracked-files=all"]);
    assert!(
        status.stdout.contains("? src/untracked.rs"),
        "untracked text file should remain unstaged:\n{}",
        status.stdout
    );
    assert!(
        status.stdout.contains("? assets/untracked.bin"),
        "untracked binary file should remain unstaged:\n{}",
        status.stdout
    );
}

fn file_by_path<'a>(files: &'a [DiffFile], path: &str) -> &'a DiffFile {
    files
        .iter()
        .find(|file| {
            file.old_path.as_deref() == Some(path) || file.new_path.as_deref() == Some(path)
        })
        .unwrap_or_else(|| panic!("missing diff file for {path}; files: {files:#?}"))
}

fn metadata_kinds(file: &DiffFile) -> Vec<FileMetadataKind> {
    file.metadata_rows
        .iter()
        .map(|row| row.kind.clone())
        .collect()
}

#[cfg(unix)]
fn make_executable(path: impl AsRef<std::path::Path>) {
    use std::os::unix::fs::PermissionsExt;

    let path = path.as_ref();
    let mut permissions = std::fs::metadata(path)
        .expect("read file metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("set executable bit");
}

#[cfg(not(unix))]
fn make_executable(path: impl AsRef<std::path::Path>) {
    let _ = path;
}
