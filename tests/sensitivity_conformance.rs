//! Conformance vectors for the sensitivity-scan vocabulary (#150): the seeded
//! strings both boundaries classify — shoreline's clone-local scanner here, and
//! downstream gates (e.g. shoreline-relay's egress gate) from the same fixture.

mod support;

use std::fs;

use serde::Deserialize;
use shoreline::session::{
    SensitivityKind, SensitivityPolicyOutcome, StoreStatusOptions, StoreStatusSensitivity,
    store_status,
};
use support::git_repo::GitRepo;

const VECTORS_PATH: &str = "tests/fixtures/sensitivity/conformance-vectors.json";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConformanceVectors {
    note: String,
    known_token_divergence: String,
    repos: Repos,
}

#[derive(Deserialize)]
struct Repos {
    positive: RepoVectors,
    negative: RepoVectors,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoVectors {
    expected_policy_outcome: String,
    files: Vec<VectorFile>,
    expected_findings: Vec<ExpectedFinding>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VectorFile {
    path: String,
    #[serde(default)]
    contents: Option<String>,
    #[serde(default)]
    fill: Option<Fill>,
    expected_kinds: Vec<String>,
    #[serde(default)]
    relay_expected_kinds: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fill {
    byte: String,
    size_bytes: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExpectedFinding {
    kind: String,
    severity: String,
    policy_outcome: String,
}

fn load_vectors() -> ConformanceVectors {
    let raw = fs::read_to_string(VECTORS_PATH).expect("read sensitivity conformance vectors");
    serde_json::from_str(&raw).expect("parse sensitivity conformance vectors")
}

fn scan_repo(vectors: &RepoVectors) -> StoreStatusSensitivity {
    let repo = GitRepo::new();
    for file in &vectors.files {
        match (&file.contents, &file.fill) {
            (Some(contents), None) => repo.write(&file.path, contents),
            (None, Some(fill)) => repo.write(&file.path, fill.byte.repeat(fill.size_bytes)),
            _ => panic!(
                "vector file {} must set exactly one of contents/fill",
                file.path
            ),
        }
    }
    store_status(StoreStatusOptions::new(repo.path()))
        .expect("store_status on a conformance repo")
        .sensitivity
}

/// Every vector string round-trips through the typed vocabulary, the real scanner
/// reproduces the fixture's expectations, and the scanner's severity/outcome
/// assignments equal the enum metadata (two-way drift guard, #150).
#[test]
fn conformance_vectors_pin_scanner_and_vocabulary_agreement() {
    let vectors = load_vectors();
    assert!(!vectors.note.is_empty());

    for (repo_vectors, label) in [
        (&vectors.repos.positive, "positive"),
        (&vectors.repos.negative, "negative"),
    ] {
        let scan = scan_repo(repo_vectors);

        // Combined outcome: fixture == scanner == lattice-combine of the rows.
        assert_eq!(
            scan.policy_outcome, repo_vectors.expected_policy_outcome,
            "{label}: combined policy outcome"
        );
        let combined = SensitivityPolicyOutcome::combine(scan.findings.iter().map(|finding| {
            SensitivityPolicyOutcome::parse(&finding.policy_outcome)
                .unwrap_or_else(|| panic!("unparseable outcome {}", finding.policy_outcome))
        }));
        assert_eq!(combined.as_str(), scan.policy_outcome, "{label}: combine");

        // Emitted kinds == the union of the per-file expectations == expectedFindings.
        let mut emitted: Vec<&str> = scan
            .findings
            .iter()
            .map(|finding| finding.kind.as_str())
            .collect();
        emitted.sort_unstable();
        let mut expected_union: Vec<&str> = repo_vectors
            .files
            .iter()
            .flat_map(|file| file.expected_kinds.iter().map(String::as_str))
            .collect();
        expected_union.sort_unstable();
        expected_union.dedup();
        assert_eq!(emitted, expected_union, "{label}: emitted finding kinds");
        let mut from_findings: Vec<&str> = repo_vectors
            .expected_findings
            .iter()
            .map(|expected| expected.kind.as_str())
            .collect();
        from_findings.sort_unstable();
        assert_eq!(emitted, from_findings, "{label}: expectedFindings kinds");

        // Each row: fixture severity/outcome, enum parseability, and enum metadata agree.
        for expected in &repo_vectors.expected_findings {
            let finding = scan
                .findings
                .iter()
                .find(|finding| finding.kind == expected.kind)
                .unwrap_or_else(|| panic!("{label}: missing finding {}", expected.kind));
            assert_eq!(finding.severity, expected.severity, "{}", expected.kind);
            assert_eq!(
                finding.policy_outcome, expected.policy_outcome,
                "{}",
                expected.kind
            );
            let kind = SensitivityKind::parse(&finding.kind)
                .unwrap_or_else(|| panic!("unparseable kind {}", finding.kind));
            assert_eq!(
                kind.severity().as_str(),
                finding.severity,
                "{}",
                finding.kind
            );
            assert_eq!(
                kind.policy_outcome().as_str(),
                finding.policy_outcome,
                "{}",
                finding.kind
            );
        }
    }
}

/// The relay's known_token detector deliberately matches the prefix anywhere in a
/// token; shoreline matches only at token start. The divergence is specified in the
/// shared vocabulary and pinned from shoreline's side: the embedded-prefix vector
/// yields NO shoreline finding while the fixture records the relay expectation.
#[test]
fn known_token_divergence_is_specified_and_pinned() {
    let vectors = load_vectors();
    assert!(
        vectors
            .known_token_divergence
            .contains("anywhere in a token"),
        "divergence note must specify the relay's anywhere-in-token match"
    );

    let divergence_files: Vec<&VectorFile> = vectors
        .repos
        .negative
        .files
        .iter()
        .filter(|file| !file.relay_expected_kinds.is_empty())
        .collect();
    assert!(!divergence_files.is_empty(), "a divergence vector exists");
    for file in divergence_files {
        assert!(
            file.expected_kinds.is_empty(),
            "{}: shoreline expects no finding",
            file.path
        );
        for kind in &file.relay_expected_kinds {
            assert_eq!(
                SensitivityKind::parse(kind),
                Some(SensitivityKind::KnownToken),
                "{}: relay expectation is expressed in the shared vocabulary",
                file.path
            );
        }
    }

    // The divergence vectors live in the negative repo — sanity-check it stays
    // negative end-to-end (shoreline's side of the divergence).
    let scan = scan_repo(&vectors.repos.negative);
    assert!(scan.findings.is_empty(), "negative repo yields no findings");
}
