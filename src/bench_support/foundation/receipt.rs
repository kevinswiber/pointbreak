use serde::{Deserialize, Serialize};

use super::{ExactBundleError, ExactBundleManifestV2};
use crate::canonical_hash::{canonical_json_bytes, sha256_bytes_hex};

pub const IMPORT_RECEIPT_SCHEMA_V1: &str = "pointbreak.import-receipt-prototype.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportReceiptPolicyPrototypeV1 {
    DurableOperational,
    LocalProvenanceEvent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptProjectionConsequenceV1 {
    OperationalLookup,
    LocalEventProjection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptBackupConsequenceV1 {
    OperationalState,
    LocalJournal,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ImportReceiptPrototypeV1 {
    pub schema: String,
    pub policy: ImportReceiptPolicyPrototypeV1,
    pub source_bundle_sha256: String,
    pub source_event_set_sha256: String,
    pub source_event_sha256: Vec<String>,
    pub local_import_context: String,
    pub receipt_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LocalProvenanceEventPrototypeV1 {
    pub source_bundle_sha256: String,
    pub source_event_set_sha256: String,
    pub source_event_sha256: Vec<String>,
    pub local_import_context: String,
}

#[derive(Serialize)]
struct ReceiptHashPreimage<'a> {
    schema: &'a str,
    policy: ImportReceiptPolicyPrototypeV1,
    source_bundle_sha256: &'a str,
    source_event_set_sha256: &'a str,
    source_event_sha256: &'a [String],
    local_import_context: &'a str,
}

impl ImportReceiptPrototypeV1 {
    pub fn new(
        policy: ImportReceiptPolicyPrototypeV1,
        manifest: &ExactBundleManifestV2,
        local_import_context: impl Into<String>,
    ) -> Result<Self, ExactBundleError> {
        manifest.validate()?;
        let local_import_context = local_import_context.into();
        if local_import_context.trim().is_empty() {
            return Err(ExactBundleError::Contract(
                "local import context must not be empty".to_owned(),
            ));
        }
        let mut receipt = Self {
            schema: IMPORT_RECEIPT_SCHEMA_V1.to_owned(),
            policy,
            source_bundle_sha256: manifest.bundle_sha256.clone(),
            source_event_set_sha256: manifest.event_set_sha256.clone(),
            source_event_sha256: manifest
                .events
                .iter()
                .map(|event| event.decoded_sha256.clone())
                .collect(),
            local_import_context,
            receipt_sha256: String::new(),
        };
        receipt.source_event_sha256.sort();
        receipt.receipt_sha256 = receipt.computed_receipt_sha256()?;
        Ok(receipt)
    }

    pub fn projection_consequence(&self) -> ReceiptProjectionConsequenceV1 {
        match self.policy {
            ImportReceiptPolicyPrototypeV1::DurableOperational => {
                ReceiptProjectionConsequenceV1::OperationalLookup
            }
            ImportReceiptPolicyPrototypeV1::LocalProvenanceEvent => {
                ReceiptProjectionConsequenceV1::LocalEventProjection
            }
        }
    }

    pub fn backup_consequence(&self) -> ReceiptBackupConsequenceV1 {
        match self.policy {
            ImportReceiptPolicyPrototypeV1::DurableOperational => {
                ReceiptBackupConsequenceV1::OperationalState
            }
            ImportReceiptPolicyPrototypeV1::LocalProvenanceEvent => {
                ReceiptBackupConsequenceV1::LocalJournal
            }
        }
    }

    pub fn local_provenance_event(&self) -> Option<LocalProvenanceEventPrototypeV1> {
        (self.policy == ImportReceiptPolicyPrototypeV1::LocalProvenanceEvent).then(|| {
            LocalProvenanceEventPrototypeV1 {
                source_bundle_sha256: self.source_bundle_sha256.clone(),
                source_event_set_sha256: self.source_event_set_sha256.clone(),
                source_event_sha256: self.source_event_sha256.clone(),
                local_import_context: self.local_import_context.clone(),
            }
        })
    }

    fn computed_receipt_sha256(&self) -> Result<String, ExactBundleError> {
        let preimage = ReceiptHashPreimage {
            schema: &self.schema,
            policy: self.policy,
            source_bundle_sha256: &self.source_bundle_sha256,
            source_event_set_sha256: &self.source_event_set_sha256,
            source_event_sha256: &self.source_event_sha256,
            local_import_context: &self.local_import_context,
        };
        let value = serde_json::to_value(preimage)
            .map_err(|error| ExactBundleError::Contract(error.to_string()))?;
        let bytes = canonical_json_bytes(&value)
            .map_err(|error| ExactBundleError::Contract(error.to_string()))?;
        Ok(sha256_bytes_hex(&bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench_support::foundation::{
        ExactBundleClosureV2, ExactBundleManifestV2, LogicalCapabilityEpochV1,
        QualificationRecordKindV1, QualificationRecordV1,
    };

    fn manifest() -> ExactBundleManifestV2 {
        ExactBundleManifestV2::new(
            "source-manifest-sha256",
            LogicalCapabilityEpochV1::foundation(),
            vec![
                QualificationRecordV1::new(
                    "event:one",
                    QualificationRecordKindV1::LegacyEvent,
                    br#"{"eventId":"event:one"}"#.to_vec(),
                ),
                QualificationRecordV1::new(
                    "content:one",
                    QualificationRecordKindV1::ObjectArtifact,
                    b"content".to_vec(),
                ),
            ],
            vec![ExactBundleClosureV2 {
                event_logical_key: "event:one".to_owned(),
                required_content_keys: vec!["content:one".to_owned()],
            }],
        )
        .expect("manifest")
    }

    #[test]
    fn receipt_is_separate_from_event_bytes_and_event_set_root() {
        let manifest = manifest();
        let original_event_bytes = manifest.events[0].decoded_bytes.clone();
        let original_event_set = manifest.event_set_sha256.clone();
        let receipt = ImportReceiptPrototypeV1::new(
            ImportReceiptPolicyPrototypeV1::DurableOperational,
            &manifest,
            "local-import-context",
        )
        .expect("receipt");

        assert_eq!(manifest.events[0].decoded_bytes, original_event_bytes);
        assert_eq!(manifest.event_set_sha256, original_event_set);
        assert_eq!(
            receipt.source_event_sha256,
            vec![manifest.events[0].decoded_sha256.clone()]
        );
        assert!(
            !serde_json::to_string(&manifest)
                .expect("manifest serializes")
                .contains("local-import-context")
        );
    }

    #[test]
    fn both_receipt_policies_expose_projection_and_backup_consequences() {
        let manifest = manifest();
        let operational = ImportReceiptPrototypeV1::new(
            ImportReceiptPolicyPrototypeV1::DurableOperational,
            &manifest,
            "local",
        )
        .expect("operational receipt");
        let provenance = ImportReceiptPrototypeV1::new(
            ImportReceiptPolicyPrototypeV1::LocalProvenanceEvent,
            &manifest,
            "local",
        )
        .expect("provenance receipt");

        assert_eq!(
            operational.projection_consequence(),
            ReceiptProjectionConsequenceV1::OperationalLookup
        );
        assert_eq!(
            operational.backup_consequence(),
            ReceiptBackupConsequenceV1::OperationalState
        );
        assert_eq!(
            provenance.projection_consequence(),
            ReceiptProjectionConsequenceV1::LocalEventProjection
        );
        assert_eq!(
            provenance.backup_consequence(),
            ReceiptBackupConsequenceV1::LocalJournal
        );
        assert!(operational.local_provenance_event().is_none());
        assert!(provenance.local_provenance_event().is_some());
    }
}
