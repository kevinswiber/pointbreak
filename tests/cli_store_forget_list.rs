mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::pointbreak_env;

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("valid json on stdout")
}

fn linked_repo(home: &str) -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    assert!(
        pointbreak_env(
            ["capture", "--repo", &repo_arg, "--allow-empty"],
            &[("POINTBREAK_HOME", home)],
        )
        .status
        .success()
    );
    assert!(
        pointbreak_env(
            ["store", "link", "acme", "--repo", &repo_arg],
            &[("POINTBREAK_HOME", home)]
        )
        .status
        .success()
    );
    repo
}

#[test]
fn store_forget_without_yes_previews_and_refuses_to_delete() {
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();
    let _repo = linked_repo(home_str);

    let forget = pointbreak_env(
        ["store", "forget", "acme"],
        &[("POINTBREAK_HOME", home_str)],
    );
    assert!(
        forget.status.success(),
        "{}",
        String::from_utf8_lossy(&forget.stderr)
    );
    let json = parse_json(&forget.stdout);
    assert_eq!(json["schema"], "pointbreak.store-forget");
    assert_eq!(json["dryRun"], true);
    assert_eq!(json["deleted"], false);
    assert!(
        home.path().join("stores/acme/family.json").is_file(),
        "the dry run deletes nothing"
    );
}

#[test]
fn store_forget_yes_on_an_orphaned_family_deletes_it() {
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();
    let repo = linked_repo(home_str);
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    assert!(
        pointbreak_env(
            ["store", "unlink", "--repo", &repo_arg],
            &[("POINTBREAK_HOME", home_str)]
        )
        .status
        .success()
    );

    let forget = pointbreak_env(
        ["store", "forget", "acme", "--yes"],
        &[("POINTBREAK_HOME", home_str)],
    );
    assert!(
        forget.status.success(),
        "{}",
        String::from_utf8_lossy(&forget.stderr)
    );
    let json = parse_json(&forget.stdout);
    assert_eq!(json["schema"], "pointbreak.store-forget");
    assert_eq!(json["deleted"], true);
    assert!(!home.path().join("stores/acme").exists());
}

#[test]
fn store_list_shows_the_linked_family_without_repo() {
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();
    let _repo = linked_repo(home_str);

    let list = pointbreak_env(["store", "list"], &[("POINTBREAK_HOME", home_str)]);
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let json = parse_json(&list.stdout);
    assert_eq!(json["schema"], "pointbreak.store-list");
    let families = json["families"].as_array().unwrap();
    assert!(families.iter().any(|entry| entry["familyRef"] == "acme"));
}

#[test]
fn store_list_with_an_empty_home_prints_an_empty_result() {
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();

    let list = pointbreak_env(["store", "list"], &[("POINTBREAK_HOME", home_str)]);
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let json = parse_json(&list.stdout);
    assert_eq!(json["schema"], "pointbreak.store-list");
    assert!(json["families"].as_array().unwrap().is_empty());
}

#[test]
fn text_store_list_digest_reports_families() {
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();
    let _repo = linked_repo(home_str);

    let out = pointbreak_env(
        ["store", "list", "--format", "text"],
        &[("POINTBREAK_HOME", home_str)],
    );
    assert!(
        out.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        !stdout.contains("\"schema\""),
        "text lane is not JSON: {stdout}"
    );
    assert!(
        stdout.contains("1 family store"),
        "count headline: {stdout}"
    );
    assert!(stdout.contains("acme"), "family named: {stdout}");
    assert!(stdout.contains("clone"), "clone count: {stdout}");
}

#[test]
fn text_store_list_digest_reports_empty_home() {
    let home = tempfile::tempdir().unwrap();
    let out = pointbreak_env(
        ["store", "list", "--format", "text"],
        &[("POINTBREAK_HOME", home.path().to_str().unwrap())],
    );
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("no family stores"), "empty line: {stdout}");
    assert!(
        !stdout.contains("\"schema\""),
        "text lane is not JSON: {stdout}"
    );
}
