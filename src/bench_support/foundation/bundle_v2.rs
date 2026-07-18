use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    LogicalCapabilityEpochV1, QualificationContractError, QualificationRecordKindV1,
    QualificationRecordV1,
};
use crate::canonical_hash::{canonical_json_bytes, sha256_bytes_hex};

pub const EXACT_BUNDLE_SCHEMA_V2: &str = "pointbreak.exact-bundle.v2";

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ExactBundleManifestV2 {
    pub schema: String,
    pub source_manifest_sha256: String,
    pub required_capabilities: LogicalCapabilityEpochV1,
    pub events: Vec<QualificationRecordV1>,
    pub content: Vec<QualificationRecordV1>,
    pub closure: Vec<ExactBundleClosureV2>,
    pub event_set_sha256: String,
    pub bundle_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ExactBundleClosureV2 {
    pub event_logical_key: String,
    pub required_content_keys: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PublicationRecordClassV2 {
    Content,
    Event,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicationStepV2 {
    pub record_class: PublicationRecordClassV2,
    pub logical_key: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExactBundleFailurePointV2 {
    None,
    BeforeFirstEvent,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ExactBundlePublicationReportV2 {
    pub content_created: usize,
    pub content_existing: usize,
    pub events_created: usize,
    pub events_existing: usize,
}

#[derive(Clone, Debug)]
pub struct DisposableBundleDestinationV2 {
    capabilities: LogicalCapabilityEpochV1,
    records: BTreeMap<String, QualificationRecordV1>,
    publication_log: Vec<PublicationStepV2>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ExactBundleError {
    #[error("exact bundle contract failed: {0}")]
    Contract(String),
    #[error("destination is missing required capability {capability}")]
    MissingDestinationCapability { capability: String },
    #[error("event {event_logical_key} requires absent content {content_logical_key}")]
    MissingRequiredContent {
        event_logical_key: String,
        content_logical_key: String,
    },
    #[error("logical key {logical_key} already exists with different decoded bytes")]
    HardConflict { logical_key: String },
    #[error("publication stopped before the first event")]
    InjectedBeforeFirstEvent,
}

#[derive(Serialize)]
struct EventSetHashPreimage<'a> {
    events: &'a [QualificationRecordV1],
}

#[derive(Serialize)]
struct BundleHashPreimage<'a> {
    schema: &'a str,
    source_manifest_sha256: &'a str,
    required_capabilities: &'a LogicalCapabilityEpochV1,
    events: &'a [QualificationRecordV1],
    content: &'a [QualificationRecordV1],
    closure: &'a [ExactBundleClosureV2],
    event_set_sha256: &'a str,
}

impl ExactBundleManifestV2 {
    pub fn new(
        source_manifest_sha256: impl Into<String>,
        required_capabilities: LogicalCapabilityEpochV1,
        records: Vec<QualificationRecordV1>,
        mut closure: Vec<ExactBundleClosureV2>,
    ) -> Result<Self, ExactBundleError> {
        let (mut events, mut content): (Vec<_>, Vec<_>) = records
            .into_iter()
            .partition(|record| is_event_kind(record.record_kind));
        events.sort_by(|left, right| left.logical_key.cmp(&right.logical_key));
        content.sort_by(|left, right| left.logical_key.cmp(&right.logical_key));
        for requirement in &mut closure {
            requirement.required_content_keys.sort();
        }
        closure.sort_by(|left, right| left.event_logical_key.cmp(&right.event_logical_key));

        let mut manifest = Self {
            schema: EXACT_BUNDLE_SCHEMA_V2.to_owned(),
            source_manifest_sha256: source_manifest_sha256.into(),
            required_capabilities,
            events,
            content,
            closure,
            event_set_sha256: String::new(),
            bundle_sha256: String::new(),
        };
        manifest.refresh_hashes()?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn refresh_hashes(&mut self) -> Result<(), ExactBundleError> {
        self.event_set_sha256 = hash_canonical(&EventSetHashPreimage {
            events: &self.events,
        })?;
        self.bundle_sha256 = hash_canonical(&BundleHashPreimage {
            schema: &self.schema,
            source_manifest_sha256: &self.source_manifest_sha256,
            required_capabilities: &self.required_capabilities,
            events: &self.events,
            content: &self.content,
            closure: &self.closure,
            event_set_sha256: &self.event_set_sha256,
        })?;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), ExactBundleError> {
        if self.schema != EXACT_BUNDLE_SCHEMA_V2 {
            return Err(ExactBundleError::Contract(format!(
                "unsupported schema {}",
                self.schema
            )));
        }
        if self.source_manifest_sha256.trim().is_empty() {
            return Err(ExactBundleError::Contract(
                "source manifest identity must not be empty".to_owned(),
            ));
        }
        self.required_capabilities
            .validate()
            .map_err(contract_error)?;
        validate_records(&self.events, PublicationRecordClassV2::Event)?;
        validate_records(&self.content, PublicationRecordClassV2::Content)?;
        validate_unique_keys(self.events.iter().chain(&self.content))?;
        validate_closure_shape(&self.closure)?;

        let expected_event_set = hash_canonical(&EventSetHashPreimage {
            events: &self.events,
        })?;
        if self.event_set_sha256 != expected_event_set {
            return Err(ExactBundleError::Contract(format!(
                "event-set hash {} does not match {}",
                self.event_set_sha256, expected_event_set
            )));
        }
        let expected_bundle = hash_canonical(&BundleHashPreimage {
            schema: &self.schema,
            source_manifest_sha256: &self.source_manifest_sha256,
            required_capabilities: &self.required_capabilities,
            events: &self.events,
            content: &self.content,
            closure: &self.closure,
            event_set_sha256: &self.event_set_sha256,
        })?;
        if self.bundle_sha256 != expected_bundle {
            return Err(ExactBundleError::Contract(format!(
                "bundle hash {} does not match {}",
                self.bundle_sha256, expected_bundle
            )));
        }
        Ok(())
    }
}

impl DisposableBundleDestinationV2 {
    pub fn new(capabilities: LogicalCapabilityEpochV1) -> Self {
        Self {
            capabilities,
            records: BTreeMap::new(),
            publication_log: Vec::new(),
        }
    }

    pub fn seed_record(&mut self, record: QualificationRecordV1) {
        self.records.insert(record.logical_key.clone(), record);
    }

    pub fn record(&self, logical_key: &str) -> Option<&QualificationRecordV1> {
        self.records.get(logical_key)
    }

    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    pub fn event_count(&self) -> usize {
        self.records
            .values()
            .filter(|record| is_event_kind(record.record_kind))
            .count()
    }

    pub fn content_count(&self) -> usize {
        self.records
            .values()
            .filter(|record| !is_event_kind(record.record_kind))
            .count()
    }

    pub fn publication_log(&self) -> &[PublicationStepV2] {
        &self.publication_log
    }
}

pub fn publish_exact_bundle_v2(
    destination: &mut DisposableBundleDestinationV2,
    manifest: &ExactBundleManifestV2,
    failure_point: ExactBundleFailurePointV2,
) -> Result<ExactBundlePublicationReportV2, ExactBundleError> {
    preflight(destination, manifest)?;

    let mut report = ExactBundlePublicationReportV2::default();
    publish_records(
        destination,
        &manifest.content,
        PublicationRecordClassV2::Content,
        &mut report.content_created,
        &mut report.content_existing,
    );
    if failure_point == ExactBundleFailurePointV2::BeforeFirstEvent {
        return Err(ExactBundleError::InjectedBeforeFirstEvent);
    }
    publish_records(
        destination,
        &manifest.events,
        PublicationRecordClassV2::Event,
        &mut report.events_created,
        &mut report.events_existing,
    );
    verify_published_closure(destination, manifest)?;
    Ok(report)
}

fn preflight(
    destination: &DisposableBundleDestinationV2,
    manifest: &ExactBundleManifestV2,
) -> Result<(), ExactBundleError> {
    manifest.validate()?;
    if destination.capabilities.epoch != manifest.required_capabilities.epoch {
        return Err(ExactBundleError::MissingDestinationCapability {
            capability: manifest.required_capabilities.epoch.clone(),
        });
    }
    for capability in &manifest.required_capabilities.required {
        if !destination.capabilities.required.contains(capability) {
            return Err(ExactBundleError::MissingDestinationCapability {
                capability: capability.clone(),
            });
        }
    }

    walk_closure(manifest, |_, _| Ok(()))?;

    for record in manifest.content.iter().chain(&manifest.events) {
        if let Some(existing) = destination.records.get(&record.logical_key)
            && !records_match_exactly(existing, record)
        {
            return Err(ExactBundleError::HardConflict {
                logical_key: record.logical_key.clone(),
            });
        }
    }
    Ok(())
}

fn verify_published_closure(
    destination: &DisposableBundleDestinationV2,
    manifest: &ExactBundleManifestV2,
) -> Result<(), ExactBundleError> {
    walk_closure(manifest, |event, required_content| {
        for record in std::iter::once(event).chain(required_content.iter().copied()) {
            let stored = destination
                .records
                .get(&record.logical_key)
                .ok_or_else(|| {
                    ExactBundleError::Contract(format!(
                        "published closure is missing {}",
                        record.logical_key
                    ))
                })?;
            if !records_match_exactly(stored, record) {
                return Err(ExactBundleError::HardConflict {
                    logical_key: record.logical_key.clone(),
                });
            }
        }
        Ok(())
    })
}

fn walk_closure(
    manifest: &ExactBundleManifestV2,
    mut visit: impl FnMut(
        &QualificationRecordV1,
        &[&QualificationRecordV1],
    ) -> Result<(), ExactBundleError>,
) -> Result<(), ExactBundleError> {
    let events = manifest
        .events
        .iter()
        .map(|record| (record.logical_key.as_str(), record))
        .collect::<BTreeMap<_, _>>();
    let content = manifest
        .content
        .iter()
        .map(|record| (record.logical_key.as_str(), record))
        .collect::<BTreeMap<_, _>>();

    for requirement in &manifest.closure {
        let event = events
            .get(requirement.event_logical_key.as_str())
            .copied()
            .ok_or_else(|| {
                ExactBundleError::Contract(format!(
                    "closure names absent event {}",
                    requirement.event_logical_key
                ))
            })?;
        let required_content = requirement
            .required_content_keys
            .iter()
            .map(|content_key| {
                content.get(content_key.as_str()).copied().ok_or_else(|| {
                    ExactBundleError::MissingRequiredContent {
                        event_logical_key: requirement.event_logical_key.clone(),
                        content_logical_key: content_key.clone(),
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        visit(event, &required_content)?;
    }
    Ok(())
}

fn records_match_exactly(left: &QualificationRecordV1, right: &QualificationRecordV1) -> bool {
    left.decoded_sha256 == right.decoded_sha256 && left.decoded_bytes == right.decoded_bytes
}

fn publish_records(
    destination: &mut DisposableBundleDestinationV2,
    records: &[QualificationRecordV1],
    record_class: PublicationRecordClassV2,
    created: &mut usize,
    existing: &mut usize,
) {
    for record in records {
        if destination.records.contains_key(&record.logical_key) {
            *existing += 1;
        } else {
            destination
                .records
                .insert(record.logical_key.clone(), record.clone());
            destination.publication_log.push(PublicationStepV2 {
                record_class,
                logical_key: record.logical_key.clone(),
            });
            *created += 1;
        }
    }
}

fn validate_records(
    records: &[QualificationRecordV1],
    expected_class: PublicationRecordClassV2,
) -> Result<(), ExactBundleError> {
    let mut previous = None;
    for record in records {
        record.validate().map_err(contract_error)?;
        let actual_class = if is_event_kind(record.record_kind) {
            PublicationRecordClassV2::Event
        } else {
            PublicationRecordClassV2::Content
        };
        if actual_class != expected_class {
            return Err(ExactBundleError::Contract(format!(
                "record {} is in the wrong transfer class",
                record.logical_key
            )));
        }
        if let Some(previous) = previous
            && record.logical_key.as_str() <= previous
        {
            return Err(ExactBundleError::Contract(format!(
                "record keys are not unique and sorted at {}",
                record.logical_key
            )));
        }
        previous = Some(record.logical_key.as_str());
    }
    Ok(())
}

fn validate_unique_keys<'a>(
    records: impl Iterator<Item = &'a QualificationRecordV1>,
) -> Result<(), ExactBundleError> {
    let mut keys = BTreeSet::new();
    for record in records {
        if !keys.insert(&record.logical_key) {
            return Err(ExactBundleError::Contract(format!(
                "logical key {} appears more than once",
                record.logical_key
            )));
        }
    }
    Ok(())
}

fn validate_closure_shape(closure: &[ExactBundleClosureV2]) -> Result<(), ExactBundleError> {
    let mut previous_event = None;
    for requirement in closure {
        if let Some(previous) = previous_event
            && requirement.event_logical_key.as_str() <= previous
        {
            return Err(ExactBundleError::Contract(format!(
                "closure events are not unique and sorted at {}",
                requirement.event_logical_key
            )));
        }
        let mut previous_content = None;
        for content_key in &requirement.required_content_keys {
            if let Some(previous) = previous_content
                && content_key.as_str() <= previous
            {
                return Err(ExactBundleError::Contract(format!(
                    "closure content keys are not unique and sorted at {content_key}"
                )));
            }
            previous_content = Some(content_key.as_str());
        }
        previous_event = Some(requirement.event_logical_key.as_str());
    }
    Ok(())
}

fn is_event_kind(kind: QualificationRecordKindV1) -> bool {
    matches!(
        kind,
        QualificationRecordKindV1::LegacyEvent
            | QualificationRecordKindV1::GenerationProposal
            | QualificationRecordKindV1::RelationAttestation
            | QualificationRecordKindV1::FactPort
    )
}

fn hash_canonical(value: &impl Serialize) -> Result<String, ExactBundleError> {
    let value = serde_json::to_value(value)
        .map_err(|error| ExactBundleError::Contract(error.to_string()))?;
    let bytes = canonical_json_bytes(&value)
        .map_err(|error| ExactBundleError::Contract(error.to_string()))?;
    Ok(sha256_bytes_hex(&bytes))
}

fn contract_error(error: QualificationContractError) -> ExactBundleError {
    ExactBundleError::Contract(error.to_string())
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;
    use crate::bench_support::foundation::{
        LogicalCapabilityEpochV1, QualificationRecordKindV1, QualificationRecordV1,
    };
    use crate::canonical_hash::canonical_json_bytes;

    #[derive(Deserialize)]
    struct FixtureRecord {
        logical_key: String,
        record_kind: QualificationRecordKindV1,
        decoded: serde_json::Value,
    }

    fn fixture_records() -> Vec<QualificationRecordV1> {
        let records = serde_json::from_str::<Vec<FixtureRecord>>(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/store-foundation/bundle-v2/exact.json"
        )))
        .expect("exact fixture parses");
        records
            .into_iter()
            .map(|record| {
                QualificationRecordV1::new(
                    record.logical_key,
                    record.record_kind,
                    canonical_json_bytes(&record.decoded).expect("fixture canonicalizes"),
                )
            })
            .collect()
    }

    fn exact_manifest() -> ExactBundleManifestV2 {
        ExactBundleManifestV2::new(
            "source-manifest-sha256",
            LogicalCapabilityEpochV1::foundation(),
            fixture_records(),
            vec![ExactBundleClosureV2 {
                event_logical_key: "event:root".to_owned(),
                required_content_keys: vec!["content:object".to_owned()],
            }],
        )
        .expect("exact manifest")
    }

    #[test]
    fn exact_manifest_round_trips_event_bytes_and_event_set_identity() {
        let manifest = exact_manifest();
        let encoded = serde_json::to_vec(&manifest).expect("manifest serializes");
        let decoded = serde_json::from_slice::<ExactBundleManifestV2>(&encoded)
            .expect("manifest deserializes");

        decoded.validate().expect("manifest validates");
        assert_eq!(decoded, manifest);
        assert_eq!(
            decoded.events[0].decoded_bytes,
            manifest.events[0].decoded_bytes
        );
        assert_eq!(decoded.event_set_sha256, manifest.event_set_sha256);
    }

    #[test]
    fn invalid_record_hash_fails_preflight_without_writes() {
        let mut manifest = exact_manifest();
        manifest.events[0].decoded_sha256 = "0".repeat(64);
        let mut destination =
            DisposableBundleDestinationV2::new(LogicalCapabilityEpochV1::foundation());

        let error =
            publish_exact_bundle_v2(&mut destination, &manifest, ExactBundleFailurePointV2::None)
                .expect_err("hash mismatch must fail");

        assert!(matches!(error, ExactBundleError::Contract(_)));
        assert_eq!(destination.record_count(), 0);
    }

    #[test]
    fn missing_capability_or_required_content_fails_before_any_write() {
        let manifest = exact_manifest();
        let mut capabilities = LogicalCapabilityEpochV1::foundation();
        capabilities.required.pop();
        let mut destination = DisposableBundleDestinationV2::new(capabilities);

        assert!(matches!(
            publish_exact_bundle_v2(&mut destination, &manifest, ExactBundleFailurePointV2::None,),
            Err(ExactBundleError::MissingDestinationCapability { .. })
        ));
        assert_eq!(destination.record_count(), 0);

        let mut incomplete = exact_manifest();
        incomplete.content.clear();
        incomplete
            .refresh_hashes()
            .expect("rehash incomplete fixture");
        let mut destination =
            DisposableBundleDestinationV2::new(LogicalCapabilityEpochV1::foundation());
        assert!(matches!(
            publish_exact_bundle_v2(
                &mut destination,
                &incomplete,
                ExactBundleFailurePointV2::None,
            ),
            Err(ExactBundleError::MissingRequiredContent { .. })
        ));
        assert_eq!(destination.record_count(), 0);
    }

    #[test]
    fn conflicting_existing_key_fails_preflight_without_partial_publication() {
        let manifest = exact_manifest();
        let mut destination =
            DisposableBundleDestinationV2::new(LogicalCapabilityEpochV1::foundation());
        destination.seed_record(QualificationRecordV1::new(
            "content:object",
            QualificationRecordKindV1::ObjectArtifact,
            br#"{"different":true}"#.to_vec(),
        ));

        assert!(matches!(
            publish_exact_bundle_v2(&mut destination, &manifest, ExactBundleFailurePointV2::None,),
            Err(ExactBundleError::HardConflict { .. })
        ));
        assert_eq!(destination.record_count(), 1);
        assert_eq!(destination.event_count(), 0);
    }

    #[test]
    fn publication_is_content_first_event_last_and_retry_is_idempotent() {
        let manifest = exact_manifest();
        let mut destination =
            DisposableBundleDestinationV2::new(LogicalCapabilityEpochV1::foundation());

        assert!(matches!(
            publish_exact_bundle_v2(
                &mut destination,
                &manifest,
                ExactBundleFailurePointV2::BeforeFirstEvent,
            ),
            Err(ExactBundleError::InjectedBeforeFirstEvent)
        ));
        assert_eq!(destination.content_count(), manifest.content.len());
        assert_eq!(destination.event_count(), 0);

        let first =
            publish_exact_bundle_v2(&mut destination, &manifest, ExactBundleFailurePointV2::None)
                .expect("retry publishes events");
        assert_eq!(first.content_created, 0);
        assert_eq!(first.content_existing, manifest.content.len());
        assert_eq!(first.events_created, manifest.events.len());

        let second =
            publish_exact_bundle_v2(&mut destination, &manifest, ExactBundleFailurePointV2::None)
                .expect("ambiguous retry is idempotent");
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, manifest.events.len());
        assert!(
            destination
                .publication_log()
                .windows(2)
                .all(|steps| { steps[0].record_class <= steps[1].record_class })
        );
    }

    #[test]
    fn semantic_dangling_target_is_allowed_but_required_closure_is_not() {
        let manifest = exact_manifest();
        assert!(
            manifest.events[0]
                .decoded_bytes
                .windows(b"revision:outside-bundle".len())
                .any(|window| window == b"revision:outside-bundle")
        );
        manifest
            .validate()
            .expect("semantic target is opaque to transfer");

        let incomplete = serde_json::from_str::<serde_json::Value>(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/store-foundation/bundle-v2/incomplete.json"
        )))
        .expect("incomplete fixture parses");
        assert_eq!(incomplete["required_content_key"], "content:missing");
    }

    #[test]
    fn divergent_key_is_hard_conflict_and_omission_never_deletes() {
        let manifest = exact_manifest();
        let omitted = QualificationRecordV1::new(
            "content:destination-only",
            QualificationRecordKindV1::NoteBody,
            b"destination-only".to_vec(),
        );
        let mut destination =
            DisposableBundleDestinationV2::new(LogicalCapabilityEpochV1::foundation());
        destination.seed_record(omitted.clone());

        publish_exact_bundle_v2(&mut destination, &manifest, ExactBundleFailurePointV2::None)
            .expect("bundle publishes");
        assert_eq!(
            destination.record("content:destination-only"),
            Some(&omitted)
        );

        let mut divergent = exact_manifest();
        divergent.events[0] = QualificationRecordV1::new(
            "event:root",
            QualificationRecordKindV1::LegacyEvent,
            br#"{"divergent":true}"#.to_vec(),
        );
        divergent
            .refresh_hashes()
            .expect("rehash divergent manifest");
        assert!(matches!(
            publish_exact_bundle_v2(
                &mut destination,
                &divergent,
                ExactBundleFailurePointV2::None,
            ),
            Err(ExactBundleError::HardConflict { .. })
        ));
    }

    #[test]
    fn manifest_has_no_physical_or_destination_local_coordinates() {
        let encoded = serde_json::to_string(&exact_manifest()).expect("manifest serializes");
        for forbidden in [
            "sqlite",
            "wal",
            "segment_offset",
            "physical_path",
            "receipt_id",
            "destination_local",
        ] {
            assert!(
                !encoded.to_ascii_lowercase().contains(forbidden),
                "{forbidden}"
            );
        }

        let corrupt = serde_json::from_str::<serde_json::Value>(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/store-foundation/bundle-v2/corrupt.json"
        )))
        .expect("corrupt fixture parses as untyped json");
        assert_eq!(corrupt["record_kind"], "physical_frame");
        assert!(
            serde_json::from_value::<QualificationRecordKindV1>(corrupt["record_kind"].clone())
                .is_err()
        );
    }
}
