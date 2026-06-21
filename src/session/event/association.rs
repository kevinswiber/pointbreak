//! ReviewUnit commit-range association/withdrawal payloads and their identity.
//!
//! A commit or ref association is a *structural edge* between a ReviewUnit and
//! the commit graph. It must converge across independently-authored copies of a
//! store, so identity is **writer-free and track-free**: the idempotency keys
//! and the content-hash ids below fold only the review unit and the raw
//! distinguisher (`commit_oid`, or `ref_name@head_oid`). The writer and track
//! ride on the event envelope for provenance and never enter identity.
//!
//! Withdrawal payloads carry only structural ids (no free-form reason), so two
//! peers withdrawing the same edge produce a byte-identical payload and
//! converge to `Existing` rather than conflicting.

use serde::{Deserialize, Serialize};

use super::kind::EventType;
use super::payload::EventPayload;
use crate::canonical_hash::sha256_json_prefixed;
use crate::error::Result;
use crate::model::{
    CommitAssociationId, CommitWithdrawalId, RefAssociationId, RefWithdrawalId, ReviewEndpoint,
    ReviewTargetRef, RevisionId,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitCommitAssociatedPayload {
    pub commit_association_id: CommitAssociationId,
    pub target: ReviewTargetRef,
    pub commit: ReviewEndpoint,
}

impl ReviewUnitCommitAssociatedPayload {
    pub fn idempotency_key(review_unit_id: &RevisionId, commit_oid: &str) -> String {
        format!(
            "review_unit_commit_associated:{}:{}",
            review_unit_id.as_str(),
            commit_oid
        )
    }
}

impl EventPayload for ReviewUnitCommitAssociatedPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewUnitCommitAssociated
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitRefAssociatedPayload {
    pub ref_association_id: RefAssociationId,
    pub target: ReviewTargetRef,
    pub ref_name: String,
    pub head_oid: String,
}

impl ReviewUnitRefAssociatedPayload {
    pub fn idempotency_key(review_unit_id: &RevisionId, ref_name: &str, head_oid: &str) -> String {
        format!(
            "review_unit_ref_associated:{}:{}",
            review_unit_id.as_str(),
            ref_distinguisher(ref_name, head_oid)
        )
    }
}

impl EventPayload for ReviewUnitRefAssociatedPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewUnitRefAssociated
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitCommitWithdrawnPayload {
    pub commit_withdrawal_id: CommitWithdrawalId,
    pub target: ReviewTargetRef,
    pub commit_association_id: CommitAssociationId,
}

impl ReviewUnitCommitWithdrawnPayload {
    pub fn idempotency_key(commit_association_id: &CommitAssociationId) -> String {
        format!(
            "review_unit_commit_withdrawn:{}",
            commit_association_id.as_str()
        )
    }
}

impl EventPayload for ReviewUnitCommitWithdrawnPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewUnitCommitWithdrawn
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitRefWithdrawnPayload {
    pub ref_withdrawal_id: RefWithdrawalId,
    pub target: ReviewTargetRef,
    pub ref_association_id: RefAssociationId,
}

impl ReviewUnitRefWithdrawnPayload {
    pub fn idempotency_key(ref_association_id: &RefAssociationId) -> String {
        format!("review_unit_ref_withdrawn:{}", ref_association_id.as_str())
    }
}

impl EventPayload for ReviewUnitRefWithdrawnPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewUnitRefWithdrawn
    }
}

/// The raw ref distinguisher `ref_name@head_oid`, shared by the idempotency key
/// and the display alias. A branch always canonicalizes to its full ref before
/// reaching here, so one branch yields one distinguisher regardless of entry
/// path.
fn ref_distinguisher(ref_name: &str, head_oid: &str) -> String {
    format!("{ref_name}@{head_oid}")
}

/// Content id for a commit association: a pure function of the review unit and
/// the commit OID, excluding writer/track so the same edge converges across
/// independently-authored copies.
pub(crate) fn build_commit_association_id(
    review_unit_id: &RevisionId,
    commit_oid: &str,
) -> Result<CommitAssociationId> {
    let digest = sha256_json_prefixed(&serde_json::json!({
        "reviewUnitId": review_unit_id.as_str(),
        "commitOid": commit_oid,
    }))?;
    Ok(CommitAssociationId::new(format!("assoc-commit:{digest}")))
}

/// Content id for a ref association, folding the full ref name and head OID.
pub(crate) fn build_ref_association_id(
    review_unit_id: &RevisionId,
    ref_name: &str,
    head_oid: &str,
) -> Result<RefAssociationId> {
    let digest = sha256_json_prefixed(&serde_json::json!({
        "reviewUnitId": review_unit_id.as_str(),
        "refName": ref_name,
        "headOid": head_oid,
    }))?;
    Ok(RefAssociationId::new(format!("assoc-ref:{digest}")))
}

/// Content id for a commit withdrawal, folding the association id it retracts.
pub(crate) fn build_commit_withdrawal_id(
    review_unit_id: &RevisionId,
    commit_association_id: &CommitAssociationId,
) -> Result<CommitWithdrawalId> {
    let digest = sha256_json_prefixed(&serde_json::json!({
        "reviewUnitId": review_unit_id.as_str(),
        "commitAssociationId": commit_association_id.as_str(),
    }))?;
    Ok(CommitWithdrawalId::new(format!("withdraw-commit:{digest}")))
}

/// Content id for a ref withdrawal, folding the association id it retracts.
pub(crate) fn build_ref_withdrawal_id(
    review_unit_id: &RevisionId,
    ref_association_id: &RefAssociationId,
) -> Result<RefWithdrawalId> {
    let digest = sha256_json_prefixed(&serde_json::json!({
        "reviewUnitId": review_unit_id.as_str(),
        "refAssociationId": ref_association_id.as_str(),
    }))?;
    Ok(RefWithdrawalId::new(format!("withdraw-ref:{digest}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_associated_idempotency_key_is_track_free() {
        let ru = RevisionId::new("ru:sha256:abc");
        assert_eq!(
            ReviewUnitCommitAssociatedPayload::idempotency_key(&ru, "oid123"),
            "review_unit_commit_associated:ru:sha256:abc:oid123"
        );
    }

    #[test]
    fn ref_associated_idempotency_key_joins_name_and_head() {
        let ru = RevisionId::new("ru:sha256:abc");
        assert_eq!(
            ReviewUnitRefAssociatedPayload::idempotency_key(&ru, "refs/heads/feat/x", "oidH"),
            "review_unit_ref_associated:ru:sha256:abc:refs/heads/feat/x@oidH"
        );
    }

    #[test]
    fn withdrawal_keys_use_the_association_id() {
        let cid = CommitAssociationId::new("assoc-commit:sha256:zzz");
        assert_eq!(
            ReviewUnitCommitWithdrawnPayload::idempotency_key(&cid),
            "review_unit_commit_withdrawn:assoc-commit:sha256:zzz"
        );
        let rid = RefAssociationId::new("assoc-ref:sha256:yyy");
        assert_eq!(
            ReviewUnitRefWithdrawnPayload::idempotency_key(&rid),
            "review_unit_ref_withdrawn:assoc-ref:sha256:yyy"
        );
    }

    #[test]
    fn payloads_round_trip_camel_case_and_report_event_type() {
        let p = ReviewUnitCommitAssociatedPayload {
            commit_association_id: CommitAssociationId::new("assoc-commit:sha256:zzz"),
            target: ReviewTargetRef::Revision {
                revision_id: RevisionId::new("ru:sha256:abc"),
            },
            commit: ReviewEndpoint::GitCommit {
                commit_oid: "oid123".into(),
                tree_oid: "tree9".into(),
            },
        };
        assert_eq!(p.event_type(), EventType::ReviewUnitCommitAssociated);
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["commitAssociationId"], "assoc-commit:sha256:zzz");
        assert_eq!(v["commit"]["kind"], "git_commit");
        let back: ReviewUnitCommitAssociatedPayload = serde_json::from_value(v).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn ref_associated_payload_round_trips_camel_case_and_reports_event_type() {
        let p = ReviewUnitRefAssociatedPayload {
            ref_association_id: RefAssociationId::new("assoc-ref:sha256:yyy"),
            target: ReviewTargetRef::Revision {
                revision_id: RevisionId::new("ru:sha256:abc"),
            },
            ref_name: "refs/heads/feat/x".into(),
            head_oid: "oidH".into(),
        };
        assert_eq!(p.event_type(), EventType::ReviewUnitRefAssociated);
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["refAssociationId"], "assoc-ref:sha256:yyy");
        assert_eq!(v["refName"], "refs/heads/feat/x");
        assert_eq!(v["headOid"], "oidH");
        let back: ReviewUnitRefAssociatedPayload = serde_json::from_value(v).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn withdrawal_payloads_are_ids_only_and_round_trip() {
        let commit = ReviewUnitCommitWithdrawnPayload {
            commit_withdrawal_id: CommitWithdrawalId::new("withdraw-commit:sha256:w1"),
            target: ReviewTargetRef::Revision {
                revision_id: RevisionId::new("ru:sha256:abc"),
            },
            commit_association_id: CommitAssociationId::new("assoc-commit:sha256:zzz"),
        };
        assert_eq!(commit.event_type(), EventType::ReviewUnitCommitWithdrawn);
        let v = serde_json::to_value(&commit).unwrap();
        assert_eq!(v["commitWithdrawalId"], "withdraw-commit:sha256:w1");
        assert_eq!(v["commitAssociationId"], "assoc-commit:sha256:zzz");
        assert!(
            v.get("reason").is_none(),
            "withdrawal payload has no reason"
        );
        let back: ReviewUnitCommitWithdrawnPayload = serde_json::from_value(v).unwrap();
        assert_eq!(back, commit);

        let r = ReviewUnitRefWithdrawnPayload {
            ref_withdrawal_id: RefWithdrawalId::new("withdraw-ref:sha256:w2"),
            target: ReviewTargetRef::Revision {
                revision_id: RevisionId::new("ru:sha256:abc"),
            },
            ref_association_id: RefAssociationId::new("assoc-ref:sha256:yyy"),
        };
        assert_eq!(r.event_type(), EventType::ReviewUnitRefWithdrawn);
        let rv = serde_json::to_value(&r).unwrap();
        assert_eq!(rv["refWithdrawalId"], "withdraw-ref:sha256:w2");
        assert!(rv.get("reason").is_none());
        let rback: ReviewUnitRefWithdrawnPayload = serde_json::from_value(rv).unwrap();
        assert_eq!(rback, r);
    }

    #[test]
    fn commit_association_id_is_deterministic_and_distinguisher_scoped() {
        let ru = RevisionId::new("ru:sha256:abc");
        let a = build_commit_association_id(&ru, "oid123").unwrap();
        let b = build_commit_association_id(&ru, "oid123").unwrap();
        assert_eq!(a, b);
        assert!(a.as_str().starts_with("assoc-commit:"));
        let other = build_commit_association_id(&ru, "oid999").unwrap();
        assert_ne!(a, other);
    }

    #[test]
    fn ref_association_id_folds_name_and_head() {
        let ru = RevisionId::new("ru:sha256:abc");
        let a = build_ref_association_id(&ru, "refs/heads/feat/x", "oidH").unwrap();
        let b = build_ref_association_id(&ru, "refs/heads/feat/x", "oidH2").unwrap();
        assert_ne!(a, b);
        assert!(a.as_str().starts_with("assoc-ref:"));
        let other_ref = build_ref_association_id(&ru, "refs/heads/main", "oidH").unwrap();
        assert_ne!(a, other_ref);
    }

    #[test]
    fn withdrawal_id_folds_the_association_id() {
        let ru = RevisionId::new("ru:sha256:abc");
        let cid = build_commit_association_id(&ru, "oid123").unwrap();
        let w = build_commit_withdrawal_id(&ru, &cid).unwrap();
        assert!(w.as_str().starts_with("withdraw-commit:"));
        assert_eq!(w, build_commit_withdrawal_id(&ru, &cid).unwrap());

        let rid = build_ref_association_id(&ru, "refs/heads/feat/x", "oidH").unwrap();
        let rw = build_ref_withdrawal_id(&ru, &rid).unwrap();
        assert!(rw.as_str().starts_with("withdraw-ref:"));
        assert_eq!(rw, build_ref_withdrawal_id(&ru, &rid).unwrap());
    }

    #[test]
    fn commit_association_id_digest_is_pinned() {
        // Guards against an accidental change to the id material shape.
        let ru = RevisionId::new("ru:sha256:abc");
        let id = build_commit_association_id(&ru, "oid123").unwrap();
        assert_eq!(
            id.as_str(),
            "assoc-commit:sha256:b866412f864ba09281f6b3d710f7c0ea0e545e21fd9e5a2d69217d7f17c3443e",
            "pinned digest mismatch — id material shape changed"
        );
    }
}

/// End-to-end identity/convergence contract at the `ShoreEvent` level: two
/// independently-authored copies of one structural edge (different writer/track)
/// produce identical content identity, a true re-record converges to `Existing`,
/// and a genuinely different edge is a distinct member.
#[cfg(test)]
mod convergence_tests {
    use super::*;
    use crate::model::{ActorId, LedgerId, RevisionId, TrackId};
    use crate::session::event::{EventTarget, ShoreEvent, Writer, WriterProducer};
    use crate::session::projection::freshness::event_set_hash_for_events;
    use crate::session::{EventStore, EventWriteOutcome};

    fn review_unit() -> RevisionId {
        RevisionId::new("ru:sha256:abc")
    }

    fn target_for(review_unit_id: &RevisionId, track: &str) -> EventTarget {
        let mut target = EventTarget::for_revision(
            LedgerId::new("session:default"),
            review_unit_id.clone(),
            None,
        );
        target.track_id = Some(TrackId::new(track));
        target
    }

    fn writer_for(writer: &str) -> Writer {
        Writer {
            actor_id: ActorId::new(format!("actor:{writer}")),
            producer: WriterProducer {
                name: "shore".to_owned(),
                version: "test".to_owned(),
            },
        }
    }

    fn commit_assoc_event(commit_oid: &str, writer: &str, track: &str) -> ShoreEvent {
        let ru = review_unit();
        let cid = build_commit_association_id(&ru, commit_oid).unwrap();
        let payload = ReviewUnitCommitAssociatedPayload {
            commit_association_id: cid,
            target: ReviewTargetRef::Revision {
                revision_id: ru.clone(),
            },
            commit: ReviewEndpoint::GitCommit {
                commit_oid: commit_oid.to_owned(),
                tree_oid: "tree9".to_owned(),
            },
        };
        ShoreEvent::new(
            EventType::ReviewUnitCommitAssociated,
            ReviewUnitCommitAssociatedPayload::idempotency_key(&ru, commit_oid),
            target_for(&ru, track),
            writer_for(writer),
            payload,
            "2026-06-19T00:00:00Z",
        )
        .unwrap()
    }

    fn commit_withdraw_event(commit_oid: &str, writer: &str, track: &str) -> ShoreEvent {
        let ru = review_unit();
        let cid = build_commit_association_id(&ru, commit_oid).unwrap();
        let wid = build_commit_withdrawal_id(&ru, &cid).unwrap();
        let payload = ReviewUnitCommitWithdrawnPayload {
            commit_withdrawal_id: wid,
            target: ReviewTargetRef::Revision {
                revision_id: ru.clone(),
            },
            commit_association_id: cid.clone(),
        };
        ShoreEvent::new(
            EventType::ReviewUnitCommitWithdrawn,
            ReviewUnitCommitWithdrawnPayload::idempotency_key(&cid),
            target_for(&ru, track),
            writer_for(writer),
            payload,
            "2026-06-19T00:00:00Z",
        )
        .unwrap()
    }

    #[test]
    fn same_edge_converges_across_writers() {
        let a = commit_assoc_event("oid123", "alice", "author");
        let b = commit_assoc_event("oid123", "bob", "reviewer");

        assert_eq!(a.event_id, b.event_id, "event id is key-derived");
        assert_eq!(
            a.payload_hash, b.payload_hash,
            "writer/track not in payload"
        );
        assert_ne!(
            a.writer, b.writer,
            "envelope writer differs — and that is fine"
        );
        assert_eq!(
            event_set_hash_for_events([&a]).unwrap(),
            event_set_hash_for_events([&b]).unwrap(),
            "event-set contribution is identical"
        );
    }

    #[test]
    fn re_record_returns_existing_and_distinct_edge_is_a_new_member() {
        let root = tempfile::tempdir().unwrap();
        let store = EventStore::open(root.path().join(".shore/data"));

        let a = commit_assoc_event("oid123", "alice", "author");
        let b = commit_assoc_event("oid123", "bob", "reviewer");
        let other = commit_assoc_event("oid999", "alice", "author");

        assert_eq!(
            store.record_event_once(&a).unwrap(),
            EventWriteOutcome::Created
        );
        assert_eq!(
            store.record_event_once(&b).unwrap(),
            EventWriteOutcome::Existing,
            "same edge by another writer converges to Existing"
        );
        assert_eq!(
            store.record_event_once(&other).unwrap(),
            EventWriteOutcome::Created,
            "a distinct OID is a new member, not a conflict"
        );
    }

    #[test]
    fn withdrawal_is_separable_and_convergent() {
        let a = commit_withdraw_event("oid123", "alice", "author");
        let b = commit_withdraw_event("oid123", "bob", "reviewer");

        assert_eq!(
            a.event_id, b.event_id,
            "withdrawal converges across writers"
        );
        assert_eq!(a.payload_hash, b.payload_hash, "ids-only payload converges");

        let assoc = commit_assoc_event("oid123", "alice", "author");
        assert_ne!(
            a.idempotency_key, assoc.idempotency_key,
            "withdrawal key never collides with the association key"
        );

        let root = tempfile::tempdir().unwrap();
        let store = EventStore::open(root.path().join(".shore/data"));
        assert_eq!(
            store.record_event_once(&a).unwrap(),
            EventWriteOutcome::Created
        );
        assert_eq!(
            store.record_event_once(&b).unwrap(),
            EventWriteOutcome::Existing
        );
    }
}
