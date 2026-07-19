//! Deterministic smoke entry point for durable-store qualification workloads.
//!
//! The default modes establish workload and transfer identity. The explicit
//! candidate smoke modes exercise only developer-gated qualification profiles.

use std::process::{Command, ExitCode};

use pointbreak::bench_support::foundation::{
    DisposableBundleDestinationV2, ExactBundleClosureV2, ExactBundleFailurePointV2,
    ExactBundleManifestV2, ExactBundlePublicationReportV2, ImportReceiptPolicyPrototypeV1,
    ImportReceiptPrototypeV1, LogicalCapabilityEpochV1, QualificationCorpusError,
    QualificationCorpusSummaryV1, QualificationPerformanceDiagnosticConfigurationV1,
    QualificationPerformancePairOrderV1, QualificationRunConfigurationV1,
    QualificationSnapshotTotalsV1, ReceiptBackupConsequenceV1, ReceiptProjectionConsequenceV1,
    SegmentWorkloadEvidenceV1, SnapshotDriftReportV1, SqliteWorkloadEvidenceV1,
    load_frozen_legacy_manifest_from_env, modeled_post_foundation_manifest,
    publish_exact_bundle_v2, qualification_cargo_lock_sha256, qualification_filesystem_name,
    qualification_source_commit, run_qualification_child,
    run_qualification_performance_diagnostics, run_qualification_platform_matrix,
    run_segment_workload, run_sqlite_workload, synthetic_legacy_manifest,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

const USAGE: &str = "\
Usage: cargo bench --features bench --bench store_foundation -- [--smoke|--transfer-smoke|--sqlite-smoke|--segments-smoke|--qualification-smoke|--qualification-evidence|--qualification-diagnostics|--help]\n\
       --qualification-diagnostics [--qualification-pair-order=alternating|candidate_then_baseline|baseline_then_candidate]\n\
\n\
Validates deterministic workload, transfer, candidate, or native-platform qualification contracts and prints JSON.\n\
Qualification modes use disposable roots and never select or activate production storage.\n";

#[derive(Serialize)]
struct SmokeMetadataV1 {
    schema: &'static str,
    build: BuildMetadataV1,
    dependencies: DependencyMetadataV1,
    runtime: RuntimeMetadataV1,
    configuration: ConfigurationMetadataV1,
    capabilities: LogicalCapabilityEpochV1,
    synthetic_legacy: QualificationCorpusSummaryV1,
    modeled_workload: QualificationCorpusSummaryV1,
    external_legacy: ExternalCorpusMetadataV1,
}

#[derive(Serialize)]
struct BuildMetadataV1 {
    package_version: &'static str,
    source: &'static str,
    commit: Option<&'static str>,
    describe: &'static str,
    dirty: bool,
}

#[derive(Serialize)]
struct DependencyMetadataV1 {
    cargo_lock_sha256: String,
    rustc: String,
}

#[derive(Serialize)]
struct RuntimeMetadataV1 {
    os: &'static str,
    architecture: &'static str,
    filesystem: String,
}

#[derive(Serialize)]
struct ConfigurationMetadataV1 {
    mode: &'static str,
    external_corpus_variable: &'static str,
    external_corpus_configured: bool,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ExternalCorpusMetadataV1 {
    NotConfigured,
    Validated {
        summary: QualificationCorpusSummaryV1,
        snapshot: QualificationSnapshotTotalsV1,
    },
    SnapshotDrift {
        report: SnapshotDriftReportV1,
    },
    Invalid {
        message: String,
    },
}

#[derive(Serialize)]
struct TransferSmokeMetadataV1 {
    schema: &'static str,
    mode: &'static str,
    bundle_sha256: String,
    event_set_sha256: String,
    event_count: usize,
    content_count: usize,
    closure_count: usize,
    interrupted_publication: InterruptedPublicationMetadataV1,
    completion: ExactBundlePublicationReportV2,
    idempotent_retry: ExactBundlePublicationReportV2,
    exact_bytes_verified: bool,
    receipt_alternatives: Vec<ReceiptAlternativeMetadataV1>,
}

#[derive(Serialize)]
struct SqliteSmokeMetadataV1 {
    schema: &'static str,
    mode: &'static str,
    workloads: Vec<SqliteWorkloadEvidenceV1>,
}

#[derive(Serialize)]
struct SegmentSmokeMetadataV1 {
    schema: &'static str,
    mode: &'static str,
    workloads: Vec<SegmentWorkloadEvidenceV1>,
}

#[derive(Serialize)]
struct InterruptedPublicationMetadataV1 {
    content_count: usize,
    event_count: usize,
}

#[derive(Serialize)]
struct ReceiptAlternativeMetadataV1 {
    policy: ImportReceiptPolicyPrototypeV1,
    receipt_sha256: String,
    projection: ReceiptProjectionConsequenceV1,
    backup: ReceiptBackupConsequenceV1,
    emits_local_provenance_event: bool,
}

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .first()
        .is_some_and(|argument| argument == "--qualification-child")
    {
        if arguments.len() != 2 {
            eprintln!("qualification child requires exactly one request path");
            return ExitCode::from(2);
        }
        return match run_qualification_child(std::path::Path::new(&arguments[1])) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("store foundation qualification child failed: {error}");
                ExitCode::from(1)
            }
        };
    }
    if arguments.iter().any(|argument| argument == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let requested_modes = [
        "--smoke",
        "--transfer-smoke",
        "--sqlite-smoke",
        "--segments-smoke",
        "--qualification-smoke",
        "--qualification-evidence",
        "--qualification-diagnostics",
    ]
    .into_iter()
    .filter(|mode| arguments.iter().any(|argument| argument == mode))
    .count();
    let diagnostic_pair_order = match qualification_pair_order(&arguments) {
        Ok(order) => order,
        Err(()) => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let diagnostics_requested = arguments
        .iter()
        .any(|argument| argument == "--qualification-diagnostics");
    if arguments.iter().any(|argument| {
        argument != "--smoke"
            && argument != "--transfer-smoke"
            && argument != "--sqlite-smoke"
            && argument != "--segments-smoke"
            && argument != "--qualification-smoke"
            && argument != "--qualification-evidence"
            && argument != "--qualification-diagnostics"
            && argument != "--bench"
            && !argument.starts_with("--qualification-pair-order=")
    }) || requested_modes > 1
        || (!diagnostics_requested && diagnostic_pair_order.is_some())
    {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }

    if arguments
        .iter()
        .any(|argument| argument == "--qualification-smoke")
    {
        return qualification_report(1);
    }

    if arguments
        .iter()
        .any(|argument| argument == "--qualification-evidence")
    {
        return qualification_report(5);
    }

    if diagnostics_requested {
        return qualification_diagnostics_report(
            diagnostic_pair_order.unwrap_or(QualificationPerformancePairOrderV1::Alternating),
        );
    }

    if arguments
        .iter()
        .any(|argument| argument == "--transfer-smoke")
    {
        return match transfer_smoke_metadata() {
            Ok(metadata) => {
                println!(
                    "{}",
                    serde_json::to_string(&metadata).expect("transfer smoke metadata serializes")
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("store foundation transfer smoke failed: {error}");
                ExitCode::from(1)
            }
        };
    }

    if arguments
        .iter()
        .any(|argument| argument == "--sqlite-smoke")
    {
        return match sqlite_smoke_metadata() {
            Ok(metadata) => {
                println!(
                    "{}",
                    serde_json::to_string(&metadata).expect("SQLite smoke metadata serializes")
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("store foundation SQLite smoke failed: {error}");
                ExitCode::from(1)
            }
        };
    }

    if arguments
        .iter()
        .any(|argument| argument == "--segments-smoke")
    {
        return match segment_smoke_metadata() {
            Ok(metadata) => {
                println!(
                    "{}",
                    serde_json::to_string(&metadata).expect("segment smoke metadata serializes")
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("store foundation segment smoke failed: {error}");
                ExitCode::from(1)
            }
        };
    }

    match smoke_metadata() {
        Ok((metadata, external_is_valid)) => {
            println!(
                "{}",
                serde_json::to_string(&metadata).expect("smoke metadata serializes")
            );
            if external_is_valid {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            }
        }
        Err(error) => {
            eprintln!("store foundation smoke failed: {error}");
            ExitCode::from(1)
        }
    }
}

fn qualification_report(performance_samples: u32) -> ExitCode {
    let disposable = match tempfile::tempdir() {
        Ok(root) => root,
        Err(error) => {
            eprintln!("store foundation qualification failed to create a disposable root: {error}");
            return ExitCode::from(1);
        }
    };
    let source_commit = match qualification_source_commit() {
        Ok(commit) => commit,
        Err(error) => {
            eprintln!("store foundation qualification provenance failed: {error}");
            return ExitCode::from(1);
        }
    };
    let configuration = QualificationRunConfigurationV1 {
        executable: match std::env::current_exe() {
            Ok(executable) => executable,
            Err(error) => {
                eprintln!("store foundation qualification executable lookup failed: {error}");
                return ExitCode::from(1);
            }
        },
        root: disposable.path().join("qualification-run"),
        source_commit,
        cargo_lock_sha256: qualification_cargo_lock_sha256(),
        performance_samples,
    };
    match run_qualification_platform_matrix(&configuration) {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string(&report).expect("qualification report serializes")
            );
            if report.completeness.all_results_passed {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            }
        }
        Err(error) => {
            eprintln!("store foundation qualification failed: {error}");
            ExitCode::from(1)
        }
    }
}

fn qualification_pair_order(
    arguments: &[String],
) -> Result<Option<QualificationPerformancePairOrderV1>, ()> {
    let values = arguments
        .iter()
        .filter_map(|argument| argument.strip_prefix("--qualification-pair-order="))
        .collect::<Vec<_>>();
    if values.len() > 1 {
        return Err(());
    }
    values
        .first()
        .map(|value| match *value {
            "alternating" => Ok(QualificationPerformancePairOrderV1::Alternating),
            "candidate_then_baseline" => {
                Ok(QualificationPerformancePairOrderV1::CandidateThenBaseline)
            }
            "baseline_then_candidate" => {
                Ok(QualificationPerformancePairOrderV1::BaselineThenCandidate)
            }
            _ => Err(()),
        })
        .transpose()
}

fn qualification_diagnostics_report(pair_order: QualificationPerformancePairOrderV1) -> ExitCode {
    let disposable = match tempfile::tempdir() {
        Ok(root) => root,
        Err(error) => {
            eprintln!("store foundation diagnostics failed to create a disposable root: {error}");
            return ExitCode::from(1);
        }
    };
    let source_commit = match qualification_source_commit() {
        Ok(commit) => commit,
        Err(error) => {
            eprintln!("store foundation diagnostics provenance failed: {error}");
            return ExitCode::from(1);
        }
    };
    let configuration = QualificationPerformanceDiagnosticConfigurationV1 {
        executable: match std::env::current_exe() {
            Ok(executable) => executable,
            Err(error) => {
                eprintln!("store foundation diagnostics executable lookup failed: {error}");
                return ExitCode::from(1);
            }
        },
        root: disposable.path().join("performance-diagnostics"),
        source_commit,
        cargo_lock_sha256: qualification_cargo_lock_sha256(),
        warmup_samples: 3,
        measured_samples: 21,
        pair_order,
        external_corpus_root: std::env::var_os("POINTBREAK_QUALIFICATION_CORPUS")
            .map(std::path::PathBuf::from),
    };
    match run_qualification_performance_diagnostics(&configuration) {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string(&report).expect("performance diagnostics serialize")
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("store foundation diagnostics failed: {error}");
            ExitCode::from(1)
        }
    }
}

fn sqlite_smoke_metadata() -> Result<SqliteSmokeMetadataV1, String> {
    let roots = tempfile::tempdir().map_err(|error| error.to_string())?;
    let legacy = synthetic_legacy_manifest().map_err(|error| error.to_string())?;
    let modeled = modeled_post_foundation_manifest().map_err(|error| error.to_string())?;
    let workloads = vec![
        run_sqlite_workload(&roots.path().join("synthetic-legacy"), &legacy)?,
        run_sqlite_workload(&roots.path().join("modeled-foundation"), &modeled)?,
    ];
    Ok(SqliteSmokeMetadataV1 {
        schema: "pointbreak.store-foundation-sqlite-smoke.v1",
        mode: "non_timing_sqlite_qualification",
        workloads,
    })
}

fn segment_smoke_metadata() -> Result<SegmentSmokeMetadataV1, String> {
    let roots = tempfile::tempdir().map_err(|error| error.to_string())?;
    let legacy = synthetic_legacy_manifest().map_err(|error| error.to_string())?;
    let modeled = modeled_post_foundation_manifest().map_err(|error| error.to_string())?;
    let workloads = vec![
        run_segment_workload(&roots.path().join("synthetic-legacy"), &legacy)?,
        run_segment_workload(&roots.path().join("modeled-foundation"), &modeled)?,
    ];
    Ok(SegmentSmokeMetadataV1 {
        schema: "pointbreak.store-foundation-segment-smoke.v1",
        mode: "non_timing_segment_qualification",
        workloads,
    })
}

fn transfer_smoke_metadata() -> Result<TransferSmokeMetadataV1, String> {
    let workload = modeled_post_foundation_manifest().map_err(|error| error.to_string())?;
    let manifest = ExactBundleManifestV2::new(
        workload.manifest_sha256,
        LogicalCapabilityEpochV1::foundation(),
        workload.records,
        vec![
            ExactBundleClosureV2 {
                event_logical_key: "events/000-root.json".to_owned(),
                required_content_keys: vec![
                    "artifacts/documents/blob-guide.md".to_owned(),
                    "artifacts/documents/manifest-guide.json".to_owned(),
                    "artifacts/objects/round-001.json".to_owned(),
                ],
            },
            ExactBundleClosureV2 {
                event_logical_key: "events/003-attestation-verified.json".to_owned(),
                required_content_keys: vec!["artifacts/proofs/relation-001.json".to_owned()],
            },
        ],
    )
    .map_err(|error| error.to_string())?;
    let mut destination =
        DisposableBundleDestinationV2::new(LogicalCapabilityEpochV1::foundation());
    match publish_exact_bundle_v2(
        &mut destination,
        &manifest,
        ExactBundleFailurePointV2::BeforeFirstEvent,
    ) {
        Err(pointbreak::bench_support::foundation::ExactBundleError::InjectedBeforeFirstEvent) => {}
        result => {
            return Err(format!(
                "unexpected interrupted publication result: {result:?}"
            ));
        }
    }
    let interrupted_publication = InterruptedPublicationMetadataV1 {
        content_count: destination.content_count(),
        event_count: destination.event_count(),
    };
    let completion =
        publish_exact_bundle_v2(&mut destination, &manifest, ExactBundleFailurePointV2::None)
            .map_err(|error| error.to_string())?;
    let idempotent_retry =
        publish_exact_bundle_v2(&mut destination, &manifest, ExactBundleFailurePointV2::None)
            .map_err(|error| error.to_string())?;

    let exact_bytes_verified = manifest
        .events
        .iter()
        .chain(&manifest.content)
        .all(|record| {
            destination
                .record(&record.logical_key)
                .is_some_and(|stored| {
                    stored.decoded_sha256 == record.decoded_sha256
                        && stored.decoded_bytes == record.decoded_bytes
                })
        });
    if !exact_bytes_verified {
        return Err("destination bytes differ from the selected manifest".to_owned());
    }

    let receipt_alternatives = [
        ImportReceiptPolicyPrototypeV1::DurableOperational,
        ImportReceiptPolicyPrototypeV1::LocalProvenanceEvent,
    ]
    .into_iter()
    .map(|policy| {
        let receipt = ImportReceiptPrototypeV1::new(policy, &manifest, "transfer-smoke")
            .map_err(|error| error.to_string())?;
        Ok(ReceiptAlternativeMetadataV1 {
            policy,
            receipt_sha256: receipt.receipt_sha256.clone(),
            projection: receipt.projection_consequence(),
            backup: receipt.backup_consequence(),
            emits_local_provenance_event: receipt.local_provenance_event().is_some(),
        })
    })
    .collect::<Result<Vec<_>, String>>()?;

    Ok(TransferSmokeMetadataV1 {
        schema: "pointbreak.store-foundation-transfer-smoke.v1",
        mode: "non_timing_exact_transfer",
        bundle_sha256: manifest.bundle_sha256,
        event_set_sha256: manifest.event_set_sha256,
        event_count: manifest.events.len(),
        content_count: manifest.content.len(),
        closure_count: manifest.closure.len(),
        interrupted_publication,
        completion,
        idempotent_retry,
        exact_bytes_verified,
        receipt_alternatives,
    })
}

fn smoke_metadata() -> Result<(SmokeMetadataV1, bool), QualificationCorpusError> {
    let synthetic_legacy = synthetic_legacy_manifest()?;
    let modeled_workload = modeled_post_foundation_manifest()?;
    let capabilities = LogicalCapabilityEpochV1::foundation();
    capabilities.validate()?;

    let external_corpus_configured = std::env::var_os("POINTBREAK_QUALIFICATION_CORPUS").is_some();
    let (external_legacy, external_is_valid) = if external_corpus_configured {
        match load_frozen_legacy_manifest_from_env() {
            Ok(manifest) => (
                ExternalCorpusMetadataV1::Validated {
                    summary: QualificationCorpusSummaryV1::from_manifest(&manifest),
                    snapshot: QualificationSnapshotTotalsV1::frozen_legacy(),
                },
                true,
            ),
            Err(QualificationCorpusError::SnapshotDrift(report)) => (
                ExternalCorpusMetadataV1::SnapshotDrift { report: *report },
                false,
            ),
            Err(error) => (
                ExternalCorpusMetadataV1::Invalid {
                    message: error.to_string(),
                },
                false,
            ),
        }
    } else {
        (ExternalCorpusMetadataV1::NotConfigured, true)
    };

    Ok((
        SmokeMetadataV1 {
            schema: "pointbreak.store-foundation-smoke.v1",
            build: BuildMetadataV1 {
                package_version: env!("CARGO_PKG_VERSION"),
                source: env!("POINTBREAK_BUILD_SOURCE"),
                commit: match env!("POINTBREAK_BUILD_COMMIT") {
                    "" => None,
                    commit => Some(commit),
                },
                describe: env!("POINTBREAK_BUILD_DESCRIBE"),
                dirty: env!("POINTBREAK_BUILD_DIRTY") == "true",
            },
            dependencies: DependencyMetadataV1 {
                cargo_lock_sha256: sha256_hex(include_bytes!("../Cargo.lock")),
                rustc: rustc_version(),
            },
            runtime: RuntimeMetadataV1 {
                os: std::env::consts::OS,
                architecture: std::env::consts::ARCH,
                filesystem: qualification_filesystem_name(std::path::Path::new(env!(
                    "CARGO_MANIFEST_DIR"
                ))),
            },
            configuration: ConfigurationMetadataV1 {
                mode: "non_timing_smoke",
                external_corpus_variable: "POINTBREAK_QUALIFICATION_CORPUS",
                external_corpus_configured,
            },
            capabilities,
            synthetic_legacy: QualificationCorpusSummaryV1::from_manifest(&synthetic_legacy),
            modeled_workload: QualificationCorpusSummaryV1::from_manifest(&modeled_workload),
            external_legacy,
        },
        external_is_valid,
    ))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|version| version.trim().to_owned())
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| "unavailable".to_owned())
}
