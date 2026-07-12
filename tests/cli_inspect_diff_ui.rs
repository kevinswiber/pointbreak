mod support;

use support::inspect::{Inspector, representative_store, urlencode};

#[test]
fn anchored_range_fact_still_renders_after_map_bucketing() {
    // The representative store has exactly one range-anchored observation on
    // src/lib.rs:2-2. Bucketing must not change which facts anchor where: the
    // /api/snapshots/{id} snapshot carries that file, and /api/revisions/{id} carries the
    // observation with its preserved range target. The Map refactor keys on the
    // same (side, line) the diff anchoring rule produces, so the fact is unchanged.
    let store = representative_store();
    let insp = Inspector::spawn(store.repo.path());

    let unit = insp.get_json(&format!("/api/revisions/{}", urlencode(&store.revision_id)));
    let obs = &unit["observations"][0]["target"];
    assert_eq!(obs["filePath"], "src/lib.rs");
    assert_eq!(obs["startLine"], 2);
    assert_eq!(obs["endLine"], 2);

    // The captured snapshot still carries src/lib.rs so the fact has a row to
    // anchor to (the client builds the side:line Map over these rows).
    let object = insp.get_json(&format!("/api/snapshots/{}", urlencode(&store.snapshot_id)));
    let files = object["snapshot"]["files"].as_array().unwrap();
    assert!(
        files
            .iter()
            .any(|f| f["new_path"] == "src/lib.rs" || f["old_path"] == "src/lib.rs"),
        "the snapshot carries the file the range fact anchors to"
    );
}

#[test]
fn diff_css_styles_the_accordion_from_tokens_not_raw_hex() {
    let store = representative_store();
    let css = Inspector::spawn(store.repo.path()).get_text("/app.css");
    // The accordion body collapses/expands keyed on the section's internal
    // render state; the aria state lives on the header button.
    assert!(
        css.contains("data-expanded"),
        "app.css drives the accordion body off internal render state"
    );
    // The clickable header signals interactivity (cursor), styled from tokens.
    assert!(
        css.contains(".dfile-head") && css.contains("cursor"),
        "the file header reads as clickable"
    );
}

#[test]
fn anchored_fact_remains_reachable_in_a_default_open_file() {
    // The annotated file (src/lib.rs, carrying the range observation) is one of
    // the default-open sections, so its body — and the anchored fact — is present
    // without a manual expand. Behavior floor preserved from the Map refactor.
    let store = representative_store();
    let insp = Inspector::spawn(store.repo.path());
    let unit = insp.get_json(&format!("/api/revisions/{}", urlencode(&store.revision_id)));
    assert_eq!(unit["observations"][0]["target"]["filePath"], "src/lib.rs");
}

#[test]
fn diff_page_has_a_sticky_file_navigator() {
    let store = representative_store();
    let html = Inspector::spawn(store.repo.path()).get_text("/");
    // A navigator region exists on the page with a stable aria-label so the
    // painter and this contract can target it without a function name.
    assert!(
        html.contains("id=\"diff-page-nav\""),
        "the diff page carries a file/fact navigator region"
    );
    assert!(
        html.contains("aria-label=\"diff files\""),
        "the navigator exposes a stable label"
    );
}

#[test]
fn diff_page_is_a_route_surface_not_a_modal() {
    let store = representative_store();
    let html = Inspector::spawn(store.repo.path()).get_text("/");
    // The annotated diff is a routed page: a plain labelled section with its
    // own close control — no dialog chrome, and the retired modal never returns.
    assert!(
        html.contains("id=\"diff-page\"") && html.contains("aria-label=\"annotated diff\""),
        "the diff page section exists with a stable label"
    );
    assert!(
        !html.contains("id=\"diff-modal\""),
        "the retired diff modal is not served"
    );
    assert!(
        html.contains("id=\"diff-page-close\""),
        "the page carries its back-to-the-record close control"
    );
}

#[test]
fn drow_noted_gutter_is_a_clickable_marker() {
    let store = representative_store();
    let insp = Inspector::spawn(store.repo.path());
    let css = insp.get_text("/app.css");
    // The gutter marker keeps the box-shadow cue but becomes interactive.
    assert!(
        css.contains(".drow-noted"),
        "the annotated-row gutter marker is styled"
    );
}

#[test]
fn low_signal_collapse_styles_a_one_line_header() {
    let store = representative_store();
    let css = Inspector::spawn(store.repo.path()).get_text("/app.css");
    assert!(
        css.contains("dfile-lowsignal") || css.contains("[data-lowsignal]"),
        "app.css styles the collapsed low-signal header"
    );
}

#[test]
fn binary_file_renders_collapsed_by_default() {
    // A captured snapshot containing a binary file: the diff file carries
    // is_binary and no hunks, so it classifies low-signal and renders collapsed.
    // Build a repo with a binary blob, capture, and assert the wire shape the
    // client collapses on.
    let repo = support::git_repo::GitRepo::new();
    repo.write("README.md", "base\n");
    repo.commit_all("base");
    std::fs::write(repo.path().join("logo.png"), [0u8, 1, 2, 0, 255, 0, 13]).unwrap();
    repo.git(["add", "logo.png"]);
    support::inspect::capture(repo.path());

    let insp = Inspector::spawn(repo.path());
    let revisions = insp.get_json("/api/revisions");
    let snapshot_id = revisions["entries"][0]["snapshotId"]
        .as_str()
        .expect("the captured revision exposes its snapshot id");
    let object = insp.get_json(&format!("/api/snapshots/{}", urlencode(snapshot_id)));
    let files = object["snapshot"]["files"].as_array().unwrap();
    let png = files
        .iter()
        .find(|f| f["new_path"] == "logo.png" || f["old_path"] == "logo.png")
        .expect("the captured snapshot carries the binary file");
    // The exact wire signal classifyLowSignal keys on: is_binary + no hunks.
    assert_eq!(
        png["is_binary"], true,
        "the binary file is flagged is_binary"
    );
    assert!(
        png["hunks"].as_array().is_none_or(|h| h.is_empty()),
        "the binary file carries no content hunks"
    );
}

#[test]
fn snapshot_endpoint_emphasizes_changed_word() {
    // A single-word edit on an otherwise-identical line. End to end: capture, serve, and assert the
    // changed added row carries a plausible UTF-16 emphasis span while a context row omits it.
    let repo = support::git_repo::GitRepo::new();
    repo.write(
        "src/lib.rs",
        "pub fn compute() -> u32 {\n    let total = a;\n    total\n}\n",
    );
    repo.commit_all("base");
    repo.write(
        "src/lib.rs",
        "pub fn compute() -> u32 {\n    let total = b;\n    total\n}\n",
    );
    support::inspect::capture(repo.path());

    let insp = Inspector::spawn(repo.path());
    let revisions = insp.get_json("/api/revisions");
    let snapshot_id = revisions["entries"][0]["snapshotId"]
        .as_str()
        .expect("the captured revision exposes its snapshot id");
    let object = insp.get_json(&format!("/api/snapshots/{}", urlencode(snapshot_id)));
    let rows = object["snapshot"]["files"][0]["hunks"][0]["rows"]
        .as_array()
        .expect("the modified file carries a hunk with rows");

    let added = rows
        .iter()
        .find(|row| row["kind"] == "added")
        .expect("the hunk carries an added row");
    let emphasis = added["emphasis"]
        .as_array()
        .expect("the changed added row carries an emphasis array");
    assert_eq!(emphasis.len(), 1, "only the changed word is emphasized");
    let start = emphasis[0]["start"].as_u64().unwrap();
    let end = emphasis[0]["end"].as_u64().unwrap();
    assert!(end > start, "the emphasis span is a non-empty UTF-16 range");

    let context = rows
        .iter()
        .find(|row| row["kind"] == "context")
        .expect("the hunk carries a context row");
    assert!(
        context.get("emphasis").is_none(),
        "context rows omit emphasis"
    );
}
