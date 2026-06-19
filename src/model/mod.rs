mod cursor;
mod file;
mod hunk;
mod ids;
mod lineage;
mod review;
mod review_note;
mod review_unit;
mod row;
mod validation;
mod work_object;

pub fn decode_json<T>(json: &str) -> crate::error::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    Ok(serde_json::from_str(json)?)
}

pub use cursor::CursorState;
pub use file::{DiffFile, FileStatus};
pub use hunk::ReviewHunk;
pub use ids::{
    ActorId, AssessmentId, CheckpointId, CommitAssociationId, CommitWithdrawalId, EventId, FileId,
    HunkId, InputRequestId, InputRequestResponseId, ObservationId, RefAssociationId, RefWithdrawalId,
    ReviewId, ReviewNoteId, ReviewUnitId, ReviewUnitLineageId, ReviewUnitLineageRoundId, RevisionId,
    RowId, SessionId, SnapshotId, TrackId, ValidationCheckId, WorkObjectId, WorkUnitId,
};
pub use lineage::ReviewUnitLineageBasisV1;
pub use review::{DiffSnapshot, Review, ReviewStream};
pub use review_note::{
    Anchor, AnchorResolution, AnchorResolutionReason, LineRange, ResolutionStatus, ReviewNote,
    ReviewNoteSource, Side, re_resolve_review_notes,
};
pub(crate) use review_note::{hash_normalized_lines, rows_for_line_range};
pub use review_unit::{
    CommitRangeCaptureMode, ReviewEndpoint, ReviewTargetRef, ReviewUnitSource, WorktreeCaptureMode,
};
pub use row::{DiffRow, DiffRowKind, FileMetadataKind, FileMetadataRow, ReviewRow, ReviewRowKind};
pub use validation::{ValidationStatus, ValidationTarget, ValidationTrigger};
pub use work_object::{TargetRef, TaskTargetRef, WorkObjectType};
