use serde::{Deserialize, Serialize};

use super::{FileId, ReviewHunk};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiffFile {
    pub id: FileId,
    pub status: FileStatus,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    pub old_oid: Option<String>,
    pub new_oid: Option<String>,
    pub similarity: Option<u16>,
    pub is_binary: bool,
    pub is_submodule: bool,
    pub is_mode_only: bool,
    pub metadata_rows: Vec<super::FileMetadataRow>,
    pub hunks: Vec<ReviewHunk>,
}
