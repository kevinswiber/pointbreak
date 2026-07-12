//! Internal registry of the prefix strings Pointbreak mints into ids and
//! artifact references.
//!
//! Prefixes are opaque to consumers (see "IDs are opaque" in
//! `docs/review-workflow.md`); this registry exists for internal consistency
//! and discoverability. Prefix strings feed content-derived ids, so changing
//! any existing value changes the ids newly minted for identical content and
//! breaks cross-clone convergence with existing stores — that is an explicit
//! owner decision, recorded in `docs/adr/adr-0028-id-prefix-convention.md`.
//! The convention for new entries: spelled-out, hyphenated, lowercase domain
//! names; the abbreviation set below is closed.
//!
//! Production code consumes the constants; the `cfg(test)` table beneath them
//! is the enumerable contract the registry tests and the inspector drift test
//! check against (it is test-gated so the lib target carries no test-only
//! dead code — lift the gate if production ever needs to enumerate).

/// Event envelope ids: `evt:sha256:<hex>` (hash of the idempotency key).
pub(crate) const EVENT: &str = "evt";
/// Revision ids: `rev:sha256:<hex>` and the `rev:worktree:sha256:<hex>` variant.
pub(crate) const REVISION: &str = "rev";
/// Content-object ids: `obj:sha256:<hex>` and the `obj:git:sha256:<hex>` variant.
pub(crate) const OBJECT: &str = "obj";
/// Engagement grouping ids: `engagement:sha256:<hex>`.
pub(crate) const ENGAGEMENT: &str = "engagement";
/// Observation ids: `obs:sha256:<hex>`.
pub(crate) const OBSERVATION: &str = "obs";
/// Assessment ids: `assess:sha256:<hex>`.
pub(crate) const ASSESSMENT: &str = "assess";
/// Validation check ids: `validation:sha256:<hex>`.
pub(crate) const VALIDATION: &str = "validation";
/// Input request ids: `input-request:sha256:<hex>`.
pub(crate) const INPUT_REQUEST: &str = "input-request";
/// Input request response ids: `input-request-response:sha256:<hex>`.
pub(crate) const INPUT_REQUEST_RESPONSE: &str = "input-request-response";
/// Commit association ids: `assoc-commit:sha256:<hex>`.
pub(crate) const COMMIT_ASSOCIATION: &str = "assoc-commit";
/// Ref association ids: `assoc-ref:sha256:<hex>`.
pub(crate) const REF_ASSOCIATION: &str = "assoc-ref";
/// Commit withdrawal ids: `withdraw-commit:sha256:<hex>`.
pub(crate) const COMMIT_WITHDRAWAL: &str = "withdraw-commit";
/// Ref withdrawal ids: `withdraw-ref:sha256:<hex>`.
pub(crate) const REF_WITHDRAWAL: &str = "withdraw-ref";
/// Task attempt work-object ids: `task-attempt:sha256:<hex>` (adapter-minted).
pub(crate) const TASK_ATTEMPT: &str = "task-attempt";
/// Checkpoint ids: `checkpoint:sha256:<hex>` (adapter-minted).
pub(crate) const CHECKPOINT: &str = "checkpoint";
/// Opaque signed-target subject ids: `subject:sha256:<hex>` — the hash over a
/// subject's identity-bearing fields (never its renamable kind tag) that the
/// signed envelope binds in place of the structural subject. Reconstructed for
/// display from the payload, not referenced by users.
pub(crate) const SUBJECT: &str = "subject";
/// Journal ids: `journal:claude:<session_uuid>` and the `journal:default`
/// sentinel. Generic `JournalId::new(session)` callers pass strings through
/// as-is; this prefix covers the hardcoded mints.
pub(crate) const JOURNAL: &str = "journal";
/// Review ids: the `review:default` sentinel only — no content-derived shape.
pub(crate) const REVIEW: &str = "review";
/// Actor ids: `actor:git-email:<email>`, `actor:git-name:<name>`,
/// `actor:local`, `actor:claude_code:user`, `actor:claude_code:assistant`.
/// The `actor:agent:<name>` / `actor:env:<...>` shapes arrive from user
/// configuration and are validated, never minted here.
pub(crate) const ACTOR: &str = "actor";
/// Review-stream row ids: `row:<zero-padded ordinal>` (positional, not content).
pub(crate) const ROW: &str = "row";
/// Export/inventory artifact reference to an object artifact: `object:sha256:<hex>`.
pub(crate) const ARTIFACT_OBJECT: &str = "object";
/// Export artifact reference to a content body: `body:sha256:<hex>`.
pub(crate) const ARTIFACT_BODY: &str = "body";
/// Export/inventory artifact reference to a note body: `note-body:sha256:<hex>`.
pub(crate) const NOTE_BODY: &str = "note-body";
/// Redacted sensitive-path reference: `file:sha256:<hex>` (hash of the relative
/// path). Distinct from `FileId`, which is path-based and carries no prefix.
pub(crate) const REDACTED_FILE: &str = "file";
/// What kind of string a registered prefix opens.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrefixKind {
    /// Content-derived id: `<prefix>:[infix:]sha256:<hex>`.
    ContentId,
    /// Structured, non-content id (positional, composite, or sentinel).
    StructuralId,
    /// Content-addressed artifact reference in export bundles and inventories.
    ArtifactRef,
}

/// One registered prefix: the enumerable row the registry tests check.
#[cfg(test)]
pub(crate) struct IdPrefix {
    pub(crate) prefix: &'static str,
    pub(crate) kind: PrefixKind,
    /// False only for legacy display entries that no production path mints.
    pub(crate) minted: bool,
    /// Whether the inspector's reference regex linkifies this prefix — mirrored
    /// by REF_ID_PREFIXES in src/cli/inspect/web/src/classNames.ts; the drift
    /// test keeps the two lists identical.
    pub(crate) linkified: bool,
}

/// Every prefix Pointbreak has ever put in front of an id or artifact reference
/// — the single enumerable source. Content-id entries first, then structural
/// and artifact references. The legacy `review-unit`
/// and `snap` display entries were retired in #344 (no production path minted
/// them and the inspector no longer linkifies them).
#[cfg(test)]
pub(crate) const ID_PREFIXES: &[IdPrefix] = &[
    IdPrefix {
        prefix: EVENT,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: true,
    },
    IdPrefix {
        prefix: REVISION,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: true,
    },
    IdPrefix {
        prefix: OBJECT,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: true,
    },
    IdPrefix {
        prefix: ENGAGEMENT,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: true,
    },
    IdPrefix {
        prefix: OBSERVATION,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: true,
    },
    IdPrefix {
        prefix: ASSESSMENT,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: true,
    },
    IdPrefix {
        prefix: VALIDATION,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: true,
    },
    IdPrefix {
        prefix: INPUT_REQUEST,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: true,
    },
    IdPrefix {
        prefix: INPUT_REQUEST_RESPONSE,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: true,
    },
    IdPrefix {
        prefix: COMMIT_ASSOCIATION,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: true,
    },
    IdPrefix {
        prefix: REF_ASSOCIATION,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: true,
    },
    IdPrefix {
        prefix: COMMIT_WITHDRAWAL,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: true,
    },
    IdPrefix {
        prefix: REF_WITHDRAWAL,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: true,
    },
    IdPrefix {
        prefix: TASK_ATTEMPT,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: true,
    },
    IdPrefix {
        prefix: CHECKPOINT,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: true,
    },
    IdPrefix {
        prefix: SUBJECT,
        kind: PrefixKind::ContentId,
        minted: true,
        linkified: false,
    },
    IdPrefix {
        prefix: JOURNAL,
        kind: PrefixKind::StructuralId,
        minted: true,
        linkified: false,
    },
    IdPrefix {
        prefix: REVIEW,
        kind: PrefixKind::StructuralId,
        minted: true,
        linkified: false,
    },
    IdPrefix {
        prefix: ACTOR,
        kind: PrefixKind::StructuralId,
        minted: true,
        linkified: false,
    },
    IdPrefix {
        prefix: ROW,
        kind: PrefixKind::StructuralId,
        minted: true,
        linkified: false,
    },
    IdPrefix {
        prefix: ARTIFACT_OBJECT,
        kind: PrefixKind::ArtifactRef,
        minted: true,
        linkified: false,
    },
    IdPrefix {
        prefix: ARTIFACT_BODY,
        kind: PrefixKind::ArtifactRef,
        minted: true,
        linkified: false,
    },
    IdPrefix {
        prefix: NOTE_BODY,
        kind: PrefixKind::ArtifactRef,
        minted: true,
        linkified: false,
    },
    IdPrefix {
        prefix: REDACTED_FILE,
        kind: PrefixKind::ArtifactRef,
        minted: true,
        linkified: false,
    },
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    // The ratification lock: every minted prefix string is frozen as-is (ADR-0028).
    // Changing a value here changes newly minted content ids and is an owner decision.
    #[test]
    fn prefix_constants_are_frozen() {
        assert_eq!(EVENT, "evt");
        assert_eq!(REVISION, "rev");
        assert_eq!(OBJECT, "obj");
        assert_eq!(ENGAGEMENT, "engagement");
        assert_eq!(OBSERVATION, "obs");
        assert_eq!(ASSESSMENT, "assess");
        assert_eq!(VALIDATION, "validation");
        assert_eq!(INPUT_REQUEST, "input-request");
        assert_eq!(INPUT_REQUEST_RESPONSE, "input-request-response");
        assert_eq!(COMMIT_ASSOCIATION, "assoc-commit");
        assert_eq!(REF_ASSOCIATION, "assoc-ref");
        assert_eq!(COMMIT_WITHDRAWAL, "withdraw-commit");
        assert_eq!(REF_WITHDRAWAL, "withdraw-ref");
        assert_eq!(TASK_ATTEMPT, "task-attempt");
        assert_eq!(CHECKPOINT, "checkpoint");
        assert_eq!(JOURNAL, "journal");
        assert_eq!(REVIEW, "review");
        assert_eq!(ACTOR, "actor");
        assert_eq!(ROW, "row");
        assert_eq!(ARTIFACT_OBJECT, "object");
        assert_eq!(ARTIFACT_BODY, "body");
        assert_eq!(NOTE_BODY, "note-body");
        assert_eq!(REDACTED_FILE, "file");
    }

    #[test]
    fn registry_prefixes_are_unique() {
        let mut seen = HashSet::new();
        for entry in ID_PREFIXES {
            assert!(
                seen.insert(entry.prefix),
                "duplicate registry prefix: {}",
                entry.prefix
            );
        }
    }

    #[test]
    fn registry_prefixes_use_the_reserved_charset() {
        for entry in ID_PREFIXES {
            let mut chars = entry.prefix.chars();
            let first = chars.next().expect("prefix must be non-empty");
            assert!(
                first.is_ascii_lowercase(),
                "{} must open lowercase",
                entry.prefix
            );
            assert!(
                chars.all(|c| c.is_ascii_lowercase() || c == '-'),
                "{} must be lowercase letters and hyphens",
                entry.prefix
            );
            assert!(
                !entry.prefix.ends_with('-'),
                "{} must not end with a hyphen",
                entry.prefix
            );
        }
    }

    #[test]
    fn every_minted_constant_is_registered_exactly_once() {
        let minted = [
            EVENT,
            REVISION,
            OBJECT,
            ENGAGEMENT,
            OBSERVATION,
            ASSESSMENT,
            VALIDATION,
            INPUT_REQUEST,
            INPUT_REQUEST_RESPONSE,
            COMMIT_ASSOCIATION,
            REF_ASSOCIATION,
            COMMIT_WITHDRAWAL,
            REF_WITHDRAWAL,
            TASK_ATTEMPT,
            CHECKPOINT,
            SUBJECT,
            JOURNAL,
            REVIEW,
            ACTOR,
            ROW,
            ARTIFACT_OBJECT,
            ARTIFACT_BODY,
            NOTE_BODY,
            REDACTED_FILE,
        ];
        assert_eq!(ID_PREFIXES.len(), minted.len());
        for prefix in minted {
            let entry = ID_PREFIXES
                .iter()
                .find(|e| e.prefix == prefix)
                .unwrap_or_else(|| panic!("{prefix} missing from the table"));
            assert!(entry.minted, "{prefix} is a production-minted prefix");
            assert_eq!(
                ID_PREFIXES.iter().filter(|e| e.prefix == prefix).count(),
                1,
                "{prefix} must appear in the table exactly once"
            );
        }
    }

    #[test]
    fn registry_kind_partition_is_stable() {
        let count = |kind: PrefixKind| ID_PREFIXES.iter().filter(|e| e.kind == kind).count();
        // 16 minted content-id prefixes: the 2 legacy display entries were retired
        // in #344, and `note:` retired with the imported-notes pipeline (nothing
        // mints or links it; recorded ids inside old t:07 payloads are opaque
        // strings that no surface projects).
        assert_eq!(count(PrefixKind::ContentId), 16);
        assert_eq!(count(PrefixKind::StructuralId), 4);
        assert_eq!(count(PrefixKind::ArtifactRef), 4);
    }

    #[test]
    fn inspector_ref_prefixes_match_the_registry() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/cli/inspect/web/src/classNames.ts");
        if !path.exists() {
            // The published crate excludes src/cli/inspect/web/** (Cargo.toml
            // `exclude`); the drift guard only means something in the repo.
            eprintln!("skipping inspector drift check: web sources not present");
            return;
        }
        let source = std::fs::read_to_string(&path).expect("read classNames.ts");
        let mut web = parse_ref_id_prefixes(&source);
        let mut registry: Vec<&str> = ID_PREFIXES
            .iter()
            .filter(|entry| entry.linkified)
            .map(|entry| entry.prefix)
            .collect();
        web.sort_unstable();
        registry.sort_unstable();
        assert_eq!(
            web, registry,
            "REF_ID_PREFIXES (classNames.ts) and the registry's linkified entries drifted; \
             change both together"
        );
    }

    #[test]
    fn linkified_entries_are_content_ids() {
        for entry in ID_PREFIXES {
            if entry.linkified {
                assert_eq!(
                    entry.kind,
                    PrefixKind::ContentId,
                    "{}: only content ids are linkifiable",
                    entry.prefix
                );
            }
            if !entry.minted {
                assert!(
                    entry.linkified,
                    "{}: a legacy entry that is neither minted nor linkified is dead weight",
                    entry.prefix
                );
            }
        }
    }

    /// The quoted strings inside the `REF_ID_PREFIXES = [ … ]` block.
    fn parse_ref_id_prefixes(source: &str) -> Vec<&str> {
        let start = source
            .find("REF_ID_PREFIXES = [")
            .expect("classNames.ts must declare REF_ID_PREFIXES");
        let block = &source[start..];
        let end = block.find(']').expect("REF_ID_PREFIXES block must close");
        let block = &block[..end];
        let prefixes: Vec<&str> = block.split('"').skip(1).step_by(2).collect();
        assert!(
            !prefixes.is_empty(),
            "REF_ID_PREFIXES parsed empty — the parser or the list is broken"
        );
        prefixes
    }

    #[test]
    fn no_legacy_unminted_entries_remain() {
        // The retired review-unit/snap display entries were the only unminted rows;
        // after #344 every registered prefix is production-minted.
        let legacy: Vec<&str> = ID_PREFIXES
            .iter()
            .filter(|e| !e.minted)
            .map(|e| e.prefix)
            .collect();
        assert!(
            legacy.is_empty(),
            "no unminted legacy display entries should remain: {legacy:?}"
        );
    }

    #[test]
    fn promoted_content_ids_are_linkified() {
        // The #344 promotion: these production-minted content ids now linkify (as
        // non-clickable chips on the web side). Kept coherent with REF_ID_PREFIXES
        // by `inspector_ref_prefixes_match_the_registry`.
        for prefix in [
            OBJECT,
            ENGAGEMENT,
            CHECKPOINT,
            TASK_ATTEMPT,
            COMMIT_ASSOCIATION,
            REF_ASSOCIATION,
            COMMIT_WITHDRAWAL,
            REF_WITHDRAWAL,
        ] {
            let entry = ID_PREFIXES
                .iter()
                .find(|e| e.prefix == prefix)
                .unwrap_or_else(|| panic!("{prefix} missing from the table"));
            assert!(entry.linkified, "{prefix} should be linkified after #344");
        }
    }
}
