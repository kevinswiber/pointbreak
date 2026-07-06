mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore_env;

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("valid json on stdout")
}

fn captured_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

#[test]
fn store_link_emits_camelcase_json_with_family_and_clone_refs() {
    let repo = captured_repo();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();

    assert!(
        shore_env(
            ["capture", "--repo", &repo_arg],
            &[("SHORE_HOME", home_str)]
        )
        .status
        .success()
    );

    let link = shore_env(
        ["store", "link", "acme", "--repo", &repo_arg],
        &[("SHORE_HOME", home_str)],
    );
    assert!(
        link.status.success(),
        "link: {}",
        String::from_utf8_lossy(&link.stderr)
    );
    let json = parse_json(&link.stdout);
    assert_eq!(json["familyRef"], "acme");
    assert!(!json["cloneRef"].as_str().unwrap().is_empty());
    assert_eq!(json["createdFamily"], true);
}

#[test]
fn store_link_text_digest_mentions_the_family() {
    let repo = captured_repo();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();
    assert!(
        shore_env(
            ["capture", "--repo", &repo_arg],
            &[("SHORE_HOME", home_str)]
        )
        .status
        .success()
    );

    let link = shore_env(
        [
            "store", "link", "acme", "--repo", &repo_arg, "--format", "text",
        ],
        &[("SHORE_HOME", home_str)],
    );
    assert!(link.status.success());
    let stdout = String::from_utf8(link.stdout).unwrap();
    assert!(stdout.contains("acme"), "{stdout}");
    assert!(
        !stdout.contains("\"schema\""),
        "text lane is not JSON: {stdout}"
    );
}

#[test]
fn store_link_default_fold_discloses_removed_unsigned_events() {
    let repo = captured_repo();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();

    assert!(
        shore_env(
            ["capture", "--repo", &repo_arg],
            &[("SHORE_HOME", home_str)]
        )
        .status
        .success()
    );
    let status = shore_env(
        ["store", "status", "--repo", &repo_arg],
        &[("SHORE_HOME", home_str)],
    );
    let object_ref = parse_json(&status.stdout)["inventory"]["revisionObjects"][0]["objectId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        shore_env(
            [
                "store",
                "remove",
                "--snapshot",
                &object_ref,
                "--repo",
                &repo_arg
            ],
            &[("SHORE_HOME", home_str), ("SHORE_SIGNING", "off")],
        )
        .status
        .success()
    );

    let link = shore_env(
        ["store", "link", "acme", "--repo", &repo_arg],
        &[("SHORE_HOME", home_str)],
    );
    assert!(link.status.success());
    let json = parse_json(&link.stdout);
    let diagnostics = json["diagnostics"].as_array().cloned().unwrap_or_default();
    assert!(
        diagnostics.iter().any(|d| d["message"]
            .as_str()
            .unwrap_or_default()
            .contains("unsigned removal event")),
        "expected the possession-lost disclosure: {json}"
    );
}

#[test]
fn store_unlink_round_trips() {
    let repo = captured_repo();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();
    assert!(
        shore_env(
            ["capture", "--repo", &repo_arg],
            &[("SHORE_HOME", home_str)]
        )
        .status
        .success()
    );
    assert!(
        shore_env(
            ["store", "link", "acme", "--repo", &repo_arg],
            &[("SHORE_HOME", home_str)]
        )
        .status
        .success()
    );

    let unlink = shore_env(
        ["store", "unlink", "--repo", &repo_arg],
        &[("SHORE_HOME", home_str)],
    );
    assert!(
        unlink.status.success(),
        "{}",
        String::from_utf8_lossy(&unlink.stderr)
    );

    let status = shore_env(
        ["store", "status", "--repo", &repo_arg],
        &[("SHORE_HOME", home_str)],
    );
    let json = parse_json(&status.stdout);
    assert_eq!(json["mode"], "local");
    assert!(json.get("repositoryFamilyRef").is_none());
}

#[test]
fn store_link_without_a_slug_surfaces_the_workflow_suggestion_error() {
    let repo = captured_repo();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();
    assert!(
        shore_env(
            ["capture", "--repo", &repo_arg],
            &[("SHORE_HOME", home_str)]
        )
        .status
        .success()
    );

    let link = shore_env(
        ["store", "link", "--repo", &repo_arg],
        &[("SHORE_HOME", home_str)],
    );
    assert!(
        !link.status.success(),
        "an omitted slug must not silently pick one"
    );
    let stderr = String::from_utf8_lossy(&link.stderr);
    assert!(stderr.contains("slug"), "names the missing input: {stderr}");
}
