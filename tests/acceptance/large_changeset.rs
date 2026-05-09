use shore::git::ingest_tracked_diff;
use shore::model::{Annotation, CursorState, DiffSnapshot, ReviewRowKind, ReviewStream, RowId};
use shore::sidecar::agent_context::{
    AgentAnnotation, AgentContext, AgentFileContext, Range, apply_file_order, resolve_annotations,
};
use shore::stream::{LayoutSnapshot, NavigationCommand, ViewportSpec};

use crate::support::git_repo::GitRepo;
use crate::support::snapshots::{StreamSummary, stream_summary};

#[test]
fn bounded_large_changeset_exercises_full_read_only_pipeline() {
    let repo = bounded_changeset_repo();
    let snapshot = ingest_tracked_diff(repo.path()).expect("large changeset should ingest");
    let context = large_changeset_context();

    assert_eq!(snapshot.files.len(), 5);
    assert_eq!(hunk_count(&snapshot), 5);

    let ordered = apply_file_order(snapshot.files.clone(), &context);
    assert!(ordered.diagnostics.is_empty(), "{:#?}", ordered.diagnostics);
    let ordered_snapshot = DiffSnapshot::new(
        snapshot.review_id.clone(),
        snapshot.snapshot_id.clone(),
        ordered.files,
    );
    let resolved = resolve_annotations(&ordered_snapshot.files, &context);
    assert!(
        resolved.diagnostics.is_empty(),
        "{:#?}",
        resolved.diagnostics
    );
    assert_eq!(resolved.annotations.len(), 4);

    let built = ReviewStream::from_snapshot_and_sidecar(&snapshot, &context);
    assert!(built.diagnostics.is_empty(), "{:#?}", built.diagnostics);
    assert_eq!(
        file_header_paths(&built.stream),
        vec![
            "src/untracked.rs",
            "src/file_b.rs",
            "src/file_a.rs",
            "assets/data.bin",
            "scripts/run.sh",
        ]
    );

    let summary = stream_summary(&built.stream);
    assert_eq!(
        summary,
        StreamSummary {
            file_headers: 5,
            hunk_headers: 5,
            diff_rows: 33,
            metadata_rows: 2,
            annotation_rows: 4,
            empty_rows: 0,
            total_rows: 49,
        }
    );

    assert_navigation_endpoints(&built.stream);

    let layout = LayoutSnapshot::from_stream(&built.stream, ViewportSpec::new(100, 8));
    assert_eq!(layout.content_height, summary.total_rows);
    assert_eq!(layout.row_spans.len(), summary.total_rows);

    let stream_json = serde_json::to_string(&built.stream).expect("stream serializes");
    let decoded_stream: ReviewStream =
        shore::model::decode_json(&stream_json).expect("stream deserializes");
    assert_eq!(decoded_stream, built.stream);

    let snapshot_json =
        serde_json::to_string(&ordered_snapshot).expect("ordered snapshot serializes");
    let decoded_snapshot: DiffSnapshot =
        shore::model::decode_json(&snapshot_json).expect("ordered snapshot deserializes");
    let annotations_json =
        serde_json::to_string(&resolved.annotations).expect("annotations serialize");
    let decoded_annotations: Vec<Annotation> =
        shore::model::decode_json(&annotations_json).expect("annotations deserialize");
    let rebuilt_stream =
        ReviewStream::from_snapshot_and_annotations(&decoded_snapshot, &decoded_annotations);

    assert_eq!(stream_summary(&rebuilt_stream), summary);
    assert_eq!(row_ids(&rebuilt_stream), row_ids(&built.stream));
}

fn bounded_changeset_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/file_a.rs", numbered_source("file_a", &[]));
    repo.write("src/file_b.rs", numbered_source("file_b", &[]));
    repo.write("assets/data.bin", [0, 159, 146, 150]);
    repo.write("scripts/run.sh", "#!/bin/sh\necho shore\n");
    repo.commit_all("base");

    repo.write(
        "src/file_a.rs",
        numbered_source("file_a", &[(2, 102), (14, 114)]),
    );
    repo.write(
        "src/file_b.rs",
        numbered_source("file_b", &[(4, 204), (17, 217)]),
    );
    repo.write("assets/data.bin", [0, 159, 146, 151]);
    make_executable(repo.path().join("scripts/run.sh"));
    repo.write(
        "src/untracked.rs",
        "pub fn untracked_01() {}\npub fn untracked_02() {}\npub fn untracked_03() {}\n",
    );

    repo
}

fn large_changeset_context() -> AgentContext {
    AgentContext {
        schema: Some("shore.agent-context".to_owned()),
        version: 1,
        summary: None,
        ownership: None,
        files: vec![
            sidecar_file("src/untracked.rs", &[("annotation-untracked", 2)]),
            sidecar_file("src/file_b.rs", &[("annotation-file-b-4", 4)]),
            sidecar_file(
                "src/file_a.rs",
                &[("annotation-file-a-2", 2), ("annotation-file-a-14", 14)],
            ),
        ],
    }
}

fn sidecar_file(path: &str, annotations: &[(&str, u32)]) -> AgentFileContext {
    AgentFileContext {
        path: path.to_owned(),
        old_path: None,
        summary: None,
        annotations: annotations
            .iter()
            .map(|(summary, line)| AgentAnnotation {
                id: Some(format!("sidecar:{summary}")),
                old_range: None,
                new_range: Some(Range {
                    start: *line,
                    end: *line,
                }),
                summary: Some((*summary).to_owned()),
                rationale: None,
                tags: Vec::new(),
                confidence: None,
                source: None,
                author: None,
                created_at: None,
            })
            .collect(),
    }
}

fn numbered_source(prefix: &str, replacements: &[(u32, u32)]) -> String {
    (1..=20)
        .map(|line| {
            let value = replacements
                .iter()
                .find_map(|(target_line, value)| (*target_line == line).then_some(*value))
                .unwrap_or(line);
            format!("pub fn {prefix}_{line:02}() -> u32 {{ {value} }}\n")
        })
        .collect()
}

fn hunk_count(snapshot: &DiffSnapshot) -> usize {
    snapshot.files.iter().map(|file| file.hunks.len()).sum()
}

fn file_header_paths(stream: &ReviewStream) -> Vec<&str> {
    stream
        .rows
        .iter()
        .filter_map(|row| match &row.kind {
            ReviewRowKind::FileHeader { path, .. } => Some(path.as_str()),
            _ => None,
        })
        .collect()
}

fn assert_navigation_endpoints(stream: &ReviewStream) {
    let hunk_rows = row_ids_matching(stream, |kind| {
        matches!(kind, ReviewRowKind::HunkHeader { .. })
    });
    assert_eq!(hunk_rows.len(), 5);

    let previous_from_first = stream.navigate(
        &CursorState::at_row(hunk_rows[0].clone()),
        NavigationCommand::PreviousHunk,
    );
    assert_eq!(
        previous_from_first.cursor,
        CursorState::at_row(hunk_rows[0].clone())
    );
    assert!(previous_from_first.clamped);

    let next_from_first = stream.navigate(
        &CursorState::at_row(hunk_rows[0].clone()),
        NavigationCommand::NextHunk,
    );
    assert_eq!(
        next_from_first.cursor,
        CursorState::at_row(hunk_rows[1].clone())
    );
    assert!(!next_from_first.clamped);

    let next_from_last = stream.navigate(
        &CursorState::at_row(hunk_rows[4].clone()),
        NavigationCommand::NextHunk,
    );
    assert_eq!(
        next_from_last.cursor,
        CursorState::at_row(hunk_rows[4].clone())
    );
    assert!(next_from_last.clamped);

    let annotation_rows = row_ids_matching(stream, |kind| {
        matches!(kind, ReviewRowKind::Annotation { .. })
    });
    assert_eq!(annotation_rows.len(), 4);

    let first_annotated = stream.navigate(
        &CursorState::at_row(RowId::new("row:0000")),
        NavigationCommand::NextAnnotatedHunk,
    );
    assert_eq!(
        first_annotated.cursor,
        CursorState::at_row(annotation_rows[0].clone())
    );
    assert!(!first_annotated.clamped);

    let last_annotated = annotation_rows
        .last()
        .expect("annotation row exists")
        .clone();
    let next_from_last_annotated = stream.navigate(
        &CursorState::at_row(last_annotated.clone()),
        NavigationCommand::NextAnnotatedHunk,
    );
    assert_eq!(
        next_from_last_annotated.cursor,
        CursorState::at_row(last_annotated)
    );
    assert!(next_from_last_annotated.clamped);
}

fn row_ids_matching(
    stream: &ReviewStream,
    predicate: impl Fn(&ReviewRowKind) -> bool,
) -> Vec<RowId> {
    stream
        .rows
        .iter()
        .filter_map(|row| predicate(&row.kind).then_some(row.id.clone()))
        .collect()
}

fn row_ids(stream: &ReviewStream) -> Vec<RowId> {
    stream.rows.iter().map(|row| row.id.clone()).collect()
}

#[cfg(unix)]
fn make_executable(path: impl AsRef<std::path::Path>) {
    use std::os::unix::fs::PermissionsExt;

    let path = path.as_ref();
    let mut permissions = std::fs::metadata(path)
        .expect("read file metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("set executable bit");
}

#[cfg(not(unix))]
fn make_executable(path: impl AsRef<std::path::Path>) {
    let _ = path;
}
