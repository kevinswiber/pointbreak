mod support;

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

#[test]
fn store_status_emits_local_json_without_storage_paths() {
    let repo = GitRepo::new();

    let output = shore(["store", "status", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("{\"schema\":\"shore.store-status\""));
    let json = parse_json(stdout.as_bytes());

    assert_eq!(json["schema"], "shore.store-status");
    assert_eq!(json["version"], 1);
    assert_eq!(json["mode"], "local");
    assert_eq!(json["storeRef"], "worktree-local");
    assert!(json.get("cloneRef").is_none());
    assert!(json.get("repositoryFamilyRef").is_none());
    assert!(!stdout.contains(".shore"));
    assert!(!stdout.contains("state.json"));
    assert!(!stdout.contains("artifacts/"));
}

#[test]
fn store_status_emits_linked_refs_without_storage_paths() {
    let fixture = LinkedWorktreeFixture::new();
    seed_clone_local_registration(&fixture.linked_path);

    let output = shore([
        "store",
        "status",
        "--repo",
        fixture.linked_path.to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json = parse_json(stdout.as_bytes());

    assert_eq!(json["schema"], "shore.store-status");
    assert_eq!(json["version"], 1);
    assert_eq!(json["mode"], "linked");
    assert_eq!(json["storeRef"], "store:random:test-store");
    assert_eq!(json["cloneRef"], "clone:random:test-clone");
    assert_eq!(json["repositoryFamilyRef"], "clone:random:test-clone");
    assert!(!stdout.contains(fixture.main.path().to_str().unwrap()));
    assert!(!stdout.contains(fixture.linked_path.to_str().unwrap()));
    assert!(!stdout.contains(".git"));
    assert!(!stdout.contains(".shore"));
    assert!(!stdout.contains("state.json"));
    assert!(!stdout.contains("artifacts/"));
}

#[test]
fn store_status_includes_inventory_without_artifact_paths() {
    let repo = GitRepo::new();
    repo.write("README.md", "base\n");
    repo.commit_all("base");
    repo.write("README.md", "changed\n");
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let body_dir = tempfile::tempdir().expect("create body file directory");
    let body_file = body_dir.path().join("body.txt");
    fs::write(&body_file, "x".repeat(4097)).unwrap();
    shore([
        "review",
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:inventory",
        "--title",
        "large body",
        "--body-file",
        body_file.to_str().unwrap(),
    ]);

    let output = shore(["store", "status", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json = parse_json(stdout.as_bytes());
    let inventory = &json["inventory"];
    let shore_dir = repo.path().join(".shore");
    let (event_count, event_bytes) = directory_file_stats(&shore_dir.join("events"));
    let (snapshot_count, snapshot_bytes) =
        directory_file_stats(&shore_dir.join("artifacts/snapshots"));
    let (note_count, note_bytes) = directory_file_stats(&shore_dir.join("artifacts/notes"));

    assert_eq!(inventory["eventCount"], event_count);
    assert_eq!(inventory["eventBytes"], event_bytes);
    assert_eq!(inventory["artifactCount"], snapshot_count + note_count);
    assert_eq!(inventory["artifactBytes"], snapshot_bytes + note_bytes);
    assert_eq!(
        inventory["totalBytes"],
        event_bytes + snapshot_bytes + note_bytes
    );
    assert!(inventory["largestArtifacts"].as_array().unwrap().len() >= 2);
    assert_eq!(inventory["untrackedBytes"], 0);
    assert!(!stdout.contains(".shore"));
    assert!(!stdout.contains("artifacts/"));
    assert!(!stdout.contains("state.json"));
}

#[test]
fn store_status_includes_redacted_sensitivity_findings() {
    let repo = GitRepo::new();
    repo.write(
        "src/token.txt",
        "let key = \"sk-test000000000000000000000000\";\n",
    );
    repo.write("keys/dev.pem", "-----BEGIN PRIVATE KEY-----\nredacted\n");
    repo.write(".env", "DATABASE_URL=postgres://user:pass@example/db\n");
    repo.write(
        "config/value.txt",
        "token = hQ7x9Zp4Lm2N8vR5sT1aBcD3eFgH6jK0\n",
    );
    repo.write("target/generated/cache.bin", "x".repeat(1024 * 1024 + 1));

    let output = shore(["store", "status", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json = parse_json(stdout.as_bytes());
    let sensitivity = &json["sensitivity"];
    let findings = sensitivity["findings"].as_array().unwrap();

    assert_eq!(sensitivity["policyOutcome"], "block");
    assert!(
        findings
            .iter()
            .any(|finding| finding["kind"] == "known_token")
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding["kind"] == "private_key")
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding["kind"] == "sensitive_filename")
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding["kind"] == "high_entropy")
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding["kind"] == "generated_path")
    );
    assert!(findings.iter().all(|finding| {
        finding["references"]
            .as_array()
            .unwrap()
            .iter()
            .all(|reference| reference.as_str().unwrap().starts_with("file:sha256:"))
    }));
    assert!(!stdout.contains("sk-test"));
    assert!(!stdout.contains("PRIVATE KEY"));
    assert!(!stdout.contains(".env"));
    assert!(!stdout.contains("target/generated"));
}

fn seed_clone_local_registration(worktree: &Path) {
    let common_dir = git_stdout(
        worktree,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let git_dir = git_stdout(worktree, ["rev-parse", "--absolute-git-dir"]);
    let object_format = git_stdout(worktree, ["rev-parse", "--show-object-format"]);
    let shared_store = PathBuf::from(common_dir.trim()).join("shoreline");
    fs::create_dir_all(&shared_store).unwrap();
    fs::write(
        shared_store.join("manifest.json"),
        format!(
            "{{\"schema\":\"shore.store-manifest\",\"version\":1,\"storeId\":\"store:random:test-store\",\"cloneId\":\"clone:random:test-clone\",\"repositoryFamilyId\":\"clone:random:test-clone\",\"git\":{{\"commonDir\":{},\"gitDir\":{},\"worktreeRoot\":{},\"objectFormat\":{}}}}}",
            serde_json::to_string(common_dir.trim()).unwrap(),
            serde_json::to_string(git_dir.trim()).unwrap(),
            serde_json::to_string(worktree.to_str().unwrap()).unwrap(),
            serde_json::to_string(object_format.trim()).unwrap(),
        ),
    )
    .unwrap();

    fs::create_dir_all(worktree.join(".shore")).unwrap();
    fs::write(
        worktree.join(".shore/store-registration.json"),
        "{\"schema\":\"shore.store-registration\",\"version\":1,\"mode\":\"cloneLocal\",\"storeRef\":\"store:random:test-store\",\"cloneRef\":\"clone:random:test-clone\",\"repositoryFamilyRef\":\"clone:random:test-clone\"}",
    )
    .unwrap();
}

struct LinkedWorktreeFixture {
    main: GitRepo,
    _linked_parent: tempfile::TempDir,
    linked_path: PathBuf,
}

impl LinkedWorktreeFixture {
    fn new() -> Self {
        let main = GitRepo::new();
        main.write("README.md", "base\n");
        main.commit_all("base");

        let linked_parent = tempfile::tempdir().expect("create linked worktree parent");
        let linked_path = linked_parent.path().join("linked");
        run_git_os(
            main.path(),
            [
                OsString::from("worktree"),
                OsString::from("add"),
                OsString::from("-b"),
                OsString::from("linked"),
                linked_path.as_os_str().to_owned(),
            ],
        );

        Self {
            main,
            _linked_parent: linked_parent,
            linked_path,
        }
    }
}

fn git_stdout<I, S>(cwd: &Path, args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = run_git(cwd, args);
    String::from_utf8(output.stdout).unwrap()
}

fn run_git<I, S>(cwd: &Path, args: I) -> std::process::Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect::<Vec<_>>();
    let output = Command::new("git")
        .args(&args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("run git {:?} in {}: {error}", args, cwd.display()));
    assert!(
        output.status.success(),
        "git {:?} failed in {}\nstdout:\n{}\nstderr:\n{}",
        args,
        cwd.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn run_git_os<I>(cwd: &Path, args: I)
where
    I: IntoIterator<Item = OsString>,
{
    run_git(cwd, args);
}

fn directory_file_stats(dir: &Path) -> (usize, u64) {
    let mut count = 0;
    let mut bytes = 0;
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            count += 1;
            bytes += fs::metadata(path).unwrap().len();
        }
    }
    (count, bytes)
}

fn parse_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout is json")
}
