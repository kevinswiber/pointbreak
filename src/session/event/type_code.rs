//! The frozen, append-only `EventType` ↔ type-code registry.
//!
//! A type code is a short opaque token (`"t:NN"`) assigned to an event family
//! **once**, when the family is first introduced, and **never reassigned**: renaming
//! the Rust variant or its display string never changes the code, and a retired
//! family keeps its code reserved forever so old signed events stay decodable.
//!
//! The signed views and the stored envelope bind the **code**, not the renamable
//! snake_case name. [`EventType::as_str`](super::EventType::as_str) stays a display
//! lookup the projection reads; it is never a signed/identity value.

use super::EventType;

/// The single source of truth for the frozen registry. Forward and inverse lookups
/// both read this table so they cannot drift.
///
/// **Append-only, never reassigned.** A new family appends a new `t:NN` at the end;
/// an existing entry's code is never changed and a retired family keeps its code
/// reserved forever (so old signed events stay decodable). Do not reorder this table.
const REGISTRY: [(EventType, &str); 16] = [
    (EventType::ReviewInitialized, "t:01"),
    (EventType::WorkObjectProposed, "t:02"),
    (EventType::ReviewObservationRecorded, "t:03"),
    (EventType::ReviewAssessmentRecorded, "t:04"),
    (EventType::InputRequestOpened, "t:05"),
    (EventType::InputRequestResponded, "t:06"),
    (EventType::ReviewNoteImported, "t:07"),
    (EventType::RevisionRefAssociated, "t:08"),
    (EventType::RevisionRefWithdrawn, "t:09"),
    (EventType::RevisionCommitAssociated, "t:10"),
    (EventType::RevisionCommitWithdrawn, "t:11"),
    (EventType::ValidationCheckRecorded, "t:12"),
    (EventType::TaskCheckpointCaptured, "t:13"),
    (EventType::TaskObservationRecorded, "t:14"),
    (EventType::EventSignatureRecorded, "t:15"),
    (EventType::ArtifactRemoved, "t:16"),
];

/// The frozen opaque type code (`"t:NN"`) for an event family. This is the signed /
/// identity value; never the renamable [`EventType::as_str`](super::EventType::as_str).
pub(crate) fn type_code(ty: EventType) -> &'static str {
    REGISTRY
        .iter()
        .find_map(|(candidate, code)| (*candidate == ty).then_some(*code))
        .expect("every EventType variant has a frozen code in REGISTRY")
}

/// Decode a frozen type code back to its event family; `None` for an unknown code.
pub(crate) fn event_type_from_code(code: &str) -> Option<EventType> {
    REGISTRY
        .iter()
        .find_map(|(ty, candidate)| (*candidate == code).then_some(*ty))
}

/// serde adapter that (de)serializes an [`EventType`] as its frozen opaque code, for
/// the **stored envelope** field (`ShoreEvent.event_type`) via `#[serde(with = ...)]`.
///
/// The stored wire value is the `"t:NN"` code, so a future display rename of an event
/// family never rewrites stored bytes (a projection-only change). Display/projection
/// surfaces keep [`EventType`]'s own readable snake_case serde; only the stored
/// envelope binds the code. A snake_case value in the code position fails to decode —
/// the strict reader turns that into a typed schema-break error.
pub(crate) mod serde_code {
    use super::{EventType, event_type_from_code, type_code};
    use serde::{Deserialize, Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(
        event_type: &EventType,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(type_code(*event_type))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<EventType, D::Error> {
        let code = String::deserialize(deserializer)?;
        event_type_from_code(&code)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown event type code: {code}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frozen registry. A reorder, rename, or reassignment must make this test
    /// fail — the codes are identity and can never shift.
    const FROZEN: [(EventType, &str); 16] = [
        (EventType::ReviewInitialized, "t:01"),
        (EventType::WorkObjectProposed, "t:02"),
        (EventType::ReviewObservationRecorded, "t:03"),
        (EventType::ReviewAssessmentRecorded, "t:04"),
        (EventType::InputRequestOpened, "t:05"),
        (EventType::InputRequestResponded, "t:06"),
        (EventType::ReviewNoteImported, "t:07"),
        (EventType::RevisionRefAssociated, "t:08"),
        (EventType::RevisionRefWithdrawn, "t:09"),
        (EventType::RevisionCommitAssociated, "t:10"),
        (EventType::RevisionCommitWithdrawn, "t:11"),
        (EventType::ValidationCheckRecorded, "t:12"),
        (EventType::TaskCheckpointCaptured, "t:13"),
        (EventType::TaskObservationRecorded, "t:14"),
        (EventType::EventSignatureRecorded, "t:15"),
        (EventType::ArtifactRemoved, "t:16"),
    ];

    #[test]
    fn type_code_is_frozen_and_append_only() {
        for (ty, code) in FROZEN {
            assert_eq!(type_code(ty), code, "forward code drifted for {ty:?}");
            assert_eq!(
                event_type_from_code(code),
                Some(ty),
                "inverse round-trip drifted for {code}"
            );
        }
    }

    #[test]
    fn type_codes_are_unique() {
        let codes: Vec<_> = FROZEN.iter().map(|(_, c)| *c).collect();
        let mut deduped = codes.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(codes.len(), deduped.len(), "duplicate type code assigned");
    }

    #[test]
    fn unknown_code_decodes_to_none() {
        assert_eq!(event_type_from_code("t:00"), None);
        assert_eq!(event_type_from_code("review_initialized"), None);
        assert_eq!(event_type_from_code("t:99"), None);
    }
}
