mod support;

use std::process::{Command, Output};

use support::git_repo::GitRepo;

const PRODUCT_SELECTORS: [&str; 16] = [
    "POINTBREAK_ACTOR_ID",
    "POINTBREAK_SIGNING",
    "POINTBREAK_SIGNING_KEY",
    "POINTBREAK_FORMAT",
    "POINTBREAK_THEME",
    "POINTBREAK_LOG",
    "POINTBREAK_BACKEND",
    "POINTBREAK_PERF",
    "SHORE_ACTOR_ID",
    "SHORE_SIGNING",
    "SHORE_SIGNING_KEY",
    "SHORE_FORMAT",
    "SHORE_THEME",
    "SHORE_LOG",
    "SHORE_BACKEND",
    "SHORE_PERF",
];

fn command(args: &[&str]) -> Command {
    let mut command = Command::new(support::pointbreak_bin());
    command.args(args);
    for selector in PRODUCT_SELECTORS {
        command.env_remove(selector);
    }
    command.env_remove("RUST_LOG");
    command.env_remove("BAT_THEME");
    command
}

fn output(command: &mut Command) -> Output {
    command.output().expect("run pointbreak")
}

fn actor_id(output: &Output) -> String {
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    document["actorId"].as_str().unwrap().to_owned()
}

#[test]
fn canonical_actor_selector_wins_and_old_name_is_ignored() {
    let repo = GitRepo::new();
    let repo_path = repo.path().to_str().unwrap();

    let canonical = output(
        command(&["identity", "whoami", "--repo", repo_path])
            .env("POINTBREAK_ACTOR_ID", "actor:agent:canonical")
            .env("SHORE_ACTOR_ID", "actor:agent:old-name"),
    );
    assert_eq!(actor_id(&canonical), "actor:agent:canonical");

    let old_only = output(
        command(&["identity", "whoami", "--repo", repo_path])
            .env("SHORE_ACTOR_ID", "actor:agent:old-name"),
    );
    assert_eq!(
        actor_id(&old_only),
        "actor:git-email:shore-tests@example.com"
    );
}

#[test]
fn canonical_signing_selectors_are_read_and_old_names_are_ignored() {
    let mode_repo = support::dump_repo();
    let mode_path = mode_repo.path().to_str().unwrap();
    let canonical_mode =
        output(command(&["capture", "--repo", mode_path]).env("POINTBREAK_SIGNING", "unexpected"));
    let mode_stderr = String::from_utf8_lossy(&canonical_mode.stderr);
    assert!(canonical_mode.status.success(), "{mode_stderr}");
    assert!(mode_stderr.contains("signing_mode_unrecognized"));

    let key_repo = support::dump_repo();
    let key_path = key_repo.path().to_str().unwrap();
    let canonical_key = output(
        command(&["capture", "--repo", key_path]).env("POINTBREAK_SIGNING_KEY", "missing-key"),
    );
    let key_stderr = String::from_utf8_lossy(&canonical_key.stderr);
    assert!(canonical_key.status.success(), "{key_stderr}");
    assert!(key_stderr.contains("signing_key_unreadable"));

    let old_repo = support::dump_repo();
    let old_path = old_repo.path().to_str().unwrap();
    let old = output(
        command(&["capture", "--repo", old_path])
            .env("SHORE_SIGNING", "unexpected")
            .env("SHORE_SIGNING_KEY", "missing-key"),
    );
    let old_stderr = String::from_utf8_lossy(&old.stderr);
    assert!(old.status.success(), "{old_stderr}");
    assert!(!old_stderr.contains("signing_mode_unrecognized"));
    assert!(!old_stderr.contains("signing_key_unreadable"));
}

#[test]
fn canonical_format_selector_is_read_and_old_name_is_ignored() {
    let canonical = output(command(&["version"]).env("POINTBREAK_FORMAT", "text"));
    assert!(canonical.status.success());
    assert!(String::from_utf8_lossy(&canonical.stdout).starts_with("pointbreak "));

    let old = output(command(&["version"]).env("SHORE_FORMAT", "text"));
    assert!(old.status.success());
    assert!(String::from_utf8_lossy(&old.stdout).starts_with('{'));
}

#[test]
fn canonical_theme_selector_is_read_and_old_name_is_ignored() {
    let repo = support::dump_repo();
    let repo_path = repo.path().to_str().unwrap();
    let capture = output(&mut command(&["capture", "--repo", repo_path]));
    assert!(capture.status.success());
    let args = ["diff", "--repo", repo_path, "--color", "always"];

    let canonical = output(
        command(&args)
            .env("COLORTERM", "truecolor")
            .env("POINTBREAK_THEME", "no-such-theme"),
    );
    assert!(!canonical.status.success());
    assert!(String::from_utf8_lossy(&canonical.stderr).contains("no-such-theme"));

    let old = output(
        command(&args)
            .env("COLORTERM", "truecolor")
            .env("SHORE_THEME", "no-such-theme"),
    );
    assert!(
        old.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&old.stderr)
    );
}

#[test]
fn canonical_log_selector_is_read_and_old_name_is_ignored() {
    let canonical = output(command(&["version"]).env("POINTBREAK_LOG", "["));
    assert!(!canonical.status.success());
    assert!(String::from_utf8_lossy(&canonical.stderr).contains("invalid log filter"));

    let old = output(command(&["version"]).env("SHORE_LOG", "["));
    assert!(old.status.success());
}

#[test]
fn canonical_backend_selector_is_read_and_old_name_is_ignored() {
    let repo = support::dump_repo();
    let repo_path = repo.path().to_str().unwrap();

    let canonical =
        output(command(&["history", "--repo", repo_path]).env("POINTBREAK_BACKEND", "memory"));
    assert!(!canonical.status.success());
    assert!(String::from_utf8_lossy(&canonical.stderr).contains("POINTBREAK_BACKEND"));

    let old = output(command(&["history", "--repo", repo_path]).env("SHORE_BACKEND", "memory"));
    assert!(
        old.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&old.stderr)
    );
}

#[test]
fn perf_selector_uses_the_canonical_name() {
    assert_eq!(pointbreak::environment::PERF, "POINTBREAK_PERF");
}
