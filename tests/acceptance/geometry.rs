use shoreline::model::{
    DiffRow, DiffRowKind, ObjectId, ReviewId, ReviewRow, ReviewRowKind, ReviewStream, RowId,
};
use shoreline::stream::{LayoutSnapshot, ViewportSpec};

#[test]
fn no_wrapping_geometry_assigns_one_visual_row_per_stream_row() {
    let stream = geometry_stream();
    let layout = LayoutSnapshot::from_stream(&stream, ViewportSpec::new(12, 3));

    assert_eq!(layout.row_spans.len(), stream.rows.len());
    assert_eq!(layout.content_height, stream.rows.len());

    for (index, span) in layout.row_spans.iter().enumerate() {
        assert_eq!(span.row_id, RowId::new(format!("row:{index:04}")));
        assert_eq!(span.ordinal, index);
        assert_eq!(span.start, index);
        assert_eq!(span.end, index + 1);
        assert_eq!(span.height, 1);
        if index > 0 {
            assert_eq!(layout.row_spans[index - 1].end, span.start);
        }
    }
}

#[test]
fn reveal_row_clamps_scroll_so_target_is_visible() {
    let stream = geometry_stream();
    let layout = LayoutSnapshot::from_stream(&stream, ViewportSpec::new(12, 3));

    let reveal = layout
        .reveal_row(&RowId::new("row:0004"))
        .expect("target row reveals");

    assert_eq!(reveal.row_id, RowId::new("row:0004"));
    assert_eq!(reveal.scroll_top, 2);
    assert_eq!(reveal.viewport_start, 2);
    assert_eq!(reveal.viewport_end, 5);
    assert!(reveal.contains_target);
}

#[test]
fn horizontal_overflow_does_not_change_vertical_geometry() {
    let stream = geometry_stream();
    let narrow = LayoutSnapshot::from_stream(&stream, ViewportSpec::new(8, 3));
    let wide = LayoutSnapshot::from_stream(&stream, ViewportSpec::new(120, 3));

    assert_eq!(narrow.content_height, wide.content_height);
    assert_eq!(narrow.row_spans, wide.row_spans);
    assert_eq!(
        narrow.reveal_row(&RowId::new("row:0004")),
        wide.reveal_row(&RowId::new("row:0004"))
    );
}

fn geometry_stream() -> ReviewStream {
    ReviewStream {
        review_id: ReviewId::new("review-1"),
        object_id: ObjectId::new("snapshot-1"),
        rows: vec![
            row(
                0,
                ReviewRowKind::EmptyState {
                    message: "empty placeholder".to_owned(),
                },
            ),
            row(
                1,
                ReviewRowKind::Diff {
                    row: DiffRow {
                        kind: DiffRowKind::Context,
                        old_line: Some(1),
                        new_line: Some(1),
                        text: "short".to_owned(),
                    },
                },
            ),
            row(
                2,
                ReviewRowKind::Diff {
                    row: DiffRow {
                        kind: DiffRowKind::Added,
                        old_line: None,
                        new_line: Some(2),
                        text: "this line is intentionally much wider than the narrow viewport"
                            .to_owned(),
                    },
                },
            ),
            row(
                3,
                ReviewRowKind::Diff {
                    row: DiffRow {
                        kind: DiffRowKind::Removed,
                        old_line: Some(3),
                        new_line: None,
                        text: "removed".to_owned(),
                    },
                },
            ),
            row(
                4,
                ReviewRowKind::EmptyState {
                    message: "last logical row".to_owned(),
                },
            ),
        ],
    }
}

fn row(ordinal: usize, kind: ReviewRowKind) -> ReviewRow {
    ReviewRow {
        id: RowId::new(format!("row:{ordinal:04}")),
        ordinal,
        file_id: None,
        hunk_id: None,
        kind,
    }
}
