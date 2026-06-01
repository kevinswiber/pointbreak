mod support;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

#[test]
fn store_link_imports_local_facts_into_clone_local_store_for_other_worktrees() {
    let fixture = CloneWorktreeFixture::new();
    fs::write(fixture.seed.join("README.md"), "changed in seed\n").unwrap();
    let capture = shore([
        "review",
        "capture",
        "--repo",
        fixture.seed.to_str().unwrap(),
    ]);
    assert!(
        capture.status.success(),
        "capture stderr:\n{}",
        String::from_utf8_lossy(&capture.stderr)
    );

    let link = shore(["store", "link", "--repo", fixture.seed.to_str().unwrap()]);
    assert!(
        link.status.success(),
        "link stderr:\n{}",
        String::from_utf8_lossy(&link.stderr)
    );
    let link_stdout = String::from_utf8(link.stdout).unwrap();
    let link_json = parse_json(link_stdout.as_bytes());
    assert_eq!(link_json["schema"], "shore.store-link");
    assert_eq!(link_json["version"], 1);
    assert_eq!(link_json["mode"], "linked");
    assert_eq!(link_json["eventsCreated"], 1);
    assert_eq!(link_json["artifactsCreated"], 1);
    assert_eq!(link_json["sensitivity"]["policyOutcome"], "allow");
    assert!(!link_stdout.contains(fixture.main.path().to_str().unwrap()));
    assert!(!link_stdout.contains(fixture.seed.to_str().unwrap()));
    assert!(!link_stdout.contains(".shore"));
    assert!(!link_stdout.contains(".git"));

    run_git_os(
        fixture.main.path(),
        [
            OsString::from("worktree"),
            OsString::from("remove"),
            OsString::from("--force"),
            fixture.seed.as_os_str().to_owned(),
        ],
    );
    let reader = fixture.add_worktree("reader");

    let reader_link = shore(["store", "link", "--repo", reader.to_str().unwrap()]);
    assert!(
        reader_link.status.success(),
        "reader link stderr:\n{}",
        String::from_utf8_lossy(&reader_link.stderr)
    );
    let status = shore(["store", "status", "--repo", reader.to_str().unwrap()]);
    assert!(
        status.status.success(),
        "status stderr:\n{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status_stdout = String::from_utf8(status.stdout).unwrap();
    let status_json = parse_json(status_stdout.as_bytes());

    assert_eq!(status_json["mode"], "linked");
    assert!(
        status_json["storeRef"]
            .as_str()
            .unwrap()
            .starts_with("store:random:")
    );
    assert!(
        status_json["cloneRef"]
            .as_str()
            .unwrap()
            .starts_with("clone:random:")
    );
    assert_eq!(status_json["inventory"]["eventCount"], 1);
    assert_eq!(status_json["inventory"]["artifactCount"], 1);
    assert!(!reader.join(".shore/events").exists());
    assert!(!status_stdout.contains(reader.to_str().unwrap()));
    assert!(!status_stdout.contains(".git"));
    assert!(!status_stdout.contains(".shore"));
}

#[test]
fn store_link_reports_blocked_sensitivity_findings_and_continues_end_to_end() {
    let fixture = CloneWorktreeFixture::new();
    let secret = "sk-test-0123456789abcdef0123456789";
    fs::create_dir_all(fixture.seed.join("src")).unwrap();
    fs::write(
        fixture.seed.join("src/token.txt"),
        format!("api token = {secret}\n"),
    )
    .unwrap();

    let link = shore(["store", "link", "--repo", fixture.seed.to_str().unwrap()]);

    assert!(
        link.status.success(),
        "link stderr:\n{}",
        String::from_utf8_lossy(&link.stderr)
    );
    let stdout = String::from_utf8(link.stdout).unwrap();
    let json = parse_json(stdout.as_bytes());
    assert_eq!(json["mode"], "linked");
    assert_eq!(json["sensitivity"]["policyOutcome"], "block");
    assert!(
        json["sensitivity"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["kind"] == "known_token")
    );
    assert!(!stdout.contains(secret));
    assert!(!stdout.contains(fixture.seed.to_str().unwrap()));
    assert!(!stdout.contains("src/token.txt"));
}

struct CloneWorktreeFixture {
    main: GitRepo,
    _worktree_parent: tempfile::TempDir,
    seed: PathBuf,
}

impl CloneWorktreeFixture {
    fn new() -> Self {
        let main = GitRepo::new();
        main.write("README.md", "base\n");
        main.commit_all("base");

        let worktree_parent = tempfile::tempdir().expect("create worktree parent");
        let seed = worktree_parent.path().join("seed");
        add_worktree(main.path(), &seed, "seed");

        Self {
            main,
            _worktree_parent: worktree_parent,
            seed,
        }
    }

    fn add_worktree(&self, branch: &str) -> PathBuf {
        let path = self._worktree_parent.path().join(branch);
        add_worktree(self.main.path(), &path, branch);
        path
    }
}

fn add_worktree(repo: &Path, path: &Path, branch: &str) {
    run_git_os(
        repo,
        [
            OsString::from("worktree"),
            OsString::from("add"),
            OsString::from("-b"),
            OsString::from(branch),
            path.as_os_str().to_owned(),
        ],
    );
}

fn run_git_os<I>(cwd: &Path, args: I)
where
    I: IntoIterator<Item = OsString>,
{
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("run git in {}: {error}", cwd.display()));
    assert!(
        output.status.success(),
        "git failed in {}\nstdout:\n{}\nstderr:\n{}",
        cwd.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout is json")
}
