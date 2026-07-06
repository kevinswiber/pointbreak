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
/// `shore capture` records a one-file worktree diff.
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

#[test]
fn diff_revision_flag_resolves_short_ids() {
    let repo = modified_repo();
    let captured = capture(repo.path());
    let full = captured["revision"]["id"].as_str().unwrap().to_owned();
    let digest = full.rsplit_once("sha256:").unwrap().1.to_owned();
    let path = repo.path().to_str().unwrap();

    let full_out = shore(["diff", "--repo", path, "--revision", &full, "--stat"]);
    assert!(
        full_out.status.success(),
        "stderr:\n{}",
        err_text(&full_out)
    );

    // Prefixed short form resolves to the same revision.
    let prefixed = format!("rev:{}", &digest[..8]);
    let prefixed_out = shore(["diff", "--repo", path, "--revision", &prefixed, "--stat"]);
    assert!(
        prefixed_out.status.success(),
        "stderr:\n{}",
        err_text(&prefixed_out)
    );
    assert_eq!(out_text(&prefixed_out), out_text(&full_out));

    // Bare fragment: `--revision` implies exactly one id kind, so it resolves too.
    let bare_out = shore(["diff", "--repo", path, "--revision", &digest[..8], "--stat"]);
    assert!(
        bare_out.status.success(),
        "stderr:\n{}",
        err_text(&bare_out)
    );
    assert_eq!(out_text(&bare_out), out_text(&full_out));
}

// --- Theme surface: --theme flag, SHORE_THEME / BAT_THEME env, detection gating ---

/// Byte pins for the two built-in palettes and the intraline emphasis tints.
const DARK_KEYWORD: &str = "\x1b[38;2;179;136;255m";
const LIGHT_KEYWORD: &str = "\x1b[38;2;122;68;212m";
const DARK_ADD_TINT: &str = "\x1b[48;2;0;96;0m";
const LIGHT_ADD_TINT: &str = "\x1b[48;2;160;239;160m";

/// Run `shore diff --color always` against `path` with extra args and env.
/// Every caller pins `COLORTERM` explicitly (CI environments differ).
fn diff_env(path: &Path, extra_args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut args = vec![
        "diff",
        "--repo",
        path.to_str().unwrap(),
        "--color",
        "always",
    ];
    args.extend_from_slice(extra_args);
    shore_env(args, envs)
}

#[test]
fn shore_diff_truecolor_default_keeps_the_dark_palette() {
    let repo = modified_repo();
    capture(repo.path());
    let out = diff_env(repo.path(), &[], &[("COLORTERM", "truecolor")]);
    assert!(out.status.success(), "{}", err_text(&out));
    let text = out_text(&out);
    // Today's dark keyword bytes are the compatibility-frozen default.
    assert!(text.contains(DARK_KEYWORD));
    // Intraline emphasis is a background tint on the added row, not underline.
    assert!(text.contains(DARK_ADD_TINT));
    assert!(!text.contains("\x1b[4m"));
}

#[test]
fn shore_diff_theme_flag_selects_the_light_palette() {
    let repo = modified_repo();
    capture(repo.path());
    let out = diff_env(
        repo.path(),
        &["--theme", "light"],
        &[("COLORTERM", "truecolor")],
    );
    assert!(out.status.success(), "{}", err_text(&out));
    let text = out_text(&out);
    assert!(text.contains(LIGHT_KEYWORD));
    assert!(text.contains(LIGHT_ADD_TINT));
}

#[test]
fn shore_diff_shore_theme_env_selects_light() {
    let repo = modified_repo();
    capture(repo.path());
    let out = diff_env(
        repo.path(),
        &[],
        &[("COLORTERM", "truecolor"), ("SHORE_THEME", "light")],
    );
    assert!(out.status.success(), "{}", err_text(&out));
    assert!(out_text(&out).contains(LIGHT_KEYWORD));
}

#[test]
fn shore_diff_theme_flag_beats_shore_theme_env() {
    let repo = modified_repo();
    capture(repo.path());
    let out = diff_env(
        repo.path(),
        &["--theme", "dark"],
        &[("COLORTERM", "truecolor"), ("SHORE_THEME", "light")],
    );
    assert!(out.status.success(), "{}", err_text(&out));
    assert!(out_text(&out).contains(DARK_KEYWORD));
}

#[test]
fn shore_diff_shore_theme_beats_bat_theme() {
    let repo = modified_repo();
    capture(repo.path());
    let out = diff_env(
        repo.path(),
        &[],
        &[
            ("COLORTERM", "truecolor"),
            ("SHORE_THEME", "dark"),
            ("BAT_THEME", "light"),
        ],
    );
    assert!(out.status.success(), "{}", err_text(&out));
    assert!(out_text(&out).contains(DARK_KEYWORD));
}

#[test]
fn shore_diff_bat_theme_is_inherited_when_shore_theme_unset() {
    let repo = modified_repo();
    capture(repo.path());
    let out = diff_env(
        repo.path(),
        &[],
        &[("COLORTERM", "truecolor"), ("BAT_THEME", "light")],
    );
    assert!(out.status.success(), "{}", err_text(&out));
    assert!(out_text(&out).contains(LIGHT_KEYWORD));
}

#[test]
fn shore_diff_named_theme_recolors_tokens() {
    let repo = modified_repo();
    capture(repo.path());
    let out = diff_env(
        repo.path(),
        &["--theme", "Monokai Extended"],
        &[("COLORTERM", "truecolor")],
    );
    assert!(out.status.success(), "{}", err_text(&out));
    let text = out_text(&out);
    // Theme-derived truecolor foregrounds, not the built-in dark keyword hue.
    assert!(text.contains("\x1b[38;2;"));
    assert!(!text.contains(DARK_KEYWORD));
}

#[test]
fn shore_diff_theme_names_are_case_insensitive() {
    // bat/delta never fail a miscased name (they warn and fall back to the
    // default, masking the miss); shore resolves the requested theme.
    let repo = modified_repo();
    capture(repo.path());
    let canonical = diff_env(
        repo.path(),
        &["--theme", "Monokai Extended"],
        &[("COLORTERM", "truecolor")],
    );
    let lower = diff_env(
        repo.path(),
        &["--theme", "monokai extended"],
        &[("COLORTERM", "truecolor")],
    );
    assert!(canonical.status.success() && lower.status.success());
    assert_eq!(out_text(&lower), out_text(&canonical));
    // Env-carried names get the same treatment.
    let env = diff_env(
        repo.path(),
        &[],
        &[("COLORTERM", "truecolor"), ("SHORE_THEME", "nord")],
    );
    assert!(env.status.success(), "{}", err_text(&env));
}

#[test]
fn shore_diff_unknown_explicit_theme_fails_with_the_valid_list() {
    let repo = modified_repo();
    capture(repo.path());
    // Validation only happens when the truecolor lane is built, so the setup
    // pins COLORTERM and --color always (diff_env).
    let flag = diff_env(
        repo.path(),
        &["--theme", "no-such-theme"],
        &[("COLORTERM", "truecolor")],
    );
    assert!(!flag.status.success());
    let stderr = err_text(&flag);
    assert!(stderr.contains("no-such-theme"));
    assert!(stderr.contains("Monokai Extended"));

    // SHORE_THEME is the same explicit posture.
    let env = diff_env(
        repo.path(),
        &[],
        &[("COLORTERM", "truecolor"), ("SHORE_THEME", "no-such-theme")],
    );
    assert!(!env.status.success());
    assert!(err_text(&env).contains("no-such-theme"));
}

#[test]
fn shore_diff_unknown_bat_theme_warns_and_falls_back() {
    let repo = modified_repo();
    capture(repo.path());
    let out = diff_env(
        repo.path(),
        &[],
        &[("COLORTERM", "truecolor"), ("BAT_THEME", "no-such-theme")],
    );
    // Inherited source: warn and fall back, never fail.
    assert!(out.status.success(), "{}", err_text(&out));
    let stderr = err_text(&out);
    assert!(stderr.contains("BAT_THEME"));
    assert!(stderr.contains("no-such-theme"));
    // Piped stdout means no detection, so auto falls back to the dark built-in.
    assert!(out_text(&out).contains(DARK_KEYWORD));
}

#[test]
fn shore_diff_named16_lane_ignores_theme_selection() {
    let repo = modified_repo();
    capture(repo.path());
    // Empty COLORTERM is not truecolor/24bit, forcing the named lane even
    // when the CI ambience sets it.
    let with_theme = diff_env(repo.path(), &["--theme", "light"], &[("COLORTERM", "")]);
    let without = diff_env(repo.path(), &[], &[("COLORTERM", "")]);
    assert!(with_theme.status.success() && without.status.success());
    let text = out_text(&with_theme);
    assert_eq!(text, out_text(&without));
    assert!(text.contains("\x1b[35m")); // named keyword magenta, unchanged
    assert!(!text.contains("38;2")); // no truecolor on the named lane
    // No validation off the truecolor lane: a bogus name is inert here.
    let bogus = diff_env(
        repo.path(),
        &["--theme", "no-such-theme"],
        &[("COLORTERM", "")],
    );
    assert!(bogus.status.success());
}

#[test]
fn shore_diff_stat_never_resolves_themes() {
    let repo = modified_repo();
    capture(repo.path());
    // Exactly the explicit unknown-name failure setup plus --stat: lane
    // construction, and with it validation, is skipped under --stat.
    let out = diff_env(
        repo.path(),
        &["--stat"],
        &[("COLORTERM", "truecolor"), ("SHORE_THEME", "no-such-theme")],
    );
    assert!(out.status.success(), "{}", err_text(&out));
}

#[test]
fn shore_diff_light_and_named_colored_output_strips_to_plain() {
    let repo = modified_repo();
    capture(repo.path());
    let plain = shore([
        "diff",
        "--repo",
        repo.path().to_str().unwrap(),
        "--color",
        "never",
    ]);
    assert!(plain.status.success());
    for theme in ["light", "Monokai Extended"] {
        let colored = diff_env(
            repo.path(),
            &["--theme", theme],
            &[("COLORTERM", "truecolor")],
        );
        assert!(colored.status.success(), "{}", err_text(&colored));
        assert_eq!(
            strip_ansi(&out_text(&colored)),
            out_text(&plain),
            "presentation purity for theme {theme:?}"
        );
    }
}
