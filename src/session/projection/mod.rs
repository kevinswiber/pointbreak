pub mod artifact_removal;
pub(crate) mod body_content;
pub(crate) mod commit_oid_grouping;
pub(crate) mod commit_range;
pub(crate) mod cosignature;
pub(crate) mod engagement;
pub(crate) mod freshness;
pub(crate) mod liveness;
mod read;
pub(crate) mod revisions_by_base;
pub mod state;
pub(crate) mod supersession;
pub(crate) mod task;
#[cfg(test)]
pub(crate) mod test_support;

pub use artifact_removal::{ArtifactRemovalProjection, RemovalClaim, RemovalOperativeStatus};
pub use body_content::BodyContentState;
pub use commit_oid_grouping::CommitOidGroupingProjection;
pub use commit_range::{
    CommitEdgeSource, CurrentCommitAssociation, CurrentRefAssociation,
    RevisionCommitRangeProjection, RevisionCommitRangeView, WithdrawnCommitAssociation,
    WithdrawnRefAssociation,
};
pub use engagement::{EngagementGrouping, EngagementLifecycle, EngagementView};
pub use liveness::{LivenessScope, LivenessToken};
pub(crate) use read::skipped_to_diagnostics;
pub use read::{load_durable_notes_for_repo, read_events, read_events_for_display, rebuild_state};
pub use revisions_by_base::RevisionsByBase;
pub use state::{ProjectionDiagnostic, SessionState};
pub use supersession::SupersessionView;
