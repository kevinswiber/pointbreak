use std::path::Path;
use std::process::Command;
use std::{fs, io};

#[test]
fn package_and_library_identity_are_pointbreak() {
    assert_eq!(env!("CARGO_PKG_NAME"), "pointbreak");

    let _ = std::any::type_name::<pointbreak::model::DiffSnapshot>();
}

#[test]
fn installed_command_remains_shore() {
    let temp_dir = tempfile::tempdir().expect("create temp command dir");
    let command_path = temp_dir.path().join("shore.exe");
    fs::copy(env!("CARGO_BIN_EXE_shore"), &command_path).expect("copy shore binary as shore.exe");
    ensure_executable(&command_path).expect("make copied binary executable");

    let output = Command::new(command_path)
        .arg("--help")
        .output()
        .expect("run shore help");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help is utf8");
    let usage_line = stdout
        .lines()
        .find(|line| line.starts_with("Usage:"))
        .expect("help contains usage line");
    assert_eq!(usage_line, "Usage: shore [OPTIONS] <COMMAND>");
    assert!(!usage_line.contains("shore.exe"));
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
