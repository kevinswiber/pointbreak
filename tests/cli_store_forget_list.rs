mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore_env;

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("valid json on stdout")
}

fn linked_repo(home: &str) -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    assert!(
        shore_env(
            ["capture", "--repo", &repo_arg, "--allow-empty"],
            &[("SHORE_HOME", home)],
        )
        .status
        .success()
    );
    assert!(
        shore_env(
            ["store", "link", "acme", "--repo", &repo_arg],
            &[("SHORE_HOME", home)]
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

    let forget = shore_env(["store", "forget", "acme"], &[("SHORE_HOME", home_str)]);
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
        shore_env(
            ["store", "unlink", "--repo", &repo_arg],
            &[("SHORE_HOME", home_str)]
        )
        .status
        .success()
    );

    let forget = shore_env(
        ["store", "forget", "acme", "--yes"],
        &[("SHORE_HOME", home_str)],
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

    let list = shore_env(["store", "list"], &[("SHORE_HOME", home_str)]);
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

    let list = shore_env(["store", "list"], &[("SHORE_HOME", home_str)]);
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let json = parse_json(&list.stdout);
    assert_eq!(json["schema"], "pointbreak.store-list");
    assert!(json["families"].as_array().unwrap().is_empty());
}
