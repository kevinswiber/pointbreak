//! `pointbreak history --watch` re-renders only when the store's liveness
//! token (`event_set_hash`) changes, never on a bare poll tick, and runs as a
//! pure client-side poll: no daemon, no filesystem watch. It is killed on drop.

mod support;

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use support::git_repo::GitRepo;
use support::inspect::capture;
use support::pointbreak;

/// A spawned `pointbreak history --watch`, draining stdout into a shared
/// buffer in the background; killed on drop.
struct Watcher {
    child: Child,
    stdout: Arc<Mutex<String>>,
    _drain: thread::JoinHandle<()>,
}

impl Watcher {
    fn spawn(repo: &Path, poll_ms: u64) -> Self {
        Self::spawn_with_args(repo, poll_ms, &[])
    }

    fn spawn_with_args(repo: &Path, poll_ms: u64, extra_args: &[&str]) -> Self {
        let mut command = Command::new(support::pointbreak_bin());
        command.args([
            "history",
            "--repo",
            repo.to_str().unwrap(),
            "--watch",
            "--poll-ms",
            &poll_ms.to_string(),
        ]);
        let mut child = command
            .args(extra_args)
            .env_remove("POINTBREAK_LOG")
            .env_remove("RUST_LOG")
            .env_remove("POINTBREAK_FORMAT")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn pointbreak review history --watch");

        let stdout = Arc::new(Mutex::new(String::new()));
        let mut child_stdout = child.stdout.take().expect("watcher stdout");
        let sink = Arc::clone(&stdout);
        let drain = thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match child_stdout.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut guard) = sink.lock() {
                            guard.push_str(&String::from_utf8_lossy(&buf[..n]));
                        }
                    }
                }
            }
        });

        Self {
            child,
            stdout,
            _drain: drain,
        }
    }

    /// Each render is one compact JSON document on its own line, so a render
    /// count is the number of non-empty lines emitted so far.
    fn render_count(&self) -> usize {
        self.stdout
            .lock()
            .map(|guard| guard.lines().filter(|line| !line.trim().is_empty()).count())
            .unwrap_or(0)
    }

    fn renders(&self) -> Vec<Value> {
        self.stdout
            .lock()
            .map(|guard| {
                guard
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| serde_json::from_str(line).expect("watch render is JSON"))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Poll until at least `target` renders have appeared (or the timeout
    /// elapses), returning the final count.
    fn wait_for_renders(&self, target: usize, timeout: Duration) -> usize {
        let deadline = Instant::now() + timeout;
        loop {
            let count = self.render_count();
            if count >= target || Instant::now() >= deadline {
                return count;
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn watch_reprints_only_when_event_set_hash_changes() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");

    // Seed one captured review so the watcher starts from a non-empty store.
    capture(repo.path());

    let watcher = Watcher::spawn(repo.path(), 50);

    // 1) The initial state prints exactly once on startup.
    let after_initial = watcher.wait_for_renders(1, Duration::from_secs(10));
    assert_eq!(after_initial, 1, "watch prints once on startup");

    // 2) Several poll cycles pass with no store change: no reprint. This is the
    //    crux — reprints track content transitions, not wall-clock ticks.
    thread::sleep(Duration::from_millis(500));
    assert_eq!(
        watcher.render_count(),
        1,
        "watch must not reprint on a bare tick"
    );

    // 3) A real change — one new observation event — triggers exactly one
    //    reprint.
    let output = pointbreak([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "watched change",
        "--body",
        "this moves the event set hash",
    ]);
    assert!(
        output.status.success(),
        "observation add failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let after_change = watcher.wait_for_renders(2, Duration::from_secs(10));
    assert_eq!(
        after_change, 2,
        "watch reprints once when the event set changes"
    );

    // 4) The change settles: no further reprint without another change.
    thread::sleep(Duration::from_millis(500));
    assert_eq!(
        watcher.render_count(),
        2,
        "watch must not reprint again once the change is rendered"
    );
}

#[test]
fn watch_tail_renders_the_newest_n_and_appends() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    capture(repo.path());
    let path = repo.path().to_str().unwrap();
    add_observation(path, "first");
    thread::sleep(Duration::from_millis(2));
    add_observation(path, "second");

    let watcher = Watcher::spawn_with_args(repo.path(), 50, &["--tail", "2"]);
    assert_eq!(watcher.wait_for_renders(1, Duration::from_secs(10)), 1);
    assert_eq!(render_titles(&watcher.renders()[0]), ["first", "second"]);

    thread::sleep(Duration::from_millis(2));
    add_observation(path, "third");

    assert_eq!(watcher.wait_for_renders(2, Duration::from_secs(10)), 2);
    assert_eq!(render_titles(&watcher.renders()[1]), ["second", "third"]);
}

fn add_observation(repo: &str, title: &str) {
    let output = pointbreak([
        "observation",
        "add",
        "--repo",
        repo,
        "--track",
        "agent:codex",
        "--title",
        title,
    ]);
    assert!(output.status.success());
}

fn render_titles(render: &Value) -> Vec<&str> {
    render["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["summary"]["title"].as_str().unwrap())
        .collect()
}
