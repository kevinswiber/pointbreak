//! Real-socket coverage for inspect startup modes and machine authentication.

mod support;

use std::process::Command;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use support::git_repo::GitRepo;
use support::inspect::{InspectOutput, InspectSurface, Inspector, representative_store, urlencode};

fn inspect_output(repo: &std::path::Path, extra: &[&str]) -> std::process::Output {
    Command::new(support::pointbreak_bin())
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
        .expect("run pointbreak inspect")
}

#[test]
fn text_web_startup_carries_a_fragment_capability() {
    let repo = GitRepo::new();
    let inspector = Inspector::spawn_web_text(repo.path());
    let lines = inspector.startup_output().lines().collect::<Vec<_>>();

    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], "Pointbreak Review inspector");
    assert!(lines[1].starts_with("  store: "));
    let url = lines[2].strip_prefix("  url:   ").expect("web URL label");
    let capability = url
        .strip_prefix(&format!(
            "http://{}/#/timeline?token=",
            inspector.canonical_host()
        ))
        .expect("web capability URL");
    assert!(
        inspector.token().is_some_and(|token| token == capability),
        "startup URL and retained bearer differ"
    );
    assert_eq!(lines[3], "  stop:  Ctrl-C");
    assert!(
        !lines[2]
            .split('#')
            .next()
            .unwrap_or_default()
            .contains("token")
    );
    assert!(!inspector.get_text("/").is_empty());
}

#[test]
fn json_startup_is_one_compact_v1_line_with_fresh_entropy() {
    let repo = GitRepo::new();
    let first = Inspector::spawn_web_json(repo.path());
    let second = Inspector::spawn_api_json(repo.path());

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
fn all_surface_and_output_combinations_select_independently() {
    let repo = GitRepo::new();
    for (surface, output) in [
        (InspectSurface::Web, InspectOutput::Text),
        (InspectSurface::Web, InspectOutput::Json),
        (InspectSurface::ApiOnly, InspectOutput::Text),
        (InspectSurface::ApiOnly, InspectOutput::Json),
    ] {
        let inspector = match (surface, output) {
            (InspectSurface::Web, InspectOutput::Text) => Inspector::spawn_web_text(repo.path()),
            (InspectSurface::Web, InspectOutput::Json) => Inspector::spawn_web_json(repo.path()),
            (InspectSurface::ApiOnly, InspectOutput::Text) => {
                Inspector::spawn_api_text(repo.path())
            }
            (InspectSurface::ApiOnly, InspectOutput::Json) => {
                Inspector::spawn_api_json(repo.path())
            }
        };
        assert!(inspector.token().is_some(), "every process has a bearer");
        assert_eq!(
            inspector.startup_output().lines().count(),
            if output == InspectOutput::Json { 1 } else { 4 }
        );
        let (head, _) = inspector.raw_get("/");
        assert_eq!(
            head.starts_with("HTTP/1.1 200"),
            surface == InspectSurface::Web,
            "served surface must not depend on output encoding"
        );
    }
}

#[test]
fn inspect_rejects_non_loopback_before_bind_in_every_combination() {
    let repo = GitRepo::new();
    for extra in [
        &[][..],
        &["--format", "json"][..],
        &["--api-only"][..],
        &["--api-only", "--format", "json"][..],
    ] {
        let output = inspect_output(repo.path(), extra);
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("loopback"), "stderr: {stderr}");
        assert!(!stderr.contains("could not bind"), "stderr: {stderr}");
    }
}

#[test]
fn api_only_rejects_open_independently_of_output_format() {
    let repo = GitRepo::new();
    for format in [&[][..], &["--format", "json"][..]] {
        let output = Command::new(support::pointbreak_bin())
            .args([
                "inspect",
                "--repo",
                repo.path().to_str().unwrap(),
                "--port",
                "0",
                "--api-only",
                "--open",
            ])
            .args(format)
            .output()
            .expect("run pointbreak inspect");

        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("--open"), "stderr: {stderr}");
        assert!(stderr.contains("--api-only"), "stderr: {stderr}");
    }
}

fn assert_unauthorized(response: (String, String), token: &str) {
    let (head, body) = response;
    assert!(head.starts_with("HTTP/1.1 401"), "expected a 401 response");
    assert!(body.is_empty(), "401 body must be data-free");
    assert!(!head.contains(token));
    assert!(!body.contains(token));
}

#[test]
fn every_api_surface_requires_one_exact_host_and_bearer_before_routing() {
    let invalid_repo = tempfile::tempdir().unwrap();
    for inspector in [
        Inspector::spawn_web_text(invalid_repo.path()),
        Inspector::spawn_api_json(invalid_repo.path()),
    ] {
        let host = inspector.canonical_host().to_owned();
        let token = inspector.token().unwrap().to_owned();
        let authorization = format!("Bearer {token}");

        for path in [
            "/api/history",
            "/api/history/new-count",
            "/api/revisions",
            "/api/revisions/example",
            "/api/snapshots/example",
            "/api/threads",
            "/api/attention",
            "/api/freshness",
            "/api/version",
            "/api/identity",
            "/api/nope",
        ] {
            assert_unauthorized(
                inspector.raw_request("GET", path, &[("Host", &host)]),
                &token,
            );
        }
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
}

#[test]
fn exact_host_is_global_while_web_assets_are_bearer_free_and_api_only_has_none() {
    let invalid_repo = tempfile::tempdir().unwrap();
    for web in [
        Inspector::spawn_web_text(invalid_repo.path()),
        Inspector::spawn_web_json(invalid_repo.path()),
    ] {
        let token = web.token().unwrap();
        let host = web.canonical_host();

        assert_unauthorized(web.raw_request("GET", "/", &[]), token);
        assert_unauthorized(
            web.raw_request("GET", "/app.js", &[("Host", "127.0.0.1:1")]),
            token,
        );
        for path in ["/", "/index.html", "/tokens.css", "/app.css", "/app.js"] {
            let (head, body) = web.raw_request("GET", path, &[("Host", host)]);
            assert!(head.starts_with("HTTP/1.1 200"), "{path}: {head}");
            assert!(!body.is_empty(), "{path} is a fixed asset");
        }
        let (index_head, _) = web.raw_request("GET", "/", &[("Host", host)]);
        assert!(index_head.contains("Content-Security-Policy:"));
    }

    for api in [
        Inspector::spawn_api_text(invalid_repo.path()),
        Inspector::spawn_api_json(invalid_repo.path()),
    ] {
        for path in ["/", "/index.html", "/tokens.css", "/app.css", "/app.js"] {
            let (head, _) = api.raw_request("GET", path, &[("Host", api.canonical_host())]);
            assert!(head.starts_with("HTTP/1.1 404"), "{path}: {head}");
        }
    }
}

#[test]
fn authenticated_routes_include_the_shared_version_without_secret_disclosure() {
    let store = representative_store();
    let inspector = Inspector::spawn_web_text(store.repo.path());
    let token = inspector.token().unwrap().to_owned();

    let version_text = inspector.get_text("/api/version");
    let version: Value = serde_json::from_str(&version_text).unwrap();
    let cli_output = support::pointbreak(["version"]);
    assert!(cli_output.status.success());
    let cli_version: Value = serde_json::from_slice(&cli_output.stdout).unwrap();
    assert_eq!(version, cli_version);
    assert_eq!(format!("{version_text}\n").as_bytes(), cli_output.stdout);
    assert_eq!(version["build"]["source"], env!("POINTBREAK_BUILD_SOURCE"));
    match env!("POINTBREAK_BUILD_SOURCE") {
        "git" => assert_eq!(version["build"]["commit"].as_str().unwrap().len(), 40),
        "package" => assert!(version["build"]["commit"].is_null()),
        source => panic!("unexpected build source {source:?}"),
    }
    assert!(version["build"]["describe"].is_string());
    assert!(version["build"]["dirty"].is_boolean());

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
