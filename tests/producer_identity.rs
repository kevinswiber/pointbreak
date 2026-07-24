mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use pointbreak::session::event::ShoreEvent;
use pointbreak::session::{
    CaptureOptions, IngestEventsOptions, capture_worktree_review, ingest_events, read_events,
};
use support::git_repo::GitRepo;

const HISTORICAL_EVENT: &str = "friendly-valid-event.json";
const HISTORICAL_EVENT_ID: &str =
    "evt:sha256:0fe6523806ec94ccbe5380da6583477b32c6ed398c741d483c4b61a5a68a837d";
const HISTORICAL_EVENT_RECORD_HASH: &str =
    "sha256:cea1dd4ffbd3952266fb35b5a72fd369c74caa6b246ac446bcdc40f0920309a4";

fn historical_fixture_path() -> PathBuf {
    support::manifest_dir()
        .join("tests/fixtures/event_signatures")
        .join(HISTORICAL_EVENT)
}

fn historical_fixture() -> ShoreEvent {
    serde_json::from_slice(&fs::read(historical_fixture_path()).expect("read historical fixture"))
        .expect("decode historical fixture")
}

fn event_file_bytes(repo: &GitRepo) -> BTreeMap<PathBuf, Vec<u8>> {
    let events_dir = support::common_dir_store(repo.path()).join("events");
    fs::read_dir(events_dir)
        .expect("read event store")
        .map(|entry| {
            let path = entry.expect("event entry").path();
            let name = path.file_name().expect("event filename").to_owned().into();
            (name, fs::read(path).expect("read stored event"))
        })
        .collect()
}

#[test]
fn native_records_use_pointbreak_producer_in_a_mixed_historical_store() {
    let fixture_bytes = fs::read(historical_fixture_path()).expect("read fixture bytes");
    let historical = historical_fixture();
    assert_eq!(historical.event_id.as_str(), HISTORICAL_EVENT_ID);
    assert_eq!(historical.writer.producer.name, "shore");
    assert_eq!(
        historical.event_record_hash().unwrap(),
        HISTORICAL_EVENT_RECORD_HASH
    );

    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");

    ingest_events(IngestEventsOptions::new(repo.path(), vec![historical]))
        .expect("ingest historical event");
    let historical_store_bytes = event_file_bytes(&repo);

    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    capture_worktree_review(CaptureOptions::new(repo.path())).expect("capture native revision");

    let events = read_events(repo.path()).expect("read mixed event store");
    let stored_historical = events
        .iter()
        .find(|event| event.event_id.as_str() == HISTORICAL_EVENT_ID)
        .expect("historical event remains in mixed store");
    assert_eq!(stored_historical.writer.producer.name, "shore");
    assert_eq!(
        stored_historical.event_record_hash().unwrap(),
        HISTORICAL_EVENT_RECORD_HASH
    );
    let native_events = events
        .iter()
        .filter(|event| event.event_id.as_str() != HISTORICAL_EVENT_ID)
        .collect::<Vec<_>>();
    assert!(!native_events.is_empty(), "capture appended native events");
    assert!(
        native_events
            .into_iter()
            .all(|event| event.writer.producer.name == "pointbreak")
    );

    let appended_store_bytes = event_file_bytes(&repo);
    for (name, bytes) in historical_store_bytes {
        assert_eq!(
            appended_store_bytes.get(&name),
            Some(&bytes),
            "appending native events must not rewrite historical event {name:?}"
        );
    }
    assert_eq!(
        fs::read(historical_fixture_path()).expect("reread fixture bytes"),
        fixture_bytes,
        "the historical signed fixture must remain byte-for-byte unchanged"
    );
}
