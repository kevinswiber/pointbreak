use serde::{Deserialize, Serialize};

use crate::crypto::EventVerificationStatus;
use crate::model::ActorId;
use crate::session::{DelegationMap, PrincipalResolution, is_agent_actor_id};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactAvailability {
    Available,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventVerificationView {
    pub verification_status: EventVerificationStatus,
    pub artifact_availability: ArtifactAvailability,
}

pub fn verification_view(
    verification_status: EventVerificationStatus,
    artifact_availability: ArtifactAvailability,
) -> EventVerificationView {
    EventVerificationView {
        verification_status,
        artifact_availability,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventVerificationPolicy {
    pub reject_invalid_signatures: bool,
    pub require_trusted_signer: bool,
    pub allow_unsigned: bool,
}

impl EventVerificationPolicy {
    pub fn advisory() -> Self {
        Self {
            reject_invalid_signatures: false,
            require_trusted_signer: false,
            allow_unsigned: true,
        }
    }

    pub fn integrity_strict() -> Self {
        Self {
            reject_invalid_signatures: true,
            require_trusted_signer: false,
            allow_unsigned: true,
        }
    }

    pub fn trusted_strict() -> Self {
        Self {
            reject_invalid_signatures: true,
            require_trusted_signer: true,
            allow_unsigned: false,
        }
    }

    pub fn with_allow_unsigned(mut self, allow_unsigned: bool) -> Self {
        self.allow_unsigned = allow_unsigned;
        self
    }

    pub fn rejects(&self, status: EventVerificationStatus) -> bool {
        match status {
            EventVerificationStatus::Valid => false,
            EventVerificationStatus::Invalid => self.reject_invalid_signatures,
            EventVerificationStatus::UntrustedKey => self.require_trusted_signer,
            EventVerificationStatus::Unsigned => !self.allow_unsigned,
        }
    }
}

/// ADR-0010: whether an agent event's identity is *sufficient* — does a
/// responsible human resolve behind it — distinct from ADR-0009's "is it
/// verified?". This is reader-side policy, never schema. It composes
/// conjunctively under the binding predicate (the binding decision narrows;
/// principal sufficiency can only narrow it further, never widen).
///
/// ADR-0010 names two further presets that are **deferred, not implemented**:
/// `require-verified-principal` (the resolving event must also verify) and
/// `require-signed-delegation` (the delegates file itself must be signed). They
/// are intentionally absent from this enum until the keys plan lands.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PrincipalPolicy {
    /// No principal requirement (default): every event is sufficient.
    #[default]
    None,
    /// Surface unresolved/ambiguous principals as advisory diagnostics, but
    /// never change an operative outcome.
    Prefer,
    /// An agent event is sufficient only when its writer resolves to a non-agent
    /// principal at `occurredAt`.
    RequireResolvablePrincipal,
}

/// The pure principal-sufficiency predicate per the ADR formula. Reads
/// `writer_actor` only to classify scheme and key the human-committed map — no
/// self-asserted field is ever the basis of the decision (ADR-0007), and the map
/// is config the agent does not control.
pub fn principal_sufficient(
    writer_actor: &ActorId,
    occurred_at: &str,
    delegation_map: Option<&DelegationMap>,
    policy: PrincipalPolicy,
) -> bool {
    match policy {
        PrincipalPolicy::None | PrincipalPolicy::Prefer => true,
        PrincipalPolicy::RequireResolvablePrincipal => {
            // Humans (and did:keys) are their own principal.
            if !is_agent_actor_id(writer_actor.as_str()) {
                return true;
            }
            // An agent needs a map that resolves it to a non-agent principal.
            // The non-agent re-check is defense in depth: the v1 parser already
            // rejects agent-scheme principals, but the ADR formula requires
            // `Resolved(<non-agent principal>)` explicitly.
            matches!(
                delegation_map.map(|map| map.resolve(writer_actor, occurred_at)),
                Some(PrincipalResolution::Resolved(ref principal))
                    if !is_agent_actor_id(principal.as_str())
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ActorId;
    use crate::session::delegation_map_from_value;

    const AGENT: &str = "actor:agent:claude-code";
    const KEVIN: &str = "actor:git-email:kevin@swiber.dev";
    const ALICE: &str = "actor:git-email:alice@example.com";
    const DID_KEY: &str = "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd";
    const AT: &str = "2026-06-11T00:00:00Z";

    fn actor(id: &str) -> ActorId {
        ActorId::new(id)
    }

    fn resolving_map() -> DelegationMap {
        delegation_map_from_value(serde_json::json!({
            "delegates": { AGENT: [
                { "principal": KEVIN, "validFrom": "2026-06-01T00:00:00Z", "validUntil": null }
            ]}
        }))
        .unwrap()
    }

    fn ambiguous_map() -> DelegationMap {
        delegation_map_from_value(serde_json::json!({
            "delegates": { AGENT: [
                { "principal": KEVIN, "validFrom": "2026-06-01T00:00:00Z", "validUntil": null },
                { "principal": ALICE, "validFrom": "2026-06-01T00:00:00Z", "validUntil": null }
            ]}
        }))
        .unwrap()
    }

    #[test]
    fn default_principal_policy_is_none() {
        assert_eq!(PrincipalPolicy::default(), PrincipalPolicy::None);
    }

    #[test]
    fn none_and_prefer_are_always_sufficient() {
        let map = resolving_map();
        for policy in [PrincipalPolicy::None, PrincipalPolicy::Prefer] {
            for writer in [AGENT, KEVIN, DID_KEY] {
                assert!(
                    principal_sufficient(&actor(writer), AT, Some(&map), policy),
                    "{writer} under {policy:?} must be sufficient"
                );
                // Even with no map and an unresolved agent.
                assert!(principal_sufficient(&actor(writer), AT, None, policy));
            }
        }
    }

    #[test]
    fn require_resolvable_principal_truth_table() {
        let require = PrincipalPolicy::RequireResolvablePrincipal;
        let resolving = resolving_map();

        // Humans are their own principal.
        assert!(principal_sufficient(
            &actor(KEVIN),
            AT,
            Some(&resolving),
            require
        ));
        // A did:key is non-agent-scheme — its own principal (CI ephemeral).
        assert!(principal_sufficient(
            &actor(DID_KEY),
            AT,
            Some(&resolving),
            require
        ));
        // Agent resolving to a non-agent principal.
        assert!(principal_sufficient(
            &actor(AGENT),
            AT,
            Some(&resolving),
            require
        ));
        // Agent with no covering window → not sufficient.
        let unknown = delegation_map_from_value(serde_json::json!({ "delegates": {} })).unwrap();
        assert!(!principal_sufficient(
            &actor(AGENT),
            AT,
            Some(&unknown),
            require
        ));
        // Agent with no map supplied → not sufficient.
        assert!(!principal_sufficient(&actor(AGENT), AT, None, require));
        // Agent ambiguous → not sufficient (never auto-picked).
        assert!(!principal_sufficient(
            &actor(AGENT),
            AT,
            Some(&ambiguous_map()),
            require
        ));
    }
}
