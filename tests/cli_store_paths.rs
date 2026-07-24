mod support;

use std::process::{Command, Output};

use support::git_repo::GitRepo;

fn command(repo: &GitRepo, home: &std::path::Path, format: &str) -> Command {
    let mut command = Command::new(support::pointbreak_bin());
    command
        .args(["store", "paths", "--repo"])
        .arg(repo.path())
        .args(["--format", format])
        .env_remove("SHORE_HOME")
        .env("POINTBREAK_HOME", home);
    command
}

fn output(command: &mut Command) -> Output {
    command.output().expect("run pointbreak store paths")
}

fn planted_old_layout(repo: &GitRepo) {
    std::fs::create_dir_all(repo.path().join(".shore/data/events")).unwrap();
    let common = repo.path().join(".git");
    std::fs::create_dir_all(common.join("shore/events")).unwrap();
    std::fs::write(
        common.join("shore.link.json"),
        r#"{"schema":"shore.store-link","version":1,"familyRef":"old","cloneRef":"old"}"#,
    )
    .unwrap();
}

#[test]
fn store_paths_json_reports_canonical_resolved_paths() {
    let repo = GitRepo::new();
    let repository = pointbreak::paths::RepositoryPaths::resolve(repo.path()).unwrap();
    let common = pointbreak::paths::CommonDirPaths::resolve(repo.path()).unwrap();
    let home = tempfile::tempdir().unwrap();
    planted_old_layout(&repo);

    let output = output(&mut command(&repo, home.path(), "json"));
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["schema"], "pointbreak.store-paths");
    assert_eq!(document["version"], 1);
    assert_eq!(document["tier"], "clone-local");
    assert_eq!(
        document["worktreeStore"],
        repository.worktree_store().to_str().unwrap()
    );
    assert_eq!(
        document["commonStore"],
        common.store_dir().to_str().unwrap()
    );
    assert_eq!(document["binding"], common.binding().to_str().unwrap());
    assert_eq!(document["home"], home.path().to_str().unwrap());
    assert_eq!(document["keys"], home.path().join("keys").to_str().unwrap());
    assert_eq!(document["diagnostics"], serde_json::json!([]));

    let rendered = String::from_utf8(output.stdout).unwrap();
    assert!(!rendered.contains(".shore"));
    assert!(!rendered.contains("shore.link.json"));
}

#[test]
fn store_paths_text_reports_the_same_five_paths() {
    let repo = GitRepo::new();
    let repository = pointbreak::paths::RepositoryPaths::resolve(repo.path()).unwrap();
    let common = pointbreak::paths::CommonDirPaths::resolve(repo.path()).unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = output(&mut command(&repo, home.path(), "text"));
    assert!(output.status.success());

    let text = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "tier: clone-local".to_owned(),
        format!("worktree store: {}", repository.worktree_store().display()),
        format!("common store: {}", common.store_dir().display()),
        format!("binding: {}", common.binding().display()),
        format!("home: {}", home.path().display()),
        format!("keys: {}", home.path().join("keys").display()),
    ] {
        assert!(text.contains(&expected), "missing {expected:?} in:\n{text}");
    }
}

#[test]
fn version_registry_adds_only_store_paths_to_the_frozen_document_set() {
    let output = support::pointbreak(["version"]);
    assert!(output.status.success());
    let mut actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        actual["documents"]["pointbreak.store-paths"],
        serde_json::json!(1)
    );
    actual["documents"]
        .as_object_mut()
        .unwrap()
        .remove("pointbreak.store-paths");
    actual["cliVersion"] = serde_json::json!("0.6.0");
    assert!(
        actual.as_object_mut().unwrap().remove("build").is_some(),
        "current v1 adds build without rewriting the historical v1 fixture"
    );

    let expected: serde_json::Value = serde_json::from_slice(include_bytes!(
        "fixtures/naming-cutover/protocol/version-v1.json"
    ))
    .unwrap();
    assert_eq!(actual, expected);
}
