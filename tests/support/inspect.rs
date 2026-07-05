//! Shared HTTP harness for the `shore inspect` integration suites.
//!
//! The inspector's JSON builders live in the binary crate
//! (`src/cli/inspect/api.rs`), so they are not reachable from an integration
//! test by a direct call. These tests instead exercise the genuine production
//! JSON end to end: they spawn the real `shore inspect --port 0` server (which
//! prints its bound URL and supports an ephemeral port) and issue raw HTTP/1.1
//! GETs. The store is always built at test time through the `shore` CLI, so it
//! tracks the current on-disk layout without hard-coding any store path.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use super::git_repo::GitRepo;
use super::shore;

/// A repository plus a worktree on a fresh branch with one captured Revision.
pub struct WorktreeCapture {
    pub _main: GitRepo,
    pub _parent: tempfile::TempDir,
    pub worktree: PathBuf,
    pub revision_id: String,
}

impl WorktreeCapture {
    pub fn on_branch(dir_name: &str, branch: &str) -> Self {
        let main = GitRepo::new();
        main.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        main.commit_all("base");

        let parent = tempfile::tempdir().expect("worktree parent");
        let worktree = parent.path().join(dir_name);
        add_worktree(main.path(), &worktree, branch);
        std::fs::write(worktree.join("src/lib.rs"), "pub fn value() -> u32 { 2 }\n").unwrap();

        let revision_id = capture(&worktree);

        Self {
            _main: main,
            _parent: parent,
            worktree,
            revision_id,
        }
    }
}

/// A spawned `shore inspect` server bound to an ephemeral port, killed on drop.
pub struct Inspector {
    child: Child,
    addr: String,
    stderr: Arc<Mutex<String>>,
    _stdout_drain: thread::JoinHandle<()>,
}

impl Inspector {
    pub fn spawn(repo: &Path) -> Self {
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

    pub fn get_json(&self, path: &str) -> Value {
        let body = self.get_text(path);
        serde_json::from_str(&body)
            .unwrap_or_else(|error| panic!("parse {path} body: {error}\n{body}"))
    }

    /// GET a path expected to succeed, returning the raw 200 response body.
    ///
    /// The inspector is a blocking HTTP/1.1 server that closes each connection
    /// after responding. Under load (notably on Linux CI) the close can race
    /// ahead of the client's read and surface as a connection reset before the
    /// body is drained. GETs are idempotent, so retry a few times with a short
    /// backoff before giving up.
    pub fn get_text(&self, path: &str) -> String {
        let mut last_error = String::new();
        for attempt in 0..12 {
            match self.try_get(path) {
                Ok((_, body)) => return body,
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
    pub fn get_error(&self, path: &str) -> (String, Value) {
        let (status, body) = self.raw_get(path);
        let body: Value = serde_json::from_str(&body).expect("error body is json");
        (status, body)
    }

    /// GET a path returning the raw status line and body, with no status
    /// assertion — for socket-level route/error coverage.
    pub fn raw_get(&self, path: &str) -> (String, String) {
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
        (status, body.to_owned())
    }

    /// Issue a raw request line (method + target) and return the status line,
    /// for exercising non-GET routes.
    pub fn request(&self, method: &str, path: &str) -> String {
        let mut stream = TcpStream::connect(&self.addr).expect("connect to inspector");
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.addr
        );
        stream.write_all(request.as_bytes()).expect("send request");
        let _ = stream.shutdown(Shutdown::Write);
        let mut response = Vec::new();
        stream.read_to_end(&mut response).expect("read response");
        let text = String::from_utf8_lossy(&response);
        text.lines().next().unwrap_or_default().to_owned()
    }

    fn try_get(&self, path: &str) -> Result<(String, String), String> {
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
        Ok((head.to_owned(), body.to_owned()))
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

/// Run `shore review capture` against a repo, returning the captured Revision id.
pub fn capture(repo: &Path) -> String {
    let output = shore(["capture", "--repo", repo.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "capture stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).expect("parse capture JSON");
    json["revision"]["id"]
        .as_str()
        .expect("capture returns a Revision id")
        .to_owned()
}

/// A populated store assembled once through the `shore` CLI, reused by the
/// endpoint-contract and validation read-surface suites: one captured Revision
/// carrying a range-targeted observation, an operative input request, a
/// superseded + a superseding assessment (two tracks), and two validation checks
/// (passed `cargo test` on the agent track, failed `cargo clippy` on the human
/// track).
pub struct RepresentativeStore {
    pub repo: GitRepo,
    pub revision_id: String,
    pub snapshot_id: String,
}

/// Build the [`RepresentativeStore`] entirely through the real CLI, so it tracks
/// the current on-disk store layout without hard-coding any path.
pub fn representative_store() -> RepresentativeStore {
    let repo = GitRepo::new();
    repo.write(
        "src/lib.rs",
        "pub fn value() -> u32 {\n    1\n}\n\npub fn other() -> u32 {\n    2\n}\n",
    );
    repo.commit_all("base");
    repo.write(
        "src/lib.rs",
        "pub fn value() -> u32 {\n    42\n}\n\npub fn other() -> u32 {\n    7\n}\n",
    );

    let repo_arg = repo.path().to_str().unwrap().to_owned();
    let capture = run_shore_json(&["capture", "--repo", &repo_arg]);
    let revision_id = capture["revision"]["id"]
        .as_str()
        .expect("capture returns a Revision id")
        .to_owned();
    let snapshot_id = capture["revision"]["objectId"]
        .as_str()
        .expect("capture returns a snapshot id")
        .to_owned();

    // Range-targeted observation on the agent track.
    run_shore(&[
        "observation",
        "add",
        "--repo",
        &repo_arg,
        "--track",
        "agent:codex",
        "--title",
        "Observed change",
        "--body",
        "the return value changed",
        "--file",
        "src/lib.rs",
        "--start-line",
        "2",
        "--end-line",
        "2",
    ]);

    // Operative input request.
    run_shore(&[
        "review",
        "input-request",
        "open",
        "--repo",
        &repo_arg,
        "--track",
        "agent:codex",
        "--title",
        "Need a decision",
        "--reason",
        "manual-decision-required",
        "--body",
        "should we ship this?",
    ]);

    // A first assessment (agent track), then a superseding assessment (human
    // track) that replaces it, so current-assessment resolution is exercised.
    let first = run_shore_json(&[
        "assessment",
        "add",
        "--repo",
        &repo_arg,
        "--track",
        "agent:codex",
        "--assessment",
        "needs-changes",
        "--summary",
        "not yet",
    ]);
    let first_assessment_id = first["assessmentId"]
        .as_str()
        .expect("assessment add returns an assessment id")
        .to_owned();
    run_shore(&[
        "assessment",
        "add",
        "--repo",
        &repo_arg,
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--summary",
        "ship it",
        "--replaces",
        &first_assessment_id,
    ]);

    // Two validation checks across two tracks: a passed cargo test and a failed
    // cargo clippy carrying an exit code + command.
    run_shore(&[
        "validation",
        "add",
        "--repo",
        &repo_arg,
        "--track",
        "agent:codex",
        "--check-name",
        "cargo test",
        "--status",
        "passed",
    ]);
    run_shore(&[
        "validation",
        "add",
        "--repo",
        &repo_arg,
        "--track",
        "human:kevin",
        "--check-name",
        "cargo clippy",
        "--status",
        "failed",
        "--exit-code",
        "1",
        "--command",
        "cargo clippy -- -D warnings",
    ]);

    RepresentativeStore {
        repo,
        revision_id,
        snapshot_id,
    }
}

/// Run a `shore` subcommand, asserting success and surfacing stderr on failure.
fn run_shore(args: &[&str]) {
    let output = shore(args);
    assert!(
        output.status.success(),
        "shore {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Run a `shore` subcommand and parse its stdout as JSON.
fn run_shore_json(args: &[&str]) -> Value {
    let output = shore(args);
    assert!(
        output.status.success(),
        "shore {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("parse shore {args:?} JSON: {error}"))
}

/// Capture a revision in a supersession thread. With no `predecessor`, captures
/// the current worktree as a fresh revision. With a `predecessor`, mutates a
/// tracked file first (so the snapshot id differs) and records the new revision
/// as superseding the predecessor. Returns the captured revision id.
pub fn capture_supersession_round(repo: &Path, predecessor: Option<&str>) -> String {
    let mut args = vec![
        "capture".to_owned(),
        "--repo".to_owned(),
        repo.to_str().unwrap().to_owned(),
    ];
    if let Some(predecessor) = predecessor {
        // A successor must carry different content, or it would collapse to the
        // same snapshot id. Append a unique line to a tracked file.
        let target = repo.join("src/lib.rs");
        let mut contents = std::fs::read_to_string(&target).unwrap_or_default();
        contents.push_str(&format!(
            "\n// supersedes {}\n",
            predecessor.replace(':', "_")
        ));
        std::fs::write(&target, contents).expect("write successor content");
        args.push("--supersedes".to_owned());
        args.push(predecessor.to_owned());
    }

    let output = shore(args);
    assert!(
        output.status.success(),
        "capture supersession round stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value =
        serde_json::from_slice(&output.stdout).expect("parse supersession capture JSON");
    json["revision"]["id"]
        .as_str()
        .expect("supersession capture returns a Revision id")
        .to_owned()
}

pub fn add_worktree(repo: &Path, path: &Path, branch: &str) {
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

pub fn run_git<I>(cwd: &Path, args: I)
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

/// Minimal percent-encoding for the `:` characters in a Revision id query.
pub fn urlencode(value: &str) -> String {
    value.replace(':', "%3A")
}
