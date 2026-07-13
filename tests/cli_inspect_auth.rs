//! Real-socket coverage for inspect startup modes and machine authentication.

mod support;

use std::process::Command;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use support::git_repo::GitRepo;
use support::inspect::{Inspector, representative_store, urlencode};

fn inspect_output(repo: &std::path::Path, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_shore"))
        .args([
            "inspect",
            "--repo",
            repo.to_str().unwrap(),
            "--host",
            "192.0.2.1",
            "--port",
            "0",
        ])
        .args(extra)
        .output()
        .expect("run shore inspect")
}

#[test]
fn human_startup_keeps_the_browser_banner() {
    let repo = GitRepo::new();
    let inspector = Inspector::spawn_human(repo.path());
    let lines = inspector.startup_output().lines().collect::<Vec<_>>();

    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], "Pointbreak Review inspector");
    assert!(lines[1].starts_with("  store: "));
    assert_eq!(
        lines[2],
        format!("  url:   http://{}/", inspector.canonical_host())
    );
    assert_eq!(lines[3], "  stop:  Ctrl-C");
    assert!(!inspector.get_text("/").is_empty());
}

#[test]
fn authenticated_startup_is_one_compact_v1_line_with_fresh_entropy() {
    let repo = GitRepo::new();
    let first = Inspector::spawn_authenticated(repo.path());
    let second = Inspector::spawn_authenticated(repo.path());

    for inspector in [&first, &second] {
        let output = inspector.startup_output();
        assert!(output.ends_with('\n'));
        assert_eq!(output.lines().count(), 1);
        let startup: Value = serde_json::from_str(output.trim()).expect("startup JSON");
        assert_eq!(startup["schema"], "pointbreak.inspect-startup");
        assert_eq!(startup["version"], 1);
        assert_eq!(startup["host"], "127.0.0.1");
        assert!(startup["port"].as_u64().is_some_and(|port| port > 0));
        let token = startup["token"].as_str().expect("startup token");
        assert!(
            inspector.token().is_some_and(|actual| actual == token),
            "harness did not retain the startup bearer"
        );
        assert!(
            token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        );
        assert!(URL_SAFE_NO_PAD.decode(token).unwrap().len() >= 32);
    }

    assert!(
        first.token() != second.token(),
        "two starts reused one bearer"
    );
}

#[test]
fn inspect_rejects_non_loopback_before_bind_in_both_modes() {
    let repo = GitRepo::new();
    for extra in [&[][..], &["--startup-format", "json"][..]] {
        let output = inspect_output(repo.path(), extra);
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("loopback"), "stderr: {stderr}");
        assert!(!stderr.contains("could not bind"), "stderr: {stderr}");
    }
}

#[test]
fn authenticated_mode_rejects_open() {
    let repo = GitRepo::new();
    let output = Command::new(env!("CARGO_BIN_EXE_shore"))
        .args([
            "inspect",
            "--repo",
            repo.path().to_str().unwrap(),
            "--port",
            "0",
            "--startup-format",
            "json",
            "--open",
        ])
        .output()
        .expect("run shore inspect");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--open"), "stderr: {stderr}");
    assert!(stderr.contains("json"), "stderr: {stderr}");
}

fn assert_unauthorized(response: (String, String), token: &str) {
    let (head, body) = response;
    assert!(head.starts_with("HTTP/1.1 401"), "expected a 401 response");
    assert!(body.is_empty(), "401 body must be data-free");
    assert!(!head.contains(token));
    assert!(!body.contains(token));
}

#[test]
fn machine_requests_require_one_exact_host_and_bearer_before_routing() {
    let invalid_repo = tempfile::tempdir().unwrap();
    let inspector = Inspector::spawn_authenticated(invalid_repo.path());
    let host = inspector.canonical_host().to_owned();
    let token = inspector.token().unwrap().to_owned();
    let authorization = format!("Bearer {token}");

    assert_unauthorized(
        inspector.raw_request(
            "GET",
            "/api/freshness",
            &[("Authorization", &authorization)],
        ),
        &token,
    );
    assert_unauthorized(
        inspector.raw_request(
            "GET",
            "/api/freshness",
            &[("Host", "127.0.0.1:1"), ("Authorization", &authorization)],
        ),
        &token,
    );
    assert_unauthorized(
        inspector.raw_request("GET", "/api/freshness", &[("Host", &host)]),
        &token,
    );
    assert_unauthorized(
        inspector.raw_request(
            "GET",
            "/api/freshness",
            &[("Host", &host), ("Authorization", "Basic nope")],
        ),
        &token,
    );
    assert_unauthorized(
        inspector.raw_request(
            "GET",
            "/api/freshness",
            &[("Host", &host), ("Authorization", "Bearer wrong")],
        ),
        &token,
    );
    assert_unauthorized(
        inspector.raw_request(
            "GET",
            "/api/nope",
            &[
                ("Host", &host),
                ("Authorization", &authorization),
                ("Authorization", &authorization),
            ],
        ),
        &token,
    );
    assert_unauthorized(
        inspector.raw_request("POST", "/api/history", &[("Host", &host)]),
        &token,
    );
}

#[test]
fn authenticated_routes_include_the_shared_version_without_secret_disclosure() {
    let store = representative_store();
    let inspector = Inspector::spawn_authenticated(store.repo.path());
    let token = inspector.token().unwrap().to_owned();

    let version_text = inspector.get_text("/api/version");
    let version: Value = serde_json::from_str(&version_text).unwrap();
    let cli_output = support::shore(["version"]);
    assert!(cli_output.status.success());
    let cli_version: Value = serde_json::from_slice(&cli_output.stdout).unwrap();
    assert_eq!(version, cli_version);
    assert_eq!(format!("{version_text}\n").as_bytes(), cli_output.stdout);

    let snapshot_text =
        inspector.get_text(&format!("/api/snapshots/{}", urlencode(&store.snapshot_id)));
    let snapshot: Value = serde_json::from_str(&snapshot_text).unwrap();
    assert_eq!(snapshot["schema"], "pointbreak.review-snapshot");
    let freshness_text = inspector.get_text("/api/freshness");
    let freshness: Value = serde_json::from_str(&freshness_text).unwrap();
    assert_eq!(freshness["schema"], "pointbreak.inspect-freshness");

    let (error_head, error_body) = inspector.raw_get("/api/nope");
    assert!(error_head.starts_with("HTTP/1.1 404"));
    assert!(error_body.contains("no such route"));
    assert!(inspector.request("POST", "/api/history").contains("405"));
    assert!(error_head.contains("X-Content-Type-Options: nosniff"));
    assert!(error_head.contains("Referrer-Policy: no-referrer"));

    for capture in [
        version_text,
        snapshot_text,
        freshness_text,
        error_head,
        error_body,
        inspector.stderr_text(),
    ] {
        assert!(
            !capture.contains(&token),
            "secret escaped its startup field"
        );
    }
}
