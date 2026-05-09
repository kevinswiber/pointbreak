use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use shore::model::{
    Anchor, CursorState, DiffFile, DiffRow, DiffRowKind, DiffSnapshot, FileId, FileMetadataKind,
    FileMetadataRow, FileStatus, HunkId, LineRange, ResolutionStatus, Review, ReviewHunk, ReviewId,
    ReviewNote, ReviewNoteId, ReviewNoteSource, ReviewStream, RowId, Side, SnapshotId,
};
use shore::stream::{LayoutSnapshot, ViewportSpec};

#[test]
fn top_level_review_models_round_trip_through_json() {
    let review = Review {
        id: ReviewId::new("review-1"),
    };
    let snapshot = DiffSnapshot::empty(review.id.clone());
    let stream = ReviewStream::empty(review.id.clone());

    let review_json = serde_json::to_string(&review).expect("review serializes");
    let snapshot_json = serde_json::to_string(&snapshot).expect("snapshot serializes");
    let stream_json = serde_json::to_string(&stream).expect("stream serializes");

    let decoded_review: Review = serde_json::from_str(&review_json).expect("review deserializes");
    let decoded_snapshot: DiffSnapshot =
        serde_json::from_str(&snapshot_json).expect("snapshot deserializes");
    let decoded_stream: ReviewStream =
        serde_json::from_str(&stream_json).expect("stream deserializes");

    assert_eq!(decoded_review.id, review.id);
    assert_eq!(decoded_snapshot.review_id, review.id);
    assert_eq!(decoded_stream.review_id, review.id);
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct DurableIds {
    review_id: ReviewId,
    file_id: FileId,
    review_note_id: ReviewNoteId,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct SnapshotLocalIds {
    snapshot_id: SnapshotId,
    hunk_id: HunkId,
    row_id: RowId,
}

#[test]
fn durable_and_snapshot_local_ids_round_trip_as_distinct_types() {
    let durable = DurableIds {
        review_id: ReviewId::new("review-1"),
        file_id: FileId::new("src/lib.rs"),
        review_note_id: ReviewNoteId::new("note-1"),
    };
    let snapshot_local = SnapshotLocalIds {
        snapshot_id: SnapshotId::new("snapshot-1"),
        hunk_id: HunkId::new("hunk-1"),
        row_id: RowId::new("row-1"),
    };

    let durable_json = serde_json::to_string(&durable).expect("durable ids serialize");
    let snapshot_json =
        serde_json::to_string(&snapshot_local).expect("snapshot-local ids serialize");

    assert_eq!(
        serde_json::from_str::<DurableIds>(&durable_json).expect("durable ids deserialize"),
        durable
    );
    assert_eq!(
        serde_json::from_str::<SnapshotLocalIds>(&snapshot_json)
            .expect("snapshot-local ids deserialize"),
        snapshot_local
    );
}

#[test]
fn anchors_store_durable_location_without_row_ids() {
    let anchor = Anchor {
        file_id: FileId::new("src/lib.rs"),
        side: Side::New,
        line_range: LineRange::new(10, 12),
        hunk_signature: "sha256:hunk-body".to_owned(),
        target_text_hash: "sha256:target-text".to_owned(),
        status: ResolutionStatus::Exact,
    };

    let json = serde_json::to_value(&anchor).expect("anchor serializes");

    assert_eq!(json["file_id"], "src/lib.rs");
    assert_eq!(json["side"], "new");
    assert_eq!(json["line_range"]["start"], 10);
    assert_eq!(json["line_range"]["end"], 12);
    assert!(json.get("row_id").is_none());

    let decoded: Anchor = serde_json::from_value(json).expect("anchor deserializes");
    assert_eq!(decoded, anchor);
}

#[test]
fn review_notes_store_title_body_and_anchor_without_annotation_names() {
    let note = ReviewNote {
        id: ReviewNoteId::new("note-1"),
        anchor: Anchor {
            file_id: FileId::new("src/lib.rs"),
            side: Side::New,
            line_range: LineRange::new(10, 12),
            hunk_signature: "sha256:hunk-body".to_owned(),
            target_text_hash: "sha256:target-text".to_owned(),
            status: ResolutionStatus::Exact,
        },
        source: ReviewNoteSource::Sidecar,
        title: "check new call".to_owned(),
        body: Some("full note body".to_owned()),
        tags: vec!["serialization".to_owned()],
        confidence: Some("high".to_owned()),
        external_source: Some("codex".to_owned()),
        author: None,
        created_at: None,
    };

    let json = serde_json::to_value(&note).expect("note serializes");

    assert_eq!(json["title"], "check new call");
    assert_eq!(json["body"], "full note body");
    assert!(json.get("summary").is_none());
    assert!(json.get("rationale").is_none());

    let decoded: ReviewNote = serde_json::from_value(json).expect("note deserializes");
    assert_eq!(decoded, note);
}

#[test]
fn core_review_state_round_trips_with_notes_cursor_and_layout() {
    let review = Review {
        id: ReviewId::new("review-full"),
    };
    let snapshot = snapshot_with_diff(review.id.clone());
    let notes = vec![review_note_for_snapshot(&snapshot)];
    let stream = ReviewStream::from_snapshot_with_resolved_notes(&snapshot, &notes);
    let note_row_id = stream
        .rows
        .iter()
        .find(|row| {
            matches!(
                row.kind,
                shore::model::ReviewRowKind::Note {
                    ref title,
                    ..
                } if title == "check new call"
            )
        })
        .expect("stream should contain note row")
        .id
        .clone();
    let cursor = CursorState::at_row(note_row_id);
    let layout = LayoutSnapshot::from_stream(&stream, ViewportSpec::new(80, 10));

    let decoded_review = round_trip(&review);
    let decoded_snapshot = round_trip(&snapshot);
    let decoded_notes = round_trip(&notes);
    let decoded_stream = round_trip(&stream);
    let decoded_cursor = round_trip(&cursor);
    let decoded_layout = round_trip(&layout);

    assert_eq!(decoded_review, review);
    assert_eq!(decoded_snapshot.review_id, review.id);
    assert_eq!(
        decoded_snapshot.snapshot_id,
        SnapshotId::new("snapshot-full")
    );
    assert_eq!(decoded_notes[0].id, ReviewNoteId::new("note-1"));
    assert_eq!(decoded_notes[0].anchor.file_id, FileId::new("src/lib.rs"));
    assert_eq!(decoded_stream, stream);
    assert_eq!(decoded_cursor, cursor);
    assert_eq!(decoded_layout, layout);

    let rebuilt_stream =
        ReviewStream::from_snapshot_with_resolved_notes(&decoded_snapshot, &decoded_notes);
    assert_eq!(row_ids(&rebuilt_stream), row_ids(&stream));
    assert_eq!(rebuilt_stream, stream);
}

#[test]
fn malformed_model_json_returns_shore_error() {
    let error = shore::model::decode_json::<ReviewStream>("{\"review_id\":")
        .expect_err("malformed JSON should fail");

    assert!(error.to_string().contains("json parse failed"));
}

fn round_trip<T>(value: &T) -> T
where
    T: Serialize + DeserializeOwned,
{
    let json = serde_json::to_string(value).expect("value serializes");
    serde_json::from_str(&json).expect("value deserializes")
}

fn row_ids(stream: &ReviewStream) -> Vec<RowId> {
    stream.rows.iter().map(|row| row.id.clone()).collect()
}

fn snapshot_with_diff(review_id: ReviewId) -> DiffSnapshot {
    DiffSnapshot::new(
        review_id,
        SnapshotId::new("snapshot-full"),
        vec![DiffFile {
            id: FileId::new("src/lib.rs"),
            status: FileStatus::Modified,
            old_path: Some("src/lib.rs".to_owned()),
            new_path: Some("src/lib.rs".to_owned()),
            old_mode: Some("100644".to_owned()),
            new_mode: Some("100644".to_owned()),
            old_oid: Some("old-oid".to_owned()),
            new_oid: Some("new-oid".to_owned()),
            similarity: None,
            is_binary: false,
            is_submodule: false,
            is_mode_only: false,
            synthetic: false,
            metadata_rows: vec![FileMetadataRow {
                kind: FileMetadataKind::ModeChange,
                text: "mode unchanged fixture metadata".to_owned(),
            }],
            hunks: vec![ReviewHunk {
                id: HunkId::new("src/lib.rs:1:1"),
                header: "@@ -1,3 +1,4 @@".to_owned(),
                old_start: 1,
                old_lines: 3,
                new_start: 1,
                new_lines: 4,
                rows: vec![
                    DiffRow {
                        kind: DiffRowKind::Context,
                        old_line: Some(1),
                        new_line: Some(1),
                        text: "fn main() {".to_owned(),
                    },
                    DiffRow {
                        kind: DiffRowKind::Removed,
                        old_line: Some(2),
                        new_line: None,
                        text: "    old_call();".to_owned(),
                    },
                    DiffRow {
                        kind: DiffRowKind::Added,
                        old_line: None,
                        new_line: Some(2),
                        text: "    new_call();".to_owned(),
                    },
                    DiffRow {
                        kind: DiffRowKind::Added,
                        old_line: None,
                        new_line: Some(3),
                        text: "    extra_call();".to_owned(),
                    },
                    DiffRow {
                        kind: DiffRowKind::Context,
                        old_line: Some(3),
                        new_line: Some(4),
                        text: "}".to_owned(),
                    },
                ],
            }],
        }],
    )
}

fn review_note_for_snapshot(snapshot: &DiffSnapshot) -> ReviewNote {
    ReviewNote {
        id: ReviewNoteId::new("note-1"),
        anchor: Anchor {
            file_id: FileId::new("src/lib.rs"),
            side: Side::New,
            line_range: LineRange::new(2, 2),
            hunk_signature: snapshot.files[0].hunks[0].signature(),
            target_text_hash: "sha256:new-call".to_owned(),
            status: ResolutionStatus::Exact,
        },
        source: ReviewNoteSource::Sidecar,
        title: "check new call".to_owned(),
        body: Some("round-trip fixture body".to_owned()),
        tags: vec!["serialization".to_owned()],
        confidence: Some("high".to_owned()),
        external_source: Some("test".to_owned()),
        author: Some("codex".to_owned()),
        created_at: Some("2026-05-08T00:00:00Z".to_owned()),
    }
}
