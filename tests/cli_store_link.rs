mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::pointbreak_env;

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
        pointbreak_env(
            ["capture", "--repo", &repo_arg],
            &[("POINTBREAK_HOME", home_str)]
        )
        .status
        .success()
    );

    let link = pointbreak_env(
        ["store", "link", "acme", "--repo", &repo_arg],
        &[("POINTBREAK_HOME", home_str)],
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
        pointbreak_env(
            ["capture", "--repo", &repo_arg],
            &[("POINTBREAK_HOME", home_str)]
        )
        .status
        .success()
    );

    let link = pointbreak_env(
        [
            "store", "link", "acme", "--repo", &repo_arg, "--format", "text",
        ],
        &[("POINTBREAK_HOME", home_str)],
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
        pointbreak_env(
            ["capture", "--repo", &repo_arg],
            &[("POINTBREAK_HOME", home_str)]
        )
        .status
        .success()
    );
    let status = pointbreak_env(
        ["store", "status", "--repo", &repo_arg],
        &[("POINTBREAK_HOME", home_str)],
    );
    let object_ref = parse_json(&status.stdout)["inventory"]["revisionObjects"][0]["objectId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        pointbreak_env(
            [
                "store",
                "remove",
                "--snapshot",
                &object_ref,
                "--repo",
                &repo_arg
            ],
            &[("POINTBREAK_HOME", home_str), ("POINTBREAK_SIGNING", "off")],
        )
        .status
        .success()
    );

    let link = pointbreak_env(
        ["store", "link", "acme", "--repo", &repo_arg],
        &[("POINTBREAK_HOME", home_str)],
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
fn store_link_dry_run_previews_without_writing() {
    let repo = captured_repo();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();
    assert!(
        pointbreak_env(
            ["capture", "--repo", &repo_arg],
            &[("POINTBREAK_HOME", home_str)]
        )
        .status
        .success()
    );

    let dry = pointbreak_env(
        ["store", "link", "acme", "--dry-run", "--repo", &repo_arg],
        &[("POINTBREAK_HOME", home_str)],
    );
    assert!(
        dry.status.success(),
        "dry-run exits 0 on a clean path: {}",
        String::from_utf8_lossy(&dry.stderr)
    );
    let json = parse_json(&dry.stdout);
    assert_eq!(json["schema"], "pointbreak.store-link-preview");
    assert_eq!(json["familyRef"], "acme");
    assert_eq!(json["wouldCreateFamily"], true);
    assert_eq!(json["exportFidelity"], "full");
    assert!(json["foldedEventsToCreate"].as_u64().unwrap() >= 1);
    // No binding was flipped, and a subsequent status still reports the local store.
    assert!(!repo.path().join(".pointbreak/store.local.json").exists());
    let status = pointbreak_env(
        ["store", "status", "--repo", &repo_arg],
        &[("POINTBREAK_HOME", home_str)],
    );
    assert_eq!(parse_json(&status.stdout)["mode"], "local");
}

#[test]
fn store_link_dry_run_blocks_on_ephemeral_without_override() {
    let repo = captured_repo();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();
    assert!(
        pointbreak_env(
            ["capture", "--repo", &repo_arg],
            &[("POINTBREAK_HOME", home_str)]
        )
        .status
        .success()
    );
    assert!(
        pointbreak_env(
            ["store", "mode", "ephemeral", "--repo", &repo_arg],
            &[("POINTBREAK_HOME", home_str)]
        )
        .status
        .success()
    );

    let dry = pointbreak_env(
        ["store", "link", "acme", "--dry-run", "--repo", &repo_arg],
        &[("POINTBREAK_HOME", home_str)],
    );
    assert!(
        !dry.status.success(),
        "an ephemeral worktree blocks the dry-run"
    );
    let stderr = String::from_utf8_lossy(&dry.stderr);
    assert!(stderr.contains("ephemeral"), "names the gate: {stderr}");
    assert!(!repo.path().join(".pointbreak/store.local.json").exists());
}

#[test]
fn store_unlink_round_trips() {
    let repo = captured_repo();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();
    assert!(
        pointbreak_env(
            ["capture", "--repo", &repo_arg],
            &[("POINTBREAK_HOME", home_str)]
        )
        .status
        .success()
    );
    assert!(
        pointbreak_env(
            ["store", "link", "acme", "--repo", &repo_arg],
            &[("POINTBREAK_HOME", home_str)]
        )
        .status
        .success()
    );

    let unlink = pointbreak_env(
        ["store", "unlink", "--repo", &repo_arg],
        &[("POINTBREAK_HOME", home_str)],
    );
    assert!(
        unlink.status.success(),
        "{}",
        String::from_utf8_lossy(&unlink.stderr)
    );

    let status = pointbreak_env(
        ["store", "status", "--repo", &repo_arg],
        &[("POINTBREAK_HOME", home_str)],
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
        pointbreak_env(
            ["capture", "--repo", &repo_arg],
            &[("POINTBREAK_HOME", home_str)]
        )
        .status
        .success()
    );

    let link = pointbreak_env(
        ["store", "link", "--repo", &repo_arg],
        &[("POINTBREAK_HOME", home_str)],
    );
    assert!(
        !link.status.success(),
        "an omitted slug must not silently pick one"
    );
    let stderr = String::from_utf8_lossy(&link.stderr);
    assert!(stderr.contains("slug"), "names the missing input: {stderr}");
}

#[test]
fn text_store_unlink_digest_reports_detachment() {
    let repo = captured_repo();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();
    let env = [("POINTBREAK_HOME", home_str)];
    assert!(
        pointbreak_env(["capture", "--repo", &repo_arg], &env)
            .status
            .success()
    );
    assert!(
        pointbreak_env(["store", "link", "acme", "--repo", &repo_arg], &env)
            .status
            .success()
    );

    let out = pointbreak_env(
        ["store", "unlink", "--repo", &repo_arg, "--format", "text"],
        &env,
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
    assert!(stdout.contains("unlinked"), "detachment verb: {stdout}");
    assert!(stdout.contains("acme"), "previous family named: {stdout}");
}

#[test]
fn text_store_unlink_digest_reports_not_linked() {
    let repo = captured_repo();
    let home = tempfile::tempdir().unwrap();
    let out = pointbreak_env(
        [
            "store",
            "unlink",
            "--repo",
            repo.path().to_str().unwrap(),
            "--format",
            "text",
        ],
        &[("POINTBREAK_HOME", home.path().to_str().unwrap())],
    );
    assert!(
        out.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("not linked"), "no-op case: {stdout}");
    assert!(
        !stdout.contains("\"schema\""),
        "text lane is not JSON: {stdout}"
    );
}

#[test]
fn text_store_link_dry_run_digest_previews() {
    let repo = captured_repo();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();
    let env = [("POINTBREAK_HOME", home_str)];
    assert!(
        pointbreak_env(["capture", "--repo", &repo_arg], &env)
            .status
            .success()
    );

    let dry = pointbreak_env(
        [
            "store",
            "link",
            "acme",
            "--dry-run",
            "--repo",
            &repo_arg,
            "--format",
            "text",
        ],
        &env,
    );
    assert!(
        dry.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    let stdout = String::from_utf8(dry.stdout).unwrap();
    assert!(
        !stdout.contains("\"schema\""),
        "text lane is not JSON: {stdout}"
    );
    assert!(stdout.contains("would link"), "preview verb: {stdout}");
    assert!(stdout.contains("acme"), "family named: {stdout}");
    assert!(stdout.contains("would fold"), "fold preview: {stdout}");
}
