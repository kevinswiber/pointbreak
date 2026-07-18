//! Deterministic smoke entry point for durable-store qualification workloads.
//!
//! This target establishes workload identity and metadata only. It does not
//! select or time a storage implementation.

use std::process::{Command, ExitCode};

use pointbreak::bench_support::foundation::{
    DisposableBundleDestinationV2, ExactBundleClosureV2, ExactBundleFailurePointV2,
    ExactBundleManifestV2, ExactBundlePublicationReportV2, ImportReceiptPolicyPrototypeV1,
    ImportReceiptPrototypeV1, LogicalCapabilityEpochV1, QualificationCorpusError,
    QualificationCorpusSummaryV1, QualificationSnapshotTotalsV1, ReceiptBackupConsequenceV1,
    ReceiptProjectionConsequenceV1, SnapshotDriftReportV1, load_frozen_legacy_manifest_from_env,
    modeled_post_foundation_manifest, publish_exact_bundle_v2, qualification_filesystem_name,
    synthetic_legacy_manifest,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

const USAGE: &str = "\
Usage: cargo bench --features bench --bench store_foundation -- [--smoke|--transfer-smoke|--help]\n\
\n\
Validates deterministic workload or exact-transfer contracts and prints one JSON metadata record.\n\
No production storage implementation is selected or timed by this target.\n";

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
    if arguments.iter().any(|argument| argument == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if arguments.iter().any(|argument| {
        argument != "--smoke" && argument != "--transfer-smoke" && argument != "--bench"
    }) || (arguments.iter().any(|argument| argument == "--smoke")
        && arguments
            .iter()
            .any(|argument| argument == "--transfer-smoke"))
    {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
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
