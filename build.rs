use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::{env, fs};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DerivedIdentity {
    pub(crate) source: &'static str,
    pub(crate) commit: Option<String>,
    pub(crate) describe: String,
    pub(crate) dirty: bool,
}

pub(crate) fn derive_identity(
    manifest_dir: &Path,
    package_version: &str,
) -> Result<DerivedIdentity, String> {
    let dot_git = manifest_dir.join(".git");
    let metadata = match fs::symlink_metadata(&dot_git) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DerivedIdentity {
                source: "package",
                commit: None,
                describe: format!("package:{package_version}"),
                dirty: false,
            });
        }
        Err(error) => {
            return Err(format!(
                "could not inspect manifest-root Git metadata {}: {error}",
                dot_git.display()
            ));
        }
    };

    if !metadata.is_dir() && !metadata.is_file() {
        return Err(format!(
            "Git metadata at {} is neither a directory nor a linked-worktree file",
            dot_git.display()
        ));
    }

    let top_level = PathBuf::from(git_stdout(manifest_dir, &["rev-parse", "--show-toplevel"])?);
    let canonical_manifest = manifest_dir.canonicalize().map_err(|error| {
        format!(
            "could not canonicalize manifest root {}: {error}",
            manifest_dir.display()
        )
    })?;
    let canonical_top_level = top_level.canonicalize().map_err(|error| {
        format!(
            "Git metadata at {} returned an invalid worktree root {}: {error}",
            dot_git.display(),
            top_level.display()
        )
    })?;
    if canonical_top_level != canonical_manifest {
        return Err(format!(
            "Git metadata at {} resolves to {}, not the manifest root {}",
            dot_git.display(),
            canonical_top_level.display(),
            canonical_manifest.display()
        ));
    }

    let commit = git_stdout(manifest_dir, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    if commit.len() != 40
        || !commit
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!(
            "Git metadata at {} returned a non-canonical commit id: {commit:?}",
            dot_git.display()
        ));
    }

    let base_describe = git_stdout(manifest_dir, &["describe", "--tags", "--always"])?;
    let dirty = !git_stdout(
        manifest_dir,
        &["status", "--porcelain=v1", "--untracked-files=no"],
    )?
    .is_empty();
    let describe = git_stdout(manifest_dir, &["describe", "--tags", "--always", "--dirty"])?;
    if describe.is_empty() || describe.contains(['\r', '\n']) {
        return Err(format!(
            "Git metadata at {} returned an invalid describe value: {describe:?}",
            dot_git.display()
        ));
    }
    let expected_describe = if dirty {
        format!("{base_describe}-dirty")
    } else {
        base_describe
    };
    if describe != expected_describe {
        return Err(format!(
            "Git metadata at {} changed while build identity was being derived: expected {expected_describe:?}, got {describe:?}",
            dot_git.display()
        ));
    }

    Ok(DerivedIdentity {
        source: "git",
        commit: Some(commit),
        describe,
        dirty,
    })
}

fn main() {
    if let Err(error) = run() {
        panic!("could not derive truthful Pointbreak build identity: {error}");
    }
}

fn run() -> Result<(), String> {
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "Cargo did not provide CARGO_MANIFEST_DIR".to_owned())?;
    let package_version = env::var("CARGO_PKG_VERSION")
        .map_err(|_| "Cargo did not provide CARGO_PKG_VERSION".to_owned())?;
    let identity = derive_identity(&manifest_dir, &package_version)?;

    println!("cargo:rerun-if-changed=build.rs");
    if identity.source == "git" {
        emit_git_rerun_directives(&manifest_dir)?;
    }
    println!(
        "cargo:rustc-env=POINTBREAK_BUILD_SOURCE={}",
        identity.source
    );
    println!(
        "cargo:rustc-env=POINTBREAK_BUILD_COMMIT={}",
        identity.commit.as_deref().unwrap_or("")
    );
    println!(
        "cargo:rustc-env=POINTBREAK_BUILD_DESCRIBE={}",
        identity.describe
    );
    println!("cargo:rustc-env=POINTBREAK_BUILD_DIRTY={}", identity.dirty);
    Ok(())
}

fn emit_git_rerun_directives(manifest_dir: &Path) -> Result<(), String> {
    let mut paths = BTreeSet::new();
    let dot_git = manifest_dir.join(".git");
    if dot_git.is_file() {
        paths.insert(dot_git);
    }
    for name in ["HEAD", "index", "packed-refs", "refs/tags"] {
        paths.insert(resolve_git_path(manifest_dir, name)?);
    }

    if let Some(reference) = git_stdout_optional(manifest_dir, &["symbolic-ref", "-q", "HEAD"])? {
        paths.insert(resolve_git_path(manifest_dir, &reference)?);
    }

    let tracked = git_output(manifest_dir, &["ls-files", "-z"])?;
    for relative in tracked.stdout.split(|byte| *byte == 0) {
        if relative.is_empty() {
            continue;
        }
        let relative = std::str::from_utf8(relative).map_err(|_| {
            "Git tracked-file inventory contains a non-UTF-8 path that Cargo cannot watch"
                .to_owned()
        })?;
        if relative.contains(['\r', '\n']) {
            return Err(format!(
                "Git tracked-file inventory contains a line-breaking path Cargo cannot watch: {relative:?}"
            ));
        }
        paths.insert(manifest_dir.join(relative));
    }

    for path in paths {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    Ok(())
}

fn resolve_git_path(manifest_dir: &Path, name: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(git_stdout(
        manifest_dir,
        &["rev-parse", "--git-path", name],
    )?);
    Ok(if path.is_absolute() {
        path
    } else {
        manifest_dir.join(path)
    })
}

fn git_stdout(manifest_dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = git_output(manifest_dir, args)?;
    String::from_utf8(output.stdout)
        .map(|stdout| stdout.trim_end_matches(['\r', '\n']).to_owned())
        .map_err(|_| format!("git {} returned non-UTF-8 output", args.join(" ")))
}

fn git_stdout_optional(manifest_dir: &Path, args: &[&str]) -> Result<Option<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(manifest_dir)
        .args(args)
        .output()
        .map_err(|error| format!("could not run git {}: {error}", args.join(" ")))?;
    if output.status.success() {
        return String::from_utf8(output.stdout)
            .map(|stdout| Some(stdout.trim_end_matches(['\r', '\n']).to_owned()))
            .map_err(|_| format!("git {} returned non-UTF-8 output", args.join(" ")));
    }
    if output.status.code() == Some(1) && output.stderr.is_empty() {
        return Ok(None);
    }
    Err(git_failure(manifest_dir, args, &output))
}

fn git_output(manifest_dir: &Path, args: &[&str]) -> Result<Output, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(manifest_dir)
        .args(args)
        .output()
        .map_err(|error| format!("could not run git {}: {error}", args.join(" ")))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(git_failure(manifest_dir, args, &output))
    }
}

fn git_failure(manifest_dir: &Path, args: &[&str], output: &Output) -> String {
    format!(
        "Git metadata at {} failed `git {}` ({}): {}",
        manifest_dir.join(".git").display(),
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    )
}
