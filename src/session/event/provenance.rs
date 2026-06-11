use serde::{Deserialize, Serialize};

/// Bounded vocabulary naming the local import seam that stamped an event.
///
/// ADR-0009: the binding predicate reads presence only; `via` and
/// `receivedAt` are operator-facing detail.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IngestVia {
    IngestEvents,
    BundleApply,
}

/// Local importer bookkeeping stamped on every event that enters the store
/// through a foreign-event seam. Trustworthy to this store under the
/// single-writer contract; never a signed fact, never trustworthy to a third
/// party reading a mirrored or copied store (ADR-0009).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestProvenance {
    pub via: IngestVia,
    pub received_at: String,
}

/// Stamps every event with this store's own ingest provenance, overwriting any
/// inbound stamp: hop metadata from elsewhere is not a fact (ADR-0009).
pub(crate) fn stamp_ingest_provenance(
    events: &[super::ShoreEvent],
    via: IngestVia,
    received_at: &str,
) -> Vec<super::ShoreEvent> {
    events
        .iter()
        .cloned()
        .map(|mut event| {
            event.ingest = Some(IngestProvenance {
                via,
                received_at: received_at.to_owned(),
            });
            event
        })
        .collect()
}
