mod support;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::pointbreak;

#[test]
fn revision_list_runs_at_top_level() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak(["revision", "list", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["schema"], "pointbreak.review-revision-list");
}

#[test]
fn revision_list_object_filter_resolves_a_short_id() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let path = repo.path().to_str().unwrap();

    let listed = parse_json(&pointbreak(["revision", "list", "--repo", path]).stdout);
    let object_id = listed["entries"][0]["objectId"]
        .as_str()
        .unwrap()
        .to_owned();
    // object_id = "obj:sha256:<hex>"; form the prefixed short id from the digest.
    let digest = object_id.rsplit_once("sha256:").unwrap().1;
    let prefixed_short = format!("obj:{}", &digest[..8]);

    let filtered = pointbreak([
        "revision",
        "list",
        "--repo",
        path,
        "--object",
        &prefixed_short,
    ]);
    assert!(
        filtered.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&filtered.stderr)
    );
    let json = parse_json(&filtered.stdout);
    assert_eq!(json["revisionCount"], 1);
    // The listing lens filtered on the resolved FULL object id.
    assert_eq!(json["entries"][0]["objectId"], object_id);
}

#[test]
fn revision_list_emits_v1_json_with_freshness_metadata() {
    let repo = modified_repo();
    let capture =
        parse_json(&pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let output = pointbreak(["revision", "list", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);

    assert_eq!(json["schema"], "pointbreak.review-revision-list");
    assert_eq!(json["version"], 1);
    assert!(
        json["eventSetHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(json["eventCount"], 2);
    assert_eq!(json["revisionCount"], 1);

    let entry = &json["entries"][0];
    assert_eq!(entry["revisionId"], capture["revision"]["id"]);
    assert!(!entry["capturedAt"].as_str().unwrap().is_empty());
    assert!(entry["revisionId"].as_str().unwrap().starts_with("rev:"));
    assert!(entry["objectId"].as_str().unwrap().starts_with("obj:"));
    assert!(entry["source"].is_object());
    assert!(entry["base"].is_object());
    assert!(entry["target"].is_object());
    assert!(
        entry["objectArtifactContentHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
}

#[test]
fn revision_list_does_not_expose_storage_paths() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak(["revision", "list", "--repo", repo.path().to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = parse_json(&output.stdout);

    assert!(!stdout.contains(".pointbreak/data/events"));
    assert!(!stdout.contains("artifacts/"));
    assert!(json.get("statePath").is_none());
    assert!(json["entries"][0].get("payloadHash").is_none());
    assert!(json["entries"][0].get("eventId").is_none());
}

#[test]
fn revision_list_json_pretty_prints() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
        "revision",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--format",
        "json-pretty",
    ]);

    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\n"));
}

#[test]
fn revision_list_returns_multiple_entries_in_capture_order() {
    let repo = modified_repo();
    let first =
        parse_json(&pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let second =
        parse_json(&pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let output = pointbreak(["revision", "list", "--repo", repo.path().to_str().unwrap()]);
    let json = parse_json(&output.stdout);
    let entries = json["entries"].as_array().unwrap();

    assert_ne!(first["revision"]["id"], second["revision"]["id"]);
    assert_eq!(json["revisionCount"], 2);
    assert_eq!(entries.len(), 2);
    let ids: Vec<&str> = entries
        .iter()
        .map(|entry| entry["revisionId"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&first["revision"]["id"].as_str().unwrap()));
    assert!(ids.contains(&second["revision"]["id"].as_str().unwrap()));
    assert!(
        entries[0]["capturedAt"].as_str().unwrap() <= entries[1]["capturedAt"].as_str().unwrap()
    );
}

#[test]
fn revision_list_succeeds_without_events() {
    let repo = GitRepo::new();

    let output = pointbreak(["revision", "list", "--repo", repo.path().to_str().unwrap()]);
    let json = parse_json(&output.stdout);

    assert!(output.status.success());
    assert_eq!(json["eventCount"], 0);
    assert_eq!(json["revisionCount"], 0);
    assert!(json["entries"].as_array().unwrap().is_empty());
}

#[test]
fn revision_list_reads_capture_from_the_shared_store_after_seed_worktree_removed() {
    let fixture = CloneWorktreeFixture::new();
    fs::write(fixture.seed.join("README.md"), "changed in seed\n").unwrap();
    let capture =
        parse_json(&pointbreak(["capture", "--repo", fixture.seed.to_str().unwrap()]).stdout);

    // The capture wrote through to the shared common-dir store; removing the seed
    // worktree cannot strand the record. No `store link` step.
    run_git_os(
        fixture.main.path(),
        [
            OsString::from("worktree"),
            OsString::from("remove"),
            OsString::from("--force"),
            fixture.seed.as_os_str().to_owned(),
        ],
    );
    let reader = fixture.add_worktree("reader");
    assert!(!reader.join(".pointbreak/data/events").exists());

    let output = pointbreak(["revision", "list", "--repo", reader.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "list stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json = parse_json(stdout.as_bytes());

    assert_eq!(json["eventCount"], 2);
    assert_eq!(json["revisionCount"], 1);
    assert_eq!(json["entries"][0]["revisionId"], capture["revision"]["id"]);
    assert!(json["diagnostics"].as_array().unwrap().is_empty());
    assert!(!stdout.contains(".git"));
    assert!(!stdout.contains(".pointbreak/data"));
}

#[test]
fn revision_list_omits_ambient_ambiguous_current_diagnostic_from_shared_store() {
    let fixture = CloneWorktreeFixture::new();
    fs::write(fixture.seed.join("README.md"), "changed once\n").unwrap();
    let first =
        parse_json(&pointbreak(["capture", "--repo", fixture.seed.to_str().unwrap()]).stdout);
    fs::write(fixture.seed.join("README.md"), "changed twice\n").unwrap();
    let second =
        parse_json(&pointbreak(["capture", "--repo", fixture.seed.to_str().unwrap()]).stdout);

    // Both captures wrote through to the shared common-dir store; a sibling reader
    // sees them with no `store link` step.
    let reader = fixture.add_worktree("reader");

    let output = pointbreak(["revision", "list", "--repo", reader.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "list stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    let ids = json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["revisionId"].as_str().unwrap())
        .collect::<Vec<_>>();

    assert_ne!(first["revision"]["id"], second["revision"]["id"]);
    assert_eq!(json["eventCount"], 4);
    assert_eq!(json["revisionCount"], 2);
    assert!(ids.contains(&first["revision"]["id"].as_str().unwrap()));
    assert!(ids.contains(&second["revision"]["id"].as_str().unwrap()));
    assert!(
        !json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["code"].as_str() == Some("ambiguous_current_revision")
            }),
        "routine Revision list should not emit ambient current ambiguity diagnostics"
    );
}

#[test]
fn unit_list_renders_commit_range_source_without_paths() {
    let repo = support::committed_repo();
    pointbreak([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--base",
        "HEAD~1",
    ]);

    let output = pointbreak(["revision", "list", "--repo", repo.path().to_str().unwrap()]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = parse_json(&output.stdout);

    let entry = &json["entries"][0];
    assert_eq!(entry["source"]["kind"], "git_commit_range");
    assert_eq!(entry["source"]["mode"], "base_tree_to_target_tree");
    assert_eq!(entry["base"]["kind"], "git_commit");
    assert_eq!(entry["target"]["kind"], "git_commit");
    assert!(
        !stdout.contains("worktreeRoot"),
        "range capture unit list must not expose a worktree path"
    );
}

#[test]
fn unit_list_shows_unreachable_revisions_by_default_and_filters_explicitly() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");

    // A range capture anchored to a commit on a soon-deleted branch → orphan.
    repo.git(["checkout", "-b", "feature"]);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("feature work");
    let orphan = parse_json(
        &pointbreak([
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--base",
            "main",
        ])
        .stdout,
    );
    let orphan_id = orphan["revision"]["id"].as_str().unwrap().to_owned();
    repo.git(["checkout", "main"]);
    repo.git(["branch", "-D", "feature"]);

    // A floating worktree capture on main → never hidden.
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let floating =
        parse_json(&pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    let floating_id = floating["revision"]["id"].as_str().unwrap().to_owned();
    assert_ne!(orphan_id, floating_id);

    // Default: both recorded revisions remain discoverable regardless of Git
    // reachability.
    let default_ids = unit_list_ids(&repo, &[]);
    assert!(default_ids.contains(&floating_id));
    assert!(default_ids.contains(&orphan_id));

    // --all remains a compatibility spelling of the default.
    let all_ids = unit_list_ids(&repo, &["--all"]);
    assert_eq!(all_ids, default_ids);

    // --orphans surfaces only the orphan.
    assert_eq!(
        unit_list_ids(&repo, &["--orphans"]),
        vec![orphan_id.clone()]
    );

    // --orphans takes precedence over --all.
    assert_eq!(
        unit_list_ids(&repo, &["--orphans", "--all"]),
        vec![orphan_id]
    );
}

#[test]
fn unit_list_keeps_revision_after_capture_target_is_amended() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.git(["checkout", "-b", "feature"]);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("captured change");

    let captured = parse_json(
        &pointbreak([
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--base",
            "main",
        ])
        .stdout,
    );
    let revision_id = captured["revision"]["id"].as_str().unwrap().to_owned();
    let captured_target = captured["revision"]["target"]["commitOid"]
        .as_str()
        .unwrap()
        .to_owned();

    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    repo.git(["add", "--all"]);
    repo.git(["commit", "--amend", "--no-edit"]);
    let amended_target = repo.git(["rev-parse", "HEAD"]).stdout.trim().to_owned();
    assert_ne!(captured_target, amended_target);
    assert!(
        repo.git([
            "for-each-ref",
            &format!("--contains={captured_target}"),
            "--format=%(refname)",
        ])
        .stdout
        .trim()
        .is_empty(),
        "the captured target must be unreachable from live refs"
    );

    let listed = parse_json(
        &pointbreak(["revision", "list", "--repo", repo.path().to_str().unwrap()]).stdout,
    );
    let entry = listed["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["revisionId"] == revision_id)
        .unwrap_or_else(|| panic!("amended capture target removed the revision: {listed}"));
    assert_eq!(entry["mergeStatus"], "orphaned");

    let shown = pointbreak([
        "revision",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
        &revision_id,
    ]);
    assert!(
        shown.status.success(),
        "direct revision read must still succeed: {}",
        String::from_utf8_lossy(&shown.stderr)
    );
    assert_eq!(parse_json(&shown.stdout)["revision"]["id"], revision_id);
}

#[test]
fn unit_list_attaches_merge_status_and_accepts_integration_and_worktree_flags() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("second");
    let repo_arg = repo.path().to_str().unwrap();

    // A range capture anchored to HEAD — the default branch's tip — reads
    // "merged": with no explicit --integration-ref the list narrows to the
    // repository's detected default branch, the same default `revision show`
    // applies, and tip equality counts as landed (#466).
    let range = parse_json(&pointbreak(["capture", "--repo", repo_arg, "--base", "HEAD~1"]).stdout);
    let range_id = range["revision"]["id"].as_str().unwrap().to_owned();

    // A floating worktree capture reads "unknown"; its worktree path lets it
    // survive the worktree-identity scope.
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let worktree = parse_json(&pointbreak(["capture", "--repo", repo_arg]).stdout);
    let worktree_id = worktree["revision"]["id"].as_str().unwrap().to_owned();

    // Default list: each entry carries a structural merge-status.
    let default = parse_json(&pointbreak(["revision", "list", "--repo", repo_arg]).stdout);
    let status_for = |id: &str| -> String {
        default["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["revisionId"] == id)
            .unwrap()["mergeStatus"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(status_for(&range_id), "merged");
    assert_eq!(status_for(&worktree_id), "unknown");

    // --integration-ref and --worktree parse; the worktree-identity scope keeps
    // the worktree capture.
    let scoped = pointbreak([
        "revision",
        "list",
        "--repo",
        repo_arg,
        "--integration-ref",
        "refs/heads/main",
        "--worktree",
        repo_arg,
    ]);
    assert!(
        scoped.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&scoped.stderr)
    );
    let scoped_ids: Vec<String> = parse_json(&scoped.stdout)["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["revisionId"].as_str().unwrap().to_owned())
        .collect();
    assert!(scoped_ids.contains(&worktree_id));
}

/// The merge-status `revision list` reports for `revision_id`.
fn unit_list_merge_status(repo: &GitRepo, revision_id: &str) -> String {
    let listed = parse_json(
        &pointbreak(["revision", "list", "--repo", repo.path().to_str().unwrap()]).stdout,
    );
    listed["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["revisionId"] == revision_id)
        .unwrap_or_else(|| panic!("revision {revision_id} not listed: {listed}"))["mergeStatus"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn unit_list_reads_merged_for_landed_capture_with_deleted_source_branch() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    let repo_arg = repo.path().to_str().unwrap();

    // Capture a committed range on a feature branch.
    repo.git(["checkout", "-b", "feature"]);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("change");
    let captured =
        parse_json(&pointbreak(["capture", "--repo", repo_arg, "--base", "main"]).stdout);
    let revision_id = captured["revision"]["id"].as_str().unwrap().to_owned();

    // Land it: a follow-up commit, fast-forward main to the branch tip, record
    // the landed commit on the same revision, delete the source branch. The
    // associated commit IS main's tip and no other ref contains it — the most
    // recently landed revision, which broad reachability misreads as a live tip.
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    repo.commit_all("follow-up");
    repo.git(["checkout", "main"]);
    repo.git(["merge", "--ff-only", "feature"]);
    let record = pointbreak([
        "association",
        "record",
        "--repo",
        repo_arg,
        "--track",
        "agent:codex",
        "--revision",
        &revision_id,
        "--commit",
        "HEAD",
    ]);
    assert!(
        record.status.success(),
        "association record failed: {}",
        String::from_utf8_lossy(&record.stderr)
    );
    repo.git(["branch", "-D", "feature"]);

    // The default list narrows to the detected default branch, so the landed
    // tip reads "merged" — agreeing with `revision show`'s liveness headline.
    assert_eq!(unit_list_merge_status(&repo, &revision_id), "merged");

    // A live unmerged branch still reads "open" under the narrow default.
    repo.git(["checkout", "-b", "unmerged"]);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 4 }\n");
    repo.commit_all("unmerged change");
    let open = parse_json(&pointbreak(["capture", "--repo", repo_arg, "--base", "main"]).stdout);
    let open_id = open["revision"]["id"].as_str().unwrap().to_owned();
    assert_eq!(unit_list_merge_status(&repo, &open_id), "open");
}

#[test]
fn unit_list_keeps_broad_merge_status_without_a_detectable_default_branch() {
    let repo = support::committed_repo();
    // No origin/HEAD and neither `main` nor `master` exists: there is no
    // detectable default branch, so merge-status keeps broad reachability.
    repo.git(["branch", "-m", "main", "trunk"]);
    let repo_arg = repo.path().to_str().unwrap();

    let range = parse_json(&pointbreak(["capture", "--repo", repo_arg, "--base", "HEAD~1"]).stdout);
    let range_id = range["revision"]["id"].as_str().unwrap().to_owned();

    // The trunk tip is live and no other ref contains it → broad reads "open".
    assert_eq!(unit_list_merge_status(&repo, &range_id), "open");
}

#[test]
fn unit_list_reads_side_branch_only_landing_as_orphaned_but_still_shown() {
    // A commit landed only onto a NON-default live branch: orphan visibility
    // stays broad (reachable from develop → the entry is shown), while the
    // narrow default integration ref classifies it off the default branch —
    // the same answer `revision show` gives. Locks the intended semantics.
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    let repo_arg = repo.path().to_str().unwrap();

    // Capture commit C at develop's tip, then advance develop past C so C is
    // interior: reachable from develop, not from main, and not itself a tip.
    repo.git(["checkout", "-b", "develop"]);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("change");
    let captured =
        parse_json(&pointbreak(["capture", "--repo", repo_arg, "--base", "main"]).stdout);
    let revision_id = captured["revision"]["id"].as_str().unwrap().to_owned();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    repo.commit_all("develop advance");
    repo.git(["checkout", "main"]);

    // Shown by the default list (the helper panics when the entry is absent),
    // with the narrow merge-status.
    assert_eq!(unit_list_merge_status(&repo, &revision_id), "orphaned");

    // Not a broad orphan: `--orphans` (broad reachability) excludes it.
    assert!(!unit_list_ids(&repo, &["--orphans"]).contains(&revision_id));
}

/// Run `revision list` with extra flags and return the entry ids in order.
fn unit_list_ids(repo: &GitRepo, extra: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "revision".to_owned(),
        "list".to_owned(),
        "--repo".to_owned(),
        repo.path().to_str().unwrap().to_owned(),
    ];
    args.extend(extra.iter().map(|flag| (*flag).to_owned()));
    let output = pointbreak(args);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json(&output.stdout)["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["revisionId"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn revision_list_filter_by_is_superseded() {
    let repo = modified_repo();
    let path = repo.path().to_str().unwrap();
    let first = parse_json(&pointbreak(["capture", "--repo", path]).stdout);
    let predecessor = first["revision"]["id"].as_str().unwrap().to_owned();
    // A successor must carry different content or it collapses to the same snapshot id.
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    pointbreak(["capture", "--repo", path, "--supersedes", &predecessor]);

    let json = parse_json(
        &pointbreak([
            "revision",
            "list",
            "--repo",
            path,
            "--filter",
            "is:superseded",
        ])
        .stdout,
    );
    let ids: Vec<&str> = json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["revisionId"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![predecessor.as_str()],
        "only the superseded revision matches"
    );
    assert_eq!(json["revisionCount"], 1);
}

#[test]
fn revision_list_filter_by_tag_key() {
    let repo = modified_repo();
    let path = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", path]);
    pointbreak([
        "observation",
        "add",
        "--repo",
        path,
        "--track",
        "agent:codex",
        "--title",
        "Landed",
        "--tag",
        "state-change:landed",
    ]);

    // First-colon key facet matches the revision whose observation carries the tag.
    let json = parse_json(
        &pointbreak([
            "revision",
            "list",
            "--repo",
            path,
            "--filter",
            "tag:state-change",
        ])
        .stdout,
    );
    assert_eq!(json["revisionCount"], 1);
}

#[test]
fn revision_list_filter_rejects_type_qualifier_on_revision_surface() {
    let repo = modified_repo();
    let path = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", path]);

    // `type:` is a known-but-unsupported qualifier on the revision surface:
    // a diagnostic and a non-zero exit, never a silent-empty match.
    let out = pointbreak([
        "revision",
        "list",
        "--repo",
        path,
        "--filter",
        "type:observation",
    ]);
    assert!(
        !out.status.success(),
        "type: is unsupported on the revision surface"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("type"),
        "the message names the qualifier: {stderr}"
    );
}

#[test]
fn revision_list_flagless_output_unchanged() {
    let repo = modified_repo();
    let path = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", path]);
    // An observation would appear in an overview build; the flagless path builds none,
    // so its bytes are the shared list document with no filter-added fields.
    pointbreak([
        "observation",
        "add",
        "--repo",
        path,
        "--track",
        "agent:codex",
        "--title",
        "Noise",
        "--tag",
        "state-change:landed",
    ]);

    let out = pointbreak(["revision", "list", "--repo", path]);
    let json = parse_json(&out.stdout);
    assert_eq!(json["schema"], "pointbreak.review-revision-list");
    let entry = &json["entries"][0];
    assert!(
        entry.get("overview").is_none(),
        "flagless path builds no overview (zero new cost)"
    );
    assert!(
        entry.get("attention").is_none(),
        "no filter-derived fields leak into the flagless doc"
    );
}

/// `observation list --tag` stays byte-exact whole-string AND the same store is
/// matched by `revision list --filter 'tag:<key>'` via the shared first-colon key facet.
#[test]
fn tag_shared_convention_holds_across_observation_list_and_revision_filter() {
    let repo = modified_repo();
    let path = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", path]);
    pointbreak([
        "observation",
        "add",
        "--repo",
        path,
        "--track",
        "agent:codex",
        "--title",
        "Landed",
        "--tag",
        "state-change:landed",
    ]);

    // `observation list --tag` is exact whole-string (byte-untouched).
    let obs = parse_json(
        &pointbreak([
            "observation",
            "list",
            "--repo",
            path,
            "--tag",
            "state-change:landed",
        ])
        .stdout,
    );
    assert!(
        obs["observations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["tags"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t == "state-change:landed")),
        "exact whole-string --tag still matches"
    );
    // A partial key is NOT a whole-string tag, so `observation list --tag` finds nothing.
    let partial = parse_json(
        &pointbreak([
            "observation",
            "list",
            "--repo",
            path,
            "--tag",
            "state-change",
        ])
        .stdout,
    );
    assert!(
        partial["observations"].as_array().unwrap().is_empty(),
        "--tag stays exact whole-string (never the key facet)"
    );
    // But the revision grammar's key facet matches the same store (dual index).
    let rev = parse_json(
        &pointbreak([
            "revision",
            "list",
            "--repo",
            path,
            "--filter",
            "tag:state-change",
        ])
        .stdout,
    );
    assert_eq!(rev["revisionCount"], 1);
}

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("parse CLI JSON")
}

fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

struct CloneWorktreeFixture {
    main: GitRepo,
    _worktree_parent: tempfile::TempDir,
    seed: PathBuf,
}

impl CloneWorktreeFixture {
    fn new() -> Self {
        let main = GitRepo::new();
        main.write("README.md", "base\n");
        main.commit_all("base");

        let worktree_parent = tempfile::tempdir().expect("create worktree parent");
        let seed = worktree_parent.path().join("seed");
        add_worktree(main.path(), &seed, "seed");

        Self {
            main,
            _worktree_parent: worktree_parent,
            seed,
        }
    }

    fn add_worktree(&self, branch: &str) -> PathBuf {
        let path = self._worktree_parent.path().join(branch);
        add_worktree(self.main.path(), &path, branch);
        path
    }
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
fn text_revision_list_digest_is_one_line_per_revision() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
        "revision",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--format",
        "text",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("\"schema\""),
        "text lane is not JSON: {stdout}"
    );
    assert!(stdout.contains("1 revision"), "count headline: {stdout}");
    assert!(stdout.contains("rev:"), "short revision id: {stdout}");
    assert!(stdout.contains("worktree"), "target endpoint: {stdout}");
    assert_eq!(
        stdout.trim_end().lines().count(),
        2,
        "header plus one line per revision: {stdout}"
    );
}

#[test]
fn text_revision_list_digest_reports_an_empty_store() {
    let repo = modified_repo();

    let output = pointbreak([
        "revision",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--format",
        "text",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("no revisions"), "empty line: {stdout}");
    assert!(
        !stdout.contains("\"schema\""),
        "text lane is not JSON: {stdout}"
    );
}
