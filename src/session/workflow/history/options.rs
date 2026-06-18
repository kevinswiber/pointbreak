use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::model::{ReviewUnitId, TrackId};
use crate::session::event::EventType;
use crate::session::{ActorAttributesMap, DelegationMap, EventVerificationPolicy, TrustSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewHistoryOptions {
    pub(super) repo: PathBuf,
    pub(super) review_unit_id: Option<ReviewUnitId>,
    pub(super) track: Option<String>,
    pub(super) event_types: Vec<EventType>,
    pub(super) include_body: bool,
    pub(super) verification_policy: Option<EventVerificationPolicy>,
    pub(super) trust_set: TrustSet,
    pub(super) actor_attributes: Option<ActorAttributesMap>,
    pub(super) delegation_map: Option<DelegationMap>,
}

impl ReviewHistoryOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            event_types: Vec::new(),
            include_body: false,
            verification_policy: None,
            trust_set: TrustSet::default(),
            actor_attributes: None,
            delegation_map: None,
        }
    }

    pub fn with_review_unit_id(mut self, review_unit_id: ReviewUnitId) -> Self {
        self.review_unit_id = Some(review_unit_id);
        self
    }

    pub fn with_track(mut self, track: impl Into<String>) -> Self {
        self.track = Some(track.into());
        self
    }

    pub fn with_event_type(mut self, event_type: EventType) -> Self {
        self.event_types.push(event_type);
        self
    }

    pub fn with_include_body(mut self, include_body: bool) -> Self {
        self.include_body = include_body;
        self
    }

    pub fn with_verification_policy(mut self, policy: EventVerificationPolicy) -> Self {
        self.verification_policy = Some(policy);
        self
    }

    pub fn with_trust_set(mut self, trust_set: TrustSet) -> Self {
        self.trust_set = trust_set;
        self
    }

    /// Supply the reader's actor-attributes map. Sibling enrichment for endorsement
    /// readbacks (the endorser's attested kind/roles) — never a classifier input (INV-2).
    pub fn with_actor_attributes(mut self, actor_attributes: Option<ActorAttributesMap>) -> Self {
        self.actor_attributes = actor_attributes;
        self
    }

    /// Supply the reader-side delegation map. Beside `with_trust_set`: config the
    /// reader provides, never store content. With it set, agent-scheme writers'
    /// entries carry a resolved principal object; without it they degrade to the
    /// mirror posture (`status: none`).
    pub fn with_delegation_map(mut self, delegation_map: DelegationMap) -> Self {
        self.delegation_map = Some(delegation_map);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewHistoryFilters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_unit_id: Option<ReviewUnitId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub event_types: Vec<EventType>,
    pub include_body: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ResolvedHistoryFilters {
    pub(super) review_unit_id: Option<ReviewUnitId>,
    pub(super) track_id: Option<TrackId>,
    pub(super) event_types: Vec<EventType>,
    pub(super) include_body: bool,
    pub(super) verification_policy: Option<EventVerificationPolicy>,
    pub(super) trust_set: TrustSet,
    pub(super) actor_attributes: Option<ActorAttributesMap>,
    pub(super) delegation_map: Option<DelegationMap>,
}

impl From<ResolvedHistoryFilters> for ReviewHistoryFilters {
    fn from(filters: ResolvedHistoryFilters) -> Self {
        Self {
            review_unit_id: filters.review_unit_id,
            track_id: filters.track_id,
            event_types: filters.event_types,
            include_body: filters.include_body,
        }
    }
}
