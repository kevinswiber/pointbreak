//! The serialized principal object that rides beside `writer` in projections
//! and command JSON, plus the human display label derived from it. Structured
//! first, formatted second: documents carry `{actorId, status, source}` and
//! surfaces (the inspector, any pretty rendering) derive
//! `claude-code (for kevin@swiber.dev)` from the object client-side.

use serde::Serialize;

use super::delegates::{DelegationMap, PrincipalResolution};
use super::writer::is_agent_actor_id;
use crate::model::ActorId;

/// The structured principal object (ADR-0010 wire shape: three fields,
/// camelCase, optional `actorId`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrincipalView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<ActorId>,
    pub status: PrincipalStatus,
    pub source: PrincipalSource,
}

/// The rendered status space. `Disavowed` is **reserved** in v1: resolution
/// against the delegates file alone cannot distinguish a deleted record from a
/// never-enrolled agent, so v1 emits only `Resolved`/`None`/`Ambiguous` and
/// disavowal manifests as `None` with a `no_delegation_record` /
/// `no_covering_window` reason on the diagnostics channel. The reserved value
/// keeps the wire vocabulary ADR-complete for a future tombstone source. See
/// the `status-vocabulary-disavowed-reserved` finding.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalStatus {
    Resolved,
    None,
    Ambiguous,
    Disavowed,
}

/// Which resolution config was consulted to produce the view.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalSource {
    Delegates,
    None,
}

/// Build the principal object for a writer actor at `occurred_at`. Returns
/// `None` for non-agent-scheme writers — humans (git identities, `actor:local`)
/// and `did:key`s are their own principal and carry no principal object. With no
/// map supplied, an agent-scheme writer degrades to `{status: none, source:
/// none}` — the mirror/bundle posture.
pub fn principal_view_for(
    writer_actor: &ActorId,
    map: Option<&DelegationMap>,
    occurred_at: &str,
) -> Option<PrincipalView> {
    if !is_agent_actor_id(writer_actor.as_str()) {
        return None;
    }
    let Some(map) = map else {
        // No resolution config supplied: mirror/bundle degradation.
        return Some(PrincipalView {
            actor_id: Option::None,
            status: PrincipalStatus::None,
            source: PrincipalSource::None,
        });
    };
    let view = match map.resolve(writer_actor, occurred_at) {
        PrincipalResolution::Resolved(principal) => PrincipalView {
            actor_id: Some(principal),
            status: PrincipalStatus::Resolved,
            source: PrincipalSource::Delegates,
        },
        // The unresolved reason rides the diagnostics channel, not the
        // three-field principal object.
        PrincipalResolution::None(_) => PrincipalView {
            actor_id: Option::None,
            status: PrincipalStatus::None,
            source: PrincipalSource::Delegates,
        },
        // Ambiguity is surfaced as a status, never collapsed to one actorId.
        PrincipalResolution::Ambiguous(_) => PrincipalView {
            actor_id: Option::None,
            status: PrincipalStatus::Ambiguous,
            source: PrincipalSource::Delegates,
        },
    };
    Some(view)
}

/// The raw resolution for a writer actor, for callers that need the failure
/// reason (diagnostics) rather than the wire object. `None` for non-agent
/// writers — they are their own principal and never resolve. Agent writers
/// always return `Some(resolution)`.
pub fn principal_resolution_for_writer(
    writer_actor: &ActorId,
    map: &DelegationMap,
    occurred_at: &str,
) -> Option<PrincipalResolution> {
    is_agent_actor_id(writer_actor.as_str()).then(|| map.resolve(writer_actor, occurred_at))
}

/// Render the human label from a principal object: `claude-code (for
/// kevin@swiber.dev)` when resolved, the bare agent name otherwise. Total — no
/// panics on odd ids.
pub fn principal_display_label(writer_actor: &ActorId, principal: &PrincipalView) -> String {
    let agent_name = writer_actor
        .as_str()
        .strip_prefix("actor:agent:")
        .unwrap_or_else(|| writer_actor.as_str());
    match (&principal.actor_id, principal.status) {
        (Some(actor_id), PrincipalStatus::Resolved) => {
            format!(
                "{agent_name} (for {})",
                principal_display_name(actor_id.as_str())
            )
        }
        _ => agent_name.to_owned(),
    }
}

/// The human-readable form of a principal actor id: the bare email or name for
/// git identities, the full id for anything else (no truncation in v1).
fn principal_display_name(actor_id: &str) -> &str {
    actor_id
        .strip_prefix("actor:git-email:")
        .or_else(|| actor_id.strip_prefix("actor:git-name:"))
        .unwrap_or(actor_id)
}

#[cfg(test)]
mod tests {
    use serde_json::{json, to_value};

    use super::*;
    use crate::session::delegation_map_from_value;

    const AGENT: &str = "actor:agent:claude-code";
    const KEVIN: &str = "actor:git-email:kevin@swiber.dev";
    const ALICE: &str = "actor:git-email:alice@example.com";
    const DID_KEY: &str = "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd";

    fn actor(id: &str) -> ActorId {
        ActorId::new(id)
    }

    fn map_with(records: serde_json::Value) -> DelegationMap {
        delegation_map_from_value(json!({ "delegates": { AGENT: records } })).unwrap()
    }

    fn open_window(principal: &str) -> serde_json::Value {
        json!([{ "principal": principal, "validFrom": "2026-06-10T00:00:00Z", "validUntil": null }])
    }

    #[test]
    fn principal_view_serializes_resolved_shape() {
        let map = map_with(open_window(KEVIN));
        let view = principal_view_for(&actor(AGENT), Some(&map), "2026-06-11T12:00:00Z")
            .expect("agent-scheme writer gets a principal object");
        assert_eq!(
            to_value(&view).unwrap(),
            json!({ "actorId": KEVIN, "status": "resolved", "source": "delegates" })
        );
    }

    #[test]
    fn non_agent_writers_get_no_principal_object() {
        let map = map_with(open_window(KEVIN));
        for non_agent in [KEVIN, "actor:git-name:Kevin Swiber", "actor:local", DID_KEY] {
            assert!(
                principal_view_for(&actor(non_agent), Some(&map), "2026-06-11T12:00:00Z").is_none(),
                "{non_agent} is its own principal"
            );
        }
    }

    #[test]
    fn missing_map_degrades_to_none_status() {
        let view = principal_view_for(&actor(AGENT), None, "2026-06-11T12:00:00Z").unwrap();
        assert_eq!(
            to_value(&view).unwrap(),
            json!({ "status": "none", "source": "none" })
        );
    }

    #[test]
    fn no_covering_window_serializes_status_none_with_delegates_source() {
        let map = map_with(json!([{
            "principal": KEVIN,
            "validFrom": "2026-06-10T00:00:00Z",
            "validUntil": "2026-06-12T00:00:00Z"
        }]));
        let view = principal_view_for(&actor(AGENT), Some(&map), "2026-07-01T00:00:00Z").unwrap();
        assert_eq!(
            to_value(&view).unwrap(),
            json!({ "status": "none", "source": "delegates" })
        );
    }

    #[test]
    fn ambiguous_resolution_serializes_status_ambiguous() {
        let map = map_with(json!([
            { "principal": KEVIN, "validFrom": "2026-06-10T00:00:00Z", "validUntil": null },
            { "principal": ALICE, "validFrom": "2026-06-15T00:00:00Z", "validUntil": null }
        ]));
        let view = principal_view_for(&actor(AGENT), Some(&map), "2026-06-16T00:00:00Z").unwrap();
        assert_eq!(
            to_value(&view).unwrap(),
            json!({ "status": "ambiguous", "source": "delegates" })
        );
    }

    #[test]
    fn display_label_renders_agent_for_principal() {
        // Resolved git-email principal.
        let map = map_with(open_window(KEVIN));
        let view = principal_view_for(&actor(AGENT), Some(&map), "2026-06-11T12:00:00Z").unwrap();
        assert_eq!(
            principal_display_label(&actor(AGENT), &view),
            "claude-code (for kevin@swiber.dev)"
        );

        // Resolved git-name principal.
        let map = map_with(open_window("actor:git-name:Kevin Swiber"));
        let view = principal_view_for(&actor(AGENT), Some(&map), "2026-06-11T12:00:00Z").unwrap();
        assert_eq!(
            principal_display_label(&actor(AGENT), &view),
            "claude-code (for Kevin Swiber)"
        );

        // Resolved opaque (did:key) principal — full id, no truncation in v1.
        let map = map_with(open_window(DID_KEY));
        let view = principal_view_for(&actor(AGENT), Some(&map), "2026-06-11T12:00:00Z").unwrap();
        assert_eq!(
            principal_display_label(&actor(AGENT), &view),
            format!("claude-code (for {DID_KEY})")
        );

        // Unresolved — no parenthetical.
        let view = principal_view_for(&actor(AGENT), None, "2026-06-11T12:00:00Z").unwrap();
        assert_eq!(principal_display_label(&actor(AGENT), &view), "claude-code");
    }
}
