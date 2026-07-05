mod support;

use std::path::Path;
use std::process::Output;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::{shore, shore_env};

fn out_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn err_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Strip ANSI SGR sequences (`ESC [ … m`) from a string. Escapes are ASCII, so
/// multibyte code points in the surrounding text pass through untouched.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            let mut lookahead = chars.clone();
            if lookahead.next() == Some('[') {
                chars = lookahead;
                for cc in chars.by_ref() {
                    if cc == 'm' {
                        break;
                    }
                }
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// A repo with one committed base and an uncommitted single-line change, so
/// `shore review capture` records a one-file worktree diff.
fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

/// Capture the current worktree and return the `shore.review-capture` document.
fn capture(path: &Path) -> Value {
    let output = shore(["capture", "--repo", path.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "capture failed:\n{}",
        err_text(&output)
    );
    serde_json::from_slice(&output.stdout).expect("capture emits JSON")
}

#[test]
fn shore_diff_prints_the_captured_unified_diff() {
    let repo = modified_repo();
    capture(repo.path());

    let output = shore(["diff", "--repo", repo.path().to_str().unwrap()]);
    assert!(output.status.success(), "stderr:\n{}", err_text(&output));
    let text = out_text(&output);
    assert!(text.contains("diff --git a/src/lib.rs b/src/lib.rs"));
    assert!(text.contains("@@"));
    assert!(text.contains("-pub fn value() -> u32 { 1 }"));
    assert!(text.contains("+pub fn value() -> u32 { 2 }"));
    assert!(text.contains("file changed")); // diffstat header
}

#[test]
fn shore_diff_stat_omits_the_body() {
    let repo = modified_repo();
    capture(repo.path());

    let output = shore(["diff", "--repo", repo.path().to_str().unwrap(), "--stat"]);
    assert!(output.status.success(), "stderr:\n{}", err_text(&output));
    let text = out_text(&output);
    assert!(text.contains("src/lib.rs"));
    assert!(text.contains("file changed"));
    assert!(!text.contains("@@")); // no hunk body under --stat
}

#[test]
fn shore_diff_requires_revision_when_multiple_candidates() {
    let repo = modified_repo();
    capture(repo.path());
    // A second, different capture in the same worktree → two candidates.
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    capture(repo.path());

    let output = shore(["diff", "--repo", repo.path().to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(
        err_text(&output).contains("multiple captured revisions"),
        "stderr:\n{}",
        err_text(&output)
    );
}

#[test]
fn shore_diff_renders_content_unavailable_when_removed() {
    let repo = modified_repo();
    let captured = capture(repo.path());
    let snapshot_id = captured["revision"]["objectId"].as_str().unwrap();

    let removed = shore([
        "store",
        "remove",
        "--repo",
        repo.path().to_str().unwrap(),
        "--snapshot",
        snapshot_id,
    ]);
    assert!(removed.status.success(), "stderr:\n{}", err_text(&removed));

    let output = shore(["diff", "--repo", repo.path().to_str().unwrap()]);
    assert!(output.status.success(), "stderr:\n{}", err_text(&output));
    let text = out_text(&output).to_lowercase();
    assert!(
        text.contains("unavailable") || text.contains("removed"),
        "stdout:\n{text}"
    );
    assert!(!text.contains("@@")); // no diff body when content is gone
}

#[test]
fn shore_diff_ignores_ambient_shore_format_json() {
    // `shore diff` is text-only: a global machine-format pin must not break it.
    let repo = modified_repo();
    capture(repo.path());

    let output = shore_env(
        ["diff", "--repo", repo.path().to_str().unwrap()],
        &[("SHORE_FORMAT", "json")],
    );
    assert!(output.status.success(), "stderr:\n{}", err_text(&output));
    let text = out_text(&output);
    assert!(text.contains("diff --git")); // text output, not JSON
    assert!(!text.trim_start().starts_with('{'));
}

#[test]
fn shore_diff_rejects_explicit_format_flag() {
    // An explicit request for a lane that does not exist is an error (ADR-0030 D2).
    let repo = modified_repo();
    capture(repo.path());

    let output = shore([
        "diff",
        "--repo",
        repo.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(!output.status.success()); // clap: unexpected argument
}

#[test]
fn shore_diff_color_always_emits_ansi() {
    // `modified_repo` captures a `.rs` change, so a language is detected and tokens exist.
    let repo = modified_repo();
    capture(repo.path());

    let out = shore([
        "diff",
        "--repo",
        repo.path().to_str().unwrap(),
        "--color",
        "always",
    ]);
    assert!(out.status.success(), "stderr:\n{}", err_text(&out));
    assert!(out_text(&out).contains('\x1b')); // ANSI escapes present
}

#[test]
fn shore_diff_color_never_and_piped_default_are_plain() {
    let repo = modified_repo();
    capture(repo.path());

    let never = shore([
        "diff",
        "--repo",
        repo.path().to_str().unwrap(),
        "--color",
        "never",
    ]);
    assert!(never.status.success(), "stderr:\n{}", err_text(&never)); // assert exit first
    assert!(!out_text(&never).contains('\x1b'));

    // The default under a piped (non-TTY) test harness is also plain (INV-D).
    let default_piped = shore(["diff", "--repo", repo.path().to_str().unwrap()]);
    assert!(
        default_piped.status.success(),
        "stderr:\n{}",
        err_text(&default_piped)
    );
    assert!(!out_text(&default_piped).contains('\x1b'));
}

#[test]
fn shore_diff_color_text_is_identical_to_plain_after_stripping_ansi() {
    // Presentation never changes content (INV-D): strip SGR from the colored
    // output and it equals the plain output byte-for-byte.
    let repo = modified_repo();
    capture(repo.path());

    let colored = shore([
        "diff",
        "--repo",
        repo.path().to_str().unwrap(),
        "--color",
        "always",
    ]);
    let plain = shore([
        "diff",
        "--repo",
        repo.path().to_str().unwrap(),
        "--color",
        "never",
    ]);
    assert!(colored.status.success() && plain.status.success());
    assert_eq!(strip_ansi(&out_text(&colored)), out_text(&plain));
}
