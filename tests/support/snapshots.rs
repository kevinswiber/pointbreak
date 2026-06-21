use shoreline::model::{ReviewRowKind, ReviewStream};

#[derive(Debug, Eq, PartialEq)]
pub struct StreamSummary {
    pub file_headers: usize,
    pub hunk_headers: usize,
    pub diff_rows: usize,
    pub metadata_rows: usize,
    pub note_rows: usize,
    pub stale_note_rows: usize,
    pub empty_rows: usize,
    pub total_rows: usize,
}

pub fn stream_summary(stream: &ReviewStream) -> StreamSummary {
    let mut summary = StreamSummary {
        file_headers: 0,
        hunk_headers: 0,
        diff_rows: 0,
        metadata_rows: 0,
        note_rows: 0,
        stale_note_rows: 0,
        empty_rows: 0,
        total_rows: stream.rows.len(),
    };

    for row in &stream.rows {
        match row.kind {
            ReviewRowKind::FileHeader { .. } => summary.file_headers += 1,
            ReviewRowKind::HunkHeader { .. } => summary.hunk_headers += 1,
            ReviewRowKind::Diff { .. } => summary.diff_rows += 1,
            ReviewRowKind::Metadata { .. } => summary.metadata_rows += 1,
            ReviewRowKind::Note { .. } => summary.note_rows += 1,
            ReviewRowKind::StaleNote { .. } => summary.stale_note_rows += 1,
            ReviewRowKind::EmptyState { .. } => summary.empty_rows += 1,
        }
    }

    summary
}

pub fn normalize_path(path: impl AsRef<std::path::Path>) -> String {
    path.as_ref().to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use shoreline::model::{
        FileId, FileStatus, LineRange, ObjectId, ResolutionStatus, ReviewId, ReviewNoteId,
        ReviewRow, ReviewRowKind, ReviewStream, RowId,
    };

    use super::*;

    #[test]
    fn stream_summary_counts_stale_note_rows() {
        let review_id = ReviewId::new("review:test");
        let snapshot_id = ObjectId::new("snapshot:test");
        let stream = ReviewStream {
            review_id,
            snapshot_id,
            rows: vec![
                ReviewRow {
                    id: RowId::new("row:0000"),
                    ordinal: 0,
                    file_id: Some(FileId::new("src/lib.rs")),
                    hunk_id: None,
                    kind: ReviewRowKind::FileHeader {
                        path: "src/lib.rs".to_owned(),
                        status: FileStatus::Modified,
                    },
                },
                ReviewRow {
                    id: RowId::new("row:0001"),
                    ordinal: 1,
                    file_id: Some(FileId::new("src/lib.rs")),
                    hunk_id: None,
                    kind: ReviewRowKind::StaleNote {
                        note_id: ReviewNoteId::new("note:stale"),
                        title: "Stale".to_owned(),
                        resolution_status: ResolutionStatus::Stale,
                        target_path: "src/lib.rs".to_owned(),
                        target_line_range: LineRange::new(99, 99),
                    },
                },
                ReviewRow {
                    id: RowId::new("row:0002"),
                    ordinal: 2,
                    file_id: None,
                    hunk_id: None,
                    kind: ReviewRowKind::StaleNote {
                        note_id: ReviewNoteId::new("note:orphan"),
                        title: "Orphan".to_owned(),
                        resolution_status: ResolutionStatus::Orphaned,
                        target_path: "src/gone.rs".to_owned(),
                        target_line_range: LineRange::new(1, 1),
                    },
                },
            ],
        };

        let summary = stream_summary(&stream);

        assert_eq!(summary.file_headers, 1);
        assert_eq!(summary.stale_note_rows, 2);
        assert_eq!(summary.total_rows, 3);
    }
}
