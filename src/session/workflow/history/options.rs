use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::model::{ReviewUnitId, TrackId};
use crate::session::event::EventType;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewHistoryOptions {
    pub(super) repo: PathBuf,
    pub(super) review_unit_id: Option<ReviewUnitId>,
    pub(super) track: Option<String>,
    pub(super) event_types: Vec<EventType>,
    pub(super) include_body: bool,
}

impl ReviewHistoryOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            event_types: Vec::new(),
            include_body: false,
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
