//! Full-document characterization guard for the `shore review-*` command JSON.
//!
//! These tests drive the real `shore` binary against a deterministic fixture
//! repository and assert that the ENTIRE serialized document (the normalized
//! stdout string, preserving key order) stays stable. Because the documents are
//! content-addressed, the only nondeterministic substrings are content hashes,
//! timestamps, and the absolute repository path, which are normalized to fixed
//! placeholders before comparison.
//!
//! Snapshots live under `tests/fixtures/review_documents/<command>.snap`. Run
//! with `BLESS=1` to (re)generate them from the current binary; otherwise the
//! normalized output is asserted against the stored snapshot.
//!
//! The guard exists so the #118 extraction of the document/envelope layer into
//! `shoreline::documents` provably preserves the documented bytes, field order,
//! renames, and `skip_serializing_if` behavior.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

fn snapshot_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/review_documents")
}

/// Canonicalized absolute path of the fixture repo, as it appears in command
/// output (e.g. `worktreeRoot`). macOS resolves `/var` to `/private/var`, so we
/// compare against the canonical form.
fn canonical_repo_path(repo: &GitRepo) -> String {
    repo.path()
        .canonicalize()
        .expect("canonicalize fixture repo path")
        .to_string_lossy()
        .into_owned()
}

/// Replace every `sha256:<64 lowercase hex>` with `sha256:<h>`.
fn normalize_hashes(text: &str) -> String {
    replace_prefixed(text, "sha256:", "sha256:<h>", |rest| {
        let hex: String = rest.chars().take(64).collect();
        if hex.len() == 64
            && hex
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            Some(64)
        } else {
            None
        }
    })
}

/// Replace every `unix-ms:<digits>` with `unix-ms:<t>`.
fn normalize_timestamps(text: &str) -> String {
    replace_prefixed(text, "unix-ms:", "unix-ms:<t>", |rest| {
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 { Some(digits) } else { None }
    })
}

/// Scan `text` for occurrences of `prefix`; when `match_len` accepts the suffix
/// (returning how many chars after the prefix the token consumes), replace the
/// whole `prefix + token` span with `replacement`.
fn replace_prefixed(
    text: &str,
    prefix: &str,
    replacement: &str,
    match_len: impl Fn(&str) -> Option<usize>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(idx) = rest.find(prefix) {
        out.push_str(&rest[..idx]);
        let after_prefix = &rest[idx + prefix.len()..];
        if let Some(token_len) = match_len(after_prefix) {
            out.push_str(replacement);
            let byte_len: usize = after_prefix
                .chars()
                .take(token_len)
                .map(char::len_utf8)
                .sum();
            rest = &after_prefix[byte_len..];
        } else {
            out.push_str(prefix);
            rest = after_prefix;
        }
    }
    out.push_str(rest);
    out
}

/// Replace a 40-char lowercase-hex git object id that immediately follows
/// `"<key>":"` with `<oid>`. Git OIDs depend on commit timestamp/author and so
/// vary between runs even for identical content.
fn normalize_git_oid(text: &str, key: &str) -> String {
    let prefix = format!("\"{key}\":\"");
    replace_prefixed(text, &prefix, &format!("\"{key}\":\"<oid>"), |rest| {
        let hex: String = rest.chars().take(40).collect();
        let closes = rest.chars().nth(40) == Some('"');
        if hex.len() == 40
            && closes
            && hex
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            Some(40)
        } else {
            None
        }
    })
}

/// Replace the JSON string value of `"worktreeRoot":"…"` with `<repo>`,
/// regardless of path shape. This is portable across platforms: on Windows the
/// captured path is an extended-length `\\?\C:\…` form with escaped backslashes
/// that a literal path replacement would miss.
fn normalize_worktree_root(text: &str) -> String {
    replace_prefixed(
        text,
        "\"worktreeRoot\":\"",
        "\"worktreeRoot\":\"<repo>",
        |rest| {
            // Consume the JSON string contents up to (not including) the closing
            // unescaped quote, honoring backslash escapes.
            let mut escaped = false;
            let mut chars = 0usize;
            for c in rest.chars() {
                if escaped {
                    escaped = false;
                    chars += 1;
                    continue;
                }
                match c {
                    '\\' => {
                        escaped = true;
                        chars += 1;
                    }
                    '"' => return Some(chars),
                    _ => chars += 1,
                }
            }
            None
        },
    )
}

/// Normalize the nondeterministic substrings while preserving every key and the
/// document's exact serialized field order.
fn normalize(raw: &str, repo_path: &str) -> String {
    // Strip Windows CRLF so snapshots compare equal regardless of the platform's
    // git autocrlf checkout behavior.
    let text = raw.replace("\r\n", "\n");
    // Replace the absolute repo path first (Unix), then the worktreeRoot value
    // generically (covers the Windows extended-length path form).
    let text = text.replace(repo_path, "<repo>");
    let text = normalize_worktree_root(&text);
    let text = normalize_hashes(&text);
    let text = normalize_timestamps(&text);
    let text = normalize_git_oid(&text, "commitOid");
    normalize_git_oid(&text, "treeOid")
}

#[track_caller]
fn run_command(repo: &GitRepo, args: &[&str]) -> String {
    let output = shore(args);
    assert!(
        output.status.success(),
        "command {args:?} failed\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let raw = String::from_utf8(output.stdout).expect("stdout is utf-8");
    normalize(&raw, &canonical_repo_path(repo))
}

#[track_caller]
fn assert_snapshot(name: &str, normalized: &str) {
    let path = snapshot_dir().join(format!("{name}.snap"));
    if std::env::var_os("BLESS").is_some() {
        fs::create_dir_all(snapshot_dir()).expect("create snapshot dir");
        fs::write(&path, normalized).expect("write snapshot");
        return;
    }
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|error| {
            panic!(
                "missing snapshot {}: {error}. Re-run with BLESS=1 to generate it.",
                path.display()
            )
        })
        .replace("\r\n", "\n");
    assert_eq!(
        normalized,
        expected,
        "documented JSON for `{name}` drifted from the stored snapshot {}.\n\
         If this change is intentional, re-run with BLESS=1 to regenerate.",
        path.display()
    );
}

/// Build the deterministic fixture repo and capture a single ReviewUnit, returning
/// the captured review-unit id (already normalized in snapshots, used here only to
/// pass back into commands as a literal argument).
fn fixture_repo() -> (GitRepo, String) {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");

    let raw = String::from_utf8(
        shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout,
    )
    .expect("capture stdout is utf-8");
    let value: Value = serde_json::from_str(&raw).expect("valid capture json");
    let review_unit_id = value["reviewUnit"]["id"]
        .as_str()
        .expect("review unit id")
        .to_owned();
    (repo, review_unit_id)
}

fn repo_arg(repo: &GitRepo) -> String {
    repo.path().to_str().unwrap().to_owned()
}

/// One test exercises all twelve documented `shore review-*` commands against a
/// single deterministic fixture, snapshotting the full normalized document for
/// each. Driving them in sequence keeps content-addressed ids stable across
/// commands (each new write references the same captured ReviewUnit).
#[test]
fn review_documents_are_byte_stable() {
    // 1. review capture (re-run on a fresh repo to snapshot the capture document
    //    itself; the shared fixture below reuses its own capture).
    {
        let repo = GitRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        let out = run_command(&repo, &["review", "capture", "--repo", &repo_arg(&repo)]);
        assert_snapshot("capture", &out);
    }

    let (repo, unit) = fixture_repo();
    let repo_path = repo_arg(&repo);

    // 2. review observation add
    let observation_add = run_command(
        &repo,
        &[
            "review",
            "observation",
            "add",
            "--repo",
            &repo_path,
            "--track",
            "agent:codex",
            "--title",
            "Observed change",
            "--body",
            "the return value changed",
        ],
    );
    assert_snapshot("observation_add", &observation_add);

    // 3. review observation list
    let observation_list = run_command(
        &repo,
        &[
            "review",
            "observation",
            "list",
            "--repo",
            &repo_path,
            "--include-body",
        ],
    );
    assert_snapshot("observation_list", &observation_list);

    // 4. review input-request open
    let open_raw = shore([
        "review",
        "input-request",
        "open",
        "--repo",
        &repo_path,
        "--track",
        "agent:codex",
        "--title",
        "Need a decision",
        "--reason",
        "manual-decision-required",
        "--body",
        "should we ship this?",
    ]);
    assert!(open_raw.status.success());
    let open_value: Value =
        serde_json::from_slice(&open_raw.stdout).expect("valid input-request open json");
    let input_request_id = open_value["inputRequestId"]
        .as_str()
        .expect("input request id")
        .to_owned();
    let input_request_open = normalize(
        &String::from_utf8(open_raw.stdout).unwrap(),
        &canonical_repo_path(&repo),
    );
    assert_snapshot("input_request_open", &input_request_open);

    // 5. review input-request fetch
    let input_request_fetch = run_command(
        &repo,
        &[
            "review",
            "input-request",
            "fetch",
            &input_request_id,
            "--repo",
            &repo_path,
            "--include-body",
        ],
    );
    assert_snapshot("input_request_fetch", &input_request_fetch);

    // 6. review input-request respond
    let input_request_respond = run_command(
        &repo,
        &[
            "review",
            "input-request",
            "respond",
            &input_request_id,
            "--repo",
            &repo_path,
            "--outcome",
            "approved",
            "--reason",
            "looks good",
        ],
    );
    assert_snapshot("input_request_respond", &input_request_respond);

    // 7. review input-request list (after a response so the view includes it)
    let input_request_list = run_command(
        &repo,
        &[
            "review",
            "input-request",
            "list",
            "--repo",
            &repo_path,
            "--status",
            "all",
            "--include-body",
        ],
    );
    assert_snapshot("input_request_list", &input_request_list);

    // 8. review assessment add
    let assessment_add = run_command(
        &repo,
        &[
            "review",
            "assessment",
            "add",
            "--repo",
            &repo_path,
            "--track",
            "human:kevin",
            "--assessment",
            "accepted",
            "--summary",
            "ship it",
        ],
    );
    assert_snapshot("assessment_add", &assessment_add);

    // 9. review assessment show
    let assessment_show = run_command(
        &repo,
        &[
            "review",
            "assessment",
            "show",
            "--repo",
            &repo_path,
            "--include-summary",
        ],
    );
    assert_snapshot("assessment_show", &assessment_show);

    // 10. review unit show
    let unit_show = run_command(
        &repo,
        &[
            "review",
            "unit",
            "show",
            "--repo",
            &repo_path,
            "--review-unit",
            &unit,
            "--include-body",
        ],
    );
    assert_snapshot("unit_show", &unit_show);

    // 11. review unit list
    let unit_list = run_command(&repo, &["review", "unit", "list", "--repo", &repo_path]);
    assert_snapshot("unit_list", &unit_list);

    // 12. review history
    let history = run_command(
        &repo,
        &["review", "history", "--repo", &repo_path, "--include-body"],
    );
    assert_snapshot("history", &history);
}
