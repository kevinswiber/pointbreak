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
    ExportFidelityStatus, import_store_bundle_with_verification, preview_import_store_bundle,
    verify_source_subset_of_target,
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
    /// Referenced artifacts absent from the source with no removal claim to explain
    /// them (old snapshots GC'd/migrated away). The fold carried the referencing
    /// events without their content; the CLI discloses this when > 0.
    pub folded_absent_artifact_count: usize,
    pub source_retired: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesystem_warning: Option<String>,
    /// The advisory history-overlap warning — set when joining an existing family
    /// whose recorded founding anchors share no root-commit OID with this clone.
    /// Never blocks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_overlap_warning: Option<String>,
}

/// The dry-run report for `shore store link --dry-run`: what the link WOULD do,
/// with zero writes. A blocking gate (1–3) or a fold-preflight event conflict
/// surfaces as `Err` before this is built, so a `StoreLinkPreview` always describes
/// a link that would succeed. An incomplete source (absent artifacts) does NOT
/// block — it is tolerated and reported via `export_fidelity` / `folded_absent_artifact_count`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreLinkPreview {
    pub family_ref: String,
    pub clone_ref: String,
    /// The link would found a new family (no `family.json` yet).
    pub would_create_family: bool,
    /// The clone-local source store has events to fold.
    pub source_present: bool,
    /// `"full"` when every referenced artifact is present or covered by an
    /// `ArtifactRemoved` claim; `"incomplete"` when some are absent with no claim —
    /// the fold tolerates those and discloses `folded_absent_artifact_count`.
    pub export_fidelity: String,
    pub folded_events_to_create: usize,
    pub folded_events_existing: usize,
    pub folded_artifacts_to_create: usize,
    pub folded_artifacts_existing: usize,
    pub folded_removal_event_count: usize,
    /// Referenced artifacts absent from the source with no removal claim — the fold
    /// would carry the referencing events without their content. Advisory, never
    /// blocking; `export_fidelity` reads `incomplete` when this is > 0.
    pub folded_absent_artifact_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesystem_warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_overlap_warning: Option<String>,
}

/// Everything `link` computes before it writes: the gate outcomes (gates 1–5), the
/// minted clone ref, and the founding root-commit anchors. Gates 1–3 refuse via
/// `Err` here; gates 4–5 are advisory warnings carried in the plan. Shared by the
/// real link and its dry-run preview so the gate ladder is spelled exactly once.
struct LinkPlan {
    slug: String,
    family_dir: PathBuf,
    worktree_root: PathBuf,
    clone_ref: String,
    root_oids: Vec<String>,
    /// True when no `family.json` exists yet — the link would found the family.
    would_create_family: bool,
    filesystem_warning: Option<String>,
    history_overlap_warning: Option<String>,
}

/// Run gates 1–5 and mint the clone ref, performing no writes. The blocking gates
/// (1–3) short-circuit with the actionable refusal; the advisory gates (4–5) carry
/// their warnings forward. Both `link_store_to_family` and `preview_link_to_family`
/// enter through here.
fn plan_link(options: &StoreLinkOptions) -> Result<LinkPlan> {
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
                "refusing to link a worktree flagged sensitive into a family store; run \
                 `shore store status --show-paths` to see which files matched, then add \
                 known-safe paths to .shore/sensitivity.json excludeGlobs for a targeted \
                 exclude, or re-run with the include-sensitive override to link it anyway"
                    .to_owned(),
            ));
        }
    }
    // (3) Family-stamp mismatch refusal (no override).
    let family_dir = user_level_store_dir(&slug)?;
    let existing_manifest = read_family_manifest(&family_dir)?;
    if let Some(manifest) = &existing_manifest
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

    let clone_ref = mint_clone_ref(&crate::git::git_common_dir(&worktree_root)?);

    Ok(LinkPlan {
        would_create_family: existing_manifest.is_none(),
        slug,
        family_dir,
        worktree_root,
        clone_ref,
        root_oids,
        filesystem_warning,
        history_overlap_warning,
    })
}

pub fn link_store_to_family(options: StoreLinkOptions) -> Result<StoreLinkResult> {
    let plan = plan_link(&options)?;

    // Preparation (all reversible until the binding flip):
    let created_family =
        ensure_family_store_scaffold(&plan.family_dir, &plan.slug, &plan.root_oids)?;

    // Fold the clone-local store forward. The verified fold + removal count +
    // retire-after-verify body lives in `fold_source_forward`.
    let source = clone_local_store_dir(&plan.worktree_root)?;
    let fold = fold_source_forward(
        &source,
        &plan.family_dir,
        &options.trust_set,
        options.retire_source,
    )?;

    register_clone(
        &plan.family_dir,
        &plan.slug,
        &plan.clone_ref,
        &plan.worktree_root,
    )?;
    // The binding flip is LAST — the point of no return.
    set_family_binding_for_repo(&options.repo, &plan.slug, &plan.clone_ref)?;

    Ok(StoreLinkResult {
        family_ref: plan.slug,
        clone_ref: plan.clone_ref,
        created_family,
        folded_events_created: fold.events_created,
        folded_events_existing: fold.events_existing,
        folded_artifacts_created: fold.artifacts_created,
        folded_removal_event_count: fold.removal_event_count,
        folded_absent_artifact_count: fold.absent_artifact_count,
        source_retired: fold.source_retired,
        filesystem_warning: plan.filesystem_warning,
        history_overlap_warning: plan.history_overlap_warning,
    })
}

/// Dry-run of `link_store_to_family`: run the shared gate ladder and the fold
/// preflight, report what the link WOULD do, and write nothing. Blocking gates and
/// the fold preflight (fidelity / event conflict) surface as `Err` with the same
/// messages the real link produces; a clean path returns the preview.
pub fn preview_link_to_family(options: StoreLinkOptions) -> Result<StoreLinkPreview> {
    let plan = plan_link(&options)?;
    let source = clone_local_store_dir(&plan.worktree_root)?;
    let fold = preview_fold(&source, &plan.family_dir)?;

    Ok(StoreLinkPreview {
        family_ref: plan.slug,
        clone_ref: plan.clone_ref,
        would_create_family: plan.would_create_family,
        source_present: fold.source_present,
        export_fidelity: fold.export_fidelity,
        folded_events_to_create: fold.events_to_create,
        folded_events_existing: fold.events_existing,
        folded_artifacts_to_create: fold.artifacts_to_create,
        folded_artifacts_existing: fold.artifacts_existing,
        folded_removal_event_count: fold.removal_event_count,
        folded_absent_artifact_count: fold.absent_artifact_count,
        filesystem_warning: plan.filesystem_warning,
        history_overlap_warning: plan.history_overlap_warning,
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

/// Opaque, deterministic clone id — keyed on the git COMMON DIR, so every worktree of
/// one physical clone shares it (one physical clone = one registry member). Raw paths
/// never reach the wire, but this 16-hex digest of one does.
fn mint_clone_ref(common_dir: &Path) -> String {
    let digest = sha256_bytes_hex(common_dir.to_string_lossy().as_bytes());
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
    absent_artifact_count: usize,
    source_retired: bool,
}

impl FoldOutcome {
    fn empty() -> Self {
        Self {
            events_created: 0,
            events_existing: 0,
            artifacts_created: 0,
            removal_event_count: 0,
            absent_artifact_count: 0,
            source_retired: false,
        }
    }
}

/// The source store's unsigned `ArtifactRemoved` events — the possession-stripping
/// population the fold restamps (and the dry-run preview discloses). Shared by
/// `fold_source_forward` and `preview_fold` so the disclosure count is spelled once.
fn count_unsigned_artifact_removals(source: &Path) -> Result<usize> {
    Ok(EventStore::open(source)
        .list_events()?
        .iter()
        .filter(|event| event.event_type == EventType::ArtifactRemoved && event.signature.is_none())
        .count())
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
    let removal_event_count = count_unsigned_artifact_removals(source)?;

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
        absent_artifact_count: imported.absent_artifact_count,
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

/// Counts a fold preview produces (mirrors `FoldOutcome`, but nothing is written).
struct FoldPreview {
    source_present: bool,
    export_fidelity: String,
    events_to_create: usize,
    events_existing: usize,
    artifacts_to_create: usize,
    artifacts_existing: usize,
    removal_event_count: usize,
    absent_artifact_count: usize,
}

/// Preview the fold without importing: an absent/empty source is a clean no-op
/// (mirrors `fold_source_forward`'s guard); otherwise count the unsigned removals
/// (shared helper) and run the fold preflight (shared `preview_import_store_bundle`,
/// which tolerates an incomplete source and reports the absent-artifact count, the
/// same way the real fold does).
fn preview_fold(source: &Path, family_dir: &Path) -> Result<FoldPreview> {
    if !source.join("events").exists() {
        return Ok(FoldPreview {
            source_present: false,
            export_fidelity: fidelity_label(ExportFidelityStatus::Full),
            events_to_create: 0,
            events_existing: 0,
            artifacts_to_create: 0,
            artifacts_existing: 0,
            removal_event_count: 0,
            absent_artifact_count: 0,
        });
    }

    let removal_event_count = count_unsigned_artifact_removals(source)?;
    let import = preview_import_store_bundle(source, family_dir)?;
    Ok(FoldPreview {
        source_present: true,
        export_fidelity: fidelity_label(import.fidelity_status),
        events_to_create: import.events_created,
        events_existing: import.events_existing,
        artifacts_to_create: import.artifacts_created,
        artifacts_existing: import.artifacts_existing,
        removal_event_count,
        absent_artifact_count: import.absent_artifact_count,
    })
}

fn fidelity_label(status: ExportFidelityStatus) -> String {
    match status {
        ExportFidelityStatus::Full => "full",
        ExportFidelityStatus::Incomplete => "incomplete",
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};
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
    fn clone_ref_keys_on_the_common_dir() {
        // The digest input is the common dir, so all worktrees of one physical clone
        // share it, and a different clone's common dir yields a different ref.
        let a = Path::new("/repo/.git");
        let b = Path::new("/other/.git");
        assert_eq!(mint_clone_ref(a), mint_clone_ref(a));
        assert_ne!(mint_clone_ref(a), mint_clone_ref(b));
        assert_eq!(mint_clone_ref(a).len(), 16);
    }

    #[test]
    fn re_linking_a_sibling_worktree_dedups_to_one_registry_member() {
        // Link the main checkout, then `git worktree add` a sibling and link IT.
        // Because the clone_ref keys on the shared common dir, the registry holds ONE
        // member, not two.
        let main = git_repo();
        let wt_parent = tempfile::tempdir().unwrap();
        let wt = wt_parent.path().join("sibling");
        run_git(main.path(), ["branch", "sib"]);
        run_git(
            main.path(),
            [
                OsStr::new("worktree"),
                OsStr::new("add"),
                wt.as_os_str(),
                OsStr::new("sib"),
            ],
        );
        let home = main.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let (main_ref, sib_ref, entries) = with_shore_home(&home, || {
            let main_ref =
                link_store_to_family(StoreLinkOptions::new(main.path(), Some("fam".to_owned())))
                    .unwrap()
                    .clone_ref;
            let sib_ref = link_store_to_family(StoreLinkOptions::new(&wt, Some("fam".to_owned())))
                .unwrap()
                .clone_ref;
            let family_dir = user_level_store_dir("fam").unwrap();
            let entries = crate::session::store::user_level::read_family_registry(&family_dir)
                .unwrap()
                .entries
                .len();
            (main_ref, sib_ref, entries)
        });

        assert_eq!(main_ref, sib_ref, "one physical clone → one clone_ref");
        assert_eq!(entries, 1, "the sibling re-link dedups, not duplicates");
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
        assert!(
            message.contains("store status --show-paths"),
            "points at the local-only command that lists the matched files: {message}"
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
            .record_event_once(&removal_event_for("sha256:deadbeef"))
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

    #[test]
    fn dry_run_on_a_clean_path_returns_a_preview_and_writes_nothing() {
        let repo = modified_git_repo();
        crate::session::capture_worktree_review(crate::session::CaptureOptions::new(repo.path()))
            .unwrap();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let before = tree_fingerprint(&home);

        let preview = with_shore_home(&home, || {
            preview_link_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
        })
        .unwrap();

        assert!(preview.would_create_family);
        assert!(preview.source_present);
        assert_eq!(preview.export_fidelity, "full");
        assert!(preview.folded_events_to_create >= 1);
        assert_eq!(preview.folded_events_existing, 0);
        assert_eq!(preview.folded_removal_event_count, 0);
        // Writes nothing: the home tree is byte-identical and no binding was written.
        assert_eq!(tree_fingerprint(&home), before);
        assert!(!repo.path().join(".shore/store.local.json").exists());
        let binding = with_shore_home(&home, || resolve_family_binding(repo.path())).unwrap();
        assert!(binding.is_none());
    }

    #[test]
    fn dry_run_reports_existing_counts_when_the_family_already_holds_the_history() {
        let repo = modified_git_repo();
        crate::session::capture_worktree_review(crate::session::CaptureOptions::new(repo.path()))
            .unwrap();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        with_shore_home(&home, || {
            link_store_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
                .unwrap()
        });
        let after_link = tree_fingerprint(&home);

        let preview = with_shore_home(&home, || {
            preview_link_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
        })
        .unwrap();

        assert!(!preview.would_create_family);
        assert_eq!(preview.folded_events_to_create, 0);
        assert!(preview.folded_events_existing >= 1);
        // The second, dry, pass added nothing to the family store.
        assert_eq!(tree_fingerprint(&home), after_link);
    }

    #[test]
    fn dry_run_blocks_on_an_ephemeral_worktree_without_the_override() {
        let repo = git_repo();
        write_store_config(repo.path(), StoreMode::Ephemeral).unwrap();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let before = tree_fingerprint(&home);

        let error = with_shore_home(&home, || {
            preview_link_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
        })
        .expect_err("an ephemeral worktree refuses without the override");

        assert!(error.to_string().contains("ephemeral"));
        assert_eq!(tree_fingerprint(&home), before);
        assert!(!repo.path().join(".shore/store.local.json").exists());
    }

    #[test]
    fn dry_run_previews_success_with_the_include_ephemeral_override() {
        let repo = git_repo();
        write_store_config(repo.path(), StoreMode::Ephemeral).unwrap();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let before = tree_fingerprint(&home);

        let preview = with_shore_home(&home, || {
            preview_link_to_family(
                StoreLinkOptions::new(repo.path(), Some("fam".to_owned()))
                    .with_include_ephemeral(true),
            )
        })
        .unwrap();

        assert!(preview.would_create_family);
        assert_eq!(tree_fingerprint(&home), before);
    }

    #[test]
    fn dry_run_blocks_on_a_sensitive_worktree_without_the_override() {
        let repo = git_repo();
        std::fs::create_dir_all(repo.path().join("keys")).unwrap();
        std::fs::write(
            repo.path().join("keys/dev.pem"),
            "-----BEGIN PRIVATE KEY-----\nredacted\n",
        )
        .unwrap();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let before = tree_fingerprint(&home);

        let error = with_shore_home(&home, || {
            preview_link_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
        })
        .expect_err("a sensitive worktree refuses without the override");

        assert!(error.to_string().contains("sensitivity.json"));
        assert_eq!(tree_fingerprint(&home), before);
    }

    #[test]
    fn dry_run_previews_success_with_the_include_sensitive_override() {
        let repo = git_repo();
        std::fs::create_dir_all(repo.path().join("keys")).unwrap();
        std::fs::write(
            repo.path().join("keys/dev.pem"),
            "-----BEGIN PRIVATE KEY-----\nredacted\n",
        )
        .unwrap();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let before = tree_fingerprint(&home);

        let preview = with_shore_home(&home, || {
            preview_link_to_family(
                StoreLinkOptions::new(repo.path(), Some("fam".to_owned()))
                    .with_include_sensitive(true),
            )
        })
        .unwrap();

        assert!(preview.would_create_family);
        assert_eq!(tree_fingerprint(&home), before);
    }

    #[test]
    fn dry_run_blocks_on_a_family_stamp_mismatch() {
        let repo = git_repo();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let error = with_shore_home(&home, || {
            let family_dir = user_level_store_dir("fam").unwrap();
            std::fs::create_dir_all(&family_dir).unwrap();
            write_family_manifest(&family_dir, "other", &[]).unwrap();
            // Snapshot AFTER the pre-stamp — the preview must add nothing more.
            let before = tree_fingerprint(&home);
            let error =
                preview_link_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
                    .expect_err("a family-stamp mismatch refuses");
            assert_eq!(tree_fingerprint(&home), before);
            error
        });
        assert!(error.to_string().contains("other"));
    }

    #[test]
    fn dry_run_tolerates_an_unaccounted_missing_artifact_and_discloses_it() {
        // An UNACCOUNTED missing artifact (bytes gone, no `ArtifactRemoved` claim to
        // explain it — an old snapshot GC'd/migrated away) is the source's existing
        // state, not a blocker: the preview reports the store linkable with an
        // absent-artifact disclosure and `export_fidelity` `incomplete`, writing
        // nothing. A claim-covered absence stays `full` — see
        // `dry_run_previews_a_store_with_a_removed_artifact_as_linkable`.
        let repo = modified_git_repo();
        crate::session::capture_worktree_review(crate::session::CaptureOptions::new(repo.path()))
            .unwrap();
        let source = crate::session::store::resolution::clone_local_store_dir(repo.path()).unwrap();
        std::fs::remove_dir_all(source.join("artifacts/objects")).unwrap();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let before = tree_fingerprint(&home);

        let preview = with_shore_home(&home, || {
            preview_link_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
        })
        .unwrap();

        assert_eq!(preview.export_fidelity, "incomplete");
        assert!(preview.folded_absent_artifact_count >= 1);
        assert_eq!(tree_fingerprint(&home), before);
    }

    #[test]
    fn dry_run_previews_a_store_with_a_removed_artifact_as_linkable() {
        // The owner-confirmed correction: a store whose artifact was legitimately
        // removed (claim recorded) then compacted (bytes erased) is NOT a fidelity
        // defect. It previews as linkable — the fold carries the referencing event and
        // the removal claim, and discloses the possession-stripping population.
        let repo = modified_git_repo();
        let capture = crate::session::capture_worktree_review(crate::session::CaptureOptions::new(
            repo.path(),
        ))
        .unwrap();
        let source = crate::session::store::resolution::clone_local_store_dir(repo.path()).unwrap();
        crate::session::EventStore::open(&source)
            .record_event_once(&removal_event_for(&capture.object_artifact_content_hash))
            .unwrap();
        std::fs::remove_dir_all(source.join("artifacts/objects")).unwrap();
        let home = repo.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let preview = with_shore_home(&home, || {
            preview_link_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
        })
        .unwrap();

        // Claim-covered absence is `full` fidelity and is NOT counted as an
        // unaccounted absence — the two absence kinds stay distinct.
        assert_eq!(preview.export_fidelity, "full");
        assert!(preview.folded_removal_event_count >= 1);
        assert_eq!(preview.folded_absent_artifact_count, 0);
    }

    #[test]
    fn dry_run_previews_the_filesystem_advisory_warning() {
        let repo = git_repo();
        let home = repo.path().join("Dropbox");
        std::fs::create_dir_all(&home).unwrap();

        let preview = with_shore_home(&home, || {
            preview_link_to_family(StoreLinkOptions::new(repo.path(), Some("fam".to_owned())))
        })
        .unwrap();

        assert!(preview.filesystem_warning.is_some());
        assert!(preview.would_create_family);
    }

    #[test]
    fn dry_run_previews_the_history_overlap_advisory_warning() {
        let founder = git_repo();
        let joiner = git_repo(); // distinct root OID (git_repo seeds unique content)
        let home = founder.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        with_shore_home(&home, || {
            link_store_to_family(StoreLinkOptions::new(
                founder.path(),
                Some("fam".to_owned()),
            ))
            .unwrap()
        });

        let preview = with_shore_home(&home, || {
            preview_link_to_family(StoreLinkOptions::new(joiner.path(), Some("fam".to_owned())))
        })
        .unwrap();

        assert!(preview.history_overlap_warning.is_some());
        assert!(!preview.would_create_family);
    }

    /// Recursive path→bytes fingerprint of a directory tree, for asserting a
    /// preview left `SHORE_HOME` byte-for-byte untouched.
    fn tree_fingerprint(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(dir: &Path, base: &Path, acc: &mut BTreeMap<PathBuf, Vec<u8>>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, base, acc);
                } else if let Ok(bytes) = std::fs::read(&path) {
                    let rel = path.strip_prefix(base).unwrap_or(&path).to_path_buf();
                    acc.insert(rel, bytes);
                }
            }
        }
        let mut acc = BTreeMap::new();
        walk(root, root, &mut acc);
        acc
    }

    fn modified_git_repo() -> tempfile::TempDir {
        let repo = git_repo();
        // An uncommitted modification gives `capture_worktree_review` a diff to record.
        std::fs::write(repo.path().join("README.md"), "changed\n").unwrap();
        repo
    }

    fn removal_event_for(content_hash: &str) -> crate::session::event::ShoreEvent {
        use crate::model::JournalId;
        use crate::session::event::{
            ArtifactRemovedPayload, EventTarget, EventType, ShoreEvent, Writer,
        };
        ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_journal(JournalId::new("journal:test")),
            Writer::shore_local("0.1.0"),
            ArtifactRemovedPayload {
                content_hash: content_hash.to_owned(),
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
