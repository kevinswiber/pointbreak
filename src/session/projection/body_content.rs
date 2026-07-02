//! Read-time resolution of note-shaped body content against recorded
//! removals, the body twin of the snapshot content seam.

use std::collections::BTreeSet;

use crate::error::Result;
use crate::session::body_artifact::{load_body_artifact, note_body_content_hash_from_path};
use crate::session::projection::artifact_removal::{
    ArtifactRemovalProjection, RemovalOperativeStatus,
};
use crate::session::projection::cosignature::CosignatureIndex;
use crate::session::signing::{RemovalPolicy, TrustSet};
use crate::session::state::ProjectionDiagnostic;
use crate::session::store::backend::StoreBackend;
use crate::session::store::content::ContentArtifacts;

/// Wire+library state of a note-shaped body (observation body, input-request
/// body, response reason, assessment summary, validation summary, imported
/// note body). Body twin of `SnapshotContentState`: `SuppressedPresent` means
/// a removal is recorded but the bytes are still stored; `PhysicallyRemoved`
/// means the bytes have been swept from the store.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyContentState {
    #[default]
    Present,
    SuppressedPresent,
    PhysicallyRemoved,
}

impl BodyContentState {
    /// The serde skip predicate: `Present` is the default and stays off the wire.
    pub fn is_present(&self) -> bool {
        matches!(self, Self::Present)
    }

    /// Whether the body content is removed (suppressed or swept).
    pub fn is_removed(&self) -> bool {
        !self.is_present()
    }
}

/// Resolved body content: the (state, text) pair the views consume.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BodyContent {
    Present(Option<String>),
    SuppressedPresent { content_hash: String },
    PhysicallyRemoved { content_hash: String },
}

impl BodyContent {
    pub(crate) fn state(&self) -> BodyContentState {
        match self {
            Self::Present(_) => BodyContentState::Present,
            Self::SuppressedPresent { .. } => BodyContentState::SuppressedPresent,
            Self::PhysicallyRemoved { .. } => BodyContentState::PhysicallyRemoved,
        }
    }

    /// The removal key, borrowed from the removed variants; `None` when present.
    /// Surfaces whose payload carries no body hash (imported notes) render this
    /// as their `removed_body_content_hash` twin of the snapshot result field.
    // Consumed by the imported-note surfaces; drop the allow with the first one.
    #[allow(dead_code)]
    pub(crate) fn removed_content_hash(&self) -> Option<&str> {
        match self {
            Self::Present(_) => None,
            Self::SuppressedPresent { content_hash } | Self::PhysicallyRemoved { content_hash } => {
                Some(content_hash)
            }
        }
    }

    /// The rendered text: hydrated bytes when present, `None` for removed states.
    pub(crate) fn into_text(self) -> Option<String> {
        match self {
            Self::Present(text) => text,
            Self::SuppressedPresent { .. } | Self::PhysicallyRemoved { .. } => None,
        }
    }
}

/// Reader-relative removal lens over note-body content hashes: the borrowed
/// bundle every body resolution reads (built once per store read, beside the
/// snapshot's own operative decision).
pub(crate) struct BodyRemovalLens<'a> {
    removal: &'a ArtifactRemovalProjection,
    trust_set: &'a TrustSet,
    policy: RemovalPolicy,
    cosig: &'a CosignatureIndex<'a>,
}

impl<'a> BodyRemovalLens<'a> {
    pub(crate) fn new(
        removal: &'a ArtifactRemovalProjection,
        trust_set: &'a TrustSet,
        policy: RemovalPolicy,
        cosig: &'a CosignatureIndex<'a>,
    ) -> Self {
        Self {
            removal,
            trust_set,
            policy,
            cosig,
        }
    }
}

/// Resolve a note-shaped body against the reader's removal lens.
///
/// The removed-vs-missing decision lives here, at the layer that holds the
/// event set, so the storage byte readers stay event-unaware: an operative
/// removal renders as an explained removed state (split suppressed-vs-swept by
/// a store presence check, regardless of `include_body` — the state is
/// metadata about the store, not a hydration choice), while absent bytes
/// WITHOUT an operative removal keep the hard `import referenced artifacts`
/// error exactly as before.
///
/// Inline bodies always render and never consult the lens: inline bytes live
/// in the immutable event log, and content-targeted removal deliberately does
/// not cover event-payload bytes (the deferred tier in
/// `docs/adr/adr-0016-content-targeted-artifact-removal-and-compaction.md`),
/// so suppressing their render would overstate erasure.
pub(crate) fn resolve_body_content(
    backend: &StoreBackend,
    lens: &BodyRemovalLens<'_>,
    include_body: bool,
    inline: Option<String>,
    artifact_path: Option<&str>,
) -> Result<BodyContent> {
    if inline.is_some() {
        return Ok(BodyContent::Present(if include_body {
            inline
        } else {
            None
        }));
    }
    let Some(path) = artifact_path else {
        return Ok(BodyContent::Present(None));
    };
    let content_hash = note_body_content_hash_from_path(path)?;
    let status =
        lens.removal
            .operative_status(&content_hash, lens.trust_set, lens.policy, lens.cosig)?;
    if matches!(
        status,
        RemovalOperativeStatus::OperativePossession | RemovalOperativeStatus::OperativeTrusted
    ) {
        let present = ContentArtifacts::from_backend(backend)
            .get_if_exists(path)?
            .is_some();
        return Ok(if present {
            BodyContent::SuppressedPresent { content_hash }
        } else {
            BodyContent::PhysicallyRemoved { content_hash }
        });
    }
    if include_body {
        return Ok(BodyContent::Present(load_body_artifact(backend, path)?));
    }
    Ok(BodyContent::Present(None))
}

/// A removal is recorded for the body content, but its bytes are still stored:
/// the suppression is reversible and a compact would reclaim them.
const BODY_CONTENT_SUPPRESSED_PRESENT: &str = "body_content_suppressed_present";
/// A removal is recorded for the body content and its bytes have been swept
/// from the store.
const BODY_CONTENT_PHYSICALLY_REMOVED: &str = "body_content_physically_removed";

/// Fold body `(state, content_hash)` pairs into the explained removal
/// diagnostics, deduped per `(content_hash, state)` and emitted in
/// deterministic hash-sorted order. Present entries and hash-less entries
/// never surface. Every emitter goes through this mapper so codes and
/// messages have exactly one owner.
pub(crate) fn body_content_diagnostics<'a>(
    entries: impl IntoIterator<Item = (BodyContentState, Option<&'a str>)>,
) -> Vec<ProjectionDiagnostic> {
    let mut removed: BTreeSet<(String, BodyContentState)> = BTreeSet::new();
    for (state, hash) in entries {
        if state.is_removed()
            && let Some(hash) = hash
        {
            removed.insert((hash.to_owned(), state));
        }
    }
    removed
        .into_iter()
        .map(|(hash, state)| match state {
            BodyContentState::SuppressedPresent => ProjectionDiagnostic {
                code: BODY_CONTENT_SUPPRESSED_PRESENT.to_owned(),
                message: format!(
                    "body content {hash} is suppressed by a recorded removal; \
                     the bytes are still stored and a compact would reclaim them"
                ),
            },
            BodyContentState::PhysicallyRemoved => ProjectionDiagnostic {
                code: BODY_CONTENT_PHYSICALLY_REMOVED.to_owned(),
                message: format!(
                    "body content {hash} was removed and its bytes have been swept from the store"
                ),
            },
            BodyContentState::Present => unreachable!("present entries are filtered above"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::JournalId;
    use crate::session::body_artifact::{
        BodyArtifactOutcome, note_body_content_hash_from_path, stage_body_artifact,
    };
    use crate::session::event::{
        ArtifactRemovedPayload, EventTarget, EventType, IngestProvenance, IngestVia, ShoreEvent,
        Writer,
    };
    use crate::session::projection::cosignature::CosignatureIndex;
    use crate::session::signing::{RemovalPolicy, TrustSet};
    use crate::session::store::backend::StoreBackend;
    use crate::session::store::content::ContentArtifacts;

    fn external_body() -> String {
        "x".repeat(5000)
    }

    /// Stage `body` as an externalized note-body artifact; write the blob only
    /// when `write_blob`. Returns `(relative_path, content_hash)`.
    fn staged_note_body(backend: &StoreBackend, body: &str, write_blob: bool) -> (String, String) {
        match stage_body_artifact(body.as_bytes()).expect("stage body") {
            BodyArtifactOutcome::Artifact {
                relative_path,
                body_envelope,
                ..
            } => {
                if write_blob {
                    ContentArtifacts::from_backend(backend)
                        .put_note_body(
                            &relative_path,
                            &body_envelope.to_json_bytes().expect("envelope bytes"),
                        )
                        .expect("write blob");
                }
                let content_hash =
                    note_body_content_hash_from_path(&relative_path).expect("hash from path");
                (relative_path, content_hash)
            }
            BodyArtifactOutcome::Inline { .. } => panic!("fixture body must externalize"),
        }
    }

    /// A bare unsigned, locally-authored (`ingest = None`) removal for `content_hash`.
    fn removal_event(content_hash: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_journal(JournalId::new("journal:fixture")),
            Writer::shore_local("test"),
            ArtifactRemovedPayload {
                content_hash: content_hash.to_owned(),
            },
            "2026-06-19T00:00:00Z",
        )
        .unwrap()
    }

    /// Mark an event as ingested through a foreign-event seam (`ingest = Some`),
    /// which drops the local-possession arm.
    fn ingested(mut event: ShoreEvent) -> ShoreEvent {
        event.ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "2026-06-19T01:00:00Z".to_owned(),
        });
        event
    }

    /// Drive `resolve_body_content` over `events` with default trust/policy.
    fn resolve(
        backend: &StoreBackend,
        events: &[ShoreEvent],
        include_body: bool,
        inline: Option<String>,
        artifact_path: Option<&str>,
    ) -> crate::error::Result<BodyContent> {
        let removal = ArtifactRemovalProjection::from_events(events).expect("removal projection");
        let cosig = CosignatureIndex::build(events).expect("cosignature index");
        let trust = TrustSet::default();
        let lens = BodyRemovalLens::new(&removal, &trust, RemovalPolicy::default(), &cosig);
        resolve_body_content(backend, &lens, include_body, inline, artifact_path)
    }

    #[test]
    fn operative_removal_with_blob_on_disk_is_suppressed_present() {
        let backend = StoreBackend::memory();
        let body = external_body();
        let (path, hash) = staged_note_body(&backend, &body, true);
        let events = vec![removal_event(&hash)];

        let content = resolve(&backend, &events, true, None, Some(&path)).expect("resolves");

        assert_eq!(content.state(), BodyContentState::SuppressedPresent);
        assert_eq!(content.removed_content_hash(), Some(hash.as_str()));
        assert_eq!(content.into_text(), None);
    }

    #[test]
    fn operative_removal_with_blob_swept_is_physically_removed() {
        let backend = StoreBackend::memory();
        let body = external_body();
        let (path, hash) = staged_note_body(&backend, &body, false);
        let events = vec![removal_event(&hash)];

        let content = resolve(&backend, &events, true, None, Some(&path)).expect("resolves");

        assert_eq!(content.state(), BodyContentState::PhysicallyRemoved);
        assert_eq!(content.removed_content_hash(), Some(hash.as_str()));
        assert_eq!(content.into_text(), None);
    }

    #[test]
    fn non_operative_claim_with_blob_present_renders_body() {
        let backend = StoreBackend::memory();
        let body = external_body();
        let (path, hash) = staged_note_body(&backend, &body, true);
        let events = vec![ingested(removal_event(&hash))];

        let content = resolve(&backend, &events, true, None, Some(&path)).expect("resolves");

        assert_eq!(content.state(), BodyContentState::Present);
        assert_eq!(content.removed_content_hash(), None);
        assert_eq!(content.into_text(), Some(body));
    }

    #[test]
    fn non_operative_claim_over_absent_blob_keeps_missing_artifact_error() {
        let backend = StoreBackend::memory();
        let body = external_body();
        let (path, hash) = staged_note_body(&backend, &body, false);
        let events = vec![ingested(removal_event(&hash))];

        let err = resolve(&backend, &events, true, None, Some(&path)).unwrap_err();

        assert!(err.to_string().contains("import referenced artifacts"));
    }

    #[test]
    fn absent_blob_without_claim_keeps_missing_artifact_error() {
        let backend = StoreBackend::memory();
        let body = external_body();
        let (path, _hash) = staged_note_body(&backend, &body, false);

        let err = resolve(&backend, &[], true, None, Some(&path)).unwrap_err();

        assert!(err.to_string().contains("import referenced artifacts"));
    }

    #[test]
    fn inline_body_renders_even_when_its_hash_carries_an_operative_removal() {
        let backend = StoreBackend::memory();
        let inline = "a small inline body".to_owned();
        let hash = format!(
            "sha256:{}",
            crate::canonical_hash::sha256_bytes_hex(inline.as_bytes())
        );
        let events = vec![removal_event(&hash)];

        let content =
            resolve(&backend, &events, true, Some(inline.clone()), None).expect("resolves");

        assert_eq!(content.state(), BodyContentState::Present);
        assert_eq!(content.into_text(), Some(inline));
    }

    #[test]
    fn include_body_false_still_reports_removed_state_but_never_loads_bytes() {
        let backend = StoreBackend::memory();
        let body = external_body();
        let (path, hash) = staged_note_body(&backend, &body, false);

        let removed = resolve(&backend, &[removal_event(&hash)], false, None, Some(&path))
            .expect("removed state resolves without a read");
        assert_eq!(removed.state(), BodyContentState::PhysicallyRemoved);

        let untouched = resolve(&backend, &[], false, None, Some(&path))
            .expect("no claim and no read must not error");
        assert_eq!(untouched.state(), BodyContentState::Present);
        assert_eq!(untouched.into_text(), None);
    }

    #[test]
    fn body_content_diagnostics_dedupes_and_orders_per_hash_and_state() {
        let entries = [
            (BodyContentState::PhysicallyRemoved, Some("sha256:aaa")),
            (BodyContentState::PhysicallyRemoved, Some("sha256:aaa")), // duplicate collapses
            (BodyContentState::SuppressedPresent, Some("sha256:bbb")),
            (BodyContentState::Present, Some("sha256:ccc")), // present never surfaces
            (BodyContentState::PhysicallyRemoved, None),     // no hash, skipped
        ];

        let diagnostics = body_content_diagnostics(entries);

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].code, "body_content_physically_removed");
        assert_eq!(
            diagnostics[0].message,
            "body content sha256:aaa was removed and its bytes have been swept from the store"
        );
        assert_eq!(diagnostics[1].code, "body_content_suppressed_present");
        assert_eq!(
            diagnostics[1].message,
            "body content sha256:bbb is suppressed by a recorded removal; \
             the bytes are still stored and a compact would reclaim them"
        );
    }

    #[test]
    fn body_content_state_serializes_snake_case_and_present_is_skipped() {
        #[derive(serde::Serialize)]
        struct Probe {
            #[serde(skip_serializing_if = "BodyContentState::is_present")]
            state: BodyContentState,
        }

        let suppressed = serde_json::to_string(&Probe {
            state: BodyContentState::SuppressedPresent,
        })
        .expect("serialize suppressed");
        assert_eq!(suppressed, r#"{"state":"suppressed_present"}"#);

        let removed = serde_json::to_string(&Probe {
            state: BodyContentState::PhysicallyRemoved,
        })
        .expect("serialize removed");
        assert_eq!(removed, r#"{"state":"physically_removed"}"#);

        let present = serde_json::to_string(&Probe {
            state: BodyContentState::default(),
        })
        .expect("serialize present");
        assert_eq!(present, "{}");
    }

    #[test]
    fn body_content_state_removed_predicate_matches_both_removed_states() {
        assert!(!BodyContentState::Present.is_removed());
        assert!(BodyContentState::SuppressedPresent.is_removed());
        assert!(BodyContentState::PhysicallyRemoved.is_removed());
    }
}
