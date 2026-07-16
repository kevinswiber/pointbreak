use std::path::Path;

#[test]
fn readme_teaches_the_pointbreak_package_and_command() {
    let readme = std::fs::read_to_string("README.md").expect("read README");

    assert!(readme.contains("cargo install pointbreak"));
    assert!(readme.contains("provides the `pointbreak` command"));
    assert!(readme.contains("0.7.0"));
    assert!(!readme.contains("cargo install shore\n"));
    assert!(!readme.contains("cargo install shore "));
}

#[test]
fn installation_documents_the_one_release_hard_cutover() {
    let installation = std::fs::read_to_string("docs/installation.md").expect("read installation");

    for required in [
        "Release `0.7.0` is a one-release hard cutover",
        "Stop every process that can write Review state",
        "Move owner-controlled state and config offline",
        "POINTBREAK_HOME",
        "<repo>/.pointbreak/",
        "<git-common-dir>/pointbreak/",
        "<git-common-dir>/pointbreak.link.json",
        "pointbreak store paths --repo <path> --format json",
        "verify readback",
        "Rollback is the inverse filesystem move",
        "no runtime fallback, compatibility alias, automatic migration, migration CLI",
    ] {
        assert!(
            installation.contains(required),
            "installation guide is missing hard-cutover guidance: {required:?}"
        );
    }

    assert!(!installation.contains("pointbreak store migrate-paths"));
    assert!(!installation.contains("pointbreak review"));
}

#[test]
fn retired_documentation_host_is_never_presented_as_live() {
    for path in LIVING_OPERATIONAL_SOURCES {
        let contents =
            std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
        if contents.contains("docs.withpointbreak.com") {
            assert!(
                contents.contains("archived") || contents.contains("retired"),
                "{path} presents docs.withpointbreak.com without an archived/retired label"
            );
        }
    }
}

#[test]
fn living_sources_teach_only_the_pointbreak_operational_contract() {
    for path in LIVING_OPERATIONAL_SOURCES {
        let contents =
            std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));

        for (index, line) in contents.lines().enumerate() {
            let line_number = index + 1;
            for (needle, purpose) in FORBIDDEN_LIVING_PATTERNS {
                if line.contains(needle) && classify_retained_reference(path, line).is_none() {
                    panic!("{path}:{line_number} presents {purpose}: {line:?}");
                }
            }
        }
    }
}

#[test]
fn generic_store_guidance_does_not_present_a_literal_path_as_universal() {
    for (path, forbidden) in [
        ("CONTRIBUTING.md", "raw `.shore/data/` files"),
        ("docs/agent-authoring.md", "same `.pointbreak/data/` store"),
        (
            "docs/assessment-model.md",
            "`show` replays `.pointbreak/data/events/`",
        ),
        (
            "docs/input-request-model.md",
            "`list` and `fetch` replay `.pointbreak/data/events/`",
        ),
        (
            "docs/input-request-model.md",
            "authoritative store is the `.pointbreak/data/events/`",
        ),
    ] {
        let contents =
            std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
        assert!(
            !contents.contains(forbidden),
            "{path} should describe the resolved store instead of {forbidden:?}"
        );
    }
}

#[test]
fn legacy_product_word_detection_is_case_insensitive_and_word_bounded() {
    assert!(contains_legacy_reference("matched by Shore at scan time"));
    assert!(contains_legacy_reference("run SHORE_CONFIG=/tmp/config"));
    assert!(!contains_legacy_reference("shoreline fixture"));
}

#[test]
fn every_retained_public_legacy_reference_has_a_narrow_classification() {
    let mut paths = vec![
        Path::new("CONTRIBUTING.md").to_path_buf(),
        Path::new("README.md").to_path_buf(),
        Path::new("Justfile").to_path_buf(),
    ];
    if Path::new("CHANGELOG.md").exists() {
        paths.push(Path::new("CHANGELOG.md").to_path_buf());
    }
    for root in ["docs", "skills", "scripts", "benches"] {
        collect_files(Path::new(root), &mut paths);
    }
    for root in [
        "tests/fixtures/event_signatures",
        "tests/fixtures/legacy_stores",
        "tests/fixtures/naming-cutover",
        "tests/fixtures/packages",
        "tests/fixtures/review_documents",
    ] {
        collect_files(Path::new(root), &mut paths);
    }
    paths.extend(
        [
            "tests/agent_skill_validation_evidence.rs",
            "tests/docs_open_source_readiness.rs",
            "tests/docs_package_identity.rs",
        ]
        .into_iter()
        .map(Path::new)
        .map(Path::to_path_buf),
    );
    paths.sort();

    for path in paths {
        let audit_path = public_audit_path(&path);
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {audit_path}: {error}"));
        for (index, line) in contents.lines().enumerate() {
            if contains_legacy_reference(line)
                && classify_retained_reference(&audit_path, line).is_none()
            {
                panic!(
                    "{}:{} has an unclassified legacy reference: {:?}",
                    audit_path,
                    index + 1,
                    line
                );
            }
        }
    }
}

const LIVING_OPERATIONAL_SOURCES: &[&str] = &[
    "CONTRIBUTING.md",
    "README.md",
    "docs/agent-authoring.md",
    "docs/assessment-model.md",
    "docs/benchmarking.md",
    "docs/cli-reference.md",
    "docs/getting-started.md",
    "docs/id-prefixes.md",
    "docs/input-request-model.md",
    "docs/installation.md",
    "docs/library-api.md",
    "docs/manual-testing.md",
    "docs/releasing.md",
    "docs/review-workflow.md",
    "docs/signing-ux.md",
    "docs/storage-model.md",
    "docs/substrate-language.md",
    "docs/substrate-thesis-summary.md",
    "Justfile",
    "benches/store_backend.rs",
    "scripts/capture-inspector-screenshots.sh",
    "scripts/worktree-to-fixture.sh",
    "skills/pointbreak-author/SKILL.md",
    "skills/pointbreak-author-response/SKILL.md",
    "skills/pointbreak-reviewer/SKILL.md",
];

const FORBIDDEN_LIVING_PATTERNS: &[(&str, &str)] = &[
    ("shore ", "a legacy executable command"),
    (
        "SHORE_",
        "a legacy environment variable as current guidance",
    ),
    (".shore", "a legacy path as current placement"),
    ("pointbreak review", "the rejected review command prefix"),
    ("cargo install shore", "the retired package/install name"),
    ("cargo binstall shore", "the retired package/install name"),
    ("store migrate-paths", "a migration CLI that does not exist"),
    ("automatically migrates", "automatic migration behavior"),
    ("automatically migrate", "automatic migration behavior"),
];

fn collect_files(root: &Path, paths: &mut Vec<std::path::PathBuf>) {
    for entry in
        std::fs::read_dir(root).unwrap_or_else(|error| panic!("read {}: {error}", root.display()))
    {
        let entry = entry.unwrap_or_else(|error| panic!("read directory entry: {error}"));
        let file_type = entry
            .file_type()
            .unwrap_or_else(|error| panic!("read {} file type: {error}", entry.path().display()));
        if file_type.is_dir() {
            collect_files(&entry.path(), paths);
        } else if file_type.is_file() {
            paths.push(entry.path());
        }
    }
}

fn public_audit_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn contains_legacy_reference(line: &str) -> bool {
    line.contains(".shore")
        || line.contains("SHORE_")
        || contains_ascii_word(&line.to_ascii_lowercase(), "shore")
}

#[test]
fn public_audit_paths_are_platform_independent() {
    let path = public_audit_path(Path::new(r"docs\adr\adr-0001-example.md"));

    assert_eq!(path, "docs/adr/adr-0001-example.md");
    assert_eq!(
        classify_retained_reference(&path, "frozen shore.note-body identifier"),
        Some("accepted ADR history")
    );
}

fn contains_ascii_word(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(start, _)| {
        let end = start + needle.len();
        let before = haystack[..start].chars().next_back();
        let after = haystack[end..].chars().next();
        !before.is_some_and(is_word_character) && !after.is_some_and(is_word_character)
    })
}

fn is_word_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

fn classify_retained_reference(path: &str, line: &str) -> Option<&'static str> {
    if path.starts_with("docs/adr/") {
        return Some("accepted ADR history");
    }
    if path == "CHANGELOG.md" {
        return Some("published changelog history");
    }
    if [
        "tests/fixtures/event_signatures/",
        "tests/fixtures/legacy_stores/",
        "tests/fixtures/naming-cutover/",
        "tests/fixtures/packages/",
        "tests/fixtures/review_documents/",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix))
    {
        return Some("frozen fixture or captured machine document");
    }
    if [
        "tests/agent_skill_validation_evidence.rs",
        "tests/docs_open_source_readiness.rs",
        "tests/docs_package_identity.rs",
    ]
    .contains(&path)
    {
        return Some("test intentionally quoting rejected or historical strings");
    }

    match path {
        "README.md" if line.contains("assets/shore-inspector-") => {
            Some("checked-in screenshot basename")
        }
        "docs/installation.md"
            if line.starts_with("   |")
                && [
                    "<repo>/.shore/",
                    "<git-common-dir>/shore/",
                    "<git-common-dir>/shore.link.json",
                    "$XDG_DATA_HOME/shore",
                    "$HOME/.shore",
                    "%APPDATA%\\shore",
                ]
                .iter()
                .any(|old_path| line.contains(old_path)) =>
        {
            Some("explicit pre-0.7.0 location in the cutover table")
        }
        "docs/assessment-model.md"
        | "docs/cli-reference.md"
        | "docs/input-request-model.md"
        | "docs/library-api.md"
            if line.contains("shore.") =>
        {
            Some("frozen persisted protocol identifier")
        }
        "docs/storage-model.md" if line.contains("shore.") => {
            Some("frozen persisted protocol identifier")
        }
        "docs/storage-model.md" if line.contains(".shore-write") => {
            Some("frozen atomic-write temporary filename")
        }
        "Justfile" if line.contains("shore(\\.exe)?|--bin shore") => {
            Some("negative release-surface assertion")
        }
        "scripts/capture-inspector-screenshots.sh"
            if line.contains("shore-inspector-") || line.contains("shore-inspect-") =>
        {
            Some("checked-in screenshot basename or inspector preference key")
        }
        "scripts/install-selftest.sh"
            if line.contains("neighbor=\"${install_dir}/shore\"")
                || line.contains("grep -i 'shore'") =>
        {
            Some("negative installer assertion and untouched-neighbor sentinel")
        }
        "scripts/install-selftest.ps1"
            if line.contains("$neighbor = Join-Path $installDir \"shore.exe\"")
                || line.contains("-match \"(?i)shore\"") =>
        {
            Some("negative installer assertion and untouched-neighbor sentinel")
        }
        "scripts/package-release-selftest.sh"
            if line.contains("payload_dir/shore")
                || line.contains("-C \"$payload_dir\" shore")
                || line.contains("shore.exe")
                || line.contains("ln -s shore") =>
        {
            Some("intentionally invalid archive or alias fixture")
        }
        _ => None,
    }
}

#[test]
fn readme_has_release_badges_for_pointbreak() {
    let readme = std::fs::read_to_string("README.md").expect("read README");

    assert!(readme.contains("https://crates.io/crates/pointbreak"));
    assert!(readme.contains("https://img.shields.io/crates/v/pointbreak"));
    assert!(readme.contains("https://docs.rs/pointbreak"));
    assert!(readme.contains("https://docs.rs/pointbreak/badge.svg"));
    assert!(
        readme.contains("https://github.com/withpointbreak/pointbreak/actions/workflows/ci.yml")
    );
    assert!(readme.contains("actions/workflows/ci.yml/badge.svg"));
}

#[test]
fn cargo_metadata_points_to_pointbreak_repository() {
    let manifest = std::fs::read_to_string("Cargo.toml").expect("read Cargo manifest");

    assert!(manifest.contains(r#"homepage = "https://github.com/withpointbreak/pointbreak""#));
    assert!(manifest.contains(r#"repository = "https://github.com/withpointbreak/pointbreak""#));
}

#[test]
fn living_metadata_uses_the_canonical_organization_repository() {
    let stale_repository = ["kevinswiber", "pointbreak"].join("/");
    let canonical_repository = "withpointbreak/pointbreak";
    let paths = [
        ".github/ISSUE_TEMPLATE/config.yml",
        "CONTRIBUTING.md",
        "Cargo.toml",
        "README.md",
        "docs/adr/adr-0014-reviewunit-commit-range-lifecycle.md",
        "docs/id-prefixes.md",
        "docs/installation.md",
        "docs/storage-model.md",
        "extensions/vscode/package.json",
        "scripts/install-selftest.ps1",
        "scripts/install-selftest.sh",
        "scripts/install.ps1",
        "scripts/install.sh",
        "skills/README.md",
        "src/cli/inspect/web/test/css-coverage.test.ts",
    ];

    for path in paths {
        let contents = std::fs::read_to_string(path).unwrap_or_else(|error| {
            panic!("read {path}: {error}");
        });
        assert!(
            !contents.contains(&stale_repository),
            "{path} still uses the personal repository owner"
        );
        assert!(
            contents.contains(canonical_repository),
            "{path} does not name the canonical organization repository"
        );
    }
}

#[test]
fn vscode_metadata_keeps_its_identity_and_uses_canonical_support_urls() {
    let package = std::fs::read_to_string("extensions/vscode/package.json")
        .expect("read VS Code package manifest");

    assert!(package.contains(r#""publisher": "pointbreak""#));
    assert!(package.contains(r#""name": "pointbreak""#));
    assert!(package.contains("https://github.com/withpointbreak/pointbreak.git"));
    assert!(package.contains("https://github.com/withpointbreak/pointbreak/issues"));
    assert!(package.contains("https://github.com/withpointbreak/pointbreak#readme"));
}

#[test]
fn readme_drops_branded_hunk_origin_references() {
    let readme = std::fs::read_to_string("README.md").expect("read README");

    for stale in [
        "modem-dev/hunk",
        "kevinswiber/hunk",
        "docs/hunk-feedback.md",
        "Hunk is the practical inspiration",
        "real Hunk review session",
        "hunk fork",
    ] {
        assert!(
            !readme.contains(stale),
            "README still contains stale Hunk reference: {stale}"
        );
    }
    assert!(!Path::new("docs/hunk-feedback.md").exists());
}

#[test]
fn just_run_targets_the_pointbreak_binary() {
    let justfile = std::fs::read_to_string("Justfile").expect("read Justfile");

    assert!(justfile.contains("cargo +stable run --bin pointbreak --"));
}
