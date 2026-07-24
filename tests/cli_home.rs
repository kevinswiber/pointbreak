mod support;

use std::path::PathBuf;
use std::process::{Command, Output};

fn command(args: &[&str]) -> Command {
    let mut command = Command::new(support::pointbreak_bin());
    command.args(args);
    for selector in pointbreak::environment::RUNTIME_VARIABLES {
        command.env_remove(selector);
    }
    command.env_remove("SHORE_HOME");
    command.env_remove("XDG_DATA_HOME");
    command.env_remove("HOME");
    command.env_remove("APPDATA");
    command
}

fn output(command: &mut Command) -> Output {
    command.output().expect("run pointbreak")
}

fn key_path(output: &Output) -> PathBuf {
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    PathBuf::from(document["path"].as_str().unwrap())
}

#[test]
fn explicit_pointbreak_home_owns_key_placement_and_old_name_is_ignored() {
    let canonical = tempfile::tempdir().unwrap();
    let old = tempfile::tempdir().unwrap();
    let output = output(
        command(&["key", "init", "--name", "canonical"])
            .env("POINTBREAK_HOME", canonical.path())
            .env("SHORE_HOME", old.path()),
    );

    assert!(key_path(&output).starts_with(canonical.path().join("keys")));
    assert!(!old.path().join("keys").exists());
}

#[test]
fn old_home_name_cannot_redirect_default_key_placement() {
    let old = tempfile::tempdir().unwrap();
    let xdg = tempfile::tempdir().unwrap();
    let fallback = tempfile::tempdir().unwrap();
    let output = output(
        command(&["key", "init", "--name", "default-path"])
            .env("SHORE_HOME", old.path())
            .env("XDG_DATA_HOME", xdg.path())
            .env("HOME", fallback.path()),
    );

    assert!(
        key_path(&output).starts_with(xdg.path().join("pointbreak").join("keys")),
        "key path: {}",
        key_path(&output).display()
    );
    assert!(!old.path().join("keys").exists());
}

#[test]
fn empty_and_relative_explicit_homes_fail_without_creating_directories() {
    for (name, value) in [("empty", ""), ("relative", "relative-home")] {
        let fallback = tempfile::tempdir().unwrap();
        let output = output(
            command(&["key", "init", "--name", name])
                .current_dir(fallback.path())
                .env("POINTBREAK_HOME", value)
                .env("HOME", fallback.path()),
        );
        assert!(!output.status.success(), "{name} explicit home must fail");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("POINTBREAK_HOME"),
            "stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!fallback.path().join(".pointbreak").exists());
        assert!(!fallback.path().join("relative-home").exists());
    }
}

#[test]
fn one_explicit_home_owns_family_registry_and_user_store_paths() {
    let repo = support::dump_repo();
    let repo_path = repo.path().to_str().unwrap();
    let home = tempfile::tempdir().unwrap();
    let fallback = tempfile::tempdir().unwrap();

    let capture = output(
        command(&["capture", "--repo", repo_path])
            .env("POINTBREAK_HOME", home.path())
            .env("HOME", fallback.path()),
    );
    assert!(
        capture.status.success(),
        "capture stderr:\n{}",
        String::from_utf8_lossy(&capture.stderr)
    );

    let link = output(
        command(&["store", "link", "acme", "--repo", repo_path])
            .env("POINTBREAK_HOME", home.path())
            .env("HOME", fallback.path()),
    );
    assert!(
        link.status.success(),
        "link stderr:\n{}",
        String::from_utf8_lossy(&link.stderr)
    );

    let family = home.path().join("stores/acme");
    assert!(family.join("family.json").is_file());
    assert!(family.join("registry.json").is_file());
    assert!(family.join("events").is_dir());
    assert!(!fallback.path().join(".pointbreak").exists());
}
