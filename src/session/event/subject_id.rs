//! The opaque signed-target subject id (seam **S2**).
//!
//! The signed envelope binds an opaque `subjectId` — a sha256 over the subject's
//! **identity-bearing fields only** — in place of the structural subject, so a
//! future display rename of a subject's kind tag is projection-only. The
//! structural subject is carried in the event payload and reconstructed by the
//! projection for display.
//!
//! The derivation is uniform across domains: every [`TargetRef`] variant is
//! self-identifying (review sub-anchors carry a `revision_id` + sub-ids; task
//! subjects carry a `task_attempt_id` / `checkpoint_id`), so the id folds the
//! variant's identity fields with the renamable `kind` tag stripped. Domain
//! stays structurally derived (ADR-0017 §A4), never folded here. A
//! `TargetRef::Journal` carrier has no subject, so it yields `None`.

use crate::canonical_hash::sha256_json_prefixed;
use crate::error::Result;
use crate::model::{TargetRef, id_prefix};

/// Derive the opaque `subjectId` for a subject, or `None` for the fieldless
/// `TargetRef::Journal` carrier. The digest folds the subject's identity-bearing
/// fields only, never the renamable kind tag or the derived domain.
pub(crate) fn subject_id(subject: &TargetRef) -> Result<Option<String>> {
    // Fold the sub-anchor's identity fields; drop the renamable `kind` tag so a
    // future rename of the variant/kind is projection-only.
    let mut material = match subject {
        TargetRef::Journal => return Ok(None),
        TargetRef::Review(review) => serde_json::to_value(review)?,
        TargetRef::Task(task) => serde_json::to_value(task)?,
    };
    if let Some(object) = material.as_object_mut() {
        object.remove("kind");
    }

    let digest = sha256_json_prefixed(&material)?;
    Ok(Some(format!("{}:{digest}", id_prefix::SUBJECT)))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::model::{
        CheckpointId, ObservationId, ReviewTargetRef, RevisionId, Side, TaskTargetRef, WorkObjectId,
    };

    fn revision(id: &str) -> RevisionId {
        RevisionId::new(id)
    }

    #[test]
    fn subject_id_absent_for_journal_carrier() {
        assert_eq!(subject_id(&TargetRef::Journal).unwrap(), None);
    }

    #[test]
    fn subject_id_is_a_prefixed_content_id() {
        let subject = TargetRef::Review(ReviewTargetRef::Revision {
            revision_id: revision("rev:sha256:abc"),
        });

        let id = subject_id(&subject).unwrap().unwrap();
        assert!(id.starts_with("subject:sha256:"), "got {id}");
    }

    #[test]
    fn subject_id_excludes_the_renamable_kind_tag() {
        // The digest must fold identity fields only, never the `kind` tag: a
        // subject_id equals the sha256 of the same fields with `kind` stripped.
        let subject = TargetRef::Review(ReviewTargetRef::File {
            revision_id: revision("rev:sha256:abc"),
            file_path: "src/lib.rs".to_owned(),
        });

        let expected_material = json!({
            "revisionId": "rev:sha256:abc",
            "filePath": "src/lib.rs",
        });
        let expected = format!(
            "{}:{}",
            id_prefix::SUBJECT,
            sha256_json_prefixed(&expected_material).unwrap()
        );

        assert_eq!(subject_id(&subject).unwrap().unwrap(), expected);
    }

    #[test]
    fn subject_id_changes_with_revision_id() {
        let subject_x = TargetRef::Review(ReviewTargetRef::Revision {
            revision_id: revision("rev:sha256:x"),
        });
        let subject_y = TargetRef::Review(ReviewTargetRef::Revision {
            revision_id: revision("rev:sha256:y"),
        });

        assert_ne!(
            subject_id(&subject_x).unwrap(),
            subject_id(&subject_y).unwrap()
        );
    }

    #[test]
    fn subject_id_distinguishes_review_sub_anchors() {
        // A File anchor and a Range anchor on the same revision+path are different
        // subjects (Range carries side/line fields), so their ids must differ.
        let file = TargetRef::Review(ReviewTargetRef::File {
            revision_id: revision("rev:sha256:abc"),
            file_path: "src/lib.rs".to_owned(),
        });
        let range = TargetRef::Review(ReviewTargetRef::Range {
            revision_id: revision("rev:sha256:abc"),
            file_path: "src/lib.rs".to_owned(),
            side: Side::New,
            start_line: 1,
            end_line: 4,
        });

        assert_ne!(subject_id(&file).unwrap(), subject_id(&range).unwrap());
    }

    #[test]
    fn review_and_task_subjects_do_not_collide() {
        let review = TargetRef::Review(ReviewTargetRef::Observation {
            revision_id: revision("rev:sha256:abc"),
            observation_id: ObservationId::new("obs:sha256:abc"),
        });
        let task = TargetRef::Task(TaskTargetRef::TaskAttempt {
            task_attempt_id: WorkObjectId::new("task-attempt:sha256:abc"),
        });

        assert_ne!(subject_id(&review).unwrap(), subject_id(&task).unwrap());
    }

    #[test]
    fn subject_id_distinguishes_task_attempts() {
        let attempt_a = TargetRef::Task(TaskTargetRef::TaskAttempt {
            task_attempt_id: WorkObjectId::new("task-attempt:sha256:a"),
        });
        let attempt_b = TargetRef::Task(TaskTargetRef::TaskAttempt {
            task_attempt_id: WorkObjectId::new("task-attempt:sha256:b"),
        });

        assert_ne!(
            subject_id(&attempt_a).unwrap(),
            subject_id(&attempt_b).unwrap()
        );
    }

    #[test]
    fn task_checkpoint_distinguishes_from_bare_attempt() {
        let bare = TargetRef::Task(TaskTargetRef::TaskAttempt {
            task_attempt_id: WorkObjectId::new("task-attempt:sha256:abc"),
        });
        let checkpoint = TargetRef::Task(TaskTargetRef::Checkpoint {
            checkpoint_id: CheckpointId::new("checkpoint:sha256:c"),
        });

        assert_ne!(subject_id(&bare).unwrap(), subject_id(&checkpoint).unwrap());
    }
}
