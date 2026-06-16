mod enroll;
mod ingest;
mod policy;
#[cfg(test)]
pub(crate) mod test_support;
mod trust;
mod verify;
mod write;

pub use enroll::{
    ALLOWED_SIGNERS_REL_PATH, EnrollmentDiff, enroll_signer, stage_enrollment, trust_set_to_value,
};
pub use ingest::IngestEventVerification;
pub(crate) use ingest::verify_events_for_ingest;
pub use policy::{
    ArtifactAvailability, EventVerificationPolicy, EventVerificationView, PrincipalPolicy,
    principal_sufficient, verification_view,
};
pub use trust::{TrustSet, event_signature_trust_set};
pub use verify::verify_event_signature;
pub use write::EventSigningOptions;
pub(crate) use write::sign_event_if_requested;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        ArtifactAvailability, EventVerificationPolicy, TrustSet, event_signature_trust_set,
        verification_view,
    };
    use crate::crypto::{EventVerificationStatus, SignerId};
    use crate::model::ActorId;

    const FRIENDLY_ACTOR: &str = "actor:git-email:alice@example.com";
    const FRIENDLY_SIGNER: &str = "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd";

    #[test]
    fn status_values_serialize_as_contract_snake_case() {
        assert_eq!(
            serde_json::to_value(EventVerificationStatus::Valid).unwrap(),
            "valid"
        );
        assert_eq!(
            serde_json::to_value(EventVerificationStatus::Invalid).unwrap(),
            "invalid"
        );
        assert_eq!(
            serde_json::to_value(EventVerificationStatus::UntrustedKey).unwrap(),
            "untrusted_key"
        );
        assert_eq!(
            serde_json::to_value(EventVerificationStatus::Unsigned).unwrap(),
            "unsigned"
        );
    }

    #[test]
    fn policy_presets_reject_the_contract_statuses() {
        let advisory = EventVerificationPolicy::advisory();
        assert!(!advisory.rejects(EventVerificationStatus::Valid));
        assert!(!advisory.rejects(EventVerificationStatus::Invalid));
        assert!(!advisory.rejects(EventVerificationStatus::UntrustedKey));
        assert!(!advisory.rejects(EventVerificationStatus::Unsigned));

        let integrity = EventVerificationPolicy::integrity_strict();
        assert!(integrity.rejects(EventVerificationStatus::Invalid));
        assert!(!integrity.rejects(EventVerificationStatus::UntrustedKey));
        assert!(!integrity.rejects(EventVerificationStatus::Unsigned));

        let trusted = EventVerificationPolicy::trusted_strict();
        assert!(trusted.rejects(EventVerificationStatus::Invalid));
        assert!(trusted.rejects(EventVerificationStatus::UntrustedKey));
        assert!(trusted.rejects(EventVerificationStatus::Unsigned));
        assert!(
            !trusted
                .with_allow_unsigned(true)
                .rejects(EventVerificationStatus::Unsigned)
        );
    }

    #[test]
    fn verification_status_is_not_artifact_availability() {
        let view = verification_view(
            EventVerificationStatus::Valid,
            ArtifactAvailability::Unavailable,
        );

        assert_eq!(view.verification_status, EventVerificationStatus::Valid);
        assert_eq!(
            view.artifact_availability,
            ArtifactAvailability::Unavailable
        );
    }

    #[test]
    fn trust_set_authorizes_self_certifying_and_friendly_actors() {
        let trust = event_signature_trust_set(json!({
            "allowedSigners": {
                FRIENDLY_ACTOR: [FRIENDLY_SIGNER]
            }
        }))
        .unwrap();
        let signer = SignerId::parse(FRIENDLY_SIGNER).unwrap();

        assert!(trust.authorizes(
            &ActorId::new(FRIENDLY_SIGNER),
            &signer,
            "2026-06-03T00:00:00Z"
        ));
        assert!(trust.authorizes(
            &ActorId::new(FRIENDLY_ACTOR),
            &signer,
            "2026-06-03T00:00:00Z"
        ));
        assert!(!TrustSet::default().authorizes(
            &ActorId::new(FRIENDLY_ACTOR),
            &signer,
            "2026-06-03T00:00:00Z"
        ));
    }

    #[test]
    fn checked_in_allowed_signers_file_authorizes_friendly_actor() {
        let trust = TrustSet::from_allowed_signers_file(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/event_signatures/.shore/allowed-signers.json"),
        )
        .unwrap();
        let signer = SignerId::parse(FRIENDLY_SIGNER).unwrap();

        assert!(trust.authorizes(
            &ActorId::new(FRIENDLY_ACTOR),
            &signer,
            "2026-06-03T00:00:00Z"
        ));
    }
}
