//! Public contract coverage for the versioned inspect documents shared by the
//! bundled server and its machine clients.

use std::collections::BTreeSet;

use pointbreak::documents::{
    INSPECT_FRESHNESS_SCHEMA, INSPECT_STARTUP_SCHEMA, InspectFreshnessDocument,
    InspectStartupDocument, REVIEW_SNAPSHOT_SCHEMA, document_registry,
    promoted_inspect_document_registry, review_snapshot_document, version_document,
};
use pointbreak::model::{
    DiffFile, DiffRow, DiffRowKind, DiffSnapshot, FileId, FileStatus, HunkId, ObjectId, ReviewHunk,
    ReviewId,
};
use pointbreak::session::ObjectArtifact;

fn artifact() -> ObjectArtifact {
    let file = |id: &str, path: &str, text: &str| DiffFile {
        id: FileId::new(id),
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
        hunks: vec![ReviewHunk {
            id: HunkId::new(format!("{id}:1:1")),
            header: "@@ -1 +1 @@".to_owned(),
            old_start: 1,
            old_lines: 1,
            new_start: 1,
            new_lines: 1,
            rows: vec![DiffRow {
                kind: DiffRowKind::Added,
                old_line: None,
                new_line: Some(1),
                text: text.to_owned(),
            }],
        }],
    };

    ObjectArtifact {
        schema: "shore.object".to_owned(),
        version: 2,
        snapshot: DiffSnapshot::new(
            ReviewId::new("review:test"),
            ObjectId::new("obj:sha256:test"),
            vec![
                file("file:rust", "src/lib.rs", "pub fn value() -> u32 { 2 }"),
                file("file:plain", "notes.unknown", "plain text"),
            ],
        ),
        content_hash: "sha256:stored".to_owned(),
        content_encoding: Vec::new(),
    }
}

#[test]
fn review_snapshot_retags_only_the_envelope_and_preserves_nested_content() {
    let stored = artifact();
    let stored_before = stored.clone();
    let document = review_snapshot_document(&stored);
    let value = serde_json::to_value(document).unwrap();

    assert_eq!(value["schema"], REVIEW_SNAPSHOT_SCHEMA);
    assert_eq!(value["version"], 1);
    assert_eq!(value["contentHash"], "sha256:stored");
    assert!(value.get("content_hash").is_none());
    assert_eq!(value["snapshot"]["review_id"], "review:test");
    assert_eq!(value["snapshot"]["object_id"], "obj:sha256:test");
    assert!(value["snapshot"].get("reviewId").is_none());

    let files = value["snapshot"]["files"].as_array().unwrap();
    assert_eq!(files[0]["id"], "file:rust");
    assert_eq!(files[1]["id"], "file:plain");
    let rust_row = &files[0]["hunks"][0]["rows"][0];
    assert_eq!(rust_row["kind"], "added");
    assert_eq!(rust_row["old_line"], serde_json::Value::Null);
    assert_eq!(rust_row["new_line"], 1);
    assert_eq!(rust_row["text"], "pub fn value() -> u32 { 2 }");
    assert!(
        rust_row["tokens"]
            .as_array()
            .is_some_and(|tokens| !tokens.is_empty())
    );
    let plain_row = &files[1]["hunks"][0]["rows"][0];
    assert!(plain_row.get("tokens").is_none());
    assert!(plain_row.get("emphasis").is_none());

    assert_eq!(stored, stored_before);
    assert_eq!(stored.schema, "shore.object");
    assert_eq!(stored.version, 2);
}

#[test]
fn freshness_and_startup_documents_have_exact_v1_shapes() {
    assert_eq!(
        serde_json::to_value(InspectFreshnessDocument::new(7, Some("stamp".to_owned()))).unwrap(),
        serde_json::json!({
            "schema": INSPECT_FRESHNESS_SCHEMA,
            "version": 1,
            "eventCount": 7,
            "commitGraphStamp": "stamp"
        })
    );
    assert_eq!(
        serde_json::to_value(InspectFreshnessDocument::new(7, None)).unwrap(),
        serde_json::json!({
            "schema": INSPECT_FRESHNESS_SCHEMA,
            "version": 1,
            "eventCount": 7
        })
    );

    let startup = InspectStartupDocument::new("127.0.0.1", 43123, "secret-token");
    let compact = serde_json::to_string(&startup).unwrap();
    assert_eq!(
        compact,
        format!(
            "{{\"schema\":\"{INSPECT_STARTUP_SCHEMA}\",\"version\":1,\"host\":\"127.0.0.1\",\"port\":43123,\"token\":\"secret-token\"}}"
        )
    );
}

#[test]
fn compatibility_registry_is_cli_documents_plus_the_exact_promoted_set() {
    let promoted = promoted_inspect_document_registry();
    assert_eq!(
        promoted,
        &[
            (REVIEW_SNAPSHOT_SCHEMA, 1),
            (INSPECT_FRESHNESS_SCHEMA, 1),
            (INSPECT_STARTUP_SCHEMA, 1),
        ]
    );

    let registered = document_registry().iter().copied().collect::<BTreeSet<_>>();
    for entry in promoted {
        assert!(registered.contains(entry));
    }
    assert!(!registered.iter().any(|(schema, _)| matches!(
        *schema,
        "pointbreak.inspect-attention" | "pointbreak.inspect-identity"
    )));

    let version = serde_json::to_value(version_document()).unwrap();
    for (schema, expected_version) in promoted {
        assert_eq!(version["documents"][schema], *expected_version);
    }
}
