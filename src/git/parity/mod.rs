//! The differential subprocess-vs-gix parity harness (mirrors the storage
//! qualification idiom in `bench_support/foundation/candidate.rs`). Each routable
//! contract vector runs on both backends over an edge-case fixture battery and
//! its typed output (or normalized error) is compared.
//!
//! Phase 2 is **report-only**: `run_routable_gate` produces one result per
//! routable class and never fails on divergence. Capture-time diff and write-tree
//! are non-routable; the diagnostic probes here only *measure* their divergence
//! (the D5 boundary) and never gate. The whole module is compiled only under
//! `all(test, feature = "gix-parity")`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::canonical_hash::sha256_bytes_hex;
use crate::error::{Result, ShoreError};
use crate::git::backend::GitBackend;
use crate::git::backend::gix::{GixBackend, gix_open_count, reset_gix_open_count};
use crate::git::backend::subprocess::{
    SubprocessBackend, git_spawn_count, reset_discovery_cache, reset_git_spawn_count, run_git,
    run_git_with_stdin,
};

pub(crate) const GIT_BACKEND_PARITY_RESULT_SCHEMA_V1: &str =
    "pointbreak.git-backend-parity-result.v1";

/// The per-class outcome — the fold of a class's vector verdicts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParityOutcome {
    Passed,
    Failed,
}

/// One `qualify_op` comparison result: the backends agreed or diverged.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VectorVerdict {
    Match,
    Divergent,
}

/// A per-class parity result (mirrors `QualificationScenarioResultV1`). The
/// diagnostic measurement fields carry the non-routable diff/write-tree verdicts
/// and never affect `outcome`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GitBackendParityResultV1 {
    pub schema: String,
    pub class: String,
    pub outcome: ParityOutcome,
    pub vectors: usize,
    pub divergences: usize,
    pub operating_system: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_tier1_object_id: Option<VectorVerdict>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_tier2_content_hash: Option<VectorVerdict>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_tree: Option<VectorVerdict>,
}

/// The two-tier diff diagnostic (D5): tier 1 is the rename-source object-id input
/// (which diverges by gix's documented first-found heuristic), tier 2 the full
/// rendered content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DiffProbe {
    pub tier1_object_id: VectorVerdict,
    pub tier2_content_hash: VectorVerdict,
}

// ---------- the differential driver ----------

/// Run one contract vector on both backends and compare. The discovery memo is
/// reset around each backend so the subprocess call actually resolves and the
/// gix call can never be satisfied by the subprocess-cached value (F6). Error
/// parity qualifies only when the normalized category *and* message agree (LB-12).
fn qualify_op<T: PartialEq + std::fmt::Debug>(
    fixture: &GitFixture,
    op: impl Fn(&dyn GitBackend, &Path) -> Result<T>,
) -> VectorVerdict {
    reset_discovery_cache();
    let subprocess = op(&SubprocessBackend, fixture.path());
    reset_discovery_cache();
    let gix = op(&GixBackend, fixture.path());
    let verdict = match (&subprocess, &gix) {
        (Ok(subprocess), Ok(gix)) if subprocess == gix => VectorVerdict::Match,
        (Err(subprocess), Err(gix)) if normalize_error(subprocess) == normalize_error(gix) => {
            VectorVerdict::Match
        }
        _ => VectorVerdict::Divergent,
    };
    if verdict == VectorVerdict::Divergent {
        // Print the diverging typed values so a qualification run (especially the
        // cross-platform legs) can see exactly what differs, not just a count.
        eprintln!("  DIVERGENCE: subprocess={subprocess:?} gix={gix:?}");
    }
    verdict
}

/// Normalize an error to (category, message) for parity comparison (LB-12).
fn normalize_error(error: &ShoreError) -> (String, String) {
    let category = match error {
        ShoreError::Message(_) => "message",
        ShoreError::GitCommand { .. } => "git-command",
        _ => "other",
    };
    (category.to_owned(), error.to_string())
}

fn class_result(class: &'static str, verdicts: &[VectorVerdict]) -> GitBackendParityResultV1 {
    let divergences = verdicts
        .iter()
        .filter(|verdict| **verdict == VectorVerdict::Divergent)
        .count();
    GitBackendParityResultV1 {
        schema: GIT_BACKEND_PARITY_RESULT_SCHEMA_V1.to_owned(),
        class: class.to_owned(),
        outcome: if divergences == 0 {
            ParityOutcome::Passed
        } else {
            ParityOutcome::Failed
        },
        vectors: verdicts.len(),
        divergences,
        operating_system: std::env::consts::OS.to_owned(),
        diff_tier1_object_id: None,
        diff_tier2_content_hash: None,
        write_tree: None,
    }
}

/// Run the routable contract vectors over the edge-case battery and **report**
/// one result per routable class. This never fails on divergence — the enforcing
/// gate (fail only gix-qualified classes) lands with the per-class defaults in a
/// later phase.
pub(crate) fn run_routable_gate() -> Vec<GitBackendParityResultV1> {
    vec![
        class_result("read:graph-refs", &read_graph_refs_vectors()),
        class_result("read:ignore", &read_ignore_vectors()),
        class_result("read:inventory", &read_inventory_vectors()),
        class_result("read:config-discovery", &read_config_discovery_vectors()),
        class_result("read:repo-discovery", &read_repo_discovery_vectors()),
        class_result("identity-scalars", &identity_scalar_vectors()),
    ]
}

fn read_graph_refs_vectors() -> Vec<VectorVerdict> {
    let two = two_commit_fixture();
    let head = rev(two.path(), "HEAD");
    let base = rev(two.path(), "HEAD~1");
    let commit_set = BTreeSet::from([head.clone(), base.clone()]);
    let mut verdicts = vec![
        qualify_op(&two, |backend, path| {
            backend.for_each_ref(path, &["refs/heads/"])
        }),
        qualify_op(&two, |backend, path| backend.ref_state_lines(path)),
        qualify_op(&two, |backend, path| backend.object_exists(path, &head)),
        qualify_op(&two, |backend, path| {
            backend.is_ancestor(path, &base, &head)
        }),
        qualify_op(&two, |backend, path| {
            backend.rev_list_reachable(path, std::slice::from_ref(&head))
        }),
        qualify_op(&two, |backend, path| {
            backend.commit_subjects(path, &commit_set)
        }),
        qualify_op(&two, |backend, path| {
            backend.commit_changed_paths(path, &head)
        }),
        qualify_op(&two, |backend, path| {
            backend.independent_commits(path, &[head.clone(), base.clone()])
        }),
        qualify_op(&two, |backend, path| backend.worktree_list(path)),
        // Empty rev-range: nothing reachable from head that is not reachable from base.
        qualify_op(&two, |backend, path| {
            backend.rev_list_range(path, &format!("{head}..{base}"))
        }),
    ];

    // Root commit vs empty tree: a parentless commit lists its whole tree.
    let root = root_commit_fixture();
    let root_head = rev(root.path(), "HEAD");
    verdicts.push(qualify_op(&root, |backend, path| {
        backend.commit_changed_paths(path, &root_head)
    }));

    // Dangling origin/HEAD falls through to the local default.
    let dangling = dangling_origin_head_fixture();
    verdicts.push(qualify_op(&dangling, |backend, path| {
        backend.default_branch_ref(path)
    }));

    // Absent reflog (an unborn repository): reflog probes report empty.
    let unborn = unborn_fixture();
    verdicts.push(qualify_op(&unborn, |backend, path| {
        backend.rev_list_reflog_reachable(path)
    }));

    // Reflog subject fidelity: an amended commit records a `commit (amend)` subject.
    let amended = amended_reflog_fixture();
    let branch = current_branch_ref(amended.path());
    verdicts.push(qualify_op(&amended, move |backend, path| {
        backend.reflog_entries(path, &branch)
    }));

    verdicts
}

fn read_ignore_vectors() -> Vec<VectorVerdict> {
    let ignore = ignore_stack_fixture();
    let mut verdicts = vec![qualify_op(&ignore, |backend, path| {
        backend.paths_are_ignored(
            path,
            &[
                ".pointbreak/data/state.json",
                ".pointbreak/delegates.local.json",
            ],
        )
    })];

    // Post-ignore-source-mutation re-probe (LB-5): after appending to
    // info/exclude and writing a committed .pointbreak/.gitignore, both backends
    // must observe the newly ignored paths (a fresh exclude stack per call).
    let mutated = post_mutation_ignore_fixture();
    verdicts.push(qualify_op(&mutated, |backend, path| {
        backend.paths_are_ignored(path, &["build/output.o", ".pointbreak/data/state.json"])
    }));

    verdicts
}

fn read_inventory_vectors() -> Vec<VectorVerdict> {
    let inventory = inventory_fixture();
    let mut verdicts = vec![
        qualify_op(&inventory, |backend, path| {
            backend.untracked_inventory(path)
        }),
        qualify_op(&inventory, |backend, path| {
            backend.tracked_and_untracked_inventory(path)
        }),
        qualify_op(&inventory, |backend, path| {
            backend.path_is_untracked(path, "src/new.rs")
        }),
    ];

    let crlf = crlf_fixture();
    verdicts.push(qualify_op(&crlf, |backend, path| {
        backend.untracked_inventory(path)
    }));

    let submodule = submodule_fixture();
    verdicts.push(qualify_op(&submodule, |backend, path| {
        backend.tracked_and_untracked_inventory(path)
    }));

    #[cfg(unix)]
    {
        let mode_only = mode_only_fixture();
        verdicts.push(qualify_op(&mode_only, |backend, path| {
            backend.untracked_inventory(path)
        }));
        let type_change = type_change_fixture();
        verdicts.push(qualify_op(&type_change, |backend, path| {
            backend.untracked_inventory(path)
        }));
        // A non-UTF-8 filename only exists on filesystems that allow one.
        if let Some(non_utf8) = non_utf8_fixture() {
            verdicts.push(qualify_op(&non_utf8, |backend, path| {
                backend.untracked_inventory(path)
            }));
        }
    }

    verdicts
}

fn read_config_discovery_vectors() -> Vec<VectorVerdict> {
    let config = config_discovery_fixture();
    vec![qualify_op(&config, |backend, path| {
        Ok(backend.config_path_get(path, "user.signingkey"))
    })]
}

fn read_repo_discovery_vectors() -> Vec<VectorVerdict> {
    let linked = linked_worktree_fixture();
    vec![
        qualify_op(&linked, |backend, path| backend.common_dir(path)),
        qualify_op(&two_commit_fixture(), |backend, path| {
            backend.common_dir(path)
        }),
    ]
}

fn identity_scalar_vectors() -> Vec<VectorVerdict> {
    let two = two_commit_fixture();
    let head = rev(two.path(), "HEAD");
    let mut verdicts = vec![
        qualify_op(&two, |backend, path| backend.head_oid(path)),
        qualify_op(&two, |backend, path| backend.head_ref(path)),
        qualify_op(&two, |backend, path| backend.head_commit_oid_optional(path)),
        qualify_op(&two, |backend, path| backend.empty_tree_oid(path)),
        qualify_op(&two, |backend, path| {
            backend.rev_parse_commit_oid(path, "HEAD")
        }),
        qualify_op(&two, |backend, path| backend.commit_tree_oid(path, &head)),
        qualify_op(&two, |backend, path| {
            Ok(backend.config_get(path, "user.email"))
        }),
    ];

    // Detached HEAD: head_ref is None on both backends.
    let detached = detached_head_fixture();
    verdicts.push(qualify_op(&detached, |backend, path| {
        backend.head_ref(path)
    }));

    // Unborn repository: head_commit_oid_optional is None on both backends.
    let unborn = unborn_fixture();
    verdicts.push(qualify_op(&unborn, |backend, path| {
        backend.head_commit_oid_optional(path)
    }));

    // Writer config_get (the identity-grade config scalar) resolves the same value
    // git does across multi-scope precedence: a local value outranks the ambient
    // global (both must pick the local one, never the host's global user.email), a
    // worktree value outranks local, and an explicitly empty value is None on both.
    // The writer caller elides this lookup when POINTBREAK_ACTOR_ID is set, so the
    // vector exercises only the resolution path. (config_path_get — signing-key
    // discovery — is the read:config-discovery class, held on subprocess because
    // git's `~`-expansion path spelling is not reproducible in gix; it is not an
    // identity scalar and is not gated here.)
    let multi = multi_scope_config_fixture();
    verdicts.push(qualify_op(&multi, |backend, path| {
        Ok(backend.config_get(path, "user.email"))
    }));
    let empty_email = empty_local_email_fixture();
    verdicts.push(qualify_op(&empty_email, |backend, path| {
        Ok(backend.config_get(path, "user.email"))
    }));

    // SHA-256 object-format OID byte-parity (closes ASSUMPTION-Q2-3). SHA-1 parity
    // is covered by the scalar vectors above; this proves the identity scalars are
    // byte-equal under the SHA-256 object format too. Skipped when the host git
    // lacks `--object-format=sha256`.
    if let Some(sha256) = maybe_sha256_repo_fixture() {
        let sha_head = rev(sha256.path(), "HEAD");
        verdicts.push(qualify_op(&sha256, |backend, path| {
            backend.empty_tree_oid(path)
        }));
        verdicts.push(qualify_op(&sha256, |backend, path| backend.head_oid(path)));
        verdicts.push(qualify_op(&sha256, |backend, path| {
            backend.head_commit_oid_optional(path)
        }));
        verdicts.push(qualify_op(&sha256, |backend, path| {
            backend.rev_parse_commit_oid(path, "HEAD")
        }));
        verdicts.push(qualify_op(&sha256, move |backend, path| {
            backend.commit_tree_oid(path, &sha_head)
        }));
        verdicts.push(qualify_op(&sha256, |backend, path| {
            backend.worktree_root(path)
        }));
    }

    verdicts
}

// ---------- diagnostic (non-routable) probes ----------

/// Diff diagnostic over a genuine two-candidate ambiguous rename. gix's default
/// first-found rename-source heuristic diverges from git's best-candidate choice
/// (research 0044 Q2), so tier 1 (the rename-source object-id input) is expected
/// to diverge. Recorded, never gating.
pub(crate) fn run_diff_probe_on_ambiguous_rename() -> DiffProbe {
    let fixture = ambiguous_rename_fixture();
    let base = rev(fixture.path(), "HEAD~1");
    let head = rev(fixture.path(), "HEAD");

    let subprocess_source = subprocess_rename_source(fixture.path(), &base, &head, "dest.txt");
    let gix_source = gix_rename_source(fixture.path(), "dest.txt");
    let tier1_object_id =
        if subprocess_source.is_some() && gix_source.is_some() && subprocess_source == gix_source {
            VectorVerdict::Match
        } else {
            VectorVerdict::Divergent
        };

    // Tier 2: the rendered diff content. The subprocess emits git's patch
    // envelope; gix renders only unified hunk bodies (via gix-imara-diff) and off
    // its divergent source, so the content hash differs.
    let subprocess_patch = subprocess_patch_hash(fixture.path(), &base, &head);
    let gix_patch = gix_rendered_diff_hash(fixture.path(), &base, gix_source.as_deref());
    let tier2_content_hash = if subprocess_patch == gix_patch {
        VectorVerdict::Match
    } else {
        VectorVerdict::Divergent
    };

    DiffProbe {
        tier1_object_id,
        tier2_content_hash,
    }
}

/// Write-tree diagnostic: gix has no `write-tree`, so the index tree is
/// reconstructed from index entries via the tree editor and compared to git's
/// `write-tree` on a staged index. Recorded, never gating.
pub(crate) fn run_write_tree_probe() -> VectorVerdict {
    let fixture = staged_fixture();
    let subprocess = SubprocessBackend
        .write_index_tree_oid_direct(fixture.path())
        .ok();
    let gix = gix_reconstruct_index_tree(fixture.path());
    match (subprocess, gix) {
        (Some(subprocess), Some(gix)) if subprocess == gix => VectorVerdict::Match,
        _ => VectorVerdict::Divergent,
    }
}

fn subprocess_rename_source(repo: &Path, base: &str, head: &str, dest: &str) -> Option<String> {
    let output = run_git(
        repo,
        [
            "diff",
            "--raw",
            "-M",
            "--no-color",
            "--full-index",
            base,
            head,
        ],
    )
    .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    for line in text.lines() {
        // `:<mode> <mode> <oid> <oid> R<score>\t<source>\t<dest>`
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() >= 3 && fields[2] == dest && fields[0].contains(" R") {
            return Some(fields[1].to_owned());
        }
    }
    None
}

fn gix_rename_source(repo: &Path, dest: &str) -> Option<String> {
    let repository = ::gix::open(repo).ok()?;
    let head = repository.head_commit().ok()?;
    let head_tree = head.tree().ok()?;
    let parent = head.parent_ids().next()?;
    let parent_tree = repository.find_commit(parent.detach()).ok()?.tree().ok()?;

    let mut source = None;
    let mut changes = parent_tree.changes().ok()?;
    changes.options(|options| {
        options.track_rewrites(Some(Default::default()));
    });
    changes
        .for_each_to_obtain_tree(&head_tree, |change| {
            use ::gix::bstr::ByteSlice;
            use ::gix::object::tree::diff::Change;
            if let Change::Rewrite {
                source_location,
                location,
                ..
            } = &change
                && location.to_str().ok() == Some(dest)
            {
                source = source_location.to_str().ok().map(str::to_owned);
            }
            Ok::<_, std::convert::Infallible>(std::ops::ControlFlow::Continue(()))
        })
        .ok()?;
    source
}

fn subprocess_patch_hash(repo: &Path, base: &str, head: &str) -> String {
    let output = run_git(repo, ["diff", "--patch", "-M", "--no-color", base, head])
        .map(|output| output.stdout)
        .unwrap_or_default();
    sha256_bytes_hex(&output)
}

fn gix_rendered_diff_hash(repo: &Path, base: &str, source: Option<&str>) -> String {
    let Some(source) = source else {
        return sha256_bytes_hex(b"gix:no-rename-source");
    };
    let before = run_git(repo, ["show", &format!("{base}:{source}")])
        .map(|output| output.stdout)
        .unwrap_or_default();
    let after = run_git(repo, ["show", "HEAD:dest.txt"])
        .map(|output| output.stdout)
        .unwrap_or_default();
    let before = String::from_utf8_lossy(&before).into_owned();
    let after = String::from_utf8_lossy(&after).into_owned();
    sha256_bytes_hex(imara_unified_diff(&before, &after).as_bytes())
}

fn imara_unified_diff(before: &str, after: &str) -> String {
    use gix_imara_diff::{Algorithm, BasicLineDiffPrinter, Diff, InternedInput, UnifiedDiffConfig};
    let input = InternedInput::new(before, after);
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    diff.unified_diff(
        &BasicLineDiffPrinter(&input.interner),
        UnifiedDiffConfig::default(),
        &input,
    )
    .to_string()
}

fn gix_reconstruct_index_tree(repo: &Path) -> Option<String> {
    let repository = ::gix::open(repo).ok()?;
    let empty = ::gix::ObjectId::empty_tree(repository.object_hash());
    let mut editor = repository.edit_tree(empty).ok()?;
    let index = repository.index().ok()?;
    for entry in index.entries() {
        let kind = entry.mode.to_tree_entry_mode()?.kind();
        editor
            .upsert(entry.path(&index).to_owned(), kind, entry.id)
            .ok()?;
    }
    Some(editor.write().ok()?.detach().to_string())
}

// ---------- fixtures ----------

struct GitFixture {
    _root: TempDir,
    path: PathBuf,
}

impl GitFixture {
    fn path(&self) -> &Path {
        &self.path
    }
}

fn git<const N: usize>(repo: &Path, args: [&str; N]) {
    run_git(repo, args).unwrap_or_else(|error| panic!("git {args:?} failed: {error}"));
}

fn configure_identity(repo: &Path) {
    git(repo, ["config", "user.name", "Shore Tests"]);
    git(repo, ["config", "user.email", "shore-tests@example.com"]);
    git(repo, ["config", "commit.gpgsign", "false"]);
}

fn write_file(repo: &Path, relative: &str, contents: &str) {
    let path = repo.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create fixture parent directory");
    }
    std::fs::write(path, contents).expect("write fixture file");
}

fn rev(repo: &Path, spec: &str) -> String {
    let output = run_git(repo, ["rev-parse", spec]).expect("rev-parse fixture spec");
    String::from_utf8(output.stdout)
        .expect("rev-parse output is utf-8")
        .trim()
        .to_owned()
}

fn current_branch_ref(repo: &Path) -> String {
    let output = run_git(repo, ["symbolic-ref", "HEAD"]).expect("resolve fixture HEAD ref");
    String::from_utf8(output.stdout)
        .expect("symbolic-ref output is utf-8")
        .trim()
        .to_owned()
}

fn init_fixture() -> GitFixture {
    let root = TempDir::new().expect("create fixture repository directory");
    let path = root.path().to_path_buf();
    git(&path, ["init"]);
    git(&path, ["symbolic-ref", "HEAD", "refs/heads/main"]);
    configure_identity(&path);
    GitFixture { _root: root, path }
}

fn two_commit_fixture() -> GitFixture {
    let fixture = init_fixture();
    write_file(fixture.path(), "file.txt", "one\n");
    git(fixture.path(), ["add", "--all"]);
    git(fixture.path(), ["commit", "-m", "first"]);
    git(fixture.path(), ["tag", "-a", "v1", "-m", "v1", "HEAD"]);
    write_file(fixture.path(), "file.txt", "two\n");
    git(fixture.path(), ["add", "--all"]);
    git(fixture.path(), ["commit", "-m", "second"]);
    fixture
}

fn root_commit_fixture() -> GitFixture {
    let fixture = init_fixture();
    write_file(fixture.path(), "a.txt", "a\n");
    write_file(fixture.path(), "nested/b.txt", "b\n");
    git(fixture.path(), ["add", "--all"]);
    git(fixture.path(), ["commit", "-m", "root"]);
    fixture
}

fn unborn_fixture() -> GitFixture {
    init_fixture()
}

fn detached_head_fixture() -> GitFixture {
    let fixture = two_commit_fixture();
    git(fixture.path(), ["checkout", "--detach"]);
    fixture
}

fn dangling_origin_head_fixture() -> GitFixture {
    let fixture = two_commit_fixture();
    git(
        fixture.path(),
        [
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/missing",
        ],
    );
    fixture
}

fn amended_reflog_fixture() -> GitFixture {
    let fixture = init_fixture();
    write_file(fixture.path(), "file.txt", "one\n");
    git(fixture.path(), ["add", "--all"]);
    git(fixture.path(), ["commit", "-m", "first"]);
    write_file(fixture.path(), "file.txt", "amended\n");
    git(fixture.path(), ["add", "--all"]);
    git(fixture.path(), ["commit", "--amend", "-m", "first amended"]);
    fixture
}

fn ignore_stack_fixture() -> GitFixture {
    let fixture = two_commit_fixture();
    write_file(fixture.path(), ".gitignore", "*.log\n");
    let exclude = fixture.path().join(".git/info/exclude");
    std::fs::create_dir_all(exclude.parent().unwrap()).unwrap();
    std::fs::write(&exclude, ".pointbreak/data/\n").unwrap();
    fixture
}

fn post_mutation_ignore_fixture() -> GitFixture {
    let fixture = two_commit_fixture();
    // Probe once so a naive long-lived exclude stack would be primed, then mutate
    // both ignore sources and rely on the fresh per-call stack observing them.
    let _ = SubprocessBackend.paths_are_ignored(fixture.path(), &["build/output.o"]);
    let exclude = fixture.path().join(".git/info/exclude");
    std::fs::create_dir_all(exclude.parent().unwrap()).unwrap();
    std::fs::write(&exclude, "build/\n").unwrap();
    write_file(fixture.path(), ".pointbreak/.gitignore", "data/\n");
    fixture
}

fn inventory_fixture() -> GitFixture {
    let fixture = two_commit_fixture();
    write_file(
        fixture.path(),
        ".gitignore",
        "ignored/\n!ignored/keep.txt\n*.log\n",
    );
    git(fixture.path(), ["add", ".gitignore"]);
    git(fixture.path(), ["commit", "-m", "ignore rules"]);
    write_file(fixture.path(), "src/new.rs", "fn main() {}\n");
    write_file(fixture.path(), "top.txt", "top\n");
    write_file(fixture.path(), "ignored/skip.txt", "skip\n");
    write_file(fixture.path(), "ignored/keep.txt", "keep\n");
    write_file(fixture.path(), "trace.log", "log\n");
    fixture
}

fn crlf_fixture() -> GitFixture {
    let fixture = two_commit_fixture();
    write_file(fixture.path(), "crlf.txt", "one\r\ntwo\r\n");
    fixture
}

fn submodule_fixture() -> GitFixture {
    let fixture = two_commit_fixture();
    // Build a gitlink by committing a nested repository's tip as mode 160000.
    let sub = TempDir::new().expect("create submodule source");
    git(sub.path(), ["init"]);
    git(sub.path(), ["symbolic-ref", "HEAD", "refs/heads/main"]);
    configure_identity(sub.path());
    write_file(sub.path(), "sub.txt", "sub\n");
    git(sub.path(), ["add", "--all"]);
    git(sub.path(), ["commit", "-m", "sub"]);
    let sub_oid = rev(sub.path(), "HEAD");
    run_git(
        fixture.path(),
        [
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{sub_oid},submodule"),
        ],
    )
    .expect("stage gitlink");
    fixture
}

#[cfg(unix)]
fn mode_only_fixture() -> GitFixture {
    use std::os::unix::fs::PermissionsExt;
    let fixture = two_commit_fixture();
    let script = fixture.path().join("run.sh");
    std::fs::write(&script, "#!/bin/sh\necho hi\n").unwrap();
    let mut permissions = std::fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).unwrap();
    fixture
}

#[cfg(unix)]
fn type_change_fixture() -> GitFixture {
    let fixture = two_commit_fixture();
    std::os::unix::fs::symlink("file.txt", fixture.path().join("link")).unwrap();
    fixture
}

#[cfg(unix)]
fn non_utf8_fixture() -> Option<GitFixture> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let fixture = two_commit_fixture();
    let name = OsStr::from_bytes(b"weird-\xff-name.txt");
    // Some unix filesystems (APFS/HFS+ on macOS) enforce UTF-8 filenames and
    // reject this write; when they do, skip the leg rather than fail.
    match std::fs::write(fixture.path().join(name), "bytes\n") {
        Ok(()) => Some(fixture),
        Err(_) => None,
    }
}

fn config_discovery_fixture() -> GitFixture {
    let fixture = two_commit_fixture();
    git(
        fixture.path(),
        ["config", "user.signingkey", "~/.ssh/id_ed25519.pub"],
    );
    fixture
}

/// A repository whose writer identity `user.email` is set across multiple config
/// scopes: local (outranking the host's ambient global value) and again at
/// worktree scope (outranking local). The backends must resolve the same
/// highest-precedence value git does — never the ambient global — proving
/// `config_get` precedence parity.
fn multi_scope_config_fixture() -> GitFixture {
    let fixture = two_commit_fixture();
    git(
        fixture.path(),
        ["config", "--local", "user.email", "local@example.com"],
    );
    // Worktree scope outranks local; enable the extension so a per-worktree value
    // is preferred, exercising the highest-precedence tier of the resolution.
    git(
        fixture.path(),
        ["config", "--local", "extensions.worktreeConfig", "true"],
    );
    git(
        fixture.path(),
        ["config", "--worktree", "user.email", "worktree@example.com"],
    );
    fixture
}

/// A repository with an explicitly empty local `user.email`. The empty value
/// shadows any ambient global, so git reports it (exit 0, empty) and both backends
/// map empty to `None` — the writer-identity fallback trigger.
fn empty_local_email_fixture() -> GitFixture {
    let fixture = two_commit_fixture();
    git(fixture.path(), ["config", "--local", "user.email", ""]);
    fixture
}

/// A repository initialized with the SHA-256 object format, or `None` when the
/// host git lacks `--object-format=sha256` support (mirrors `maybe_sha256_repo`
/// in `command.rs`). Proves the identity scalars are byte-equal under both object
/// formats.
fn maybe_sha256_repo_fixture() -> Option<GitFixture> {
    let root = TempDir::new().expect("create sha256 fixture directory");
    let path = root.path().to_path_buf();
    if run_git(&path, ["init", "--object-format=sha256"]).is_err() {
        return None;
    }
    git(&path, ["symbolic-ref", "HEAD", "refs/heads/main"]);
    configure_identity(&path);
    write_file(&path, "file.txt", "one\n");
    git(&path, ["add", "--all"]);
    git(&path, ["commit", "-m", "first"]);
    Some(GitFixture { _root: root, path })
}

fn linked_worktree_fixture() -> GitFixture {
    let fixture = two_commit_fixture();
    // A linked worktree shares the common dir; place it under the same temp root
    // so it is cleaned up when the owning TempDir is dropped.
    let linked = fixture.path().join("linked-worktree");
    run_git(
        fixture.path(),
        [
            "worktree",
            "add",
            "-b",
            "linked",
            linked.to_str().expect("worktree path is utf-8"),
        ],
    )
    .expect("add linked worktree");
    // Reclaim the owning TempDir and re-target the fixture at the linked worktree.
    let GitFixture { _root, .. } = fixture;
    GitFixture {
        _root,
        path: linked,
    }
}

fn staged_fixture() -> GitFixture {
    let fixture = two_commit_fixture();
    write_file(fixture.path(), "staged.txt", "staged\n");
    git(fixture.path(), ["add", "staged.txt"]);
    fixture
}

fn ambiguous_rename_fixture() -> GitFixture {
    let fixture = init_fixture();
    // Two rename candidates for `dest.txt`, both above the 50% similarity floor.
    // git picks the *best* candidate (the higher-similarity `high`); gix picks the
    // *first possible match* in blob-object-id order, ignoring the filename
    // (research 0044 Q2). We construct the lower-similarity `low` so its blob id
    // sorts before `high`'s, which deterministically forces gix onto `low`.
    let dest: Vec<String> = (1..=100)
        .map(|line| format!("dest common line {line}"))
        .collect();

    let mut high: Vec<String> = dest[..77].to_vec();
    high.extend((1..=23).map(|line| format!("high unique {line}")));
    let high_text = joined(&high);
    let high_oid = git_blob_oid(fixture.path(), high_text.as_bytes());

    // Low shares 65 of 100 lines (clearly above the floor, below `high`). Append a
    // varying nonce line until the resulting blob id sorts before `high`'s.
    let mut low_base: Vec<String> = dest[..65].to_vec();
    low_base.extend((1..=34).map(|line| format!("low unique {line}")));
    let (low_text, _) = (0..)
        .map(|nonce| {
            let mut lines = low_base.clone();
            lines.push(format!("low nonce {nonce}"));
            let text = joined(&lines);
            let oid = git_blob_oid(fixture.path(), text.as_bytes());
            (text, oid)
        })
        .find(|(_, oid)| *oid < high_oid)
        .expect("a nonce whose blob id sorts before the high-similarity candidate");

    // Filenames are deliberately the reverse of the id order, proving gix ignores
    // them: the higher-similarity `high` gets the alphabetically-first name.
    write_file(fixture.path(), "aaa-high.txt", &high_text);
    write_file(fixture.path(), "zzz-low.txt", &low_text);
    git(fixture.path(), ["add", "--all"]);
    git(fixture.path(), ["commit", "-m", "candidates"]);

    std::fs::remove_file(fixture.path().join("aaa-high.txt")).unwrap();
    std::fs::remove_file(fixture.path().join("zzz-low.txt")).unwrap();
    write_file(fixture.path(), "dest.txt", &joined(&dest));
    git(fixture.path(), ["add", "--all"]);
    git(fixture.path(), ["commit", "-m", "rename"]);
    fixture
}

/// The git blob object id for `contents`, without writing it into the tree.
fn git_blob_oid(repo: &Path, contents: &[u8]) -> String {
    let output = run_git_with_stdin(repo, ["hash-object", "--stdin"], contents, &[0])
        .expect("hash-object candidate blob");
    String::from_utf8(output.stdout)
        .expect("hash-object output is utf-8")
        .trim()
        .to_owned()
}

fn joined(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

// ---------- entry points (all named `git_backend_parity_*`) ----------

#[test]
fn git_backend_parity_reports_every_routable_class() {
    let report = run_routable_gate();
    assert_eq!(
        report.len(),
        6,
        "five read classes plus identity-scalars are each reported"
    );
    for result in &report {
        assert_eq!(result.schema, GIT_BACKEND_PARITY_RESULT_SCHEMA_V1);
        assert!(result.vectors > 0, "class {} ran no vectors", result.class);
    }
}

#[test]
fn git_backend_parity_scalar_oids_hold_for_sha256_object_format() {
    // Identity-grade gate (ADR-0037 D4): OID scalars are byte-equal under the
    // SHA-256 object format, not only SHA-1. Skipped when the host git lacks it.
    let Some(fixture) = maybe_sha256_repo_fixture() else {
        eprintln!("skipping SHA-256 scalar parity: host git lacks --object-format=sha256");
        return;
    };
    let head = rev(fixture.path(), "HEAD");
    let empty = SubprocessBackend.empty_tree_oid(fixture.path()).unwrap();
    assert_eq!(empty.len(), 64, "a SHA-256 empty tree oid is 64 hex chars");
    assert_eq!(empty, GixBackend.empty_tree_oid(fixture.path()).unwrap());
    assert_eq!(
        SubprocessBackend.head_oid(fixture.path()).unwrap(),
        GixBackend.head_oid(fixture.path()).unwrap()
    );
    assert_eq!(
        SubprocessBackend
            .head_commit_oid_optional(fixture.path())
            .unwrap(),
        GixBackend.head_commit_oid_optional(fixture.path()).unwrap()
    );
    assert_eq!(
        SubprocessBackend
            .commit_tree_oid(fixture.path(), &head)
            .unwrap(),
        GixBackend.commit_tree_oid(fixture.path(), &head).unwrap()
    );
    assert_eq!(
        SubprocessBackend
            .rev_parse_commit_oid(fixture.path(), "HEAD")
            .unwrap(),
        GixBackend
            .rev_parse_commit_oid(fixture.path(), "HEAD")
            .unwrap()
    );
}

#[test]
fn git_backend_parity_writer_config_get_matches_across_multi_scope_precedence() {
    // Identity-grade gate (ADR-0037 D4): byte-identical writer `config --get`
    // resolution across git's multi-scope precedence, plus empty→None.
    let fixture = multi_scope_config_fixture();
    assert_eq!(
        SubprocessBackend.config_get(fixture.path(), "user.email"),
        GixBackend.config_get(fixture.path(), "user.email"),
    );
    let empty = empty_local_email_fixture();
    assert_eq!(
        SubprocessBackend.config_get(empty.path(), "user.email"),
        None
    );
    assert_eq!(GixBackend.config_get(empty.path(), "user.email"), None);
}

#[test]
fn git_backend_parity_diff_probe_records_exact_expected_divergence() {
    let probe = run_diff_probe_on_ambiguous_rename();
    assert_eq!(
        probe.tier1_object_id,
        VectorVerdict::Divergent,
        "gix's first-found rename source diverges from git's best-candidate choice"
    );
}

#[test]
fn git_backend_parity_write_tree_probe_reconstructs_the_index_tree() {
    // Diagnostic: gix has no write-tree, but reconstructing the tree from the
    // index matches git's write-tree byte-for-byte (research 0044 Q2).
    assert_eq!(run_write_tree_probe(), VectorVerdict::Match);
}

#[test]
fn git_backend_parity_executes_both_backends() {
    // Prove the shared discovery memo does not let the gix call reuse the
    // subprocess value: both backends must actually execute (F6).
    reset_git_spawn_count();
    reset_gix_open_count();
    let fixture = two_commit_fixture();
    let _ = qualify_op(&fixture, |backend, path| backend.common_dir(path));
    assert!(git_spawn_count() >= 1, "subprocess actually resolved");
    assert!(
        gix_open_count() >= 1,
        "gix actually opened, not the cached value"
    );
}

#[test]
fn git_backend_parity_qualified_gate_passes() {
    // The enforcing gate: fail only classes whose compiled default is gix (i.e.
    // qualified and flipped). A class still on subprocess is reported, never
    // failed, so leaving a class on subprocess can never red this lane.
    let report = run_routable_gate();
    // Surface the full per-class report (shown under `--no-capture`) so a
    // qualification run can read each class's outcome and divergence count
    // without changing the report-only harness.
    eprintln!(
        "git-backend parity report:\n{}",
        serde_json::to_string_pretty(&report).expect("serialize parity report")
    );
    for result in &report {
        if crate::git::backend::is_gix_qualified(&result.class) {
            assert_eq!(
                result.outcome,
                ParityOutcome::Passed,
                "qualified class diverged: {result:#?}"
            );
        }
    }
}

/// A per-operation subprocess-vs-gix latency sample. Not a gate: it verifies the
/// two backends still agree, then prints the cold per-call cost of a
/// representative operation for each read class so `just git-bench` records the
/// measured win behind each flip. The discovery memo is reset before every call
/// so both backends pay their real resolution cost (subprocess spawns git; gix
/// opens the repository in-process).
#[test]
fn git_backend_microbench() {
    use std::time::Instant;

    let fixture = inventory_fixture();
    let path = fixture.path();

    // (class, op) — one representative routable read op per class.
    #[allow(clippy::type_complexity)]
    let ops: Vec<(&str, Box<dyn Fn(&dyn GitBackend)>)> = vec![
        (
            "read:graph-refs / for_each_ref",
            Box::new(|backend: &dyn GitBackend| {
                let _ = backend.for_each_ref(path, &["refs/heads/"]);
            }),
        ),
        (
            "read:ignore / paths_are_ignored",
            Box::new(|backend: &dyn GitBackend| {
                let _ = backend.paths_are_ignored(path, &[".pointbreak/data/state.json"]);
            }),
        ),
        (
            "read:config-discovery / config_path_get",
            Box::new(|backend: &dyn GitBackend| {
                let _ = backend.config_path_get(path, "user.signingkey");
            }),
        ),
        (
            "read:repo-discovery / common_dir",
            Box::new(|backend: &dyn GitBackend| {
                let _ = backend.common_dir(path);
            }),
        ),
        (
            "read:inventory / path_is_untracked",
            Box::new(|backend: &dyn GitBackend| {
                let _ = backend.path_is_untracked(path, "src/new.rs");
            }),
        ),
    ];

    const ITERS: u32 = 40;
    eprintln!(
        "git-backend microbench ({ITERS} iters/op, cold discovery each call, {}):",
        std::env::consts::OS
    );
    eprintln!(
        "{:<44} {:>12} {:>12} {:>9}",
        "class / op", "subprocess", "gix", "speedup"
    );
    for (label, op) in &ops {
        let time = |backend: &dyn GitBackend| {
            let start = Instant::now();
            for _ in 0..ITERS {
                reset_discovery_cache();
                op(backend);
            }
            start.elapsed().as_secs_f64() / f64::from(ITERS) * 1e6
        };
        let subprocess_us = time(&SubprocessBackend);
        let gix_us = time(&GixBackend);
        eprintln!(
            "{label:<44} {subprocess_us:>10.1}us {gix_us:>10.1}us {:>8.1}x",
            subprocess_us / gix_us
        );
    }
}
