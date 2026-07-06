//! Promote a clone-local store to the user-level family tier (`shore store link`)
//! and detach it (`shore store unlink`). Link relocates the authoritative write
//! store to `<shore-home-root>/stores/<slug>/`: all gates fire before any family
//! write, and the local binding flip is the last step (the point of no return), so
//! a mid-link crash leaves the clone still resolving clone-local.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::canonical_hash::sha256_bytes_hex;
use crate::error::{Result, ShoreError};
use crate::session::event::EventType;
use crate::session::store::bundle::{
    import_store_bundle_with_verification, verify_source_subset_of_target,
};
use crate::session::store::resolution::clone_local_store_dir;
use crate::session::store::sensitivity::scan_worktree_sensitivity;
use crate::session::store::store_config::{
    StoreMode, clear_family_binding_for_repo, resolve_family_binding, resolve_store_mode,
    set_family_binding_for_repo,
};
use crate::session::store::store_init::ShoreStorePaths;
use crate::session::store::user_level::{
    deregister_clone, ensure_family_store_scaffold, flag_unsupported_filesystem,
    read_family_manifest, register_clone, user_level_store_dir, validate_family_slug,
};
use crate::session::{EventStore, EventVerificationPolicy, TrustSet};

/// The sentinel `scan_worktree_sensitivity` emits for a worktree that must not be
/// fanned into a family store without an explicit override (mirrors migrate).
const SENSITIVITY_BLOCK: &str = "block";

#[derive(Clone, Debug)]
pub struct StoreLinkOptions {
    repo: PathBuf,
    slug: Option<String>,
    include_ephemeral: bool,
    include_sensitive: bool,
    retire_source: bool,
    trust_set: TrustSet,
}

impl StoreLinkOptions {
    pub fn new(repo: impl AsRef<Path>, slug: Option<String>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            slug,
            include_ephemeral: false,
            include_sensitive: false,
            retire_source: false,
            trust_set: TrustSet::default(),
        }
    }

    pub fn with_include_ephemeral(mut self, include_ephemeral: bool) -> Self {
        self.include_ephemeral = include_ephemeral;
        self
    }

    pub fn with_include_sensitive(mut self, include_sensitive: bool) -> Self {
        self.include_sensitive = include_sensitive;
        self
    }

    pub fn with_retire_source(mut self, retire_source: bool) -> Self {
        self.retire_source = retire_source;
        self
    }

    /// The reader's trust set, threaded from the CLI (mirrors compact), so the
    /// fold's advisory verification can resolve signatures.
    pub fn with_trust_set(mut self, trust_set: TrustSet) -> Self {
        self.trust_set = trust_set;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreLinkResult {
    pub family_ref: String,
    pub clone_ref: String,
    /// True when this link created the family (a new `family.json` was written).
    pub created_family: bool,
    pub folded_events_created: usize,
    pub folded_events_existing: usize,
    pub folded_artifacts_created: usize,
    /// The source's unsigned `ArtifactRemoved` events — the possession-stripping
    /// population the fold restamps. The CLI prints the re-issue disclosure when this
    /// is > 0. Populated by the fold; 0 for an empty source.
    pub folded_removal_event_count: usize,
    pub source_retired: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesystem_warning: Option<String>,
    /// The advisory history-overlap warning — set when joining an existing family
    /// whose recorded founding anchors share no root-commit OID with this clone.
    /// Never blocks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_overlap_warning: Option<String>,
}

pub fn link_store_to_family(options: StoreLinkOptions) -> Result<StoreLinkResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root().to_path_buf();

    let slug = match &options.slug {
        Some(slug) => {
            validate_family_slug(slug)?;
            slug.clone()
        }
        None => return Err(no_slug_error(&worktree_root)),
    };

    // Gate order — every gate fires BEFORE any family write.
    // (1) Ephemeral-worktree refusal.
    if !options.include_ephemeral && resolve_store_mode(&worktree_root)? == StoreMode::Ephemeral {
        return Err(ShoreError::Message(
            "refusing to link an ephemeral worktree into a family store; re-run with the \
             include-ephemeral override to link it anyway"
                .to_owned(),
        ));
    }
    // (2) Sensitivity block refusal.
    if !options.include_sensitive {
        let scan = scan_worktree_sensitivity(&worktree_root)?;
        if scan.policy_outcome == SENSITIVITY_BLOCK {
            return Err(ShoreError::Message(
                "refusing to link a worktree flagged sensitive into a family store; add \
                 known-safe paths to .shore/sensitivity.json excludeGlobs for a targeted \
                 exclude, or re-run with the include-sensitive override to link it anyway"
                    .to_owned(),
            ));
        }
    }
    // (3) Family-stamp mismatch refusal (no override).
    let family_dir = user_level_store_dir(&slug)?;
    if let Some(manifest) = read_family_manifest(&family_dir)?
        && manifest.family_id != slug
    {
        return Err(ShoreError::Message(format!(
            "family store {} is stamped for family `{}`, not `{}`; refusing to link",
            family_dir.display(),
            manifest.family_id,
            slug
        )));
    }
    // (4) Filesystem heuristic → warning only (never blocks).
    let filesystem_warning = flag_unsupported_filesystem(&family_dir);
    // (5) Advisory history-overlap → warning only (never blocks). Compared against
    // the FOUNDING clone's anchors recorded in family.json; a fresh family (no
    // manifest yet) or an anchorless set skips the advisory.
    let root_oids = root_commit_oids(&worktree_root)?;
    let history_overlap_warning = history_overlap_warning_for(&family_dir, &slug, &root_oids)?;

    // Preparation (all reversible until the binding flip):
    let clone_ref = mint_clone_ref(&worktree_root);
    let created_family = ensure_family_store_scaffold(&family_dir, &slug, &root_oids)?;

    // Fold the clone-local store forward. The verified fold + removal count +
    // retire-after-verify body lives in `fold_source_forward`.
    let source = clone_local_store_dir(&worktree_root)?;
    let fold = fold_source_forward(
        &source,
        &family_dir,
        &options.trust_set,
        options.retire_source,
    )?;

    register_clone(&family_dir, &slug, &clone_ref, &worktree_root)?;
    // The binding flip is LAST — the point of no return.
    set_family_binding_for_repo(&options.repo, &slug, &clone_ref)?;

    Ok(StoreLinkResult {
        family_ref: slug,
        clone_ref,
        created_family,
        folded_events_created: fold.events_created,
        folded_events_existing: fold.events_existing,
        folded_artifacts_created: fold.artifacts_created,
        folded_removal_event_count: fold.removal_event_count,
        source_retired: fold.source_retired,
        filesystem_warning,
        history_overlap_warning,
    })
}

#[derive(Clone, Debug)]
pub struct StoreUnlinkOptions {
    repo: PathBuf,
}

impl StoreUnlinkOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreUnlinkResult {
    /// The family this clone was detached from; `None` when it was not linked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_family_ref: Option<String>,
    /// Whether a registry entry was removed (false for a not-linked or
    /// already-forgotten family).
    pub deregistered: bool,
}

/// Detach this clone from its family store: read the binding, deregister the clone
/// best-effort, then clear the binding. Moves no data (detach-only). A
/// missing/dangling family dir must NOT fail the unlink — the user may be escaping a
/// `forget`/`rm -rf`, so deregistration is best-effort and the binding is cleared
/// regardless.
pub fn unlink_store_from_family(options: StoreUnlinkOptions) -> Result<StoreUnlinkResult> {
    let worktree_root = ShoreStorePaths::resolve(&options.repo)?
        .worktree_root()
        .to_path_buf();

    // Read the binding BEFORE clearing so we know which family to deregister from.
    let Some(binding) = resolve_family_binding(&worktree_root)? else {
        return Ok(StoreUnlinkResult {
            previous_family_ref: None,
            deregistered: false,
        });
    };

    // Best-effort deregister: `deregister_clone` reads the family registry and is a
    // clean `false` no-op when the family dir is gone.
    let family_dir = user_level_store_dir(&binding.family_ref)?;
    let deregistered = deregister_clone(&family_dir, &binding.clone_ref)?;

    clear_family_binding_for_repo(&options.repo)?;

    Ok(StoreUnlinkResult {
        previous_family_ref: Some(binding.family_ref),
        deregistered,
    })
}

/// This clone's root-commit anchors: `git rev-list --max-parents=0 HEAD`, one OID
/// per line (a repo can have several roots). Best-effort: a repo with no commits yet
/// has no HEAD — treat that as an empty anchor set (the advisory is skipped), never
/// an error.
fn root_commit_oids(worktree_root: &Path) -> Result<Vec<String>> {
    let output = std::process::Command::new("git")
        .args(["rev-list", "--max-parents=0", "HEAD"])
        .current_dir(worktree_root)
        .output()
        .map_err(|error| ShoreError::Message(format!("run git rev-list: {error}")))?;
    if !output.status.success() {
        // No HEAD yet (empty repo) or similar: best-effort empty anchor set.
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect())
}

/// Warn when joining an EXISTING family whose founding anchors are non-empty, this
/// clone's anchors are non-empty, and the two sets are disjoint. A fresh family (no
/// manifest), an anchorless family, or an anchorless clone all skip the advisory
/// (best-effort). Never blocks.
fn history_overlap_warning_for(
    family_dir: &Path,
    slug: &str,
    clone_roots: &[String],
) -> Result<Option<String>> {
    let Some(manifest) = read_family_manifest(family_dir)? else {
        return Ok(None); // fresh family — this clone founds it
    };
    if manifest.root_commit_oids.is_empty() || clone_roots.is_empty() {
        return Ok(None);
    }
    let overlaps = clone_roots
        .iter()
        .any(|oid| manifest.root_commit_oids.contains(oid));
    Ok((!overlaps).then(|| {
        format!(
            "this clone shares no git history with family `{slug}` (no common root \
             commit); if this is a different project, unlink and choose another slug"
        )
    }))
}

/// Opaque, deterministic clone id. The normalized worktree root is the git toplevel
/// absolute path (`ShoreStorePaths::resolve`); raw paths never reach the wire, but
/// this 16-hex digest of one does.
fn mint_clone_ref(worktree_root: &Path) -> String {
    let digest = sha256_bytes_hex(worktree_root.to_string_lossy().as_bytes());
    digest[..16].to_owned()
}

fn no_slug_error(worktree_root: &Path) -> ShoreError {
    match suggest_family_slug(worktree_root) {
        Some(suggestion) => ShoreError::Message(format!(
            "no family slug given; re-run as `shore store link <slug>` (suggested: `{suggestion}`)"
        )),
        None => ShoreError::Message(
            "no family slug given and none could be suggested from the worktree name; \
             re-run as `shore store link <slug>`"
                .to_owned(),
        ),
    }
}

/// A link-time suggestion only (never the key — the human confirms it): slugify the
/// worktree directory basename. A remote-name suggestion is a documented future
/// augment; V1 has no cheap remote-URL git helper, so basename-only.
fn suggest_family_slug(worktree_root: &Path) -> Option<String> {
    let base = worktree_root
        .file_name()?
        .to_string_lossy()
        .to_ascii_lowercase();
    let slug: String = base
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    let slug = slug.trim_matches('-').to_owned();
    (!slug.is_empty()).then_some(slug)
}

/// Counts a fold produces.
struct FoldOutcome {
    events_created: usize,
    events_existing: usize,
    artifacts_created: usize,
    removal_event_count: usize,
    source_retired: bool,
}

impl FoldOutcome {
    fn empty() -> Self {
        Self {
            events_created: 0,
            events_existing: 0,
            artifacts_created: 0,
            removal_event_count: 0,
            source_retired: false,
        }
    }
}

/// Fold the clone-local store forward into the family store: verify-and-import
/// (advisory policy — reported, never blocking), count the source's unsigned
/// `ArtifactRemoved` events (the possession-stripping population the fold restamps),
/// and — under `--retire-source` — delete the source only after
/// `verify_source_subset_of_target` passes. An absent/empty source is a clean no-op.
fn fold_source_forward(
    source: &Path,
    family_dir: &Path,
    trust_set: &TrustSet,
    retire_source: bool,
) -> Result<FoldOutcome> {
    // An absent/empty clone-local store is a clean no-op.
    if !source.join("events").exists() {
        return Ok(FoldOutcome::empty());
    }

    // Count the possession-stripping population BEFORE the fold restamps events with
    // BundleApply provenance: a prior UNSIGNED ArtifactRemoved loses operative
    // suppression in the family store. The CLI discloses the "re-issue `shore store
    // remove` natively" guidance when this is > 0.
    let removal_event_count = EventStore::open(source)
        .list_events()?
        .iter()
        .filter(|event| event.event_type == EventType::ArtifactRemoved && event.signature.is_none())
        .count();

    // Verified fold — advisory verification is reported, never blocking; the trust
    // set is threaded from the CLI (mirrors compact).
    let imported = import_store_bundle_with_verification(
        source,
        family_dir,
        EventVerificationPolicy::advisory(),
        trust_set.clone(),
    )?;

    let mut outcome = FoldOutcome {
        events_created: imported.events_created,
        events_existing: imported.events_existing,
        artifacts_created: imported.artifacts_created,
        removal_event_count,
        source_retired: false,
    };

    if retire_source {
        // Delete the clone-local store only after an independent subset
        // re-verification confirms every durable source file in the family store
        // (mirrors `store migrate`'s retire flow exactly).
        verify_source_subset_of_target(source, family_dir)?;
        std::fs::remove_dir_all(source).map_err(|error| {
            ShoreError::Message(format!(
                "remove retired source store {}: {error}",
                source.display()
            ))
        })?;
        outcome.source_retired = true;
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::session::store::store_config::{
        StoreMode, resolve_family_binding, write_store_config,
    };
    use crate::session::store::user_level::{user_level_store_dir, write_family_manifest};

    #[test]
    fn fresh_link_writes_binding_registry_and_scaffold() {
        let repo = git_repo();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let (result, family_dir) = with_shore_home(&home, || {
            let result =
                link_store_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())));
            let family_dir = user_level_store_dir("fam").unwrap();
            (result, family_dir)
        });
        let result = result.unwrap();

        assert!(result.created_family, "a fresh family is created");
        assert_eq!(result.family_ref, "fam");
        assert_eq!(result.clone_ref.len(), 16, "clone_ref is 16 hex chars");

        // Scaffold landed.
        assert!(family_dir.join("family.json").is_file());
        assert!(family_dir.join("events").is_dir());

        // Registry lists this clone.
        let registry =
            crate::session::store::user_level::read_family_registry(&family_dir).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(registry.entries[0].clone_ref, result.clone_ref);

        // Binding flipped last — the clone now resolves the family.
        let binding = resolve_family_binding(repo.path()).unwrap().expect("bound");
        assert_eq!(binding.family_ref, "fam");
        assert_eq!(binding.clone_ref, result.clone_ref);
    }

    #[test]
    fn ephemeral_worktree_refuses_without_override_and_links_with_it() {
        let repo = git_repo();
        write_store_config(repo.path(), StoreMode::Ephemeral).unwrap();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let refused = with_shore_home(&home, || {
            link_store_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
        })
        .expect_err("an ephemeral worktree refuses without the override");
        assert!(refused.to_string().contains("ephemeral"));

        let (linked, resolved, family_dir) = with_shore_home(&home, || {
            let linked = link_store_to_family(
                StoreLinkOptions::new(repo.path(), Some("fam".to_owned()))
                    .with_include_ephemeral(true),
            )
            .unwrap();
            let resolved = crate::session::store::resolution::resolve_store(repo.path())
                .unwrap()
                .store_dir()
                .to_path_buf();
            (linked, resolved, user_level_store_dir("fam").unwrap())
        });
        assert!(linked.created_family);
        // The override promotes: the clone now resolves the family store, not the
        // discardable `.shore/data` (committed ephemeral is overridden by the local
        // binding write).
        assert_eq!(resolved, family_dir);
    }

    #[test]
    fn linking_a_local_ephemeral_override_promotes_to_the_family_store() {
        // Regression: a LOCAL ephemeral pin (the documented private override in
        // `.shore/store.local.json`, not the committed one). `link --include-ephemeral`
        // must clear the pin so the binding takes effect — otherwise the link reports
        // success while the clone keeps resolving `.shore/data`.
        let repo = git_repo();
        std::fs::create_dir_all(repo.path().join(".shore")).unwrap();
        std::fs::write(
            repo.path().join(".shore/store.local.json"),
            r#"{"schema":"shore.store-config","version":1,"mode":"ephemeral"}"#,
        )
        .unwrap();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let (resolved, family_dir) = with_shore_home(&home, || {
            link_store_to_family(
                StoreLinkOptions::new(repo.path(), Some("fam".to_owned()))
                    .with_include_ephemeral(true),
            )
            .expect("link succeeds against a local ephemeral override");
            let resolved = crate::session::store::resolution::resolve_store(repo.path())
                .unwrap()
                .store_dir()
                .to_path_buf();
            (resolved, user_level_store_dir("fam").unwrap())
        });
        assert_eq!(
            resolved, family_dir,
            "the clone resolves the family store, not .shore/data"
        );
    }

    #[test]
    fn sensitivity_block_refuses_without_override_and_links_with_it() {
        let repo = git_repo();
        // A private-key marker file blocks the sensitivity gate. Untracked is
        // inventoried and scanned.
        std::fs::create_dir_all(repo.path().join("keys")).unwrap();
        std::fs::write(
            repo.path().join("keys/dev.pem"),
            "-----BEGIN PRIVATE KEY-----\nredacted\n",
        )
        .unwrap();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let refused = with_shore_home(&home, || {
            link_store_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
        })
        .expect_err("a sensitive worktree refuses without the override");
        let message = refused.to_string();
        assert!(
            message.contains("sensitivity.json"),
            "names the exclude fix: {message}"
        );

        let linked = with_shore_home(&home, || {
            link_store_to_family(
                StoreLinkOptions::new(repo.path(), Some("fam".to_owned()))
                    .with_include_sensitive(true),
            )
        })
        .unwrap();
        assert!(linked.created_family);
    }

    #[test]
    fn a_family_stamp_mismatch_refuses_with_no_override() {
        let repo = git_repo();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let error = with_shore_home(&home, || {
            // Pre-stamp the `fam` directory for a DIFFERENT family (tamper/corruption).
            let family_dir = user_level_store_dir("fam").unwrap();
            std::fs::create_dir_all(&family_dir).unwrap();
            write_family_manifest(&family_dir, "other", &[]).unwrap();
            link_store_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
        })
        .expect_err("a family-stamp mismatch refuses");
        assert!(
            error.to_string().contains("other"),
            "names the stamped family"
        );
    }

    #[test]
    fn a_missing_slug_errors_with_a_suggestion() {
        let repo = git_repo();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let error = with_shore_home(&home, || {
            link_store_to_family(StoreLinkOptions::new(repo.path(), None))
        })
        .expect_err("no slug is an actionable error");
        let message = error.to_string();
        assert!(
            message.contains("shore store link"),
            "names the command: {message}"
        );
        assert!(
            message.contains("suggested"),
            "carries a suggestion: {message}"
        );
    }

    #[test]
    fn joining_an_unrelated_family_yields_a_history_overlap_warning() {
        // Founder: repo A creates the family; its root-commit anchors land in
        // family.json. Joiner: repo B (independent init + commit — a different root
        // OID) links the same slug. Stamp matches (same slug), so the only signal is
        // the advisory: warn, never block.
        let founder = git_repo();
        let joiner = git_repo();
        let home = founder.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let founded = with_shore_home(&home, || {
            link_store_to_family(StoreLinkOptions::new(
                founder.path(),
                Some("fam".to_owned()),
            ))
        })
        .unwrap();
        assert!(
            founded.history_overlap_warning.is_none(),
            "the founder never warns"
        );

        let joined = with_shore_home(&home, || {
            link_store_to_family(StoreLinkOptions::new(joiner.path(), Some("fam".to_owned())))
        })
        .unwrap();
        assert!(
            joined.history_overlap_warning.is_some(),
            "an unrelated clone joining an existing family warns"
        );
        assert!(!joined.created_family, "it joined; it did not found");
    }

    #[test]
    fn a_true_clone_joins_its_family_without_a_history_warning() {
        // The one real second-clone fixture this phase uses: `git clone` shares the
        // founder's root OID, so the advisory stays quiet.
        let founder = git_repo();
        let clone_parent = tempfile::tempdir().unwrap();
        let clone_dir = clone_parent.path().join("clone-b");
        let status = Command::new("git")
            .args([
                OsStr::new("clone"),
                founder.path().as_os_str(),
                clone_dir.as_os_str(),
            ])
            .status()
            .unwrap();
        assert!(status.success());
        let home = founder.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        with_shore_home(&home, || {
            link_store_to_family(StoreLinkOptions::new(
                founder.path(),
                Some("fam".to_owned()),
            ))
        })
        .unwrap();
        let joined = with_shore_home(&home, || {
            link_store_to_family(StoreLinkOptions::new(&clone_dir, Some("fam".to_owned())))
        })
        .unwrap();
        assert!(
            joined.history_overlap_warning.is_none(),
            "a real clone shares the root commit — no warning"
        );
    }

    #[test]
    fn a_sync_managed_store_root_yields_a_filesystem_warning() {
        let repo = git_repo();
        // SHORE_HOME under a Dropbox-shaped path: the fs heuristic warns but never
        // blocks.
        let home = repo.path().join("Dropbox");
        std::fs::create_dir_all(&home).unwrap();

        let result = with_shore_home(&home, || {
            link_store_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
        })
        .unwrap();
        assert!(
            result.filesystem_warning.is_some(),
            "sync-managed root warns"
        );
        assert!(result.created_family, "the warning does not block the link");
    }

    #[test]
    fn link_folds_existing_clone_history() {
        let repo = modified_git_repo();
        crate::session::capture_worktree_review(crate::session::CaptureOptions::new(repo.path()))
            .unwrap();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let (result, family_dir) = with_shore_home(&home, || {
            let result =
                link_store_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())));
            let family_dir = user_level_store_dir("fam").unwrap();
            (result, family_dir)
        });
        let result = result.unwrap();

        assert!(
            result.folded_events_created >= 1,
            "the capture history folds forward"
        );
        let family_events = crate::session::EventStore::open(&family_dir)
            .list_events()
            .unwrap();
        assert!(
            !family_events.is_empty(),
            "the family store now lists the folded events"
        );
    }

    #[test]
    fn an_unsigned_artifact_removed_in_the_source_is_counted() {
        let repo = modified_git_repo();
        // Plant an unsigned ArtifactRemoved directly in the clone-local store.
        let source = crate::session::store::resolution::clone_local_store_dir(repo.path()).unwrap();
        crate::session::EventStore::open(&source)
            .record_event_once(&unsigned_removal_event())
            .unwrap();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let result = with_shore_home(&home, || {
            link_store_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
        })
        .unwrap();

        assert_eq!(
            result.folded_removal_event_count, 1,
            "the unsigned removal is disclosed"
        );
    }

    #[test]
    fn retire_source_deletes_the_clone_store_after_a_verified_fold() {
        let repo = modified_git_repo();
        crate::session::capture_worktree_review(crate::session::CaptureOptions::new(repo.path()))
            .unwrap();
        let source = crate::session::store::resolution::clone_local_store_dir(repo.path()).unwrap();
        assert!(
            source.join("events").is_dir(),
            "the clone-local store is populated"
        );
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let result = with_shore_home(&home, || {
            link_store_to_family(
                StoreLinkOptions::new(repo.path(), Some("fam".to_owned())).with_retire_source(true),
            )
        })
        .unwrap();

        assert!(result.source_retired);
        assert!(
            !source.exists(),
            "the clone-local store is retired only after verification"
        );
    }

    #[test]
    fn an_empty_source_folds_as_a_clean_no_op() {
        let repo = git_repo(); // no capture → clone-local store has no events
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let result = with_shore_home(&home, || {
            link_store_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
        })
        .unwrap();

        assert_eq!(result.folded_events_created, 0);
        assert_eq!(result.folded_removal_event_count, 0);
        assert!(
            result.created_family,
            "the family is still created with no history to fold"
        );
    }

    #[test]
    fn a_linked_clone_unlinks_and_leaves_the_family_store_intact() {
        let repo = git_repo();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let family_dir = with_shore_home(&home, || {
            link_store_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
                .unwrap();
            user_level_store_dir("fam").unwrap()
        });

        let result = with_shore_home(&home, || {
            unlink_store_from_family(StoreUnlinkOptions::new(repo.path()))
        })
        .unwrap();

        assert_eq!(result.previous_family_ref.as_deref(), Some("fam"));
        assert!(result.deregistered);
        // Binding gone; registry entry gone; the family store + its files untouched.
        assert!(resolve_family_binding(repo.path()).unwrap().is_none());
        let registry =
            crate::session::store::user_level::read_family_registry(&family_dir).unwrap();
        assert!(registry.entries.is_empty());
        assert!(
            family_dir.join("family.json").is_file(),
            "detach moves no data"
        );
        assert!(family_dir.join("events").is_dir());
    }

    #[test]
    fn unlink_when_not_linked_is_a_no_op() {
        let repo = git_repo();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let result = with_shore_home(&home, || {
            unlink_store_from_family(StoreUnlinkOptions::new(repo.path()))
        })
        .unwrap();

        assert!(result.previous_family_ref.is_none());
        assert!(!result.deregistered);
    }

    #[test]
    fn unlink_with_a_forgotten_family_dir_still_clears_the_binding() {
        let repo = git_repo();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let family_dir = with_shore_home(&home, || {
            link_store_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
                .unwrap();
            user_level_store_dir("fam").unwrap()
        });
        // Simulate a `forget` / rm -rf of the whole family store.
        std::fs::remove_dir_all(&family_dir).unwrap();

        let result = with_shore_home(&home, || {
            unlink_store_from_family(StoreUnlinkOptions::new(repo.path()))
        })
        .unwrap();

        assert_eq!(result.previous_family_ref.as_deref(), Some("fam"));
        assert!(
            !result.deregistered,
            "nothing to deregister from a forgotten family"
        );
        assert!(
            resolve_family_binding(repo.path()).unwrap().is_none(),
            "the binding is cleared despite the dangling family dir"
        );
    }

    fn modified_git_repo() -> tempfile::TempDir {
        let repo = git_repo();
        // An uncommitted modification gives `capture_worktree_review` a diff to record.
        std::fs::write(repo.path().join("README.md"), "changed\n").unwrap();
        repo
    }

    fn unsigned_removal_event() -> crate::session::event::ShoreEvent {
        use crate::model::JournalId;
        use crate::session::event::{
            ArtifactRemovedPayload, EventTarget, EventType, ShoreEvent, Writer,
        };
        ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key("sha256:deadbeef"),
            EventTarget::for_journal(JournalId::new("journal:test")),
            Writer::shore_local("0.1.0"),
            ArtifactRemovedPayload {
                content_hash: "sha256:deadbeef".to_owned(),
            },
            "2026-06-19T00:00:00Z",
        )
        .expect("removal event builds")
    }

    /// Set `SHORE_HOME` for the duration of `f`. nextest's process-per-test keeps the
    /// mutation contained (the `keys/home.rs` seam). SAFETY: single-threaded test
    /// process.
    fn with_shore_home<T>(home: &Path, f: impl FnOnce() -> T) -> T {
        unsafe {
            std::env::set_var("SHORE_HOME", home);
        }
        let out = f();
        unsafe {
            std::env::remove_var("SHORE_HOME");
        }
        out
    }

    fn git_repo() -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("temp git repo");
        run_git(repo.path(), ["init"]);
        run_git(repo.path(), ["config", "user.name", "Shore Tests"]);
        run_git(
            repo.path(),
            ["config", "user.email", "shore-tests@example.com"],
        );
        run_git(repo.path(), ["config", "commit.gpgsign", "false"]);
        // Seed unique content per repo so two independent `git_repo()` calls yield
        // distinct root-commit OIDs (identical content + author + same-second commit
        // would otherwise collide to one OID). A `git clone` of this repo still
        // shares its root, so the true-clone case stays quiet on the advisory.
        std::fs::write(
            repo.path().join("README.md"),
            format!("base {}\n", repo.path().display()),
        )
        .unwrap();
        run_git(repo.path(), ["add", "--all"]);
        run_git(repo.path(), ["commit", "-m", "base"]);
        repo
    }

    fn run_git<I, S>(cwd: &Path, args: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(output.status.success(), "git failed: {output:?}");
    }
}
