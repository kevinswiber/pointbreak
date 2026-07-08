mod support;

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::{common_dir_store, shore};

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
    assert!(stdout.starts_with("{\"schema\":\"pointbreak.store-status\""));
    let json = parse_json(stdout.as_bytes());

    assert_eq!(json["schema"], "pointbreak.store-status");
    assert_eq!(json["version"], 1);
    // The single-store view: one store per clone, no clone/family refs.
    assert_eq!(json["mode"], "local");
    assert_eq!(json["storeRef"], "local");
    assert!(json.get("cloneRef").is_none());
    assert!(json.get("repositoryFamilyRef").is_none());
    assert!(!stdout.contains(".shore"));
    assert!(!stdout.contains("state.json"));
    assert!(!stdout.contains("artifacts/"));
}

// The "linked" store-status mode with clone/repository-family refs is retired:
// store registration was removed with the shared-store default, so every worktree
// reports the single-store view (`mode: "local"`, `storeRef: "local"`, no
// clone/family refs) — covered by `store_status_emits_local_json_without_storage_paths`,
// and the shared-store visibility itself by the shared-store-default suite. A
// linked worktree resolves the same shared store as main with no registration.
#[test]
fn linked_worktree_store_status_reports_the_shared_single_store_view() {
    let fixture = LinkedWorktreeFixture::new();

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

    assert_eq!(json["schema"], "pointbreak.store-status");
    assert_eq!(json["version"], 1);
    assert_eq!(json["mode"], "local");
    assert_eq!(json["storeRef"], "local");
    assert!(json.get("cloneRef").is_none());
    assert!(json.get("repositoryFamilyRef").is_none());
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let body_dir = tempfile::tempdir().expect("create body file directory");
    let body_file = body_dir.path().join("body.txt");
    fs::write(&body_file, "x".repeat(4097)).unwrap();
    shore([
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
    let store_dir = common_dir_store(repo.path());
    let (event_count, event_bytes) = directory_file_stats(&store_dir.join("events"));
    let (snapshot_count, snapshot_bytes) =
        directory_file_stats(&store_dir.join("artifacts/objects"));
    let (note_count, note_bytes) = directory_file_stats(&store_dir.join("artifacts/notes"));

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
    // The additive audit fields are always present; no config → zero/empty.
    assert_eq!(json["sensitivity"]["excludedPathCount"], 0);
    assert!(
        json["sensitivity"]["excludeGlobs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn store_status_reports_exclude_glob_audit_counts() {
    let repo = GitRepo::new();
    repo.write(
        "fixtures/dev.pem",
        "-----BEGIN PRIVATE KEY-----\nredacted\n",
    );
    repo.write(
        ".shore/sensitivity.json",
        r#"{"schema":"shore.sensitivity-config","version":1,"excludeGlobs":["fixtures/**"]}"#,
    );
    repo.commit_all("base");

    let output = shore(["store", "status", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    let sensitivity = &json["sensitivity"];
    // The excluded fixture no longer blocks, and the opt-out is auditable:
    // count of skipped paths plus each configured glob's match count.
    assert_eq!(sensitivity["policyOutcome"], "allow");
    assert_eq!(sensitivity["excludedPathCount"], 1);
    let globs = sensitivity["excludeGlobs"].as_array().unwrap();
    assert_eq!(globs.len(), 1);
    assert_eq!(globs[0]["glob"], "fixtures/**");
    assert_eq!(globs[0]["matched"], 1);
}

#[test]
fn text_store_digest_reports_counts_size_and_sensitivity() {
    let repo = GitRepo::new();
    repo.write("README.md", "base\n");
    repo.commit_all("base");
    repo.write("README.md", "changed\n");
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    // A large observation body spills to a note artifact, so the store holds at
    // least a snapshot and a note (the artifact count is plural).
    let body_dir = tempfile::tempdir().expect("create body file directory");
    let body_file = body_dir.path().join("body.txt");
    fs::write(&body_file, "x".repeat(4097)).unwrap();
    shore([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:digest",
        "--title",
        "large body",
        "--body-file",
        body_file.to_str().unwrap(),
    ]);

    let output = shore([
        "store",
        "status",
        "--repo",
        repo.path().to_str().unwrap(),
        "--format",
        "text",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("events"), "counts events: {stdout}");
    assert!(stdout.contains("artifacts"), "counts artifacts: {stdout}");
    assert!(stdout.contains("B"), "byte suffix present: {stdout}");
    assert!(stdout.contains("sensitivity"), "sensitivity line: {stdout}");
    assert!(
        !stdout.contains("\"schema\""),
        "text lane is not JSON: {stdout}"
    );
    assert!(stdout.lines().count() <= 6, "digest is bounded: {stdout}");
    // Privacy: the text lane must not leak store paths any more than the JSON lane.
    assert!(!stdout.contains(".shore"), "no store path: {stdout}");
    assert!(!stdout.contains("artifacts/"), "no artifact path: {stdout}");
    assert!(!stdout.contains("state.json"), "no state path: {stdout}");
}

#[test]
fn text_store_digest_summarizes_blocked_sensitivity_findings() {
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

    let output = shore([
        "store",
        "status",
        "--repo",
        repo.path().to_str().unwrap(),
        "--format",
        "text",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    // The blocking outcome and a bounded finding summary: at most three kinds are
    // named inline, and the surplus is summarized rather than listed.
    assert!(
        stdout.contains("sensitivity: block"),
        "block outcome: {stdout}"
    );
    assert!(
        stdout.contains("more"),
        "surplus findings summarized: {stdout}"
    );
    assert!(
        stdout.lines().count() <= 6,
        "digest stays bounded: {stdout}"
    );
    // Redaction holds on the text lane too — no secret material, no raw paths.
    assert!(!stdout.contains("sk-test"), "{stdout}");
    assert!(!stdout.contains("PRIVATE KEY"), "{stdout}");
    assert!(!stdout.contains(".env"), "{stdout}");
    assert!(!stdout.contains("target/generated"), "{stdout}");
    assert!(
        !stdout.contains("\"schema\""),
        "text lane is not JSON: {stdout}"
    );
}

#[test]
fn show_paths_lists_real_matched_paths_on_the_text_lane() {
    let repo = GitRepo::new();
    repo.write("keys/dev.pem", "-----BEGIN PRIVATE KEY-----\nredacted\n");
    repo.write(
        "src/token.txt",
        "let key = \"sk-test000000000000000000000000\";\n",
    );

    let output = shore([
        "store",
        "status",
        "--repo",
        repo.path().to_str().unwrap(),
        "--show-paths",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    // The digest still leads; the matched-path listing follows it.
    assert!(
        stdout.contains("sensitivity: block"),
        "digest present: {stdout}"
    );
    assert!(
        stdout.contains("matched paths"),
        "path section header: {stdout}"
    );
    // The real relative paths appear, grouped under their finding kind.
    assert!(stdout.contains("private_key"), "kind header: {stdout}");
    assert!(
        stdout.contains("keys/dev.pem"),
        "real path listed: {stdout}"
    );
    assert!(
        stdout.contains("src/token.txt"),
        "real path listed: {stdout}"
    );
    // It is the text lane, never the machine document.
    assert!(!stdout.contains("\"schema\""), "not JSON: {stdout}");
    // The redacted token never appears alongside the real paths.
    assert!(
        !stdout.contains("redacted-file:sha256:"),
        "text lane shows real paths, not tokens: {stdout}"
    );
    // No secret material is echoed, only the paths.
    assert!(!stdout.contains("sk-test"), "no secret value: {stdout}");
    assert!(!stdout.contains("PRIVATE KEY"), "no secret value: {stdout}");
}

#[test]
fn show_paths_reports_none_when_nothing_matched() {
    let repo = GitRepo::new();
    repo.write("README.md", "safe\n");
    repo.commit_all("base");

    let output = shore([
        "store",
        "status",
        "--repo",
        repo.path().to_str().unwrap(),
        "--show-paths",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("matched paths: none"),
        "empty scan states so explicitly: {stdout}"
    );
}

#[test]
fn show_paths_refuses_an_explicit_json_format() {
    let repo = GitRepo::new();
    repo.write("keys/dev.pem", "-----BEGIN PRIVATE KEY-----\nredacted\n");

    let output = shore([
        "store",
        "status",
        "--repo",
        repo.path().to_str().unwrap(),
        "--show-paths",
        "--format",
        "json",
    ]);

    assert!(
        !output.status.success(),
        "--show-paths with an explicit JSON format is refused"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--show-paths") && stderr.contains("text"),
        "the refusal explains the text-only constraint: {stderr}"
    );
    // The refusal path never leaks a real worktree path.
    assert!(
        !stderr.contains("keys/dev.pem"),
        "no path in the error: {stderr}"
    );
}

#[test]
fn show_paths_never_reaches_the_json_lane() {
    // The same sensitive worktree: the paths surface under --show-paths (text),
    // but the machine document keeps only redacted tokens — the no-path contract
    // on the emitted JSON holds regardless of the flag.
    let repo = GitRepo::new();
    repo.write("keys/dev.pem", "-----BEGIN PRIVATE KEY-----\nredacted\n");

    let repo_arg = repo.path().to_str().unwrap();
    let text = shore(["store", "status", "--repo", repo_arg, "--show-paths"]);
    let text_stdout = String::from_utf8(text.stdout).unwrap();
    assert!(
        text_stdout.contains("keys/dev.pem"),
        "text lists the path: {text_stdout}"
    );

    let json = shore(["store", "status", "--repo", repo_arg, "--format", "json"]);
    let json_stdout = String::from_utf8(json.stdout).unwrap();
    assert!(
        !json_stdout.contains("keys/dev.pem"),
        "the JSON document never carries the real path: {json_stdout}"
    );
    let value = parse_json(json_stdout.as_bytes());
    let references = &value["sensitivity"]["findings"][0]["references"];
    assert!(
        references[0].as_str().unwrap().starts_with("file:sha256:"),
        "the JSON reference stays redacted: {references}"
    );
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
