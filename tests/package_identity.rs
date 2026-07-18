use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;
use std::{fs, io};

use serde_json::Value;

#[cfg(windows)]
const POINTBREAK_EXECUTABLE_BASENAME: &str = "pointbreak.exe";
#[cfg(not(windows))]
const POINTBREAK_EXECUTABLE_BASENAME: &str = "pointbreak";

#[test]
fn package_and_library_identity_are_pointbreak() {
    assert_eq!(env!("CARGO_PKG_NAME"), "pointbreak");

    let _ = std::any::type_name::<pointbreak::model::DiffSnapshot>();
}

#[test]
fn package_identity_declares_only_pointbreak_binary() {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("metadata is JSON");
    let package = metadata["packages"]
        .as_array()
        .expect("packages is an array")
        .iter()
        .find(|package| package["name"] == "pointbreak")
        .expect("pointbreak package exists");
    let binary_targets = package["targets"]
        .as_array()
        .expect("targets is an array")
        .iter()
        .filter(|target| {
            target["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
        })
        .map(|target| target["name"].as_str().expect("target name"))
        .collect::<Vec<_>>();

    assert_eq!(binary_targets, ["pointbreak"]);
    assert_eq!(
        Path::new(env!("CARGO_BIN_EXE_pointbreak")).file_name(),
        Some(OsStr::new(POINTBREAK_EXECUTABLE_BASENAME))
    );

    let legacy: Value =
        serde_json::from_str(include_str!("fixtures/packages/legacy-review-package.json"))
            .expect("legacy package fixture is JSON");
    assert_eq!(legacy["ownedExecutables"], serde_json::json!(["shore"]));
    assert!(
        binary_targets
            .iter()
            .all(|target| *target != legacy["ownedExecutables"][0])
    );
}

#[test]
fn cargo_install_exposes_only_pointbreak_executable() {
    let install_root = tempfile::tempdir().expect("create cargo install root");
    let output = Command::new(env!("CARGO"))
        .args(["install", "--path", env!("CARGO_MANIFEST_DIR"), "--root"])
        .arg(install_root.path())
        .arg("--debug")
        .env("CARGO_TARGET_DIR", install_root.path().join("target"))
        .output()
        .expect("run cargo install");
    assert!(
        output.status.success(),
        "cargo install stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut installed = fs::read_dir(install_root.path().join("bin"))
        .expect("read installed bin directory")
        .map(|entry| entry.expect("installed bin entry").file_name())
        .collect::<Vec<_>>();
    installed.sort();

    assert_eq!(installed, [OsStr::new(POINTBREAK_EXECUTABLE_BASENAME)]);
    assert!(!install_root.path().join("bin").join("shore").exists());
    assert!(!install_root.path().join("bin").join("shore.exe").exists());
}

#[test]
fn cli_help_uses_pointbreak_and_keeps_the_flat_command_tree() {
    let temp_dir = tempfile::tempdir().expect("create temp command dir");
    let command_path = temp_dir.path().join("renamed-command.exe");
    fs::copy(env!("CARGO_BIN_EXE_pointbreak"), &command_path)
        .expect("copy pointbreak binary under an arbitrary filename");
    ensure_executable(&command_path).expect("make copied binary executable");

    let output = Command::new(command_path)
        .arg("--help")
        .output()
        .expect("run pointbreak help");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help is utf8");
    let usage_line = stdout
        .lines()
        .find(|line| line.starts_with("Usage:"))
        .expect("help contains usage line");
    assert_eq!(usage_line, "Usage: pointbreak [OPTIONS] <COMMAND>");
    assert!(!usage_line.contains("renamed-command.exe"));
    for command in [
        "assessment",
        "association",
        "capture",
        "history",
        "input-request",
        "observation",
        "revision",
        "store",
        "validation",
        "version",
    ] {
        assert!(
            stdout.contains(command),
            "help missing {command}:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("  review "),
        "help restored a review prefix"
    );
}

#[test]
fn cli_version_uses_pointbreak_and_preserves_the_version_document() {
    let command = env!("CARGO_BIN_EXE_pointbreak");
    let document = Command::new(command)
        .args(["version", "--format", "json"])
        .output()
        .expect("run pointbreak version JSON");
    assert!(document.status.success());
    let document: Value =
        serde_json::from_slice(&document.stdout).expect("version document is JSON");
    assert_eq!(document["schema"], "pointbreak.version");
    assert_eq!(document["version"], 1);
    assert_eq!(document["cliVersion"], env!("CARGO_PKG_VERSION"));
    assert_eq!(document["build"]["source"], env!("POINTBREAK_BUILD_SOURCE"));
    match env!("POINTBREAK_BUILD_SOURCE") {
        "git" => {
            let commit = document["build"]["commit"].as_str().expect("git commit");
            assert_eq!(commit.len(), 40);
            assert!(commit.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
        "package" => assert!(document["build"]["commit"].is_null()),
        source => panic!("unexpected build source {source:?}"),
    }
    let describe = document["build"]["describe"]
        .as_str()
        .expect("build describe");
    assert!(document["build"]["dirty"].is_boolean());

    let version = Command::new(command)
        .arg("--version")
        .output()
        .expect("run pointbreak --version");
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).expect("version is utf8"),
        format!("pointbreak {} ({describe})\n", env!("CARGO_PKG_VERSION"))
    );

    let text = Command::new(command)
        .args(["version", "--format", "text"])
        .output()
        .expect("run pointbreak version text");
    assert!(text.status.success());
    assert!(
        String::from_utf8(text.stdout)
            .expect("text version is utf8")
            .starts_with(&format!(
                "pointbreak {} ({describe})\n",
                env!("CARGO_PKG_VERSION")
            ))
    );
}

#[cfg(unix)]
fn ensure_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    fs::set_permissions(path, permissions)
}

#[cfg(not(unix))]
fn ensure_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}
