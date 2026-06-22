mod support;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use shoreline::model::ObjectId;
use shoreline::session::{
    ArtifactKind, ArtifactRef, ImportArtifactOptions, export_artifact, import_artifact,
    read_snapshot_artifact, referenced_artifacts,
};
use support::git_repo::GitRepo;
use support::shore;

/// Shared-store review fixture: the seed worktree captures one review unit,
/// which writes through to the shared common-dir store (`.git/shore`) by default.
/// The reader is a sibling worktree of the same clone, so its reads resolve the
/// same shared store with no `store link` step.
struct LinkedFixture {
    main: GitRepo,
    _worktree_parent: tempfile::TempDir,
    seed: PathBuf,
    reader: PathBuf,
    seed_revision_id: String,
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
            seed_revision_id: String::new(),
            seed_snapshot_id: String::new(),
            seed_snapshot_artifact_content_hash: String::new(),
        };
        fs::write(fixture.seed.join("README.md"), "changed in seed\n").unwrap();
        let capture = fixture.capture(&fixture.seed);
        fixture.seed_revision_id = capture["revision"]["id"]
            .as_str()
            .expect("capture has review unit id")
            .to_owned();
        fixture.seed_snapshot_id = capture["revision"]["objectId"]
            .as_str()
            .expect("capture has snapshot id")
            .to_owned();
        fixture.seed_snapshot_artifact_content_hash =
            capture["revision"]["snapshotArtifactContentHash"]
                .as_str()
                .expect("capture has snapshot artifact content hash")
                .to_owned();
        fixture
    }

    fn capture(&self, worktree: &Path) -> Value {
        run_shore_json(&["review", "capture", "--repo", worktree.to_str().unwrap()])
    }

    fn observation_add(&self, worktree: &Path, revision_id: &str, body: &str) -> Value {
        run_shore_json(&[
            "review",
            "observation",
            "add",
            "--repo",
            worktree.to_str().unwrap(),
            "--revision",
            revision_id,
            "--track",
            "agent:test-fixture",
            "--title",
            "linked body artifact",
            "--body",
            body,
        ])
    }

    fn linked_store_dir(&self) -> PathBuf {
        self.main.path().join(".git/shore")
    }

    fn history_json(&self, worktree: &Path, include_body: bool) -> Value {
        let mut args = vec!["review", "history", "--repo", worktree.to_str().unwrap()];
        if include_body {
            args.push("--include-body");
        }
        run_shore_json(&args)
    }

    fn unit_show_json(&self, worktree: &Path, revision_id: &str) -> Value {
        run_shore_json(&[
            "review",
            "show",
            "--repo",
            worktree.to_str().unwrap(),
            "--revision",
            revision_id,
            "--include-body",
        ])
    }

    /// Record one of each reviewer-facing fact on the seed's review unit.
    /// Returns the opened input request's id.
    fn seed_full_facts(&self, body: &str) -> String {
        self.observation_add(&self.seed, &self.seed_revision_id, body);
        let seed = self.seed.to_str().unwrap();
        let opened = run_shore_json(&[
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
        opened["inputRequestId"]
            .as_str()
            .expect("input request open returns id")
            .to_owned()
    }

    fn respond_input_request(&self, worktree: &Path, input_request_id: &str) -> Value {
        run_shore_json(&[
            "review",
            "input-request",
            "respond",
            input_request_id,
            "--repo",
            worktree.to_str().unwrap(),
            "--outcome",
            "approved",
            "--reason",
            "approved locally",
        ])
    }

    /// Force-remove the seed worktree; its review record survives in the shared
    /// common-dir store, which is not part of the removed worktree.
    fn remove_seed(&self) {
        run_git_os(
            self.main.path(),
            [
                OsString::from("worktree"),
                OsString::from("remove"),
                OsString::from("--force"),
                self.seed.as_os_str().to_owned(),
            ],
        );
        assert!(!self.seed.exists());
    }

    fn unit_list_json(&self, worktree: &Path) -> Value {
        run_shore_json(&["review", "revisions", "--repo", worktree.to_str().unwrap()])
    }
}

/// One fully populated seed unit (every fact kind + response) written through to
/// the shared common-dir store, with the seed worktree force-removed. The shared
/// arrangement for the deleted-source-worktree matrix.
fn populated_fixture_with_deleted_seed(body: &str) -> (LinkedFixture, String) {
    let fixture = LinkedFixture::new();
    let input_request_id = fixture.seed_full_facts(body);
    fixture.respond_input_request(&fixture.seed, &input_request_id);
    fixture.remove_seed();
    (fixture, input_request_id)
}

fn assert_no_deleted_path_in_diagnostics(fixture: &LinkedFixture, json: &Value) {
    let diagnostics = json["diagnostics"].to_string();
    assert!(
        !diagnostics.contains(fixture.seed.to_str().unwrap()),
        "diagnostics mention the deleted worktree path: {diagnostics}"
    );
}

#[test]
fn deleted_worktree_unit_list_lists_unit() {
    let (fixture, _) = populated_fixture_with_deleted_seed("m1");

    let json = fixture.unit_list_json(&fixture.reader);

    assert_eq!(json["revisionCount"], 1);
    assert_eq!(
        json["entries"][0]["revisionId"],
        Value::String(fixture.seed_revision_id.clone())
    );
    assert_no_deleted_path_in_diagnostics(&fixture, &json);
}

#[test]
fn deleted_worktree_unit_show_renders_composite_with_snapshot() {
    let body = "m".repeat(5000);
    let (fixture, _) = populated_fixture_with_deleted_seed(&body);

    let json = fixture.unit_show_json(&fixture.reader, &fixture.seed_revision_id);

    assert_eq!(json["summary"]["observationCount"], 1);
    assert_eq!(json["summary"]["inputRequestCount"], 1);
    assert_eq!(json["summary"]["assessmentCount"], 1);
    assert_eq!(json["summary"]["validationCheckCount"], 1);
    assert!(json["summary"]["snapshotRowCount"].as_u64().unwrap() > 0);
    assert_eq!(json["observations"][0]["body"], Value::String(body));
    assert_no_deleted_path_in_diagnostics(&fixture, &json);
}

#[test]
fn deleted_worktree_history_renders_timeline_with_bodies() {
    let body = "n".repeat(5000);
    let (fixture, _) = populated_fixture_with_deleted_seed(&body);

    let json = fixture.history_json(&fixture.reader, true);

    assert!(json["eventCount"].as_u64().unwrap() > 0);
    assert!(
        json.to_string().contains(&body),
        "hydrated observation body loads from the linked store"
    );
    assert_no_deleted_path_in_diagnostics(&fixture, &json);
}

#[test]
fn deleted_worktree_observation_list_renders_with_hydrated_body() {
    let body = "p".repeat(5000);
    let (fixture, _) = populated_fixture_with_deleted_seed(&body);

    let json = run_shore_json(&[
        "review",
        "observation",
        "list",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &fixture.seed_revision_id,
        "--include-body",
    ]);

    assert_eq!(json["observations"].as_array().unwrap().len(), 1);
    assert_eq!(json["observations"][0]["body"], Value::String(body));
    assert_no_deleted_path_in_diagnostics(&fixture, &json);
}

#[test]
fn deleted_worktree_input_request_list_renders_with_response() {
    let (fixture, input_request_id) = populated_fixture_with_deleted_seed("m5");

    let json = run_shore_json(&[
        "review",
        "input-request",
        "list",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &fixture.seed_revision_id,
        "--status",
        "all",
    ]);

    assert_eq!(json["inputRequests"].as_array().unwrap().len(), 1);
    assert_eq!(
        json["inputRequests"][0]["id"],
        Value::String(input_request_id)
    );
    assert_eq!(
        json["inputRequests"][0]["responses"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_no_deleted_path_in_diagnostics(&fixture, &json);
}

#[test]
fn deleted_worktree_assessment_show_renders() {
    let (fixture, _) = populated_fixture_with_deleted_seed("m6");

    let json = run_shore_json(&[
        "review",
        "assessment",
        "show",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &fixture.seed_revision_id,
    ]);

    assert_eq!(json["assessments"].as_array().unwrap().len(), 1);
    assert_eq!(json["assessments"][0]["assessment"], "accepted");
    assert_no_deleted_path_in_diagnostics(&fixture, &json);
}

#[test]
fn deleted_worktree_validation_list_renders() {
    let (fixture, _) = populated_fixture_with_deleted_seed("m7");

    let json = run_shore_json(&[
        "review",
        "validation",
        "list",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &fixture.seed_revision_id,
    ]);

    assert_eq!(json["validationChecks"].as_array().unwrap().len(), 1);
    assert_no_deleted_path_in_diagnostics(&fixture, &json);
}

#[test]
fn linked_reads_agree_on_event_set_hash_across_surfaces() {
    let fixture = LinkedFixture::new();
    fixture.seed_full_facts("short body");

    let reader_units = fixture.unit_list_json(&fixture.reader);
    let reader_history = fixture.history_json(&fixture.reader, false);
    let seed_units = fixture.unit_list_json(&fixture.seed);
    let seed_history = fixture.history_json(&fixture.seed, false);

    // Issue #140's regression signal, inverted into the standing guard: every
    // read surface in every linked checkout reports one eventSetHash.
    let hash = event_set_hash(&reader_units);
    assert!(hash.starts_with("sha256:"));
    assert_eq!(event_set_hash(&reader_history), hash);
    assert_eq!(event_set_hash(&seed_units), hash);
    assert_eq!(event_set_hash(&seed_history), hash);
    assert_eq!(reader_units["eventCount"], reader_history["eventCount"]);
    assert_eq!(reader_units["eventCount"], seed_units["eventCount"]);
}

#[test]
fn reader_capture_is_immediately_visible_via_write_through() {
    let fixture = LinkedFixture::new();

    let units = fixture.unit_list_json(&fixture.reader);
    assert_eq!(units["revisionCount"], 1);
    let before_hash = event_set_hash(&units).to_owned();

    // The reader captures in its own worktree: write-through lands it in the
    // shared common-dir store, so it is visible immediately with no `store link`.
    fs::write(fixture.reader.join("README.md"), "changed in reader\n").unwrap();
    let local_capture = fixture.capture(&fixture.reader);
    let local_unit_id = local_capture["revision"]["id"].as_str().unwrap().to_owned();

    let units = fixture.unit_list_json(&fixture.reader);
    let history = fixture.history_json(&fixture.reader, false);
    assert_eq!(units["revisionCount"], 2);
    assert!(units["entries"].to_string().contains(&local_unit_id));
    let advanced_hash = event_set_hash(&units);
    assert_ne!(advanced_hash, before_hash);
    assert_eq!(event_set_hash(&history), advanced_hash);
}

#[test]
fn reader_capture_file_target_observation_resolves_artifact() {
    let fixture = LinkedFixture::new();

    // The reader captures in its own worktree: the unit and its snapshot artifact
    // write through to the shared common-dir store.
    fs::write(fixture.reader.join("README.md"), "changed in reader\n").unwrap();
    let capture = fixture.capture(&fixture.reader);
    let local_unit_id = capture["revision"]["id"]
        .as_str()
        .expect("local capture has review unit id")
        .to_owned();

    // A file-targeted observation against that captured unit resolves its bound
    // snapshot artifact from the shared store and records the file target.
    let json = run_shore_json(&[
        "review",
        "observation",
        "add",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &local_unit_id,
        "--track",
        "agent:test-fixture",
        "--title",
        "file targeted on local capture",
        "--file",
        "README.md",
    ]);

    assert_eq!(json["target"]["kind"], "file");
    assert_eq!(json["target"]["filePath"], "README.md");
    assert_eq!(json["target"]["revisionId"], Value::String(local_unit_id));
}

#[test]
fn linked_reader_attaches_observation_to_linked_only_unit() {
    let fixture = LinkedFixture::new();

    // The reader attaches an observation to the seed's linked-only unit. Before
    // the migration this is RED: record_observation validates against the
    // reader's empty worktree-local store and fails with "unknown review unit".
    let result = fixture.observation_add(
        &fixture.reader,
        &fixture.seed_revision_id,
        "cross-worktree note",
    );

    assert_eq!(result["eventsCreated"], 1);
    assert!(
        result["observationId"].as_str().is_some(),
        "result carries an observation id: {result}"
    );
}

#[test]
fn linked_reader_opens_input_request_against_linked_only_unit() {
    let fixture = LinkedFixture::new();

    // RED today: open_input_request validates against the reader's empty local
    // store and fails with "unknown review unit".
    let result = run_shore_json(&[
        "review",
        "input-request",
        "open",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &fixture.seed_revision_id,
        "--track",
        "agent:test-fixture",
        "--title",
        "cross-worktree question",
        "--reason",
        "manual-decision-required",
        "--body",
        "approve?",
    ]);

    assert_eq!(result["eventsCreated"], 1);
    assert!(result["inputRequestId"].as_str().is_some());
}

#[test]
fn linked_reader_opens_input_request_with_observation_ref_target() {
    let fixture = LinkedFixture::new();

    // The seed records an observation on its unit and links it; the observation
    // now lives only in the linked store.
    let observation = fixture.observation_add(
        &fixture.seed,
        &fixture.seed_revision_id,
        "observation to reference",
    );
    let observation_id = observation["observationId"]
        .as_str()
        .expect("seed observation has an id")
        .to_owned();

    // RED today: resolve_input_request_target cannot see the linked-only
    // observation from the reader's local store.
    let result = run_shore_json(&[
        "review",
        "input-request",
        "open",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &fixture.seed_revision_id,
        "--observation",
        &observation_id,
        "--track",
        "agent:test-fixture",
        "--title",
        "question about that observation",
        "--reason",
        "manual-decision-required",
        "--body",
        "is this right?",
    ]);

    assert_eq!(result["eventsCreated"], 1);
    assert_eq!(result["target"]["kind"], "observation");
    assert_eq!(
        result["target"]["observationId"],
        Value::String(observation_id)
    );
}

#[test]
fn linked_reader_responds_to_linked_only_input_request() {
    let fixture = LinkedFixture::new();
    let request_id = fixture.seed_full_facts("seed body");
    // The opened request lives in the seed's local store until link; copy it to
    // the linked store so the reader can see it.

    // RED today: respond_input_request reads the reader's empty local store and
    // fails with "unknown input request".
    let result = fixture.respond_input_request(&fixture.reader, &request_id);

    assert_eq!(result["eventsCreated"], 1);
    assert_eq!(result["outcome"], "approved");
}

#[test]
fn linked_reader_respond_copies_request_event_target_fields() {
    let fixture = LinkedFixture::new();
    let request_id = fixture.seed_full_facts("seed body");

    fixture.respond_input_request(&fixture.reader, &request_id);

    // The response lands in the clone-local store (write-through). Its EventTarget
    // must be copied verbatim from the union-read request, not fabricated.
    let events = read_store_events(&fixture.linked_store_dir());
    let response = events
        .iter()
        .find(|event| {
            event.event_type == shoreline::session::event::EventType::InputRequestResponded
        })
        .expect("the response event is in the linked store");
    // The response addresses the same review-domain revision the request did, via
    // the envelope's single `subject` (the object id rides the payload, never the
    // envelope), on the same track — copied verbatim from the union-read request.
    assert_eq!(
        shoreline::model::subject_revision_id(&response.target.subject).map(|id| id.as_str()),
        Some(fixture.seed_revision_id.as_str())
    );
    assert_eq!(
        response.target.track_id.as_ref().map(|id| id.as_str()),
        Some("agent:test-fixture")
    );
}

#[test]
fn linked_reader_records_assessment_on_linked_only_unit() {
    let fixture = LinkedFixture::new();

    // RED today: record_assessment validates against the reader's empty local
    // store and fails with "unknown review unit".
    let result = run_shore_json(&[
        "review",
        "assessment",
        "add",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &fixture.seed_revision_id,
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--summary",
        "ship it",
    ]);

    assert_eq!(result["eventsCreated"], 1);
    assert!(result["assessmentId"].as_str().is_some());
}

#[test]
fn linked_reader_assessment_relates_linked_only_observation() {
    let fixture = LinkedFixture::new();
    let observation = fixture.observation_add(
        &fixture.seed,
        &fixture.seed_revision_id,
        "observation to relate",
    );
    let observation_id = observation["observationId"]
        .as_str()
        .expect("seed observation id")
        .to_owned();

    // RED today: relationship validation reads the reader's local store and
    // fails with "unknown observation".
    let result = run_shore_json(&[
        "review",
        "assessment",
        "add",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &fixture.seed_revision_id,
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--summary",
        "looks good",
        "--related-observation",
        &observation_id,
    ]);

    assert_eq!(result["eventsCreated"], 1);
}

#[test]
fn linked_reader_assessment_replaces_linked_only_assessment() {
    let fixture = LinkedFixture::new();
    let seed_assessment = run_shore_json(&[
        "review",
        "assessment",
        "add",
        "--repo",
        fixture.seed.to_str().unwrap(),
        "--revision",
        &fixture.seed_revision_id,
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--summary",
        "first pass",
    ]);
    let assessment_id = seed_assessment["assessmentId"]
        .as_str()
        .expect("seed assessment id")
        .to_owned();

    // RED today: --replaces validation reads the reader's local store and fails
    // with "unknown assessment".
    let result = run_shore_json(&[
        "review",
        "assessment",
        "add",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &fixture.seed_revision_id,
        "--track",
        "human:kevin",
        "--assessment",
        "needs-changes",
        "--summary",
        "second pass",
        "--replaces",
        &assessment_id,
    ]);

    assert_eq!(result["eventsCreated"], 1);
}

#[test]
fn linked_reader_records_validation_on_linked_only_unit() {
    let fixture = LinkedFixture::new();

    // RED today: record_validation_check validates against the reader's empty
    // local store and fails with "unknown review unit".
    let result = run_shore_json(&[
        "review",
        "validation",
        "add",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &fixture.seed_revision_id,
        "--track",
        "agent:test-fixture",
        "--check-name",
        "cargo test",
        "--status",
        "passed",
    ]);

    assert_eq!(result["eventsCreated"], 1);
}

#[test]
fn linked_fact_writes_land_in_linked_store_not_worktree_local() {
    let fixture = LinkedFixture::new();
    // Seed an input request so the reader has one to respond to, and link so the
    // baseline captures everything the seed has published.
    let request_id = fixture.seed_full_facts("seed body");
    let linked_before = event_file_names(&fixture.linked_store_dir());

    // One of each migrated fact on the reader against the seed's linked-only unit.
    let reader = fixture.reader.to_str().unwrap();
    let unit = fixture.seed_revision_id.as_str();
    fixture.observation_add(&fixture.reader, unit, "cross-worktree note");
    run_shore_json(&[
        "review",
        "input-request",
        "open",
        "--repo",
        reader,
        "--revision",
        unit,
        "--track",
        "agent:test-fixture",
        "--title",
        "q",
        "--reason",
        "manual-decision-required",
        "--body",
        "?",
    ]);
    run_shore_json(&[
        "review",
        "assessment",
        "add",
        "--repo",
        reader,
        "--revision",
        unit,
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--summary",
        "ok",
    ]);
    run_shore_json(&[
        "review",
        "validation",
        "add",
        "--repo",
        reader,
        "--revision",
        unit,
        "--track",
        "agent:test-fixture",
        "--check-name",
        "cargo test",
        "--status",
        "passed",
    ]);
    fixture.respond_input_request(&fixture.reader, &request_id);

    // Every write-through fact landed in the clone-local store, not worktree-local.
    let linked_after = event_file_names(&fixture.linked_store_dir());
    assert!(
        linked_after.len() > linked_before.len(),
        "linked store gained the write-through fact events: before={} after={}",
        linked_before.len(),
        linked_after.len()
    );
    assert!(
        event_file_names(&fixture.reader.join(".shore/data")).is_empty(),
        "reader worktree-local store received no fact events in linked mode"
    );
}

#[test]
fn linked_fact_write_state_json_is_orphan_free() {
    let fixture = LinkedFixture::new();
    fixture.observation_add(
        &fixture.reader,
        &fixture.seed_revision_id,
        "cross-worktree note",
    );

    // The fact's state.json is rebuilt in the clone-local store (write-through).
    // The StateReducer does not cross-check facts against captures, so there is
    // no orphan diagnostic even though the capture and fact may interleave.
    let bytes = fs::read(fixture.linked_store_dir().join("state.json"))
        .expect("read clone-local state.json");
    let state: Value = serde_json::from_slice(&bytes).expect("state.json is json");
    assert!(state["observationCount"].as_u64().unwrap() >= 1);
    assert!(
        !state_diagnostic_codes(&state)
            .iter()
            .any(|code| code.contains("orphan")),
        "diagnostics: {}",
        state["diagnostics"]
    );
}

#[test]
fn linked_fact_write_does_not_copy_snapshot_artifacts_to_linked_store() {
    let fixture = LinkedFixture::new();
    // Reader captures locally: its snapshot artifact lands in the reader's .shore/data.
    fs::write(fixture.reader.join("README.md"), "changed in reader\n").unwrap();
    let capture = fixture.capture(&fixture.reader);
    let local_unit = capture["revision"]["id"].as_str().unwrap().to_owned();

    let snapshots_before = snapshot_artifact_names(&fixture.linked_store_dir());
    // File-targeted observation against the locally captured unit (the artifact
    // worktree-local fallback path). The write must not push the snapshot
    // artifact to the linked store — only `store link` copies artifacts.
    run_shore_json(&[
        "review",
        "observation",
        "add",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &local_unit,
        "--track",
        "agent:test-fixture",
        "--title",
        "t",
        "--file",
        "README.md",
    ]);
    let snapshots_after = snapshot_artifact_names(&fixture.linked_store_dir());

    assert_eq!(
        snapshots_before, snapshots_after,
        "no snapshot artifacts copied to the linked store by a write"
    );
}

fn event_file_names(store_dir: &Path) -> Vec<String> {
    json_file_names(&store_dir.join("events"))
}

/// Deserialize every event file directly out of an explicit store directory.
/// `read_events(worktree)` resolves the worktree-local `.shore/data` store; in
/// linked mode write-through lands events in the clone-local store, so reading
/// those events back means reading the clone-local store directory itself.
fn read_store_events(store_dir: &Path) -> Vec<shoreline::session::event::ShoreEvent> {
    let events_dir = store_dir.join("events");
    let mut entries: Vec<PathBuf> = match fs::read_dir(&events_dir) {
        Ok(read_dir) => read_dir
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect(),
        Err(_) => Vec::new(),
    };
    entries.sort();
    entries
        .iter()
        .map(|path| {
            let bytes = fs::read(path).expect("read event file");
            serde_json::from_slice(&bytes).expect("event file is a ShoreEvent")
        })
        .collect()
}

fn snapshot_artifact_names(store_dir: &Path) -> Vec<String> {
    json_file_names(&store_dir.join("artifacts/snapshots"))
}

fn json_file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.ends_with(".json"))
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    names
}

fn state_diagnostic_codes(state: &Value) -> Vec<String> {
    state["diagnostics"]
        .as_array()
        .map(|diagnostics| {
            diagnostics
                .iter()
                .filter_map(|diagnostic| diagnostic["code"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn cross_worktree_fact_is_immediately_visible_via_write_through() {
    let fixture = LinkedFixture::new();
    let added = fixture.observation_add(&fixture.reader, &fixture.seed_revision_id, "cross note");
    let observation_id = added["observationId"].as_str().unwrap().to_owned();

    // Write-through: the seed (a separate checkout reading the shared common-dir
    // store) sees the reader's observation immediately, with no `store link`.
    let seen = observation_list_json(&fixture.seed, &fixture.seed_revision_id);
    assert!(contains_observation(&seen, &observation_id));
}

#[test]
fn file_targeted_cross_worktree_fact_is_immediately_readable() {
    let fixture = LinkedFixture::new();
    // The reader captures a unit in its worktree and records a file-targeted
    // observation with a body against it; both write through to the shared store.
    fs::write(fixture.reader.join("README.md"), "changed in reader\n").unwrap();
    let capture = fixture.capture(&fixture.reader);
    let local_unit = capture["revision"]["id"].as_str().unwrap().to_owned();
    let body = "z".repeat(5000);
    run_shore_json(&[
        "review",
        "observation",
        "add",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &local_unit,
        "--track",
        "agent:test-fixture",
        "--title",
        "file note",
        "--file",
        "README.md",
        "--body",
        &body,
    ]);

    // A sibling checkout reads the fact's body directly from the shared store,
    // with no `store link` step.
    let listed = run_shore_json(&[
        "review",
        "observation",
        "list",
        "--repo",
        fixture.seed.to_str().unwrap(),
        "--revision",
        &local_unit,
        "--include-body",
    ]);
    assert_eq!(listed["observations"][0]["body"], Value::String(body));
}

fn observation_list_json(worktree: &Path, revision_id: &str) -> Value {
    run_shore_json(&[
        "review",
        "observation",
        "list",
        "--repo",
        worktree.to_str().unwrap(),
        "--revision",
        revision_id,
    ])
}

fn contains_observation(list: &Value, observation_id: &str) -> bool {
    list["observations"].as_array().is_some_and(|observations| {
        observations
            .iter()
            .any(|observation| observation["id"] == Value::String(observation_id.to_owned()))
    })
}

fn event_set_hash(json: &Value) -> &str {
    json["eventSetHash"].as_str().expect("eventSetHash present")
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
fn linked_unit_list_without_local_events_has_no_divergence_diagnostic() {
    let fixture = LinkedFixture::new();

    let json = fixture.unit_list_json(&fixture.reader);

    assert_eq!(json["revisionCount"], 1);
    assert_eq!(json["eventCount"], 2);
    assert_eq!(
        json["entries"][0]["revisionId"],
        Value::String(fixture.seed_revision_id.clone())
    );
    assert!(
        json["eventSetHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
}

#[test]
fn linked_history_reads_full_timeline_from_linked_store() {
    let fixture = LinkedFixture::new();
    let body = "h".repeat(5000);
    fixture.observation_add(&fixture.seed, &fixture.seed_revision_id, &body);

    let json = fixture.history_json(&fixture.reader, true);

    assert_eq!(json["eventCount"], 3);
    let event_types: Vec<&str> = json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["eventType"].as_str())
        .collect();
    assert!(
        event_types.contains(&"work_object_proposed"),
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
fn linked_unit_show_resolves_linked_only_unit() {
    let fixture = LinkedFixture::new();
    let body = "o".repeat(5000);
    fixture.seed_full_facts(&body);

    let json = fixture.unit_show_json(&fixture.reader, &fixture.seed_revision_id);

    assert_eq!(
        json["revision"]["id"],
        Value::String(fixture.seed_revision_id.clone())
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

    let json = fixture.unit_show_json(&fixture.reader, &fixture.seed_revision_id);

    assert_eq!(
        json["revision"]["snapshotArtifactContentHash"],
        Value::String(fixture.seed_snapshot_artifact_content_hash.clone())
    );
    assert!(
        json["summary"]["snapshotRowCount"].as_u64().unwrap() > 0,
        "bound snapshot rows project from the linked artifact"
    );
}

#[test]
fn linked_observation_list_resolves_linked_unit() {
    let fixture = LinkedFixture::new();
    let body = "b".repeat(5000);
    fixture.seed_full_facts(&body);

    let json = run_shore_json(&[
        "review",
        "observation",
        "list",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &fixture.seed_revision_id,
        "--include-body",
    ]);

    assert_eq!(
        json["revisionId"],
        Value::String(fixture.seed_revision_id.clone())
    );
    assert_eq!(json["observations"].as_array().unwrap().len(), 1);
    assert_eq!(json["observations"][0]["body"], Value::String(body));
}

#[test]
fn linked_input_request_list_resolves_linked_unit() {
    let fixture = LinkedFixture::new();
    fixture.seed_full_facts("short body");

    let json = run_shore_json(&[
        "review",
        "input-request",
        "list",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &fixture.seed_revision_id,
    ]);

    assert_eq!(
        json["revisionId"],
        Value::String(fixture.seed_revision_id.clone())
    );
    assert_eq!(json["inputRequests"].as_array().unwrap().len(), 1);
    assert_eq!(json["inputRequests"][0]["title"], "Need approval");
}

#[test]
fn linked_input_request_fetch_resolves_linked_request() {
    let fixture = LinkedFixture::new();
    let input_request_id = fixture.seed_full_facts("short body");

    let json = run_shore_json(&[
        "review",
        "input-request",
        "fetch",
        &input_request_id,
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--include-body",
    ]);

    assert_eq!(json["inputRequest"]["id"], Value::String(input_request_id));
    assert_eq!(json["inputRequest"]["title"], "Need approval");
}

#[test]
fn linked_assessment_show_resolves_linked_unit() {
    let fixture = LinkedFixture::new();
    fixture.seed_full_facts("short body");

    let json = run_shore_json(&[
        "review",
        "assessment",
        "show",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &fixture.seed_revision_id,
    ]);

    assert_eq!(
        json["revisionId"],
        Value::String(fixture.seed_revision_id.clone())
    );
    assert_eq!(json["assessments"].as_array().unwrap().len(), 1);
    assert_eq!(json["assessments"][0]["assessment"], "accepted");
}

#[test]
fn linked_validation_list_resolves_linked_unit() {
    let fixture = LinkedFixture::new();
    fixture.seed_full_facts("short body");

    let json = run_shore_json(&[
        "review",
        "validation",
        "list",
        "--repo",
        fixture.reader.to_str().unwrap(),
        "--revision",
        &fixture.seed_revision_id,
    ]);

    assert_eq!(
        json["revisionId"],
        Value::String(fixture.seed_revision_id.clone())
    );
    assert_eq!(json["validationChecks"].as_array().unwrap().len(), 1);
    assert_eq!(json["validationChecks"][0]["checkName"], "cargo test");
}

#[test]
fn snapshot_artifact_reads_from_linked_store() {
    let fixture = LinkedFixture::new();
    let snapshot_id = ObjectId::new(fixture.seed_snapshot_id.clone());

    let artifact = read_snapshot_artifact(&fixture.reader, &snapshot_id)
        .expect("snapshot artifact reads from the linked store");

    // The snapshot-scoped v2 artifact carries no revision_id; resolving its
    // snapshot id through the linked store is what proves the read.
    assert_eq!(artifact.snapshot.object_id, snapshot_id);
}

#[test]
fn export_artifact_body_reads_from_linked_store() {
    let fixture = LinkedFixture::new();
    let body = "x".repeat(5000);
    fixture.observation_add(&fixture.seed, &fixture.seed_revision_id, &body);

    let body_ref = seed_body_artifact_ref(&fixture);
    let bytes = export_artifact(&fixture.reader, &body_ref)
        .expect("body artifact exports from the linked store");

    assert!(!bytes.is_empty());
}

#[test]
fn import_artifact_writes_through_to_linked_store() {
    let fixture = LinkedFixture::new();
    let body = "y".repeat(5000);
    fixture.observation_add(&fixture.seed, &fixture.seed_revision_id, &body);

    let body_ref = seed_body_artifact_ref(&fixture);
    let artifact_relative_path = format!(
        "artifacts/notes/{}.json",
        body_ref
            .content_hash()
            .strip_prefix("sha256:")
            .expect("body content hash is sha256-prefixed")
    );
    let bytes = fs::read(fixture.linked_store_dir().join(&artifact_relative_path)).unwrap();

    import_artifact(ImportArtifactOptions::new(&fixture.reader, body_ref, bytes))
        .expect("import into the linked reader succeeds");

    // Write-through (INV-1): import lands the artifact bytes in the clone-local
    // store (the same store reads resolve), never the reader's worktree-local
    // `.shore/data`.
    assert!(
        fixture
            .linked_store_dir()
            .join(&artifact_relative_path)
            .is_file()
    );
    assert!(
        !fixture
            .reader
            .join(".shore/data")
            .join(&artifact_relative_path)
            .exists()
    );
}

fn seed_body_artifact_ref(fixture: &LinkedFixture) -> ArtifactRef {
    // The seed's observation write-throughs to the clone-local store, so the body
    // artifact ref is derived from the clone-local events, not worktree-local.
    let events = read_store_events(&fixture.linked_store_dir());
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
        "revisions",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);

    assert_eq!(json["schema"], "shore.review-revision-list");
    assert_eq!(json["version"], 1);
    assert_eq!(json["eventCount"], 2);
    assert_eq!(json["revisionCount"], 1);
}

#[test]
fn main_worktree_of_a_clone_round_trips_a_capture_in_place() {
    // The headline acceptance test: in the MAIN worktree of a clone, a capture
    // round-trips in place — `unit list` / `unit show` / `history` resolve it with
    // NO dedicated worktree, NO `store link`, and NO `--revision`. The shared
    // common-dir store is the default for every worktree.
    let main = GitRepo::new();
    main.write("README.md", "base\n");
    main.commit_all("base");

    // A tracked change on a branch, captured in the main worktree.
    main.git(["checkout", "-b", "feature"]);
    main.write("README.md", "changed on a branch in the main worktree\n");
    let capture = run_shore_json(&["review", "capture", "--repo", main.path().to_str().unwrap()]);
    let unit_id = capture["revision"]["id"].as_str().unwrap().to_owned();

    // With NO --revision, the same worktree's reads resolve the capture in
    // place (write-through landed it in the same `.git/shore` store reads use).
    let list = run_shore_json(&[
        "review",
        "revisions",
        "--repo",
        main.path().to_str().unwrap(),
    ]);
    assert_eq!(list["revisionCount"], 1);
    assert_eq!(
        list["entries"][0]["revisionId"],
        Value::String(unit_id.clone())
    );

    let show = run_shore_json(&["review", "show", "--repo", main.path().to_str().unwrap()]);
    assert_eq!(show["revision"]["id"], Value::String(unit_id.clone()));

    let history = run_shore_json(&["review", "history", "--repo", main.path().to_str().unwrap()]);
    assert!(
        history["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry.to_string().contains(&unit_id)),
        "history includes the captured unit: {}",
        history["entries"]
    );
    // The capture landed in the shared common-dir store, not stranded worktree-local.
    let status = run_shore_json(&["store", "status", "--repo", main.path().to_str().unwrap()]);
    assert_eq!(status["mode"], "local");
    assert_eq!(status["inventory"]["eventCount"], 2);
}

#[test]
fn fresh_single_worktree_has_clean_own_only_reads() {
    // A plain single-worktree clone: a capture writes through to the shared
    // common-dir store and the same worktree's reads resolve it in place.
    let repo = GitRepo::new();
    repo.write("README.md", "base\n");
    repo.commit_all("base");
    repo.write("README.md", "changed locally\n");
    let capture = run_shore_json(&["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let unit_id = capture["revision"]["id"].as_str().unwrap().to_owned();

    let list = run_shore_json(&[
        "review",
        "revisions",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    assert_eq!(list["revisionCount"], 1);
    assert_eq!(
        list["entries"][0]["revisionId"],
        Value::String(unit_id.clone())
    );

    let status = run_shore_json(&["store", "status", "--repo", repo.path().to_str().unwrap()]);
    assert_eq!(status["mode"], "local");
    // The capture landed in the shared common-dir store, not the worktree-local one.
    assert!(
        support::common_dir_store(repo.path())
            .join("events")
            .is_dir()
    );
}
