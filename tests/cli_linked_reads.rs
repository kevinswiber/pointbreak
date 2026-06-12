mod support;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use shoreline::model::SnapshotId;
use shoreline::session::{
    ArtifactKind, ArtifactRef, ImportArtifactOptions, LineageListOptions, export_artifact,
    import_artifact, list_lineages, read_events, read_snapshot_artifact, referenced_artifacts,
};
use support::git_repo::GitRepo;
use support::shore;

/// Linked review fixture: the seed worktree captures one review unit and
/// links it into the clone-local store, then the reader worktree links with
/// nothing local to push. Reads from the reader exercise the linked store.
struct LinkedFixture {
    main: GitRepo,
    _worktree_parent: tempfile::TempDir,
    seed: PathBuf,
    reader: PathBuf,
    seed_review_unit_id: String,
    seed_snapshot_id: String,
    seed_snapshot_artifact_content_hash: String,
}

impl LinkedFixture {
    fn new() -> Self {
        let main = GitRepo::new();
        main.write("README.md", "base\n");
        main.commit_all("base");

        let worktree_parent = tempfile::tempdir().expect("create worktree parent");
        let seed = worktree_parent.path().join("seed");
        add_worktree(main.path(), &seed, "seed");
        let reader = worktree_parent.path().join("reader");
        add_worktree(main.path(), &reader, "reader");

        let mut fixture = Self {
            main,
            _worktree_parent: worktree_parent,
            seed,
            reader,
            seed_review_unit_id: String::new(),
            seed_snapshot_id: String::new(),
            seed_snapshot_artifact_content_hash: String::new(),
        };
        fs::write(fixture.seed.join("README.md"), "changed in seed\n").unwrap();
        let capture = fixture.capture(&fixture.seed);
        fixture.seed_review_unit_id = capture["reviewUnit"]["id"]
            .as_str()
            .expect("capture has review unit id")
            .to_owned();
        fixture.seed_snapshot_id = capture["reviewUnit"]["snapshotId"]
            .as_str()
            .expect("capture has snapshot id")
            .to_owned();
        fixture.seed_snapshot_artifact_content_hash =
            capture["reviewUnit"]["snapshotArtifactContentHash"]
                .as_str()
                .expect("capture has snapshot artifact content hash")
                .to_owned();
        fixture.link(&fixture.seed);
        fixture.link(&fixture.reader);
        fixture
    }

    fn capture(&self, worktree: &Path) -> Value {
        run_shore_json(&["review", "capture", "--repo", worktree.to_str().unwrap()])
    }

    fn link(&self, worktree: &Path) -> Value {
        run_shore_json(&["store", "link", "--repo", worktree.to_str().unwrap()])
    }

    fn observation_add(&self, worktree: &Path, review_unit_id: &str, body: &str) -> Value {
        run_shore_json(&[
            "review",
            "observation",
            "add",
            "--repo",
            worktree.to_str().unwrap(),
            "--review-unit",
            review_unit_id,
            "--track",
            "agent:test-fixture",
            "--title",
            "linked body artifact",
            "--body",
            body,
        ])
    }

    fn linked_store_dir(&self) -> PathBuf {
        self.main.path().join(".git/shoreline")
    }

    fn lineage_attach(&self, worktree: &Path, lineage_id: &str, review_unit_id: &str) -> Value {
        run_shore_json(&[
            "review",
            "lineage",
            "attach",
            "--repo",
            worktree.to_str().unwrap(),
            "--lineage",
            lineage_id,
            "--review-unit",
            review_unit_id,
        ])
    }

    fn history_json(&self, worktree: &Path, include_body: bool) -> Value {
        let mut args = vec!["review", "history", "--repo", worktree.to_str().unwrap()];
        if include_body {
            args.push("--include-body");
        }
        run_shore_json(&args)
    }

    fn unit_show_json(&self, worktree: &Path, review_unit_id: &str) -> Value {
        run_shore_json(&[
            "review",
            "unit",
            "show",
            "--repo",
            worktree.to_str().unwrap(),
            "--review-unit",
            review_unit_id,
            "--include-body",
        ])
    }

    /// Record one of each reviewer-facing fact on the seed's review unit.
    fn seed_full_facts(&self, body: &str) {
        self.observation_add(&self.seed, &self.seed_review_unit_id, body);
        let seed = self.seed.to_str().unwrap();
        run_shore_json(&[
            "review",
            "input-request",
            "open",
            "--repo",
            seed,
            "--track",
            "agent:test-fixture",
            "--title",
            "Need approval",
            "--reason",
            "manual-decision-required",
            "--body",
            "approve this path?",
        ]);
        run_shore_json(&[
            "review",
            "assessment",
            "add",
            "--repo",
            seed,
            "--track",
            "human:kevin",
            "--assessment",
            "accepted",
            "--summary",
            "ship it",
        ]);
        run_shore_json(&[
            "review",
            "validation",
            "add",
            "--repo",
            seed,
            "--track",
            "agent:test-fixture",
            "--check-name",
            "cargo test",
            "--status",
            "passed",
        ]);
    }

    fn unit_list_json(&self, worktree: &Path) -> Value {
        run_shore_json(&[
            "review",
            "unit",
            "list",
            "--repo",
            worktree.to_str().unwrap(),
        ])
    }
}

fn run_shore_json(args: &[&str]) -> Value {
    let output = shore(args.iter().copied());
    assert!(
        output.status.success(),
        "shore {args:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("shore stdout is json")
}

fn diagnostic_codes(json: &Value) -> Vec<&str> {
    json["diagnostics"]
        .as_array()
        .map(|diagnostics| {
            diagnostics
                .iter()
                .filter_map(|diagnostic| diagnostic["code"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

fn diagnostic_message(json: &Value, code: &str) -> String {
    json["diagnostics"]
        .as_array()
        .and_then(|diagnostics| {
            diagnostics
                .iter()
                .find(|diagnostic| diagnostic["code"] == code)
        })
        .and_then(|diagnostic| diagnostic["message"].as_str())
        .unwrap_or_else(|| panic!("no diagnostic with code {code}"))
        .to_owned()
}

fn add_worktree(repo: &Path, path: &Path, branch: &str) {
    run_git_os(
        repo,
        [
            OsString::from("worktree"),
            OsString::from("add"),
            OsString::from("-b"),
            OsString::from(branch),
            path.as_os_str().to_owned(),
        ],
    );
}

fn run_git_os<I>(cwd: &Path, args: I)
where
    I: IntoIterator<Item = OsString>,
{
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("run git in {}: {error}", cwd.display()));
    assert!(
        output.status.success(),
        "git failed in {}\nstdout:\n{}\nstderr:\n{}",
        cwd.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn linked_unit_list_reports_unsynced_local_events_diagnostic() {
    let fixture = LinkedFixture::new();
    fs::write(fixture.reader.join("README.md"), "changed in reader\n").unwrap();
    fixture.capture(&fixture.reader);

    let json = fixture.unit_list_json(&fixture.reader);

    // Store-only: the reader's local capture is not listed; only the seed's
    // linked unit is.
    assert_eq!(json["reviewUnitCount"], 1);
    assert_eq!(
        json["entries"][0]["reviewUnitId"],
        Value::String(fixture.seed_review_unit_id.clone())
    );
    let codes = diagnostic_codes(&json);
    assert!(
        codes.contains(&"clone_local_unsynced_local_events"),
        "diagnostics: {}",
        json["diagnostics"]
    );
    let message = diagnostic_message(&json, "clone_local_unsynced_local_events");
    assert!(message.contains("1 local event"), "message: {message}");
    assert!(message.contains("shore store link"), "message: {message}");
}

#[test]
fn linked_unit_list_without_local_events_has_no_divergence_diagnostic() {
    let fixture = LinkedFixture::new();

    let json = fixture.unit_list_json(&fixture.reader);

    assert_eq!(json["reviewUnitCount"], 1);
    assert_eq!(json["eventCount"], 1);
    assert_eq!(
        json["entries"][0]["reviewUnitId"],
        Value::String(fixture.seed_review_unit_id.clone())
    );
    assert!(
        json["eventSetHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(
        !diagnostic_codes(&json).contains(&"clone_local_unsynced_local_events"),
        "diagnostics: {}",
        json["diagnostics"]
    );
}

#[test]
fn linked_history_reads_full_timeline_from_linked_store() {
    let fixture = LinkedFixture::new();
    let body = "h".repeat(5000);
    fixture.observation_add(&fixture.seed, &fixture.seed_review_unit_id, &body);
    fixture.link(&fixture.seed);

    let json = fixture.history_json(&fixture.reader, true);

    assert_eq!(json["eventCount"], 2);
    let event_types: Vec<&str> = json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["eventType"].as_str())
        .collect();
    assert!(
        event_types.contains(&"review_unit_captured"),
        "{event_types:?}"
    );
    assert!(
        event_types.contains(&"review_observation_recorded"),
        "{event_types:?}"
    );
    assert!(
        json.to_string().contains(&body),
        "hydrated observation body loads from the linked store"
    );
}

#[test]
fn linked_history_emits_divergence_diagnostic_with_local_only_events() {
    let fixture = LinkedFixture::new();
    fs::write(fixture.reader.join("README.md"), "changed in reader\n").unwrap();
    let local_capture = fixture.capture(&fixture.reader);
    let local_unit_id = local_capture["reviewUnit"]["id"].as_str().unwrap();

    let json = fixture.history_json(&fixture.reader, false);

    assert!(
        diagnostic_codes(&json).contains(&"clone_local_unsynced_local_events"),
        "diagnostics: {}",
        json["diagnostics"]
    );
    // Store-only: the reader's unsynced local capture is not in the timeline.
    assert_eq!(json["eventCount"], 1);
    assert!(!json["entries"].to_string().contains(local_unit_id));
}

#[test]
fn linked_unit_show_resolves_linked_only_unit() {
    let fixture = LinkedFixture::new();
    let body = "o".repeat(5000);
    fixture.seed_full_facts(&body);
    fixture.link(&fixture.seed);

    let json = fixture.unit_show_json(&fixture.reader, &fixture.seed_review_unit_id);

    assert_eq!(
        json["reviewUnit"]["id"],
        Value::String(fixture.seed_review_unit_id.clone())
    );
    assert_eq!(json["summary"]["observationCount"], 1);
    assert_eq!(json["summary"]["inputRequestCount"], 1);
    assert_eq!(json["summary"]["assessmentCount"], 1);
    assert_eq!(json["summary"]["validationCheckCount"], 1);
    assert_eq!(
        json["observations"][0]["body"],
        Value::String(body),
        "observation body hydrates from the linked store"
    );
}

#[test]
fn linked_unit_show_loads_bound_snapshot_from_linked_store() {
    let fixture = LinkedFixture::new();

    let json = fixture.unit_show_json(&fixture.reader, &fixture.seed_review_unit_id);

    assert_eq!(
        json["reviewUnit"]["snapshotArtifactContentHash"],
        Value::String(fixture.seed_snapshot_artifact_content_hash.clone())
    );
    assert!(
        json["summary"]["snapshotRowCount"].as_u64().unwrap() > 0,
        "bound snapshot rows project from the linked artifact"
    );
}

#[test]
fn linked_unit_show_emits_divergence_diagnostic_with_local_only_events() {
    let fixture = LinkedFixture::new();
    fs::write(fixture.reader.join("README.md"), "changed in reader\n").unwrap();
    fixture.capture(&fixture.reader);

    let json = fixture.unit_show_json(&fixture.reader, &fixture.seed_review_unit_id);

    assert!(
        diagnostic_codes(&json).contains(&"clone_local_unsynced_local_events"),
        "diagnostics: {}",
        json["diagnostics"]
    );
}

#[test]
fn linked_observation_list_resolves_linked_unit() {
    let fixture = LinkedFixture::new();
    let body = "b".repeat(5000);
    fixture.seed_full_facts(&body);
    fixture.link(&fixture.seed);

    let json = run_shore_json(&[
        "review",
        "observation",
        "list",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--review-unit",
        &fixture.seed_review_unit_id,
        "--include-body",
    ]);

    assert_eq!(
        json["reviewUnitId"],
        Value::String(fixture.seed_review_unit_id.clone())
    );
    assert_eq!(json["observations"].as_array().unwrap().len(), 1);
    assert_eq!(json["observations"][0]["body"], Value::String(body));
}

#[test]
fn linked_observation_list_emits_divergence_diagnostic_with_local_only_events() {
    let fixture = LinkedFixture::new();
    let body = "c".repeat(64);
    fixture.seed_full_facts(&body);
    fixture.link(&fixture.seed);
    fs::write(fixture.reader.join("README.md"), "changed in reader\n").unwrap();
    fixture.capture(&fixture.reader);

    let json = run_shore_json(&[
        "review",
        "observation",
        "list",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--review-unit",
        &fixture.seed_review_unit_id,
    ]);

    assert!(
        diagnostic_codes(&json).contains(&"clone_local_unsynced_local_events"),
        "diagnostics: {}",
        json["diagnostics"]
    );
}

#[test]
fn linked_input_request_list_resolves_linked_unit() {
    let fixture = LinkedFixture::new();
    fixture.seed_full_facts("short body");
    fixture.link(&fixture.seed);

    let json = run_shore_json(&[
        "review",
        "input-request",
        "list",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--review-unit",
        &fixture.seed_review_unit_id,
    ]);

    assert_eq!(
        json["reviewUnitId"],
        Value::String(fixture.seed_review_unit_id.clone())
    );
    assert_eq!(json["inputRequests"].as_array().unwrap().len(), 1);
    assert_eq!(json["inputRequests"][0]["title"], "Need approval");
}

#[test]
fn linked_assessment_show_resolves_linked_unit() {
    let fixture = LinkedFixture::new();
    fixture.seed_full_facts("short body");
    fixture.link(&fixture.seed);

    let json = run_shore_json(&[
        "review",
        "assessment",
        "show",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--review-unit",
        &fixture.seed_review_unit_id,
    ]);

    assert_eq!(
        json["reviewUnitId"],
        Value::String(fixture.seed_review_unit_id.clone())
    );
    assert_eq!(json["assessments"].as_array().unwrap().len(), 1);
    assert_eq!(json["assessments"][0]["assessment"], "accepted");
}

#[test]
fn linked_validation_list_resolves_linked_unit() {
    let fixture = LinkedFixture::new();
    fixture.seed_full_facts("short body");
    fixture.link(&fixture.seed);

    let json = run_shore_json(&[
        "review",
        "validation",
        "list",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--review-unit",
        &fixture.seed_review_unit_id,
    ]);

    assert_eq!(
        json["reviewUnitId"],
        Value::String(fixture.seed_review_unit_id.clone())
    );
    assert_eq!(json["validationChecks"].as_array().unwrap().len(), 1);
    assert_eq!(json["validationChecks"][0]["checkName"], "cargo test");
}

#[test]
fn linked_lineage_list_sees_linked_lineage() {
    let fixture = LinkedFixture::new();
    let lineage_id = "review-unit-lineage:random:linked-test";
    fixture.lineage_attach(&fixture.seed, lineage_id, &fixture.seed_review_unit_id);
    fixture.link(&fixture.seed);

    let result = list_lineages(LineageListOptions::new(&fixture.reader)).unwrap();

    assert_eq!(result.lineage_count, 1);
    assert_eq!(result.entries[0].lineage_id.as_str(), lineage_id);
    assert_eq!(
        result.entries[0]
            .head_review_unit_id
            .as_ref()
            .map(|id| id.as_str()),
        Some(fixture.seed_review_unit_id.as_str())
    );
}

#[test]
fn linked_lineage_list_emits_divergence_diagnostic_with_local_only_events() {
    let fixture = LinkedFixture::new();
    let lineage_id = "review-unit-lineage:random:diag-test";
    fixture.lineage_attach(&fixture.seed, lineage_id, &fixture.seed_review_unit_id);
    fixture.link(&fixture.seed);
    fs::write(fixture.reader.join("README.md"), "changed in reader\n").unwrap();
    fixture.capture(&fixture.reader);

    let result = list_lineages(LineageListOptions::new(&fixture.reader)).unwrap();

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "clone_local_unsynced_local_events"),
        "diagnostics: {:?}",
        result.diagnostics
    );
}

#[test]
fn linked_lineage_show_resolves_rounds_from_linked_store() {
    let fixture = LinkedFixture::new();
    let lineage_id = "review-unit-lineage:random:show-test";
    fixture.lineage_attach(&fixture.seed, lineage_id, &fixture.seed_review_unit_id);
    fixture.link(&fixture.seed);

    let json = run_shore_json(&[
        "review",
        "lineage",
        "show",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--lineage",
        lineage_id,
    ]);

    assert_eq!(json["lineageId"], lineage_id);
    assert_eq!(
        json["headReviewUnitId"],
        Value::String(fixture.seed_review_unit_id.clone())
    );
    let rounds = json["rounds"].as_array().unwrap();
    assert_eq!(rounds.len(), 1);
    assert_eq!(
        rounds[0]["reviewUnitId"],
        Value::String(fixture.seed_review_unit_id.clone())
    );
    assert_eq!(rounds[0]["isHead"], true);
}

#[test]
fn snapshot_artifact_reads_from_linked_store() {
    let fixture = LinkedFixture::new();
    let snapshot_id = SnapshotId::new(fixture.seed_snapshot_id.clone());

    let artifact = read_snapshot_artifact(&fixture.reader, &snapshot_id)
        .expect("snapshot artifact reads from the linked store");

    assert_eq!(artifact.snapshot.snapshot_id, snapshot_id);
    assert_eq!(
        artifact.review_unit_id.as_str(),
        fixture.seed_review_unit_id
    );
}

#[test]
fn export_artifact_body_reads_from_linked_store() {
    let fixture = LinkedFixture::new();
    let body = "x".repeat(5000);
    fixture.observation_add(&fixture.seed, &fixture.seed_review_unit_id, &body);
    fixture.link(&fixture.seed);

    let body_ref = seed_body_artifact_ref(&fixture);
    let bytes = export_artifact(&fixture.reader, &body_ref)
        .expect("body artifact exports from the linked store");

    assert!(!bytes.is_empty());
}

#[test]
fn import_artifact_still_writes_worktree_local_in_linked_mode() {
    let fixture = LinkedFixture::new();
    let body = "y".repeat(5000);
    fixture.observation_add(&fixture.seed, &fixture.seed_review_unit_id, &body);

    let body_ref = seed_body_artifact_ref(&fixture);
    let artifact_relative_path = format!(
        "artifacts/notes/{}.json",
        body_ref
            .content_hash()
            .strip_prefix("sha256:")
            .expect("body content hash is sha256-prefixed")
    );
    let bytes = fs::read(fixture.seed.join(".shore").join(&artifact_relative_path)).unwrap();

    import_artifact(ImportArtifactOptions::new(&fixture.reader, body_ref, bytes))
        .expect("import into the linked reader succeeds");

    // Writes stay worktree-local until shared-store writes land: the artifact
    // lands in the reader's own .shore, not the linked clone-local store.
    assert!(
        fixture
            .reader
            .join(".shore")
            .join(&artifact_relative_path)
            .is_file()
    );
    assert!(
        !fixture
            .linked_store_dir()
            .join(&artifact_relative_path)
            .exists()
    );
}

fn seed_body_artifact_ref(fixture: &LinkedFixture) -> ArtifactRef {
    let events = read_events(&fixture.seed).expect("read seed worktree events");
    referenced_artifacts(&events)
        .expect("derive artifact refs from seed events")
        .into_iter()
        .find(|artifact| artifact.kind() == ArtifactKind::Body)
        .expect("seed events reference a body artifact")
}

#[test]
fn worktree_local_unit_list_is_unchanged() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    run_shore_json(&["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let json = run_shore_json(&[
        "review",
        "unit",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);

    assert_eq!(json["schema"], "shore.review-unit-list");
    assert_eq!(json["version"], 1);
    assert_eq!(json["eventCount"], 1);
    assert_eq!(json["reviewUnitCount"], 1);
    assert!(
        !diagnostic_codes(&json).contains(&"clone_local_unsynced_local_events"),
        "diagnostics: {}",
        json["diagnostics"]
    );
}
