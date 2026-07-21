mod bundle_v2;
mod candidate;
mod codec;
mod content;
mod contract;
mod corpus;
mod documents;
mod fault;
mod generated;
mod lifecycle;
mod migration;
mod performance;
mod proof;
mod receipt;
mod segments;
mod sqlite;

use std::path::Path;

pub use bundle_v2::*;
pub use candidate::*;
pub use codec::*;
pub use content::*;
pub use contract::*;
pub use corpus::*;
pub use documents::*;
pub use fault::*;
pub use generated::*;
pub use lifecycle::*;
pub use migration::*;
pub use performance::*;
pub use proof::*;
pub use receipt::*;
pub use segments::*;
pub use sqlite::*;

/// Report the filesystem type containing a qualification workload.
///
/// The platform commands are metadata-only: they inspect the supplied path
/// without reading any corpus files.
pub fn qualification_filesystem_name(path: &Path) -> String {
    platform_filesystem_name(path).unwrap_or_else(|| "unavailable".to_owned())
}

#[cfg(target_os = "macos")]
fn platform_filesystem_name(path: &Path) -> Option<String> {
    let output = std::process::Command::new("/bin/df")
        .arg("-Y")
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_macos_df_filesystem(&String::from_utf8(output.stdout).ok()?)
}

#[cfg(target_os = "macos")]
fn parse_macos_df_filesystem(output: &str) -> Option<String> {
    output
        .lines()
        .nth(1)?
        .split_whitespace()
        .nth(1)
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
}

#[cfg(target_os = "linux")]
fn platform_filesystem_name(path: &Path) -> Option<String> {
    let findmnt = std::process::Command::new("findmnt")
        .args(["--noheadings", "--output", "FSTYPE", "--target"])
        .arg(path)
        .output()
        .ok();
    if let Some(filesystem) = findmnt
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|output| parse_linux_findmnt_filesystem(&output))
    {
        return Some(filesystem);
    }

    let output = std::process::Command::new("df")
        .arg("-T")
        .arg(path)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
        .and_then(|output| parse_linux_df_filesystem(&output))
}

#[cfg(any(test, target_os = "linux"))]
fn parse_linux_findmnt_filesystem(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(any(test, target_os = "linux"))]
fn parse_linux_df_filesystem(output: &str) -> Option<String> {
    output
        .lines()
        .nth(1)?
        .split_whitespace()
        .nth(1)
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod linux_filesystem_parser_tests {
    use super::*;

    #[test]
    fn linux_filesystem_probe_parses_unambiguous_mount_types() {
        assert_eq!(
            parse_linux_findmnt_filesystem("ext4\n").as_deref(),
            Some("ext4")
        );
        assert_eq!(
            parse_linux_df_filesystem(
                "Filesystem Type 1K-blocks Used Available Use% Mounted on\n/dev/sda2 ext4 100 50 50 50% /\n"
            )
            .as_deref(),
            Some("ext4")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_filesystem_probe_reports_an_unambiguous_mount_type() {
        let filesystem = qualification_filesystem_name(Path::new(env!("CARGO_MANIFEST_DIR")));

        assert_ne!(filesystem, "unavailable");
        assert_ne!(filesystem, "ext2/ext3");
    }
}

#[cfg(test)]
mod windows_filesystem_fallback_tests {
    use super::*;

    #[test]
    fn windows_probe_falls_back_when_fsutil_requires_elevation() {
        assert_eq!(
            windows_filesystem_from_probe_outputs_v1(None, Some("NTFS\r\n")).as_deref(),
            Some("ntfs")
        );
        assert_eq!(
            windows_filesystem_from_probe_outputs_v1(
                Some("File System Name : ReFS\r\n"),
                Some("NTFS\r\n")
            )
            .as_deref(),
            Some("refs")
        );
    }
}

#[cfg(target_os = "windows")]
fn platform_filesystem_name(path: &Path) -> Option<String> {
    let canonical = path.canonicalize().ok()?;
    let volume = match windows_filesystem_location(&canonical)? {
        WindowsFilesystemLocation::Local(volume) => volume,
        WindowsFilesystemLocation::Network => return Some("smb".to_owned()),
    };
    let fsutil_output = std::process::Command::new("fsutil")
        .args(["fsinfo", "volumeinfo"])
        .arg(&volume)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok());
    if let Some(filesystem) =
        windows_filesystem_from_probe_outputs_v1(fsutil_output.as_deref(), None)
    {
        return Some(filesystem);
    }

    let drive_letter = volume.strip_suffix(':')?;
    let powershell_output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!(
            "(Get-Volume -DriveLetter {drive_letter}).FileSystem"
        ))
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok());
    windows_filesystem_from_probe_outputs_v1(None, powershell_output.as_deref())
}

#[cfg(target_os = "windows")]
#[derive(Debug, Eq, PartialEq)]
enum WindowsFilesystemLocation {
    Local(String),
    Network,
}

#[cfg(target_os = "windows")]
fn windows_filesystem_location(path: &Path) -> Option<WindowsFilesystemLocation> {
    use std::path::{Component, Prefix};

    let Component::Prefix(prefix) = path.components().next()? else {
        return None;
    };
    match prefix.kind() {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => Some(
            WindowsFilesystemLocation::Local(format!("{}:", char::from(letter))),
        ),
        Prefix::UNC(..) | Prefix::VerbatimUNC(..) => Some(WindowsFilesystemLocation::Network),
        _ => None,
    }
}

#[cfg(any(test, target_os = "windows"))]
fn windows_filesystem_from_probe_outputs_v1(
    fsutil_output: Option<&str>,
    powershell_output: Option<&str>,
) -> Option<String> {
    fsutil_output
        .and_then(parse_windows_fsutil_filesystem)
        .or_else(|| powershell_output.and_then(parse_windows_powershell_filesystem))
}

#[cfg(any(test, target_os = "windows"))]
fn parse_windows_fsutil_filesystem(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (label, value) = line.split_once(':')?;
        label
            .trim()
            .eq_ignore_ascii_case("File System Name")
            .then(|| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
    })
}

#[cfg(any(test, target_os = "windows"))]
fn parse_windows_powershell_filesystem(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
}

#[cfg(all(test, target_os = "windows"))]
mod windows_tests {
    use super::*;

    #[test]
    fn windows_probe_uses_the_volume_and_parses_its_filesystem() {
        assert_eq!(
            windows_filesystem_location(Path::new(r"C:\Users\test\qualification")),
            Some(WindowsFilesystemLocation::Local("C:".to_owned()))
        );
        assert_eq!(
            windows_filesystem_location(Path::new(r"\\server\share\qualification")),
            Some(WindowsFilesystemLocation::Network)
        );
        assert_eq!(
            parse_windows_fsutil_filesystem(
                "Volume Name :\r\nFile System Name : NTFS\r\nIs ReadWrite\r\n"
            )
            .as_deref(),
            Some("ntfs")
        );
    }

    #[test]
    fn windows_probe_reports_a_local_filesystem_type() {
        assert_eq!(
            qualification_filesystem_name(Path::new(env!("CARGO_MANIFEST_DIR"))),
            "ntfs"
        );
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn platform_filesystem_name(_path: &Path) -> Option<String> {
    None
}

#[cfg(test)]
mod migration_lifecycle_contract_tests {
    use super::*;

    #[test]
    fn exact_manifest_rejects_changed_bytes_and_identity() {
        let source = synthetic_legacy_manifest().expect("synthetic manifest");
        let expected = ExactLogicalManifestV1::from_corpus(&source).expect("exact source manifest");
        let mut changed = source.clone();
        changed.records[0].decoded_bytes.push(b'!');

        assert!(ExactLogicalManifestV1::from_corpus(&changed).is_err());
        assert!(expected.compare_corpus(&changed).is_err());
    }

    #[test]
    fn interrupted_migration_never_activates_a_partial_destination() {
        let root = tempfile::tempdir().expect("temporary migration root");
        let source = synthetic_legacy_manifest().expect("synthetic manifest");
        let result = rehearse_candidate_migration(
            &source,
            MigrationCandidateV1::SqliteWal,
            &root.path().join("destination"),
            MigrationFailurePointV1::AfterFirstWrite,
        );

        assert!(result.is_err());
        assert!(!root.path().join("destination/active").exists());
        assert_eq!(
            classify_rollback_boundary(false),
            MigrationRollbackBoundaryV1::SafeBeforeDestinationMutation
        );
        assert_eq!(
            classify_rollback_boundary(true),
            MigrationRollbackBoundaryV1::ForwardRepairRequired
        );
    }

    #[test]
    fn lifecycle_states_and_removed_proof_remain_distinct() {
        let report = validate_content_lifecycle_matrix(&content_lifecycle_fixture_v1())
            .expect("valid lifecycle matrix");

        assert!(report.required_states_complete);
        assert_eq!(report.proof_after_removal, ProofEvidenceStateV1::Removed);
        assert_ne!(
            ContentLifecycleStateV1::Missing,
            ContentLifecycleStateV1::Removed
        );
        assert_ne!(
            ContentLifecycleStateV1::TypedAbsence,
            ContentLifecycleStateV1::ValidOrphan
        );
    }

    #[test]
    fn cross_candidate_bundle_transfer_is_exact_and_conflicts_preflight() {
        let root = tempfile::tempdir().expect("temporary transfer root");
        let source = modeled_post_foundation_manifest().expect("modeled manifest");
        let report = rehearse_cross_candidate_transfers(&source, root.path())
            .expect("cross-candidate transfer");

        assert!(report.paths.iter().all(|path| path.exact_manifest_match));
        assert!(report.hard_conflict_rejected_before_write);
        assert!(report.omission_preserved_existing_content);
    }

    #[test]
    fn immutable_archive_requires_complete_manifest_bound_prefix() {
        let root = tempfile::tempdir().expect("temporary archive root");
        let profile =
            SqliteQualificationProfile::open(&root.path().join("profile")).expect("SQLite profile");
        profile
            .journal()
            .create_once("events/one.json", br#"{"event":"one"}"#)
            .expect("seed event");
        let backup = root.path().join("backup");
        profile.backup_to(&backup).expect("completed backup");
        let archive = root.path().join("archive");

        let report = copy_completed_backup_to_immutable_prefix(
            &backup,
            &archive,
            &profile.descriptor().expect("descriptor"),
            ArchiveCopyFailurePointV1::None,
        )
        .expect("immutable archive copy");
        assert!(report.completion_published_last);
        assert!(profile.verify_restore(&archive).is_ok());

        let incomplete = root.path().join("incomplete");
        assert!(
            copy_completed_backup_to_immutable_prefix(
                &backup,
                &incomplete,
                &profile.descriptor().expect("descriptor"),
                ArchiveCopyFailurePointV1::BeforeCompletion,
            )
            .is_err()
        );
        assert!(profile.verify_restore(&incomplete).is_err());
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn macos_df_parser_reads_the_filesystem_type_column() {
        let output =
            "Filesystem Type 512-blocks Mounted on\n/dev/disk3s5 apfs 100 /System/Volumes/Data\n";

        assert_eq!(parse_macos_df_filesystem(output).as_deref(), Some("apfs"));
    }

    #[test]
    fn macos_probe_reports_a_filesystem_type() {
        let filesystem = qualification_filesystem_name(Path::new(env!("CARGO_MANIFEST_DIR")));

        assert_ne!(filesystem, "unavailable");
        assert_ne!(filesystem, "/");
        assert_ne!(filesystem, "Directory");
    }
}
