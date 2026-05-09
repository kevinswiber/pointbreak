use shore::model::{ReviewRowKind, ReviewStream};

#[derive(Debug, Eq, PartialEq)]
pub struct StreamSummary {
    pub file_headers: usize,
    pub hunk_headers: usize,
    pub diff_rows: usize,
    pub metadata_rows: usize,
    pub annotation_rows: usize,
    pub empty_rows: usize,
    pub total_rows: usize,
}

pub fn stream_summary(stream: &ReviewStream) -> StreamSummary {
    let mut summary = StreamSummary {
        file_headers: 0,
        hunk_headers: 0,
        diff_rows: 0,
        metadata_rows: 0,
        annotation_rows: 0,
        empty_rows: 0,
        total_rows: stream.rows.len(),
    };

    for row in &stream.rows {
        match row.kind {
            ReviewRowKind::FileHeader { .. } => summary.file_headers += 1,
            ReviewRowKind::HunkHeader { .. } => summary.hunk_headers += 1,
            ReviewRowKind::Diff { .. } => summary.diff_rows += 1,
            ReviewRowKind::Metadata { .. } => summary.metadata_rows += 1,
            ReviewRowKind::Annotation { .. } => summary.annotation_rows += 1,
            ReviewRowKind::EmptyState { .. } => summary.empty_rows += 1,
        }
    }

    summary
}

pub fn normalize_path(path: impl AsRef<std::path::Path>) -> String {
    path.as_ref().to_string_lossy().replace('\\', "/")
}
