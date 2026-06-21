use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::util::sorted_unique;
use crate::error::{Result, ShoreError};
use crate::model::{
    AssessmentId, EventId, InputRequestId, ObservationId, ReviewTargetRef, TrackId,
};
use crate::session::body_artifact::load_body_artifact;
use crate::session::event::{
    EventType, ReviewAssessment, ReviewAssessmentRecordedPayload, ShoreEvent, Writer,
};
use crate::session::observation::ResolvedReviewUnit;

pub(crate) struct AssessmentProjectionOptions<'a> {
    pub store_dir: &'a Path,
    pub events: &'a [ShoreEvent],
    pub resolved: &'a ResolvedReviewUnit,
    pub track_filter: Option<TrackId>,
    pub include_summary: bool,
    pub include_all: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurrentAssessmentView {
    pub status: CurrentAssessmentStatus,
    pub records: Vec<AssessmentView>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CurrentAssessmentStatus {
    Unassessed,
    Resolved(ReviewAssessment),
    /// Multiple current records remain ambiguous even when their assessment
    /// values agree, because each track is still an independent assertion.
    Ambiguous(Vec<ReviewAssessment>),
}

impl CurrentAssessmentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unassessed => "unassessed",
            Self::Resolved(_) => "resolved",
            Self::Ambiguous(_) => "ambiguous",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssessmentRecordStatus {
    Current,
    Replaced,
}

impl AssessmentRecordStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Replaced => "replaced",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssessmentView {
    pub id: AssessmentId,
    pub event_id: EventId,
    pub track_id: TrackId,
    pub target: ReviewTargetRef,
    pub assessment: ReviewAssessment,
    pub summary: Option<String>,
    pub summary_content_hash: Option<String>,
    pub status: AssessmentRecordStatus,
    pub replaces: Vec<AssessmentId>,
    pub related_observations: Vec<ObservationId>,
    pub related_input_requests: Vec<InputRequestId>,
    pub created_at: String,
    pub writer: Writer,
}

pub(crate) fn project_assessments(
    options: AssessmentProjectionOptions<'_>,
) -> Result<(CurrentAssessmentView, Vec<AssessmentView>)> {
    let records = collect_assessment_records(options.events, options.resolved)?;
    let replaced_ids = records
        .values()
        .flat_map(|record| record.payload.replaces_assessment_ids.iter().cloned())
        .collect::<BTreeSet<_>>();
    let mut all_views = Vec::new();

    for record in records.into_values() {
        if options
            .track_filter
            .as_ref()
            .is_some_and(|filter| filter != &record.track_id)
        {
            continue;
        }

        let view = assessment_view_from_event(
            options.store_dir,
            record.event,
            record.payload,
            record.track_id,
            &replaced_ids,
            options.include_summary,
        )?;
        all_views.push(view);
    }

    sort_assessment_views(&mut all_views);
    let current_records = all_views
        .iter()
        .filter(|view| view.status == AssessmentRecordStatus::Current)
        .cloned()
        .collect::<Vec<_>>();
    let current_status = match current_records.as_slice() {
        [] => CurrentAssessmentStatus::Unassessed,
        [record] => CurrentAssessmentStatus::Resolved(record.assessment),
        records => CurrentAssessmentStatus::Ambiguous(
            records.iter().map(|record| record.assessment).collect(),
        ),
    };
    let assessments = if options.include_all {
        all_views
    } else {
        current_records.clone()
    };

    Ok((
        CurrentAssessmentView {
            status: current_status,
            records: current_records,
        },
        assessments,
    ))
}

struct AssessmentEventRecord<'a> {
    event: &'a ShoreEvent,
    payload: ReviewAssessmentRecordedPayload,
    track_id: TrackId,
}

fn collect_assessment_records<'a>(
    events: &'a [ShoreEvent],
    resolved: &ResolvedReviewUnit,
) -> Result<BTreeMap<AssessmentId, AssessmentEventRecord<'a>>> {
    let mut records: BTreeMap<AssessmentId, AssessmentEventRecord<'a>> = BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewAssessmentRecorded)
    {
        if crate::model::subject_revision_id(&event.target.subject) != Some(&resolved.revision_id) {
            continue;
        }

        let payload: ReviewAssessmentRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        let track_id =
            event.target.track_id.clone().ok_or_else(|| {
                ShoreError::Message("assessment event missing track id".to_owned())
            })?;
        let assessment_id = payload.assessment_id.clone();
        let replace_record = records
            .get(&assessment_id)
            .is_none_or(|record| event.event_id.as_str() < record.event.event_id.as_str());
        if replace_record {
            records.insert(
                assessment_id,
                AssessmentEventRecord {
                    event,
                    payload,
                    track_id,
                },
            );
        }
    }

    Ok(records)
}

fn assessment_view_from_event(
    store_dir: &Path,
    event: &ShoreEvent,
    payload: ReviewAssessmentRecordedPayload,
    track_id: TrackId,
    replaced_ids: &BTreeSet<AssessmentId>,
    include_summary: bool,
) -> Result<AssessmentView> {
    let summary = if include_summary {
        assessment_summary(store_dir, &payload)?
    } else {
        None
    };
    let status = if replaced_ids.contains(&payload.assessment_id) {
        AssessmentRecordStatus::Replaced
    } else {
        AssessmentRecordStatus::Current
    };
    let replaces = sorted_unique(payload.replaces_assessment_ids);
    let related_observations = sorted_unique(payload.related_observation_ids);
    let related_input_requests = sorted_unique(payload.related_input_request_ids);

    Ok(AssessmentView {
        id: payload.assessment_id,
        event_id: event.event_id.clone(),
        track_id,
        target: payload.target,
        assessment: payload.assessment,
        summary,
        summary_content_hash: payload.summary_content_hash,
        status,
        replaces,
        related_observations,
        related_input_requests,
        created_at: event.occurred_at.clone(),
        writer: event.writer.clone(),
    })
}

fn assessment_summary(
    store_dir: &Path,
    payload: &ReviewAssessmentRecordedPayload,
) -> Result<Option<String>> {
    if payload.summary.is_some() {
        return Ok(payload.summary.clone());
    }
    match payload.summary_artifact_path.as_deref() {
        Some(path) => load_body_artifact(store_dir, path),
        None => Ok(None),
    }
}

fn sort_assessment_views(assessments: &mut [AssessmentView]) {
    assessments.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.event_id.as_str().cmp(right.event_id.as_str()))
    });
}
