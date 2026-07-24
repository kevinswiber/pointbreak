use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

#[derive(Debug)]
pub struct GitOutput {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug)]
pub struct GitRepo {
    root: TempDir,
}

impl GitRepo {
    pub fn new() -> Self {
        let root = TempDir::new().expect("create temp git repository directory");
        copy_git_skeleton(root.path().join(".git"));
        Self { root }
    }

    pub fn path(&self) -> &Path {
        self.root.path()
    }

    pub fn init_at(path: impl AsRef<Path>) {
        let path = path.as_ref();
        fs::create_dir_all(path).expect("create nested git repository directory");
        copy_git_skeleton(path.join(".git"));
    }

    pub fn read(&self, path: impl AsRef<Path>) -> String {
        fs::read_to_string(self.root.path().join(path)).expect("read test repository file")
    }

    pub fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
        let path = self.root.path().join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directories");
        }
        fs::write(path, contents).expect("write test repository file");
    }

    pub fn write_fixture(
        &self,
        path: impl AsRef<Path>,
        contents: impl AsRef<[u8]>,
    ) -> std::path::PathBuf {
        let path = self.root.path().join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directories");
        }
        fs::write(&path, contents).expect("write test fixture file");
        path
    }

    pub fn remove(&self, path: impl AsRef<Path>) {
        let path = self.root.path().join(path);
        if path.is_dir() {
            fs::remove_dir_all(path).expect("remove test repository directory");
        } else {
            fs::remove_file(path).expect("remove test repository file");
        }
    }

    pub fn commit_all(&self, message: &str) {
        self.git(["add", "--all"]);
        self.git(["commit", "-m", message]);
    }

    pub fn mark_executable_in_index(&self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        self.git(["config", "core.filemode", "false"]);
        self.git([
            "update-index",
            "--chmod=+x",
            "--",
            path.to_str().expect("fixture path is utf-8"),
        ]);
    }

    pub fn git<I, S>(&self, args: I) -> GitOutput
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_git(self.root.path(), args)
    }
}

impl Default for GitRepo {
    fn default() -> Self {
        Self::new()
    }
}

/// Bootstrap a `.git` directory at `git_dir` by copying the frozen skeleton
/// template — an empty repository on the deterministic `refs/heads/main` branch,
/// with the fixture identity baked in — without spawning `git`. Baking the branch
/// keeps a capture (and the auto-recorded capture-time ref name that appears in
/// documents) independent of the host's `init.defaultBranch`, which differs
/// between developer machines (main) and CI (master). The template carries the
/// content files a fresh `git init` writes (`HEAD`, `config`, `info/exclude`);
/// the always-empty scaffold directories git cannot track are recreated here so a
/// later `add`/`commit` on real git has the structure it expects.
// Resolved locally rather than via the `support` parent: several integration tests
// pull this file in standalone with `#[path = "support/git_repo.rs"] mod git_repo;`,
// where `super` is that test's crate root, not `support`. Prefers the runtime
// `CARGO_MANIFEST_DIR` (cargo-nextest remaps it under `--workspace-remap`) so the
// skeleton resolves when the suite runs from an archive on another machine, falling
// back to the compile-time value for ordinary in-place runs.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn copy_git_skeleton(git_dir: impl AsRef<Path>) {
    let git_dir = git_dir.as_ref();
    let skeleton = manifest_dir().join("tests/support/assets/git-skeleton");
    copy_dir_recursive(&skeleton, git_dir);
    for scaffold in ["objects/info", "objects/pack", "refs/heads", "refs/tags"] {
        fs::create_dir_all(git_dir.join(scaffold)).expect("create git skeleton directory");
    }
}

fn copy_dir_recursive(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).expect("create git skeleton directory");
    for entry in fs::read_dir(src).expect("read git skeleton template") {
        let entry = entry.expect("read git skeleton entry");
        let target = dest.join(entry.file_name());
        if entry.file_type().expect("stat git skeleton entry").is_dir() {
            copy_dir_recursive(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy git skeleton file");
        }
    }
}

// Per-thread bootstrap spawn counter. A test resets it, bootstraps a fixture,
// and asserts on the exact number of `git` subprocesses the bootstrap paid.
// Keeping it thread-local makes each test's reset/act/assert protocol immune to
// concurrent fixtures on other threads under a shared-process runner, where a
// process-global counter would race across tests.
#[cfg(test)]
thread_local! {
    static GIT_SPAWN_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn record_git_spawn() {
    GIT_SPAWN_COUNT.with(|cell| cell.set(cell.get() + 1));
}

#[cfg(test)]
pub fn git_spawn_count() -> usize {
    GIT_SPAWN_COUNT.with(std::cell::Cell::get)
}

#[cfg(test)]
pub fn reset_git_spawn_count() {
    GIT_SPAWN_COUNT.with(|cell| cell.set(0));
}

fn run_git<I, S>(cwd: &Path, args: I) -> GitOutput
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect::<Vec<_>>();
    #[cfg(test)]
    record_git_spawn();
    let output = Command::new("git")
        .args(&args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("run git {:?} in {}: {error}", args, cwd.display()));

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    assert!(
        output.status.success(),
        "git {:?} failed in {}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        args,
        cwd.display(),
        output.status,
        stdout,
        stderr
    );

    GitOutput { stdout, stderr }
}
