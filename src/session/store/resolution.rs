use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, ShoreError};
use crate::git::{git_absolute_git_dir, git_common_dir, git_object_format};
use crate::session::store::manifest::{
    StoreGitProvenance, StoreManifest, load_or_create_store_manifest, read_store_manifest,
};
use crate::session::store::store_init::{ShoreStorePaths, ensure_shore_storage_excluded};
use crate::storage::{Durability, LocalStorage};

const STORE_REGISTRATION_SCHEMA: &str = "shore.store-registration";
const STORE_REGISTRATION_VERSION: u32 = 1;
const WORKTREE_LOCAL_STORE_REF: &str = "worktree-local";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoreResolution {
    pub mode: StoreResolutionMode,
    store_dir: PathBuf,
    registration: Option<StoreRegistration>,
    manifest: Option<StoreManifest>,
}

impl StoreResolution {
    pub(crate) fn store_dir(&self) -> &Path {
        &self.store_dir
    }

    pub(crate) fn command_view(&self) -> StoreResolutionView {
        match (&self.mode, &self.registration, &self.manifest) {
            (StoreResolutionMode::CloneLocal, Some(registration), Some(_manifest)) => {
                StoreResolutionView {
                    mode: "linked",
                    store_ref: registration.store_ref.clone(),
                    clone_ref: Some(registration.clone_ref.clone()),
                    repository_family_ref: Some(registration.repository_family_ref.clone()),
                }
            }
            _ => StoreResolutionView {
                mode: "local",
                store_ref: WORKTREE_LOCAL_STORE_REF.to_owned(),
                clone_ref: None,
                repository_family_ref: None,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoreResolutionMode {
    WorktreeLocal,
    CloneLocal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoreRegistration {
    schema: String,
    version: u32,
    mode: String,
    pub store_ref: String,
    pub clone_ref: String,
    pub repository_family_ref: String,
}

impl StoreRegistration {
    fn clone_local(manifest: &StoreManifest) -> Self {
        Self {
            schema: STORE_REGISTRATION_SCHEMA.to_owned(),
            version: STORE_REGISTRATION_VERSION,
            mode: "cloneLocal".to_owned(),
            store_ref: manifest.store_id.clone(),
            clone_ref: manifest.clone_id.clone(),
            repository_family_ref: manifest.repository_family_id.clone(),
        }
    }

    fn validate_schema_version(&self) -> Result<()> {
        if self.schema == STORE_REGISTRATION_SCHEMA && self.version == STORE_REGISTRATION_VERSION {
            return Ok(());
        }

        Err(ShoreError::Message(format!(
            "unsupported store registration schema/version: {} v{}",
            self.schema, self.version
        )))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoreResolutionView {
    pub mode: &'static str,
    pub store_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clone_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_family_ref: Option<String>,
}

pub(crate) fn resolve_store(repo: impl AsRef<Path>) -> Result<StoreResolution> {
    let paths = ShoreStorePaths::resolve(repo.as_ref())?;
    let Some(registration) = read_store_registration_if_exists(paths.worktree_root())? else {
        return Ok(StoreResolution {
            mode: StoreResolutionMode::WorktreeLocal,
            store_dir: paths.shore_dir().to_path_buf(),
            registration: None,
            manifest: None,
        });
    };

    let store_dir = clone_local_store_dir(paths.worktree_root())?;
    let manifest = read_store_manifest(&store_dir)?;
    validate_registration_matches_manifest(&registration, &manifest)?;

    Ok(StoreResolution {
        mode: StoreResolutionMode::CloneLocal,
        store_dir,
        registration: Some(registration),
        manifest: Some(manifest),
    })
}

pub(crate) fn register_clone_local_store(repo: impl AsRef<Path>) -> Result<StoreRegistration> {
    let paths = ShoreStorePaths::resolve(repo.as_ref())?;
    let store_dir = clone_local_store_dir(paths.worktree_root())?;
    let manifest = load_or_create_store_manifest(
        &store_dir,
        StoreGitProvenance {
            common_dir: path_string(&git_common_dir(paths.worktree_root())?, "common-dir")?,
            git_dir: path_string(&git_absolute_git_dir(paths.worktree_root())?, "git-dir")?,
            worktree_root: path_string(paths.worktree_root(), "worktree root")?,
            object_format: git_object_format(paths.worktree_root())?,
        },
    )?;
    let registration = StoreRegistration::clone_local(&manifest);

    ensure_shore_storage_excluded(paths.worktree_root())?;
    let path = store_registration_path(paths.worktree_root());
    let storage = LocalStorage::new(paths.shore_dir());
    storage.write_json_atomic(&path, &registration, Durability::Durable)?;
    Ok(registration)
}

pub(crate) fn read_store_registration(repo: impl AsRef<Path>) -> Result<StoreRegistration> {
    let paths = ShoreStorePaths::resolve(repo.as_ref())?;
    read_store_registration_path(&store_registration_path(paths.worktree_root()))
}

fn read_store_registration_if_exists(worktree_root: &Path) -> Result<Option<StoreRegistration>> {
    let path = store_registration_path(worktree_root);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("read store registration", &path, error)),
    };

    let registration: StoreRegistration = serde_json::from_slice(&bytes)?;
    registration.validate_schema_version()?;
    Ok(Some(registration))
}

fn read_store_registration_path(path: &Path) -> Result<StoreRegistration> {
    let bytes = fs::read(path).map_err(|error| io_error("read store registration", path, error))?;
    let registration: StoreRegistration = serde_json::from_slice(&bytes)?;
    registration.validate_schema_version()?;
    Ok(registration)
}

fn validate_registration_matches_manifest(
    registration: &StoreRegistration,
    manifest: &StoreManifest,
) -> Result<()> {
    if registration.store_ref == manifest.store_id
        && registration.clone_ref == manifest.clone_id
        && registration.repository_family_ref == manifest.repository_family_id
    {
        return Ok(());
    }

    Err(ShoreError::Message(
        "store registration does not match clone-local manifest".to_owned(),
    ))
}

fn clone_local_store_dir(worktree_root: &Path) -> Result<PathBuf> {
    Ok(git_common_dir(worktree_root)?.join("shoreline"))
}

fn store_registration_path(worktree_root: &Path) -> PathBuf {
    worktree_root.join(".shore/store-registration.json")
}

fn path_string(path: &Path, description: &str) -> Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        ShoreError::Message(format!(
            "git {description} path is not utf-8: {}",
            path.display()
        ))
    })
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> ShoreError {
    ShoreError::Message(format!("{action} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::*;
    use crate::git::git_common_dir;
    use crate::session::store::manifest::read_store_manifest;
    use crate::session::store::store_init::ShoreStorePaths;

    #[test]
    fn unlinked_repository_resolves_to_worktree_local_store() {
        let repo = GitRepo::new();

        let resolution = resolve_store(repo.path()).unwrap();

        assert_eq!(resolution.mode, StoreResolutionMode::WorktreeLocal);
        assert_eq!(path_file_name(resolution.store_dir()), ".shore");
        assert_existing_paths_eq(path_parent(resolution.store_dir()), repo.path());
        assert_eq!(
            ShoreStorePaths::resolve(repo.path()).unwrap().shore_dir(),
            resolution.store_dir()
        );
    }

    #[test]
    fn linked_worktree_reads_worktree_local_registration_file() {
        let fixture = LinkedWorktreeFixture::new();

        let created = register_clone_local_store(&fixture.linked_path).unwrap();
        let read = read_store_registration(&fixture.linked_path).unwrap();

        assert_eq!(read, created);
        assert!(
            fixture
                .linked_path
                .join(".shore/store-registration.json")
                .is_file()
        );
    }

    #[test]
    fn registration_points_to_clone_local_shared_store() {
        let fixture = LinkedWorktreeFixture::new();

        let registration = register_clone_local_store(&fixture.linked_path).unwrap();
        let resolution = resolve_store(&fixture.linked_path).unwrap();

        let common_dir = git_common_dir(fixture.main.path()).unwrap();
        let expected_store = common_dir.join("shoreline");
        assert_eq!(resolution.mode, StoreResolutionMode::CloneLocal);
        assert_existing_paths_eq(resolution.store_dir(), &expected_store);

        let manifest = read_store_manifest(&expected_store).unwrap();
        assert_eq!(registration.store_ref, manifest.store_id);
        assert_eq!(registration.clone_ref, manifest.clone_id);
        assert_eq!(
            registration.repository_family_ref,
            manifest.repository_family_id
        );
    }

    #[test]
    fn command_view_uses_opaque_refs_instead_of_raw_paths() {
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();

        let resolution = resolve_store(&fixture.linked_path).unwrap();
        let json = serde_json::to_string(&resolution.command_view()).unwrap();

        assert!(json.contains("\"mode\":\"linked\""));
        assert!(json.contains("\"storeRef\":\"store:random:"));
        assert!(json.contains("\"cloneRef\":\"clone:random:"));
        assert!(json.contains("\"repositoryFamilyRef\":\"clone:random:"));
        assert!(!json.contains(fixture.main.path().to_str().unwrap()));
        assert!(!json.contains(fixture.linked_path.to_str().unwrap()));
        assert!(!json.contains(".git"));
    }

    struct LinkedWorktreeFixture {
        main: GitRepo,
        _linked_parent: TempDir,
        linked_path: PathBuf,
    }

    impl LinkedWorktreeFixture {
        fn new() -> Self {
            let main = GitRepo::new();
            main.write("README.md", "base\n");
            main.git(["add", "--all"]);
            main.git(["commit", "-m", "base"]);

            let linked_parent = TempDir::new().expect("create linked worktree parent");
            let linked_path = linked_parent.path().join("linked");
            main.git_os([
                OsString::from("worktree"),
                OsString::from("add"),
                OsString::from("-b"),
                OsString::from("linked"),
                linked_path.as_os_str().to_owned(),
            ]);

            Self {
                main,
                _linked_parent: linked_parent,
                linked_path,
            }
        }
    }

    struct GitRepo {
        root: TempDir,
    }

    impl GitRepo {
        fn new() -> Self {
            let root = TempDir::new().expect("create temp git repository directory");
            let repo = Self { root };
            repo.git(["init"]);
            repo.git(["config", "user.name", "Shore Tests"]);
            repo.git(["config", "user.email", "shore-tests@example.com"]);
            repo.git(["config", "commit.gpgsign", "false"]);
            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn write(&self, path: &str, contents: &str) {
            let path = self.root.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, contents).unwrap();
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            run_git(self.root.path(), args);
        }

        fn git_os<I>(&self, args: I)
        where
            I: IntoIterator<Item = OsString>,
        {
            run_git(self.root.path(), args);
        }
    }

    fn run_git<I, S>(cwd: &Path, args: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = args
            .into_iter()
            .map(|arg| arg.as_ref().to_owned())
            .collect::<Vec<_>>();
        let output = std::process::Command::new("git")
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
    }

    fn assert_existing_paths_eq(actual: &Path, expected: &Path) {
        assert_eq!(
            actual.canonicalize().expect("canonicalize actual path"),
            expected.canonicalize().expect("canonicalize expected path")
        );
    }

    fn path_parent(path: &Path) -> &Path {
        path.parent().expect("path has parent")
    }

    fn path_file_name(path: &Path) -> &str {
        path.file_name()
            .and_then(|name| name.to_str())
            .expect("path has utf-8 file name")
    }
}
