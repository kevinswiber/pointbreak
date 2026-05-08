use shore::git::ingest_tracked_diff;
use shore::model::{
    Annotation, DiffFile, DiffRow, DiffRowKind, FileId, FileStatus, HunkId, ResolutionStatus,
    ReviewHunk, re_resolve_annotations,
};
use shore::sidecar::agent_context::{
    AgentAnnotation, AgentContext, AgentFileContext, Range, resolve_annotations,
};

use crate::support::git_repo::GitRepo;

#[test]
fn read_only_anchors_re_resolve_after_reingesting_git_diff() {
    let repo = GitRepo::new();

    repo.write("exact.rs", "fn main() {\n    exact_old();\n}\n");
    repo.write("shifted.rs", "fn main() {\n    shifted_old();\n}\n");
    repo.write("gone.rs", "fn main() {\n    gone_old();\n}\n");
    repo.write(
        "file_level.rs",
        "fn main() {\n    old_one();\n    old_two();\n}\n",
    );
    repo.commit_all("base");

    repo.write("exact.rs", "fn main() {\n    exact_new();\n}\n");
    repo.write("shifted.rs", "fn main() {\n    shifted_new();\n}\n");
    repo.write("gone.rs", "fn main() {\n    gone_new();\n}\n");
    repo.write(
        "file_level.rs",
        "fn main() {\n    new_one();\n    old_two();\n}\n",
    );

    let initial = ingest_tracked_diff(repo.path()).expect("initial diff should ingest");
    let context = AgentContext {
        schema: Some("shore.agent-context".to_owned()),
        version: 1,
        summary: None,
        ownership: None,
        files: vec![
            agent_file("exact.rs", "exact target", 2),
            agent_file("shifted.rs", "shifted target", 2),
            agent_file("gone.rs", "gone target", 2),
            agent_file("file_level.rs", "file-level target", 2),
        ],
    };
    let initial_annotations = resolve_annotations(&initial.files, &context).annotations;

    repo.write("exact.rs", "fn main() {\n    exact_new();\n}\n");
    repo.write(
        "shifted.rs",
        "fn main() {\n    shifted_extra();\n    shifted_new();\n}\n",
    );
    repo.write("gone.rs", "fn main() {\n    gone_old();\n}\n");
    repo.write(
        "file_level.rs",
        "fn main() {\n    old_one();\n    new_two();\n}\n",
    );

    let fresh = ingest_tracked_diff(repo.path()).expect("fresh diff should ingest");
    let resolved = re_resolve_annotations(&initial_annotations, &fresh.files);

    assert_status(&resolved, "exact target", ResolutionStatus::Exact);
    assert_status(&resolved, "shifted target", ResolutionStatus::Relocated);
    assert_eq!(
        annotation(&resolved, "shifted target")
            .anchor
            .line_range
            .start,
        3
    );
    assert_status(&resolved, "gone target", ResolutionStatus::Orphaned);
    assert_status(&resolved, "file-level target", ResolutionStatus::FileLevel);
}

#[test]
fn anchors_relocate_within_same_hunk_and_report_stale_or_ambiguous_targets() {
    let initial = vec![
        annotated_model_file(
            "src/same.rs",
            vec![ReviewHunk {
                id: HunkId::new("same:1"),
                header: "@@ -1,4 +1,4 @@".to_owned(),
                old_start: 1,
                old_lines: 4,
                new_start: 1,
                new_lines: 4,
                rows: vec![
                    context_row(1, 1, "fn main() {"),
                    added_row(2, "    relocated_target();"),
                    added_row(3, "    changed_call();"),
                    context_row(4, 4, "}"),
                ],
            }],
        ),
        annotated_model_file(
            "src/stale.rs",
            vec![ReviewHunk {
                id: HunkId::new("stale:1"),
                header: "@@ -1,4 +1,4 @@".to_owned(),
                old_start: 1,
                old_lines: 4,
                new_start: 1,
                new_lines: 4,
                rows: vec![
                    context_row(1, 1, "fn main() {"),
                    added_row(2, "    changed_call();"),
                    context_row(3, 3, "    stable_context();"),
                    context_row(4, 4, "}"),
                ],
            }],
        ),
        annotated_model_file(
            "src/ambiguous.rs",
            vec![ReviewHunk {
                id: HunkId::new("ambiguous:1"),
                header: "@@ -1,4 +1,4 @@".to_owned(),
                old_start: 1,
                old_lines: 4,
                new_start: 1,
                new_lines: 4,
                rows: vec![
                    context_row(1, 1, "fn main() {"),
                    added_row(2, "    ambiguous_target();"),
                    context_row(3, 3, "}"),
                ],
            }],
        ),
    ];
    let context = AgentContext {
        schema: Some("shore.agent-context".to_owned()),
        version: 1,
        summary: None,
        ownership: None,
        files: vec![
            agent_file("src/same.rs", "same hunk target", 2),
            agent_file("src/stale.rs", "stale context target", 3),
            agent_file("src/ambiguous.rs", "ambiguous target", 2),
        ],
    };
    let initial_annotations = resolve_annotations(&initial, &context).annotations;

    let fresh = vec![
        annotated_model_file(
            "src/same.rs",
            vec![ReviewHunk {
                id: HunkId::new("same:1"),
                header: "@@ -1,4 +1,4 @@".to_owned(),
                old_start: 1,
                old_lines: 4,
                new_start: 1,
                new_lines: 4,
                rows: vec![
                    context_row(1, 1, "fn main() {"),
                    added_row(3, "    relocated_target();"),
                    added_row(4, "    changed_call();"),
                    context_row(5, 5, "}"),
                ],
            }],
        ),
        annotated_model_file(
            "src/stale.rs",
            vec![ReviewHunk {
                id: HunkId::new("stale:1"),
                header: "@@ -1,4 +1,4 @@".to_owned(),
                old_start: 1,
                old_lines: 4,
                new_start: 1,
                new_lines: 4,
                rows: vec![
                    context_row(1, 1, "fn main() {"),
                    added_row(2, "    changed_call();"),
                    context_row(3, 3, "    changed_context();"),
                    context_row(4, 4, "}"),
                ],
            }],
        ),
        annotated_model_file(
            "src/ambiguous.rs",
            vec![ReviewHunk {
                id: HunkId::new("ambiguous:1"),
                header: "@@ -1,4 +1,4 @@".to_owned(),
                old_start: 1,
                old_lines: 4,
                new_start: 1,
                new_lines: 4,
                rows: vec![
                    context_row(1, 1, "fn main() {"),
                    added_row(2, "    ambiguous_target();"),
                    added_row(3, "    ambiguous_target();"),
                    context_row(4, 4, "}"),
                ],
            }],
        ),
    ];

    let resolved = re_resolve_annotations(&initial_annotations, &fresh);

    assert_status(&resolved, "same hunk target", ResolutionStatus::Relocated);
    assert_eq!(
        annotation(&resolved, "same hunk target")
            .anchor
            .line_range
            .start,
        3
    );
    assert_status(&resolved, "stale context target", ResolutionStatus::Stale);
    assert_status(&resolved, "ambiguous target", ResolutionStatus::Unresolved);
}

fn agent_file(path: &str, summary: &str, line: u32) -> AgentFileContext {
    AgentFileContext {
        path: path.to_owned(),
        old_path: None,
        summary: None,
        annotations: vec![AgentAnnotation {
            id: None,
            summary: Some(summary.to_owned()),
            rationale: None,
            old_range: None,
            new_range: Some(Range {
                start: line,
                end: line,
            }),
            tags: Vec::new(),
            confidence: None,
            source: None,
            author: None,
            created_at: None,
        }],
    }
}

fn annotation<'a>(annotations: &'a [Annotation], summary: &str) -> &'a Annotation {
    annotations
        .iter()
        .find(|annotation| annotation.summary == summary)
        .expect("annotation should exist")
}

fn assert_status(annotations: &[Annotation], summary: &str, status: ResolutionStatus) {
    assert_eq!(annotation(annotations, summary).anchor.status, status);
}

fn annotated_model_file(path: &str, hunks: Vec<ReviewHunk>) -> DiffFile {
    DiffFile {
        id: FileId::new(path),
        status: FileStatus::Modified,
        old_path: Some(path.to_owned()),
        new_path: Some(path.to_owned()),
        old_mode: None,
        new_mode: None,
        old_oid: None,
        new_oid: None,
        similarity: None,
        is_binary: false,
        is_submodule: false,
        is_mode_only: false,
        synthetic: false,
        metadata_rows: Vec::new(),
        hunks,
    }
}

fn context_row(old_line: u32, new_line: u32, text: &str) -> DiffRow {
    DiffRow {
        kind: DiffRowKind::Context,
        old_line: Some(old_line),
        new_line: Some(new_line),
        text: text.to_owned(),
    }
}

fn added_row(new_line: u32, text: &str) -> DiffRow {
    DiffRow {
        kind: DiffRowKind::Added,
        old_line: None,
        new_line: Some(new_line),
        text: text.to_owned(),
    }
}
