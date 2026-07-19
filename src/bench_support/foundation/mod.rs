mod bundle_v2;
mod candidate;
mod codec;
mod content;
mod contract;
mod corpus;
mod documents;
mod fault;
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
    let output = std::process::Command::new("stat")
        .args(["-f", "-c", "%T"])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(target_os = "windows")]
fn platform_filesystem_name(path: &Path) -> Option<String> {
    let canonical = path.canonicalize().ok()?;
    let volume = match windows_filesystem_location(&canonical)? {
        WindowsFilesystemLocation::Local(volume) => volume,
        WindowsFilesystemLocation::Network => return Some("smb".to_owned()),
    };
    let output = std::process::Command::new("fsutil")
        .args(["fsinfo", "volumeinfo"])
        .arg(volume)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_windows_fsutil_filesystem(&String::from_utf8(output.stdout).ok()?)
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

#[cfg(target_os = "windows")]
fn parse_windows_fsutil_filesystem(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (label, value) = line.split_once(':')?;
        label
            .trim()
            .eq_ignore_ascii_case("File System Name")
            .then(|| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
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
            Some("NTFS")
        );
    }

    #[test]
    fn windows_probe_reports_a_local_filesystem_type() {
        assert_eq!(
            qualification_filesystem_name(Path::new(env!("CARGO_MANIFEST_DIR")))
                .to_ascii_lowercase(),
            "ntfs"
        );
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn platform_filesystem_name(_path: &Path) -> Option<String> {
    None
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
