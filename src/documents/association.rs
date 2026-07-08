// Document builders for `shore association` verbs.
use crate::documents::{DiagnosticDocument, EventWriteDocument};
use crate::session::{
    AssociateCommitResult, AssociateRefResult, CurrentCommitAssociation, CurrentRefAssociation,
    ListAssociationsResult, WithdrawCommitResult, WithdrawRefResult, WithdrawnCommitAssociation,
    WithdrawnRefAssociation,
};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssociateCommitBody {
    revision_id: String,
    commit_association_id: String,
    commit_oid: String,
    tree_oid: String,
    event_id: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WithdrawCommitBody {
    revision_id: String,
    commit_withdrawal_id: String,
    commit_association_id: String,
    event_id: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssociateRefBody {
    revision_id: String,
    ref_association_id: String,
    ref_name: String,
    head_oid: String,
    event_id: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WithdrawRefBody {
    revision_id: String,
    ref_withdrawal_id: String,
    ref_association_id: String,
    event_id: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListAssociationsBody {
    revision_id: String,
    anchored: bool,
    current_commits: Vec<CurrentCommitAssociation>,
    current_refs: Vec<CurrentRefAssociation>,
    withdrawn_commits: Vec<WithdrawnCommitAssociation>,
    withdrawn_refs: Vec<WithdrawnRefAssociation>,
}

pub fn associate_commit_document(
    result: AssociateCommitResult,
) -> EventWriteDocument<AssociateCommitBody> {
    EventWriteDocument::new(
        "pointbreak.review-association-commit",
        AssociateCommitBody {
            revision_id: result.revision_id.as_str().to_owned(),
            commit_association_id: result.commit_association_id.as_str().to_owned(),
            commit_oid: result.commit_oid,
            tree_oid: result.tree_oid,
            event_id: result.event_id.as_str().to_owned(),
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}

pub fn withdraw_commit_document(
    result: WithdrawCommitResult,
) -> EventWriteDocument<WithdrawCommitBody> {
    EventWriteDocument::new(
        "pointbreak.review-association-commit-withdrawn",
        WithdrawCommitBody {
            revision_id: result.revision_id.as_str().to_owned(),
            commit_withdrawal_id: result.commit_withdrawal_id.as_str().to_owned(),
            commit_association_id: result.commit_association_id.as_str().to_owned(),
            event_id: result.event_id.as_str().to_owned(),
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}

pub fn associate_ref_document(result: AssociateRefResult) -> EventWriteDocument<AssociateRefBody> {
    EventWriteDocument::new(
        "pointbreak.review-association-ref",
        AssociateRefBody {
            revision_id: result.revision_id.as_str().to_owned(),
            ref_association_id: result.ref_association_id.as_str().to_owned(),
            ref_name: result.ref_name,
            head_oid: result.head_oid,
            event_id: result.event_id.as_str().to_owned(),
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}

pub fn withdraw_ref_document(result: WithdrawRefResult) -> EventWriteDocument<WithdrawRefBody> {
    EventWriteDocument::new(
        "pointbreak.review-association-ref-withdrawn",
        WithdrawRefBody {
            revision_id: result.revision_id.as_str().to_owned(),
            ref_withdrawal_id: result.ref_withdrawal_id.as_str().to_owned(),
            ref_association_id: result.ref_association_id.as_str().to_owned(),
            event_id: result.event_id.as_str().to_owned(),
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}

pub fn list_associations_document(
    result: ListAssociationsResult,
) -> DiagnosticDocument<ListAssociationsBody> {
    DiagnosticDocument::new(
        "pointbreak.review-association-list",
        ListAssociationsBody {
            revision_id: result.revision_id.as_str().to_owned(),
            anchored: result.anchored,
            current_commits: result.current_commits,
            current_refs: result.current_refs,
            withdrawn_commits: result.withdrawn_commits,
            withdrawn_refs: result.withdrawn_refs,
        },
        result.diagnostics,
    )
}
