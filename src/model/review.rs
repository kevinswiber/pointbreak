use serde::{Deserialize, Serialize};

use super::{DiffFile, ObjectId, ReviewId, ReviewRow};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub id: ReviewId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiffSnapshot {
    pub review_id: ReviewId,
    // The content-only identity. The `DiffSnapshot` type keeps its name (it is a
    // captured point-in-time diff body), but its id is an object id.
    pub object_id: ObjectId,
    pub files: Vec<DiffFile>,
}

impl DiffSnapshot {
    pub fn empty(review_id: ReviewId) -> Self {
        Self {
            review_id,
            object_id: ObjectId::new("empty"),
            files: Vec::new(),
        }
    }

    pub fn new(review_id: ReviewId, object_id: ObjectId, files: Vec<DiffFile>) -> Self {
        Self {
            review_id,
            object_id,
            files,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewStream {
    pub review_id: ReviewId,
    pub object_id: ObjectId,
    pub rows: Vec<ReviewRow>,
}

impl ReviewStream {
    pub fn empty(review_id: ReviewId) -> Self {
        Self {
            review_id,
            object_id: ObjectId::new("empty"),
            rows: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_snapshot_field_is_object_id_and_wire_is_unchanged() {
        let snapshot = DiffSnapshot::new(
            ReviewId::new("review:test"),
            ObjectId::new("obj:sha256:test"),
            Vec::new(),
        );
        // The content-only identity reads as `object_id` (the field rename).
        let _ = &snapshot.object_id;
        let json = serde_json::to_value(&snapshot).unwrap();
        // Wire key is unchanged: it was `object_id` via a serde-rename shim, and
        // is native after dropping the shim.
        assert!(json.get("object_id").is_some());
        assert!(json.get("snapshot_id").is_none());
    }
}
