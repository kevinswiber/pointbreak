use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};
#[cfg(unix)]
use tempfile::tempdir;

fn repo_root() -> PathBuf {
    env::manifest_dir()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
    assert!(
        bytes.len() >= 24,
        "asset is too short to contain a PNG header"
    );
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "asset is not a PNG");
    (
        u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
        u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
    )
}

#[test]
fn marketing_review_capture_is_neutral_and_integrity_checked() {
    let manifest_path = repo_root().join("assets/marketing/review-interface-capture.json");
    let manifest: Value = serde_json::from_slice(
        &fs::read(&manifest_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", manifest_path.display())),
    )
    .expect("capture manifest is valid JSON");

    assert_eq!(
        manifest["schema"],
        "com.withpointbreak.review-interface-capture/v2"
    );

    let pack_manifest_path = repo_root().join(
        manifest["example"]["manifestPath"]
            .as_str()
            .expect("example manifestPath is a string"),
    );
    let pack_bytes = fs::read(&pack_manifest_path).expect("read example manifest");
    let pack: Value = serde_json::from_slice(&pack_bytes).expect("example manifest is valid JSON");
    assert_eq!(manifest["example"]["name"], pack["name"]);
    assert_eq!(
        manifest["example"]["classification"],
        pack["classification"]
    );
    assert_eq!(manifest["example"]["manifestSha256"], sha256(&pack_bytes));

    let producer_commit = manifest["producer"]["commit"]
        .as_str()
        .expect("producer commit is a string");
    assert_eq!(producer_commit.len(), 40, "producer commit is complete");
    assert!(
        producer_commit.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "producer commit is hexadecimal"
    );
    assert_eq!(manifest["producer"]["name"], "shore");
    let producer_version = manifest["producer"]["version"]
        .as_str()
        .expect("producer version is a string");
    assert!(
        producer_version.split('.').count() == 3,
        "producer version is complete: {producer_version}"
    );

    let revision = manifest["record"]["revision"]
        .as_str()
        .expect("source revision is a string");
    let digest = revision
        .strip_prefix("rev:sha256:")
        .expect("source revision uses the full rev:sha256 form");
    assert_eq!(digest.len(), 64, "source revision digest is complete");
    assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));

    assert_eq!(manifest["record"]["track"], pack["record"]["track"]);
    assert_eq!(manifest["record"]["revision"], pack["record"]["revision"]);
    assert_eq!(
        manifest["record"]["selectedAssessment"],
        pack["record"]["selectedAssessment"]
    );
    assert_eq!(
        manifest["record"]["eventSetHash"],
        pack["record"]["eventSetHash"]
    );
    assert_eq!(
        manifest["record"]["verificationStatus"],
        pack["record"]["verificationStatus"]
    );
    assert_eq!(manifest["record"]["reproducibleFromPublicPack"], true);
    assert_eq!(manifest["record"]["publiclyInspectable"], false);
    assert_eq!(
        manifest["record"]["redactions"].as_array().map(Vec::len),
        Some(0)
    );

    let writers = manifest["record"]["writerActors"]
        .as_array()
        .expect("writer_actors is an array");
    assert_eq!(
        manifest["record"]["writerActors"],
        pack["record"]["writerActors"]
    );
    assert!(!writers.is_empty(), "at least one writer actor is recorded");
    for writer in writers {
        let writer = writer.as_str().expect("writer actor is a string");
        assert!(writer.starts_with("actor:"), "invalid actor id: {writer}");
        let normalized = writer.to_ascii_lowercase();
        assert!(
            !normalized.contains('@'),
            "writer exposes an email: {writer}"
        );
        assert!(
            !normalized.contains("kswiber") && !normalized.contains("kevin"),
            "writer exposes a personal principal: {writer}"
        );
    }

    assert_eq!(manifest["capture"]["viewport"]["width"], 900);
    assert_eq!(manifest["capture"]["viewport"]["height"], 506);
    assert_eq!(manifest["capture"]["deviceScaleFactor"], 2);

    for theme in ["dark", "light"] {
        let asset = &manifest["assets"][theme];
        let relative_path = asset["path"].as_str().expect("asset path is a string");
        let bytes = fs::read(repo_root().join(relative_path)).expect("read capture asset");
        assert_eq!(png_dimensions(&bytes), (1800, 1012));
        assert_eq!(asset["width"], 1800);
        assert_eq!(asset["height"], 1012);
        assert_eq!(
            asset["sha256"].as_str().expect("asset digest is a string"),
            sha256(&bytes)
        );
    }
}

#[test]
fn capture_script_supports_pack_linkage_without_changing_readme_defaults() {
    let script = fs::read_to_string(repo_root().join("scripts/capture-inspector-screenshots.sh"))
        .expect("read capture script");

    assert!(script.contains("--example-manifest <path>"));
    assert!(script.contains("--manifest <path>"));
    assert!(script.contains("--example-manifest)"));
    assert!(script.contains("--manifest)"));
    assert!(script.contains("REVISION=\"93326e73\""));
    assert!(script.contains("TRACK=\"agent:codex-450\""));
    assert!(script.contains("OUT_DIR=\"$REPO_ROOT/assets\""));
}

#[test]
#[cfg(unix)]
fn pack_aware_capture_preserves_accepted_outputs_on_command_failure() {
    let temp = tempdir().expect("temporary directory");
    let output = temp.path().join("assets");
    fs::create_dir(&output).unwrap();
    let dark = output.join("shore-inspector-dark.png");
    let light = output.join("shore-inspector-light.png");
    let manifest = output.join("review-interface-capture.json");
    fs::write(&dark, b"accepted dark").unwrap();
    fs::write(&light, b"accepted light").unwrap();
    fs::write(&manifest, b"accepted manifest").unwrap();

    let fake_curl = temp.path().join("curl");
    fs::write(&fake_curl, "#!/usr/bin/env bash\nexit 0\n").unwrap();
    make_executable(&fake_curl);
    let fake_playwright = temp.path().join("playwright-cli");
    fs::write(
        &fake_playwright,
        "#!/usr/bin/env bash\n[[ \" $* \" != *\" run-code \"* ]]\n",
    )
    .unwrap();
    make_executable(&fake_playwright);

    let path = format!(
        "{}:{}",
        temp.path().display(),
        std::env::var("PATH").unwrap()
    );
    let status = Command::new("bash")
        .arg(repo_root().join("scripts/capture-inspector-screenshots.sh"))
        .args([
            "--example-manifest",
            "examples/review/checkout-refactor/manifest.json",
            "--manifest",
        ])
        .arg(&manifest)
        .arg("--out-dir")
        .arg(&output)
        .env("PATH", path)
        .env("PLAYWRIGHT_CLI", &fake_playwright)
        .current_dir(repo_root())
        .status()
        .expect("run capture script with a failing browser command");
    assert!(!status.success());
    assert_eq!(fs::read(dark).unwrap(), b"accepted dark");
    assert_eq!(fs::read(light).unwrap(), b"accepted light");
    assert_eq!(fs::read(manifest).unwrap(), b"accepted manifest");
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

// Runtime-resolved binary/manifest paths for cross-machine (e.g. Windows) archive runs.
#[path = "support/env.rs"]
#[allow(dead_code)]
mod env;
