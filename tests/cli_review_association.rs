mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

fn parse_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout is valid JSON")
}

fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

/// Two committed revisions plus a dirty worktree, so associating both `HEAD` and
/// `HEAD~1` yields two distinct current commit associations — the divergent-tip
/// shape a squash or rebase leaves behind.
fn divergent_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("second");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    repo
}

/// Two committed revisions on `main`, clean worktree — so `--base HEAD~1`
/// captures the committed range and anchors a reachable target commit.
fn committed_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("change");
    repo
}

fn record_commit(repo: &GitRepo, commit: &str) {
    let output = shore([
        "association",
        "record",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--commit",
        commit,
    ]);
    assert!(
        output.status.success(),
        "association record --commit {commit} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn capture(repo: &GitRepo) {
    let output = shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    assert!(
        output.status.success(),
        "capture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn record_commit_writes_then_reports_existing_on_rerun() {
    let repo = modified_repo();
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();

    let first = shore([
        "association",
        "record",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--commit",
        "HEAD",
    ]);
    assert!(
        first.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let json = parse_json(&first.stdout);
    assert_eq!(json["schema"], "pointbreak.review-association-commit");
    assert_eq!(json["eventsCreated"], 1);
    assert_eq!(json["eventsCreatedByType"]["revision_commit_associated"], 1);
    let association_id = json["commitAssociationId"].as_str().unwrap();
    assert!(association_id.starts_with("assoc-commit:"));
    assert!(json["eventId"].as_str().unwrap().starts_with("evt:sha256:"));

    let again = shore([
        "association",
        "record",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--commit",
        "HEAD",
    ]);
    let json = parse_json(&again.stdout);
    assert_eq!(json["eventsCreated"], 0);
    assert_eq!(json["eventsExisting"], 1);
    assert_eq!(json["commitAssociationId"], association_id);
}

#[test]
fn withdraw_removes_from_current_list() {
    let repo = modified_repo();
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();

    let recorded = parse_json(
        &shore([
            "association",
            "record",
            "--repo",
            repo_path,
            "--track",
            "agent:codex",
            "--commit",
            "HEAD",
        ])
        .stdout,
    );
    let association_id = recorded["commitAssociationId"].as_str().unwrap();

    let current_before = parse_json(
        &shore([
            "association",
            "list",
            "--repo",
            repo_path,
            "--axis",
            "commit",
            "--current",
        ])
        .stdout,
    );
    assert_eq!(
        current_before["currentCommits"].as_array().unwrap().len(),
        1
    );

    let withdraw = shore([
        "association",
        "withdraw",
        association_id,
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
    ]);
    assert!(
        withdraw.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&withdraw.stderr)
    );
    let json = parse_json(&withdraw.stdout);
    assert_eq!(
        json["schema"],
        "pointbreak.review-association-commit-withdrawn"
    );
    assert_eq!(json["commitAssociationId"], association_id);

    let current_after = parse_json(
        &shore([
            "association",
            "list",
            "--repo",
            repo_path,
            "--axis",
            "commit",
            "--current",
        ])
        .stdout,
    );
    assert!(
        current_after["currentCommits"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn record_ref_stores_full_ref_and_head() {
    let repo = modified_repo();
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();
    let head_oid = repo.git(["rev-parse", "HEAD"]).stdout.trim().to_owned();

    let output = shore([
        "association",
        "record",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--ref",
        "refs/heads/feat/x",
        "--head",
        &head_oid,
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["schema"], "pointbreak.review-association-ref");
    assert_eq!(json["refName"], "refs/heads/feat/x");
    assert_eq!(json["headOid"], head_oid);
    assert!(
        json["refAssociationId"]
            .as_str()
            .unwrap()
            .starts_with("assoc-ref:")
    );
}

#[test]
fn record_ref_normalizes_a_short_branch_name() {
    let repo = modified_repo();
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();

    let json = parse_json(
        &shore([
            "association",
            "record",
            "--repo",
            repo_path,
            "--track",
            "agent:codex",
            "--branch",
            "feat/short",
            "--head",
            "abc123",
        ])
        .stdout,
    );
    assert_eq!(json["refName"], "refs/heads/feat/short");
}

#[test]
fn list_axis_commit_excludes_ref_associations() {
    let repo = modified_repo();
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();

    shore([
        "association",
        "record",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--commit",
        "HEAD",
    ]);
    shore([
        "association",
        "record",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--ref",
        "refs/heads/feat/x",
        "--head",
        "abc123",
    ]);

    let json = parse_json(
        &shore([
            "association",
            "list",
            "--repo",
            repo_path,
            "--axis",
            "commit",
        ])
        .stdout,
    );
    assert_eq!(json["currentCommits"].as_array().unwrap().len(), 1);
    assert!(json["currentRefs"].as_array().unwrap().is_empty());
}

#[test]
fn history_filters_to_the_commit_associated_event_type() {
    let repo = modified_repo();
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();
    shore([
        "association",
        "record",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--commit",
        "HEAD",
    ]);

    let json = parse_json(
        &shore([
            "history",
            "--repo",
            repo_path,
            "--event-type",
            "revision-commit-associated",
        ])
        .stdout,
    );
    let entries = json["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["eventType"], "revision_commit_associated");
    assert_eq!(entries[0]["summary"]["kind"], "revision_commit_associated");
}

#[test]
fn unit_list_ref_label_filter_matches_normalized_short_branch() {
    let repo = modified_repo();
    repo.git(["branch", "-M", "feat/x"]);
    capture(&repo); // auto-records refs/heads/feat/x
    let repo_path = repo.path().to_str().unwrap();

    // A short branch name is normalized to the stored full ref.
    let matched = parse_json(
        &shore([
            "revision", "list", "--repo", repo_path, "--branch", "feat/x", "--by", "label",
        ])
        .stdout,
    );
    assert_eq!(matched["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        matched["entries"][0]["commitRange"]["currentRefs"][0]["refName"],
        "refs/heads/feat/x"
    );

    let unmatched = parse_json(
        &shore([
            "revision",
            "list",
            "--repo",
            repo_path,
            "--ref",
            "refs/heads/other",
        ])
        .stdout,
    );
    assert!(unmatched["entries"].as_array().unwrap().is_empty());
}

#[test]
fn unit_list_ref_liveness_filter_matches_reachable_commit() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("change");
    repo.git(["branch", "-M", "main"]);
    let repo_path = repo.path().to_str().unwrap();

    // A commit-range capture anchors the target (HEAD) commit.
    let capture = shore(["capture", "--repo", repo_path, "--base", "HEAD~1"]);
    assert!(capture.status.success());

    let json = parse_json(
        &shore([
            "revision",
            "list",
            "--repo",
            repo_path,
            "--ref",
            "refs/heads/main",
            "--by",
            "liveness",
        ])
        .stdout,
    );
    assert_eq!(
        json["entries"].as_array().unwrap().len(),
        1,
        "the anchored target commit is reachable from main"
    );
}

#[test]
fn unit_show_includes_commit_range_and_liveness_block() {
    let repo = modified_repo();
    repo.git(["branch", "-M", "main"]);
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();
    shore([
        "association",
        "record",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--commit",
        "HEAD",
    ]);

    let json = parse_json(&shore(["revision", "show", "--repo", repo_path]).stdout);
    let commit_range = &json["commitRange"];
    assert_eq!(commit_range["anchored"], true);
    assert_eq!(commit_range["currentCommits"].as_array().unwrap().len(), 1);
    // The liveness block is layered CLI-side from the live repo.
    let per_commit = commit_range["liveness"]["perCommit"].as_array().unwrap();
    assert_eq!(per_commit.len(), 1);
    assert!(per_commit[0]["condition"].is_string());
}

#[test]
fn text_association_digest_phrases_divergence() {
    let repo = divergent_repo();
    capture(&repo);
    record_commit(&repo, "HEAD");
    record_commit(&repo, "HEAD~1");
    let repo_path = repo.path().to_str().unwrap();

    let output = shore([
        "association",
        "list",
        "--repo",
        repo_path,
        "--format",
        "text",
    ]);
    assert!(
        output.status.success(),
        "list --format text failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("anchored"),
        "digest names the anchor state: {stdout}"
    );
    assert!(
        stdout.contains("2 current commit associations"),
        "digest counts the current commit associations: {stdout}"
    );
    // The human phrasing of the diagnostic, not the raw machine code.
    assert!(
        stdout.contains("diverge"),
        "digest phrases divergence: {stdout}"
    );
    assert!(
        !stdout.contains("divergent_commit_association"),
        "the raw diagnostic code stays machine-side: {stdout}"
    );
    assert!(
        !stdout.contains("\"schema\""),
        "the text lane is not JSON: {stdout}"
    );
}

#[test]
fn text_association_digest_renders_clean_single_association() {
    let repo = modified_repo();
    capture(&repo);
    record_commit(&repo, "HEAD");
    let repo_path = repo.path().to_str().unwrap();
    let head_oid = repo.git(["rev-parse", "HEAD"]).stdout.trim().to_owned();
    shore([
        "association",
        "record",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--ref",
        "refs/heads/feat/x",
        "--head",
        &head_oid,
    ]);

    let output = shore([
        "association",
        "list",
        "--repo",
        repo_path,
        "--format",
        "text",
    ]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // "1 current commit association" — a single, un-diverged edge.
    assert!(
        stdout.contains("assoc"),
        "digest names associations: {stdout}"
    );
    assert!(
        stdout.contains("feat/x"),
        "digest shows the ref name: {stdout}"
    );
    assert!(
        !stdout.contains("diverge"),
        "a single association carries no divergence language: {stdout}"
    );
    assert!(
        !stdout.contains("\"schema\""),
        "the text lane is not JSON: {stdout}"
    );
}

#[test]
fn text_association_digest_reports_landing_when_liveness_resolves() {
    let repo = committed_repo();
    let repo_path = repo.path().to_str().unwrap();
    // A commit-range capture anchors the target (HEAD) commit, reachable from main.
    let capture = shore(["capture", "--repo", repo_path, "--base", "HEAD~1"]);
    assert!(
        capture.status.success(),
        "capture failed: {}",
        String::from_utf8_lossy(&capture.stderr)
    );
    record_commit(&repo, "HEAD");

    let output = shore([
        "association",
        "list",
        "--repo",
        repo_path,
        "--format",
        "text",
    ]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("landing:"),
        "digest carries a landing line: {stdout}"
    );
    assert!(
        !stdout.contains("landing: unknown"),
        "a reachable anchor resolves to a real headline, not the placeholder: {stdout}"
    );
}

#[test]
fn text_association_digest_reads_orphaned_anchor_as_orphaned() {
    // A range capture anchored to a commit on a soon-deleted branch: the commit
    // survives in the object store but no live ref reaches it → orphaned. Liveness
    // resolves it (a real status), so the headline is `orphaned`, never `unknown`,
    // and the read still exits 0 (INV-10).
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.git(["checkout", "-b", "feature"]);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("feature work");
    let repo_path = repo.path().to_str().unwrap();
    let capture = shore(["capture", "--repo", repo_path, "--base", "main"]);
    assert!(capture.status.success());
    let revision_id = parse_json(&capture.stdout)["revision"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    repo.git(["checkout", "main"]);
    repo.git(["branch", "-D", "feature"]);

    // The current worktree no longer resolves this revision, so name it explicitly.
    let output = shore([
        "association",
        "list",
        "--repo",
        repo_path,
        "--revision",
        &revision_id,
        "--format",
        "text",
    ]);
    assert!(
        output.status.success(),
        "the digest exits 0 even when the anchor is orphaned"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("landing: orphaned"),
        "an unreachable anchor reads as orphaned: {stdout}"
    );
}

#[test]
fn association_record_commit_emits_frozen_schema() {
    let repo = modified_repo();
    capture(&repo);
    let output = shore([
        "association",
        "record",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--commit",
        "HEAD",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["schema"], "pointbreak.review-association-commit");
    let id = json["commitAssociationId"].as_str().unwrap();
    assert!(id.starts_with("assoc-commit:"));
}

#[test]
fn association_record_axis_is_exclusive_and_ref_requires_head() {
    let repo = modified_repo();
    capture(&repo);
    let path = repo.path().to_str().unwrap();
    let head = repo.git(["rev-parse", "HEAD"]).stdout.trim().to_owned();

    // --commit alone: accepted.
    assert!(
        shore([
            "association",
            "record",
            "--repo",
            path,
            "--track",
            "t",
            "--commit",
            "HEAD",
        ])
        .status
        .success()
    );
    // --ref + --head: accepted.
    assert!(
        shore([
            "association",
            "record",
            "--repo",
            path,
            "--track",
            "t",
            "--ref",
            "main",
            "--head",
            &head,
        ])
        .status
        .success()
    );
    // --commit + --ref: rejected (exclusive group).
    assert!(
        !shore([
            "association",
            "record",
            "--repo",
            path,
            "--track",
            "t",
            "--commit",
            "HEAD",
            "--ref",
            "main",
            "--head",
            &head,
        ])
        .status
        .success()
    );
    // --ref without --head: rejected (requires).
    assert!(
        !shore([
            "association",
            "record",
            "--repo",
            path,
            "--track",
            "t",
            "--ref",
            "main",
        ])
        .status
        .success()
    );
}

#[test]
fn association_withdraw_takes_a_positional_prefixed_id() {
    let repo = modified_repo();
    capture(&repo);
    let path = repo.path().to_str().unwrap();
    let recorded = parse_json(
        &shore([
            "association",
            "record",
            "--repo",
            path,
            "--track",
            "agent:codex",
            "--commit",
            "HEAD",
        ])
        .stdout,
    );
    let association_id = recorded["commitAssociationId"].as_str().unwrap();

    let out = shore([
        "association",
        "withdraw",
        association_id,
        "--repo",
        path,
        "--track",
        "agent:codex",
    ]);
    assert!(
        out.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        parse_json(&out.stdout)["schema"],
        "pointbreak.review-association-commit-withdrawn"
    );
}

#[test]
fn association_withdraw_resolves_a_prefixed_short_id_and_rejects_bare_fragments() {
    let repo = modified_repo();
    capture(&repo);
    let path = repo.path().to_str().unwrap();
    let recorded = parse_json(
        &shore([
            "association",
            "record",
            "--repo",
            path,
            "--track",
            "agent:codex",
            "--commit",
            "HEAD",
        ])
        .stdout,
    );
    let association_id = recorded["commitAssociationId"].as_str().unwrap().to_owned();
    // association_id = "assoc-commit:sha256:<hex>".
    let digest = &association_id["assoc-commit:sha256:".len()..];
    let prefixed_short = format!("assoc-commit:{}", &digest[..8]);

    // Bare fragment: rejected — the positional accepts two kinds, so the prefix
    // is required.
    let bare = shore([
        "association",
        "withdraw",
        &digest[..8],
        "--repo",
        path,
        "--track",
        "agent:codex",
    ]);
    assert!(!bare.status.success());
    assert!(
        String::from_utf8_lossy(&bare.stderr).contains("prefix"),
        "the error names the prefixed form as the fix: {}",
        String::from_utf8_lossy(&bare.stderr)
    );

    // Prefixed short form: resolves; the emitted document carries the FULL id,
    // not the fragment.
    let out = shore([
        "association",
        "withdraw",
        &prefixed_short,
        "--repo",
        path,
        "--track",
        "agent:codex",
    ]);
    assert!(
        out.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        doc.contains(&association_id),
        "the withdrawal document references the resolved full association id: {doc}"
    );
    assert_eq!(
        parse_json(&out.stdout)["schema"],
        "pointbreak.review-association-commit-withdrawn"
    );
}

#[test]
fn association_write_verbs_document_sign_key() {
    for verb in ["record", "withdraw"] {
        let help = String::from_utf8(shore(["association", verb, "--help"]).stdout).unwrap();
        assert!(
            help.contains("signing never"),
            "{verb} --help omits the --sign-key doc:\n{help}"
        );
    }
}

#[test]
fn association_documents_stay_per_axis() {
    let repo = modified_repo();
    capture(&repo);
    let path = repo.path().to_str().unwrap();
    let head = repo.git(["rev-parse", "HEAD"]).stdout.trim().to_owned();

    // record --commit → today's associate-commit document, unchanged.
    let commit = parse_json(
        &shore([
            "association",
            "record",
            "--repo",
            path,
            "--track",
            "t",
            "--commit",
            "HEAD",
        ])
        .stdout,
    );
    assert_eq!(commit["schema"], "pointbreak.review-association-commit");
    // record --ref → today's associate-ref document, unchanged.
    let ref_assoc = parse_json(
        &shore([
            "association",
            "record",
            "--repo",
            path,
            "--track",
            "t",
            "--ref",
            "main",
            "--head",
            &head,
        ])
        .stdout,
    );
    assert_eq!(ref_assoc["schema"], "pointbreak.review-association-ref");

    // withdraw resolves the axis by prefix; each emits its own withdrawn document.
    let wc = parse_json(
        &shore([
            "association",
            "withdraw",
            commit["commitAssociationId"].as_str().unwrap(),
            "--repo",
            path,
            "--track",
            "t",
        ])
        .stdout,
    );
    assert_eq!(
        wc["schema"],
        "pointbreak.review-association-commit-withdrawn"
    );
    let wr = parse_json(
        &shore([
            "association",
            "withdraw",
            ref_assoc["refAssociationId"].as_str().unwrap(),
            "--repo",
            path,
            "--track",
            "t",
        ])
        .stdout,
    );
    assert_eq!(wr["schema"], "pointbreak.review-association-ref-withdrawn");
}

#[test]
fn association_verbs_reject_a_replaces_flag() {
    let repo = modified_repo();
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();

    let output = shore([
        "association",
        "record",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--commit",
        "HEAD",
        "--replaces",
        "anything",
    ]);
    assert!(
        !output.status.success(),
        "withdraw-only family must not accept --replaces"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--replaces")
            || String::from_utf8_lossy(&output.stderr).contains("unexpected"),
        "clap should reject --replaces: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
