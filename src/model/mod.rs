mod cursor;
mod file;
mod hunk;
mod ids;
mod review;
mod review_note;
mod review_unit;
mod row;
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
    ActorId, AssessmentId, CheckpointId, EventId, FileId, HunkId, InterventionId,
    InterventionResolutionId, ObservationId, ReviewId, ReviewNoteId, ReviewUnitId, RevisionId,
    RowId, SessionId, SnapshotId, TrackId, WorkObjectId, WorkUnitId,
};
pub use review::{DiffSnapshot, Review, ReviewStream};
pub use review_note::{
    Anchor, AnchorResolution, AnchorResolutionReason, LineRange, ResolutionStatus, ReviewNote,
    ReviewNoteSource, Side, re_resolve_review_notes,
};
pub(crate) use review_note::{hash_normalized_lines, rows_for_line_range};
pub use review_unit::{ReviewEndpoint, ReviewTargetRef, ReviewUnitSource, WorktreeCaptureMode};
pub use row::{DiffRow, DiffRowKind, FileMetadataKind, FileMetadataRow, ReviewRow, ReviewRowKind};
pub use work_object::{TargetRef, TaskTargetRef, WorkObjectType};
