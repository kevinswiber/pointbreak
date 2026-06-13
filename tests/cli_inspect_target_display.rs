//! Contract/regression tests for the path-private `targetDisplay` the inspector
//! derives at read time from already-captured fields.
//!
//! The derivation lives in the binary crate (`src/cli/inspect/api.rs`), so it is
//! not reachable from an integration test by a direct call. These tests instead
//! exercise the genuine production JSON end to end: they spawn the real
//! `shore inspect --port 0` server (which prints its bound URL and supports an
//! ephemeral port) and issue raw HTTP/1.1 GETs against `/api/units` and
//! `/api/unit`. That locks the additive on-the-wire contract — a derived
//! worktree/head label spliced in without disturbing any existing field.

mod support;

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

/// Test A: a worktree on a symbolic branch derives `label = <basename>` and a
/// short head OID, while every prior field stays intact and no branch is claimed
/// as capture-time provenance.
#[test]
fn api_units_derives_label_for_symbolic_branch_worktree() {
    let fixture = WorktreeCapture::on_branch("wt-foo", "feature/foo");
    let inspector = Inspector::spawn(&fixture.worktree);

    let units = inspector.get_json("/api/units");
    let entry = &units["entries"][0];

    assert_eq!(entry["targetDisplay"]["label"], "wt-foo");
    assert_eq!(entry["targetDisplay"]["kind"], "working_tree");
    assert_eq!(entry["targetDisplay"]["pathPrivate"], true);

    let base_oid = entry["base"]["commitOid"].as_str().unwrap();
    assert_eq!(
        entry["targetDisplay"]["head"]["commitOidShort"],
        base_oid[..7]
    );

    // Additive: the verbatim endpoints and identity fields are all still present.
    assert!(
        entry["target"]["worktreeRoot"]
            .as_str()
            .unwrap()
            .ends_with("wt-foo")
    );
    assert_eq!(entry["target"]["kind"], "git_working_tree");
    assert!(entry["base"]["treeOid"].is_string());
    assert!(entry["source"].is_object());
    assert!(
        entry["snapshotArtifactContentHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );

    // No branch is claimed as capture-time provenance.
    assert!(entry["targetDisplay"]["head"]["liveBranch"].is_null());
    assert!(entry["targetDisplay"].get("branch").is_none());
}

/// Test A (continued): the same derived block also appears on the single-unit
/// `/api/unit` document for a locally-readable unit, alongside the verbatim
/// target. Linked drill-in is covered separately by
/// `linked_inspector_drill_in_survives_deleted_source_worktree`.
#[test]
fn api_unit_splices_target_display_for_locally_readable_unit() {
    let fixture = WorktreeCapture::on_branch("wt-bar", "feature/bar");
    let inspector = Inspector::spawn(&fixture.worktree);

    let unit = inspector.get_json(&format!(
        "/api/unit?id={}",
        urlencode(&fixture.review_unit_id)
    ));
    let review_unit = &unit["reviewUnit"];

    assert_eq!(review_unit["targetDisplay"]["label"], "wt-bar");
    assert!(review_unit["targetDisplay"]["head"]["commitOidShort"].is_string());
    // The raw target endpoint is untouched by the splice.
    assert!(
        review_unit["target"]["worktreeRoot"]
            .as_str()
            .unwrap()
            .ends_with("wt-bar")
    );
    assert_eq!(review_unit["target"]["kind"], "git_working_tree");
}

/// A commit-range capture (`--base`) has a `git_commit` target, so the inspector
/// must label it with the short target OID — never the `"working tree"` floor —
/// and the wire block must stay path-private.
#[test]
fn inspector_units_render_commit_target_display_for_range_capture() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("change");

    let output = shore([
        "review",
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--base",
        "HEAD~1",
    ]);
    assert!(
        output.status.success(),
        "capture stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let inspector = Inspector::spawn(repo.path());
    let units = inspector.get_json("/api/units");
    let entry = &units["entries"][0];

    assert_eq!(entry["targetDisplay"]["kind"], "git_commit");
    let target_oid = entry["target"]["commitOid"].as_str().unwrap();
    let base_oid = entry["base"]["commitOid"].as_str().unwrap();
    assert_eq!(entry["targetDisplay"]["label"], target_oid[..7]);
    assert_eq!(
        entry["targetDisplay"]["head"]["commitOidShort"],
        base_oid[..7]
    );
    assert_eq!(entry["targetDisplay"]["pathPrivate"], true);
    assert_ne!(entry["targetDisplay"]["label"], "working tree");
    assert!(
        !units.to_string().contains("worktreeRoot"),
        "range capture unit list must not expose a worktree path"
    );
}

/// Test B: a detached-HEAD capture still derives `label = <basename>` and a short
/// head OID, with no branch claimed.
#[test]
fn api_units_derives_label_for_detached_head_capture() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.git(["checkout", "--detach"]);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    capture(repo.path());

    let inspector = Inspector::spawn(repo.path());
    let units = inspector.get_json("/api/units");
    let entry = &units["entries"][0];

    let worktree_root = entry["target"]["worktreeRoot"].as_str().unwrap();
    let expected_label = Path::new(worktree_root)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap();
    assert_eq!(entry["targetDisplay"]["label"], expected_label);

    let base_oid = entry["base"]["commitOid"].as_str().unwrap();
    assert_eq!(
        entry["targetDisplay"]["head"]["commitOidShort"],
        base_oid[..7]
    );
    assert!(entry["targetDisplay"]["head"]["liveBranch"].is_null());
}

/// Deleted-worktree fallback: after the captured worktree is force-removed, the
/// label still derives from the captured `worktreeRoot` basename when read from
/// a linked reader — proving derivation reads the captured field and never
/// probes the filesystem.
#[test]
fn api_units_label_survives_deleted_worktree() {
    let main = GitRepo::new();
    main.write("README.md", "base\n");
    main.commit_all("base");

    let parent = tempfile::tempdir().expect("worktree parent");
    let gone = parent.path().join("gone");
    add_worktree(main.path(), &gone, "gone");
    std::fs::write(gone.join("README.md"), "changed in gone\n").unwrap();
    capture(&gone);
    link_store(&gone);

    let reader = parent.path().join("reader");
    add_worktree(main.path(), &reader, "reader");
    link_store(&reader);

    // Force-remove the captured worktree's working directory.
    run_git(
        main.path(),
        [
            OsString::from("worktree"),
            OsString::from("remove"),
            OsString::from("--force"),
            gone.as_os_str().to_owned(),
        ],
    );
    assert!(!gone.exists());

    let inspector = Inspector::spawn(&reader);
    let units = inspector.get_json("/api/units");

    assert_eq!(units["reviewUnitCount"], 1);
    let entry = &units["entries"][0];
    assert_eq!(entry["targetDisplay"]["label"], "gone");
    let base_oid = entry["base"]["commitOid"].as_str().unwrap();
    assert_eq!(
        entry["targetDisplay"]["head"]["commitOidShort"],
        base_oid[..7]
    );
}

#[test]
fn api_lineages_lists_and_shows_review_unit_lineage() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    let lineage_id = "review-unit-lineage:random:inspect";

    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let first = capture_lineage_round(repo.path(), lineage_id, None);

    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let second = capture_lineage_round(repo.path(), lineage_id, Some(&first));

    let inspector = Inspector::spawn(repo.path());

    let lineages = inspector.get_json("/api/lineages");
    assert_eq!(lineages["schema"], "shore.inspect-lineages");
    assert_eq!(lineages["lineageCount"], 1);
    assert_eq!(lineages["entries"].as_array().unwrap().len(), 1);
    assert_eq!(lineages["diagnostics"].as_array().unwrap().len(), 0);

    let entry = &lineages["entries"][0];
    assert_eq!(entry["lineageId"], lineage_id);
    assert_eq!(entry["headReviewUnitId"], second);
    assert_eq!(entry["roundCount"], 2);
    assert_eq!(entry["diagnostics"].as_array().unwrap().len(), 0);
    assert_eq!(entry["rounds"].as_array().unwrap().len(), 2);
    assert_eq!(entry["rounds"][0]["reviewUnitId"], first);
    assert_eq!(entry["rounds"][0]["roundIndex"], 0);
    assert_eq!(entry["rounds"][0]["isHead"], false);
    assert_eq!(entry["rounds"][1]["reviewUnitId"], second);
    assert_eq!(entry["rounds"][1]["predecessorReviewUnitId"], first);
    assert_eq!(entry["rounds"][1]["roundIndex"], 1);
    assert_eq!(entry["rounds"][1]["isHead"], true);

    let lineages_json = lineages.to_string();
    assert!(
        !lineages_json.contains(&repo.path().to_string_lossy().to_string()),
        "lineage list JSON must not expose raw repository paths"
    );

    let lineage = inspector.get_json(&format!("/api/lineage?id={}", urlencode(lineage_id)));
    assert_eq!(lineage["schema"], "shore.review-lineage");
    assert_eq!(lineage["lineageId"], lineage_id);
    assert_eq!(lineage["headReviewUnitId"], second);
    assert_eq!(lineage["rounds"].as_array().unwrap().len(), 2);
    assert_eq!(lineage["rounds"][0]["reviewUnitId"], first);
    assert_eq!(lineage["rounds"][1]["reviewUnitId"], second);
    assert_eq!(lineage["rounds"][1]["predecessorReviewUnitId"], first);
    assert_eq!(lineage["diagnostics"].as_array().unwrap().len(), 0);
}

/// The issue-140 user story over a real socket: a linked reader (whose source
/// worktree has been deleted) can drill from the unit list into the unit
/// composite, snapshot diff, history, freshness, and lineages — all served
/// from the linked clone-local store, never the deleted worktree.
#[test]
fn linked_inspector_drill_in_survives_deleted_source_worktree() {
    let main = GitRepo::new();
    main.write("README.md", "base\n");
    main.commit_all("base");

    let parent = tempfile::tempdir().expect("worktree parent");
    let gone = parent.path().join("gone");
    add_worktree(main.path(), &gone, "gone");
    std::fs::write(gone.join("README.md"), "changed in gone\n").unwrap();
    let lineage_id = "review-unit-lineage:random:linked-http";
    let capture = capture_json_with_lineage(&gone, lineage_id);
    let unit_id = capture["reviewUnit"]["id"].as_str().unwrap().to_owned();
    let snapshot_id = capture["reviewUnit"]["snapshotId"]
        .as_str()
        .unwrap()
        .to_owned();
    record_review_facts(&gone);
    link_store(&gone);

    let reader = parent.path().join("reader");
    add_worktree(main.path(), &reader, "reader");
    link_store(&reader);

    run_git(
        main.path(),
        [
            OsString::from("worktree"),
            OsString::from("remove"),
            OsString::from("--force"),
            gone.as_os_str().to_owned(),
        ],
    );
    assert!(!gone.exists());

    let inspector = Inspector::spawn(&reader);

    let units = inspector.get_json("/api/units");
    assert_eq!(units["reviewUnitCount"], 1);
    assert_eq!(units["entries"][0]["reviewUnitId"], unit_id.as_str());
    assert_eq!(units["entries"][0]["targetDisplay"]["label"], "gone");

    let unit = inspector.get_json(&format!("/api/unit?id={}", urlencode(&unit_id)));
    assert_eq!(unit["reviewUnit"]["id"], unit_id.as_str());
    assert_eq!(unit["summary"]["observationCount"], 1);
    assert_eq!(unit["summary"]["inputRequestCount"], 1);
    assert_eq!(unit["summary"]["assessmentCount"], 1);
    assert_eq!(unit["summary"]["validationCheckCount"], 1);
    assert!(unit["currentAssessment"]["status"].is_string());

    let snapshot = inspector.get_json(&format!("/api/snapshot?id={}", urlencode(&snapshot_id)));
    assert!(snapshot["target"].get("worktreeRoot").is_none());
    assert_eq!(snapshot["worktreeRootRedacted"], true);
    assert_eq!(snapshot["contentHashScope"], "stored-artifact");
    assert_eq!(snapshot["targetDisplay"]["label"], "gone");

    let history = inspector.get_json("/api/history");
    assert!(history["eventCount"].as_u64().unwrap() > 0);
    assert_eq!(history["eventCount"], units["eventCount"]);

    let freshness = inspector.get_json("/api/freshness");
    assert_eq!(freshness["eventSetHash"], history["eventSetHash"]);

    let lineages = inspector.get_json("/api/lineages");
    assert_eq!(lineages["lineageCount"], 1);
    assert_eq!(lineages["entries"][0]["lineageId"], lineage_id);
    assert_eq!(lineages["entries"][0]["roundCount"], 1);
}

#[test]
fn linked_inspector_unit_error_message_stays_path_free_for_unknown_unit() {
    let main = GitRepo::new();
    main.write("README.md", "base\n");
    main.commit_all("base");

    let parent = tempfile::tempdir().expect("worktree parent");
    let seed = parent.path().join("seed");
    add_worktree(main.path(), &seed, "seed");
    std::fs::write(seed.join("README.md"), "changed in seed\n").unwrap();
    capture(&seed);
    link_store(&seed);
    let reader = parent.path().join("reader");
    add_worktree(main.path(), &reader, "reader");
    link_store(&reader);

    let inspector = Inspector::spawn(&reader);
    let (status, body) = inspector.get_error("/api/unit?id=review-unit%3Asha256%3Amissing");

    assert!(!status.contains("200"), "status: {status}");
    assert_eq!(
        body["error"],
        "review unit not found or unreadable: review-unit:sha256:missing"
    );
    assert!(!body["error"].as_str().unwrap().contains('/'));
}

// --- fixtures and HTTP harness ------------------------------------------------

/// A repository plus a worktree on a fresh branch with one captured ReviewUnit.
struct WorktreeCapture {
    _main: GitRepo,
    _parent: tempfile::TempDir,
    worktree: PathBuf,
    review_unit_id: String,
}

impl WorktreeCapture {
    fn on_branch(dir_name: &str, branch: &str) -> Self {
        let main = GitRepo::new();
        main.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        main.commit_all("base");

        let parent = tempfile::tempdir().expect("worktree parent");
        let worktree = parent.path().join(dir_name);
        add_worktree(main.path(), &worktree, branch);
        std::fs::write(worktree.join("src/lib.rs"), "pub fn value() -> u32 { 2 }\n").unwrap();

        let review_unit_id = capture(&worktree);

        Self {
            _main: main,
            _parent: parent,
            worktree,
            review_unit_id,
        }
    }
}

/// A spawned `shore inspect` server bound to an ephemeral port, killed on drop.
struct Inspector {
    child: Child,
    addr: String,
    stderr: Arc<Mutex<String>>,
    _stdout_drain: thread::JoinHandle<()>,
}

impl Inspector {
    fn spawn(repo: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_shore"))
            .args([
                "inspect",
                "--repo",
                repo.to_str().unwrap(),
                "--host",
                "127.0.0.1",
                "--port",
                "0",
            ])
            .env_remove("SHORE_LOG")
            .env_remove("RUST_LOG")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn shore inspect");

        // Drain stderr in the background so it never blocks the server and is
        // available to explain a failure.
        let stderr = Arc::new(Mutex::new(String::new()));
        let mut child_stderr = child.stderr.take().expect("inspector stderr");
        {
            let sink = Arc::clone(&stderr);
            thread::spawn(move || {
                let mut buffer = String::new();
                let _ = child_stderr.read_to_string(&mut buffer);
                if let Ok(mut guard) = sink.lock() {
                    *guard = buffer;
                }
            });
        }

        // Read the bound URL from stdout, then keep draining stdout in the
        // background so the server never stalls on a full pipe.
        let stdout = child.stdout.take().expect("inspector stdout");
        let mut reader = BufReader::new(stdout);
        let mut addr = String::new();
        for _ in 0..8 {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if let Some(index) = line.find("http://") {
                        addr = line[index + "http://".len()..]
                            .trim()
                            .trim_end_matches('/')
                            .to_owned();
                        break;
                    }
                }
            }
        }
        let stdout_drain = thread::spawn(move || {
            let mut sink = String::new();
            let _ = reader.read_to_string(&mut sink);
        });

        assert!(
            !addr.is_empty(),
            "inspector did not print a bound url; stderr: {}",
            drained(&stderr)
        );

        // Wait until the server actually accepts connections, failing fast with
        // diagnostics if it exits before listening.
        let mut ready = false;
        for _ in 0..100 {
            if TcpStream::connect(&addr).is_ok() {
                ready = true;
                break;
            }
            if let Ok(Some(status)) = child.try_wait() {
                panic!(
                    "inspector exited before listening (status {status}) at {addr}; stderr: {}",
                    drained(&stderr)
                );
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            ready,
            "inspector never accepted a connection at {addr}; stderr: {}",
            drained(&stderr)
        );

        Self {
            child,
            addr,
            stderr,
            _stdout_drain: stdout_drain,
        }
    }

    fn get_json(&self, path: &str) -> Value {
        // The inspector is a blocking HTTP/1.1 server that closes each connection
        // after responding. Under load (notably on Linux CI) the close can race
        // ahead of the client's read and surface as a connection reset before the
        // body is drained. GETs are idempotent, so retry a few times with a short
        // backoff before giving up.
        let mut last_error = String::new();
        for attempt in 0..12 {
            match self.try_get(path) {
                Ok(value) => return value,
                Err(error) => {
                    last_error = error;
                    thread::sleep(Duration::from_millis(20 * (attempt + 1)));
                }
            }
        }
        panic!(
            "GET {path} failed after retries: {last_error}; server stderr: {}",
            drained(&self.stderr)
        );
    }

    /// GET a path expected to fail, returning the raw status head and the
    /// parsed JSON error body.
    fn get_error(&self, path: &str) -> (String, Value) {
        let mut stream = TcpStream::connect(&self.addr).expect("connect to inspector");
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.addr
        );
        stream.write_all(request.as_bytes()).expect("send request");
        let _ = stream.shutdown(Shutdown::Write);
        let mut response = Vec::new();
        stream.read_to_end(&mut response).expect("read response");
        let text = String::from_utf8_lossy(&response);
        let (head, body) = text
            .split_once("\r\n\r\n")
            .expect("response has header/body delimiter");
        let status = head.lines().next().unwrap_or_default().to_owned();
        let body: Value = serde_json::from_str(body).expect("error body is json");
        (status, body)
    }

    fn try_get(&self, path: &str) -> Result<Value, String> {
        let mut stream = TcpStream::connect(&self.addr).map_err(|error| error.to_string())?;
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.addr
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|error| error.to_string())?;
        // Signal end-of-request so the server never waits for more input and can
        // close its read side cleanly.
        let _ = stream.shutdown(Shutdown::Write);

        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|error| error.to_string())?;

        let text = String::from_utf8_lossy(&response);
        let (head, body) = text
            .split_once("\r\n\r\n")
            .ok_or_else(|| "response had no header/body delimiter".to_owned())?;
        if !head.starts_with("HTTP/1.1 200") {
            return Err(format!("unexpected status for {path}: {head}"));
        }
        serde_json::from_str(body).map_err(|error| format!("parse {path} body: {error}\n{body}"))
    }
}

impl Drop for Inspector {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Snapshot the background-captured server stderr, after a brief flush window so
/// an early-exiting server's output has a chance to land.
fn drained(stderr: &Arc<Mutex<String>>) -> String {
    thread::sleep(Duration::from_millis(50));
    stderr.lock().map(|guard| guard.clone()).unwrap_or_default()
}

/// Run `shore review capture` against a repo, returning the captured ReviewUnit id.
fn capture(repo: &Path) -> String {
    let output = shore(["review", "capture", "--repo", repo.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "capture stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).expect("parse capture JSON");
    json["reviewUnit"]["id"]
        .as_str()
        .expect("capture returns a ReviewUnit id")
        .to_owned()
}

/// Capture with a lineage attached, returning the full capture document.
fn capture_json_with_lineage(repo: &Path, lineage_id: &str) -> Value {
    let output = shore([
        "review",
        "capture",
        "--repo",
        repo.to_str().unwrap(),
        "--lineage",
        lineage_id,
    ]);
    assert!(
        output.status.success(),
        "capture with lineage stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse capture JSON")
}

/// Record one observation, input request, assessment, and validation check
/// against the repo's single captured ReviewUnit.
fn record_review_facts(repo: &Path) {
    let repo_arg = repo.to_str().unwrap();
    for args in [
        vec![
            "review",
            "observation",
            "add",
            "--repo",
            repo_arg,
            "--track",
            "agent:test-fixture",
            "--title",
            "linked observation",
            "--body",
            "captured before the source worktree was deleted",
        ],
        vec![
            "review",
            "input-request",
            "open",
            "--repo",
            repo_arg,
            "--track",
            "agent:test-fixture",
            "--title",
            "Need approval",
            "--reason",
            "manual-decision-required",
            "--body",
            "approve this path?",
        ],
        vec![
            "review",
            "assessment",
            "add",
            "--repo",
            repo_arg,
            "--track",
            "human:kevin",
            "--assessment",
            "accepted",
            "--summary",
            "ship it",
        ],
        vec![
            "review",
            "validation",
            "add",
            "--repo",
            repo_arg,
            "--track",
            "agent:test-fixture",
            "--check-name",
            "cargo test",
            "--status",
            "passed",
        ],
    ] {
        let output = shore(args.iter().copied());
        assert!(
            output.status.success(),
            "shore {args:?} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn capture_lineage_round(repo: &Path, lineage_id: &str, predecessor: Option<&str>) -> String {
    let mut args = vec![
        "review".to_owned(),
        "capture".to_owned(),
        "--repo".to_owned(),
        repo.to_str().unwrap().to_owned(),
        "--lineage".to_owned(),
        lineage_id.to_owned(),
    ];
    if let Some(predecessor) = predecessor {
        args.push("--predecessor".to_owned());
        args.push(predecessor.to_owned());
    }

    let output = shore(args);
    assert!(
        output.status.success(),
        "capture with lineage stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value =
        serde_json::from_slice(&output.stdout).expect("parse capture with lineage JSON");
    json["reviewUnit"]["id"]
        .as_str()
        .expect("capture with lineage returns a ReviewUnit id")
        .to_owned()
}

fn link_store(repo: &Path) {
    let output = shore(["store", "link", "--repo", repo.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "store link stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn add_worktree(repo: &Path, path: &Path, branch: &str) {
    run_git(
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

fn run_git<I>(cwd: &Path, args: I)
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
        "git failed in {}:\n{}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Minimal percent-encoding for the `:` characters in a ReviewUnit id query.
fn urlencode(value: &str) -> String {
    value.replace(':', "%3A")
}
