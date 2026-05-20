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
        let repo = Self { root };

        repo.git(["init"]);
        repo.git(["config", "user.name", "Shore Tests"]);
        repo.git(["config", "user.email", "shore-tests@example.com"]);
        repo.git(["config", "commit.gpgsign", "false"]);

        repo
    }

    pub fn path(&self) -> &Path {
        self.root.path()
    }

    pub fn init_at(path: impl AsRef<Path>) {
        let path = path.as_ref();
        fs::create_dir_all(path).expect("create nested git repository directory");
        run_git(path, ["init"]);
        run_git(path, ["config", "user.name", "Shore Tests"]);
        run_git(path, ["config", "user.email", "shore-tests@example.com"]);
        run_git(path, ["config", "commit.gpgsign", "false"]);
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

fn run_git<I, S>(cwd: &Path, args: I) -> GitOutput
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
