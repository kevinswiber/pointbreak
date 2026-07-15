//! Attention item types and the deterministic, fallible `attention_from_events`
//! builder. See the module doc comment in `mod.rs` for the renders-never-gates
//! contract (ADR-0019).

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::error::{Result, ShoreError};
use crate::model::{
    ActorId, AssessmentId, InputRequestId, ObservationId, ReviewTargetRef, RevisionId, TrackId,
    ValidationCheckId, ValidationStatus, ValidationTarget,
};
use crate::session::event::{
    AssertionMode, EventType, InputRequestReasonCode, ReviewAssessment, ShoreEvent,
    ValidationCheckRecordedPayload, WorkObjectProposal, WorkObjectProposedPayload,
};
use crate::session::identity::instant::{compare_event_instants, parse_event_instant};
use crate::session::projection::supersession::SupersessionView;
use crate::session::state::ProjectionDiagnostic;
use crate::session::workflow::assessment::collect_assessment_records_by_revision;
use crate::session::workflow::input_request::{
    InputRequestProjectionRecords, collect_input_request_projection_records, open_input_request_ids,
};
use crate::session::workflow::util::sorted_unique;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionTier {
    Primary,
    Secondary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionFreshnessState {
    Current,
    Superseded,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionFreshness {
    pub state: AttentionFreshnessState,
    /// Named successors, never a single winner (invariant 2).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub superseded_by: Vec<RevisionId>,
}

impl AttentionFreshness {
    fn current() -> Self {
        Self {
            state: AttentionFreshnessState::Current,
            superseded_by: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionItem {
    /// Unique, deterministic key: `{kind}:{anchor id}` — the wire kind tag plus
    /// the anchoring fact/object id. Kind-qualified because different kinds can
    /// share one anchor (ambiguous + competing can both key on a revision id;
    /// stale + follow-up on the same assessment id).
    pub id: String,
    pub tier: AttentionTier,
    /// The work anchor. `None` only for thread-scoped competing_heads items.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<RevisionId>,
    pub freshness: AttentionFreshness,
    /// The anchoring event's `occurred_at` (per-kind rule fixed in the owning
    /// task).
    pub observed_at: String,
    #[serde(flatten)]
    pub detail: AttentionDetail,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AttentionDetail {
    OpenInputRequest {
        input_request_id: InputRequestId,
        mode: AssertionMode,
        reason_code: InputRequestReasonCode,
        title: String,
        track_id: TrackId,
        opened_by: ActorId,
    },
    AmbiguousAssessment {
        /// Every current record on the revision, as peers — never a winner.
        assessments: Vec<AttentionAssessmentRecord>,
    },
    CompetingHeads {
        /// The thread's current heads, sorted for determinism — a peer set, not a
        /// priority ranking.
        head_revision_ids: Vec<RevisionId>,
        thread_revision_count: usize,
    },
    StaleAssessment {
        assessment_id: AssessmentId,
        assessment: ReviewAssessment,
        track_id: TrackId,
        recorded_by: ActorId,
        /// Every current head in the assessment's supersession thread. This is
        /// intentionally distinct from freshness.supersededBy, which names only
        /// direct superseders.
        head_revision_ids: Vec<RevisionId>,
    },
    FailedValidation {
        validation_check_id: ValidationCheckId,
        check_name: String,
        /// `failed` or `errored` only.
        status: ValidationStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_code: Option<i64>,
        track_id: TrackId,
        recorded_by: ActorId,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        log_artifact_content_hashes: Vec<String>,
    },
    FollowUpOutstanding {
        assessment_id: AssessmentId,
        track_id: TrackId,
        recorded_by: ActorId,
        /// The still-open linked requests — non-empty by construction.
        open_input_request_ids: Vec<InputRequestId>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionAssessmentRecord {
    pub assessment_id: AssessmentId,
    pub assessment: ReviewAssessment,
    pub track_id: TrackId,
    pub recorded_by: ActorId,
    pub recorded_at: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related_observation_ids: Vec<ObservationId>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related_input_request_ids: Vec<InputRequestId>,
    /// Whether the assessment's payload target is the revision itself (vs a
    /// file/range/observation/request within it). The suppression predicates
    /// accept only revision-scoped judgments as positive witnesses; never
    /// serialized, so the wire shape is unchanged.
    #[serde(skip)]
    pub revision_scoped: bool,
}

/// The tier of an attention item is a pure function of its kind (+ request mode
/// in v1): an advisory open request is `Secondary`; everything else is
/// `Primary`. This rule lives in exactly ONE place — no other layer re-derives
/// tiering (D3, resolved: the field stays).
fn tier_for(detail: &AttentionDetail) -> AttentionTier {
    match detail {
        AttentionDetail::OpenInputRequest {
            mode: AssertionMode::Advisory,
            ..
        } => AttentionTier::Secondary,
        _ => AttentionTier::Primary,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttentionProjection {
    pub items: Vec<AttentionItem>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

/// Derive attention state from the event log. Deterministic and fallible: the
/// collectors this wraps (`SupersessionView::from_events`, the input-request and
/// assessment collectors) return `Result`, and their errors propagate — they are
/// not folded into diagnostics (invariant 3). `scope` rides through the core
/// because competing-heads containment (Task 2.9) is only decidable while the
/// `SupersessionView` is in hand.
pub(crate) fn attention_from_events(
    events: &[ShoreEvent],
    scope: Option<&RevisionId>,
) -> Result<AttentionProjection> {
    // One SupersessionView per call — the freshness, competing-heads, stale, and
    // failed-validation collectors all read from this single construction.
    let supersession = SupersessionView::from_events(events)?;
    let current_assessments = current_assessment_records_by_revision(events)?;
    let captured_at = captured_at_map(events)?;
    let request_records = collect_input_request_projection_records(events)?;
    let open_request_ids = open_input_request_ids(&request_records);
    let mut items: Vec<AttentionItem> = Vec::new();

    open_input_request_items(&request_records, &supersession, &mut items)?;
    ambiguous_assessment_items(&current_assessments, &supersession, &mut items);
    competing_heads_items(&supersession, &captured_at, &mut items);
    stale_assessment_items(&current_assessments, &supersession, &mut items);
    failed_validation_items(events, &supersession, &current_assessments, &mut items)?;
    follow_up_outstanding_items(
        &current_assessments,
        &open_request_ids,
        &supersession,
        &mut items,
    );

    // Scoping lives inside the core: a competing-heads item is thread-scoped
    // (revision_id == None), so whether it covers the scoped revision is only
    // decidable while the SupersessionView is in hand (invariant 3).
    if let Some(scope) = scope {
        let scope_component = supersession.component_of(scope);
        items.retain(|item| item_covers_scope(item, scope, scope_component));
    }

    sort_items(&mut items);

    Ok(AttentionProjection {
        items,
        diagnostics: supersession.diagnostics,
    })
}

/// Whether an item is in scope for a `--revision` read: anchored items match the
/// scoped revision exactly; a thread-scoped competing-heads item covers the scope
/// when the scope shares its supersession component.
fn item_covers_scope(
    item: &AttentionItem,
    scope: &RevisionId,
    scope_component: Option<&BTreeSet<RevisionId>>,
) -> bool {
    match &item.revision_id {
        Some(revision_id) => revision_id == scope,
        None => match &item.detail {
            AttentionDetail::CompetingHeads {
                head_revision_ids, ..
            } => scope_component.is_some_and(|component| {
                head_revision_ids
                    .iter()
                    .any(|head| component.contains(head))
            }),
            _ => false,
        },
    }
}

/// Deterministic result order: primary tier before secondary; within a tier the
/// oldest `observed_at` first (the longest-waiting ask surfaces first); the
/// kind-qualified id as the final tiebreak.
fn sort_items(items: &mut [AttentionItem]) {
    items.sort_by(|left, right| {
        tier_rank(left.tier)
            .cmp(&tier_rank(right.tier))
            .then_with(|| compare_event_instants(&left.observed_at, &right.observed_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn tier_rank(tier: AttentionTier) -> u8 {
    match tier {
        AttentionTier::Primary => 0,
        AttentionTier::Secondary => 1,
    }
}

/// One `follow_up_outstanding` item per current `accepted_with_follow_up`
/// assessment whose `related_input_request_ids` include at least one still-open
/// request. The broad form (untracked follow-ups with no linked request) is
/// deliberately deferred: nothing in the store types "the follow-up" for that
/// case, so an unlinked card could only clear by replacing a deliberately chosen
/// terminal assessment. The linked open request also lists as its own
/// open_input_request item — two facts, two cards, ambiguity preserved.
fn follow_up_outstanding_items(
    current_by_revision: &BTreeMap<RevisionId, Vec<AttentionAssessmentRecord>>,
    open_request_ids: &BTreeSet<InputRequestId>,
    supersession: &SupersessionView,
    items: &mut Vec<AttentionItem>,
) {
    for (revision_id, peers) in current_by_revision {
        for record in peers {
            if record.assessment != ReviewAssessment::AcceptedWithFollowUp {
                continue;
            }
            let open_linked: Vec<InputRequestId> = record
                .related_input_request_ids
                .iter()
                .filter(|id| open_request_ids.contains(*id))
                .cloned()
                .collect();
            if open_linked.is_empty() {
                continue;
            }
            let detail = AttentionDetail::FollowUpOutstanding {
                assessment_id: record.assessment_id.clone(),
                track_id: record.track_id.clone(),
                recorded_by: record.recorded_by.clone(),
                open_input_request_ids: open_linked,
            };
            items.push(AttentionItem {
                id: format!("follow_up_outstanding:{}", record.assessment_id.as_str()),
                tier: tier_for(&detail),
                revision_id: Some(revision_id.clone()),
                freshness: freshness_for(supersession, revision_id),
                observed_at: record.recorded_at.clone(),
                detail,
            });
        }
    }
}

/// A `RevisionId -> capture occurred_at` map built from the revision-domain
/// `WorkObjectProposed` events, deduped by the lowest-event-id representative so
/// duplicate proposals resolve deterministically. Feeds competing-heads
/// `observed_at` and the final ordering (Task 2.9).
fn captured_at_map(events: &[ShoreEvent]) -> Result<BTreeMap<RevisionId, String>> {
    let mut representatives: BTreeMap<RevisionId, &ShoreEvent> = BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::WorkObjectProposed)
    {
        let payload: WorkObjectProposedPayload = serde_json::from_value(event.payload.clone())?;
        if let WorkObjectProposal::Revision { revision, .. } = payload.work_object {
            representatives
                .entry(revision.id)
                .and_modify(|current| {
                    if event.event_id.as_str() < current.event_id.as_str() {
                        *current = event;
                    }
                })
                .or_insert(event);
        }
    }
    Ok(representatives
        .into_iter()
        .map(|(id, event)| (id, event.occurred_at.clone()))
        .collect())
}

struct ValidationRecord<'a> {
    event: &'a ShoreEvent,
    payload: ValidationCheckRecordedPayload,
    track_id: TrackId,
}

impl ValidationRecord<'_> {
    /// Latest-per-check ordering key: `completed_at` when present, else the
    /// event's `occurred_at`.
    fn sort_time(&self) -> &str {
        self.payload
            .completed_at
            .as_deref()
            .unwrap_or(&self.event.occurred_at)
    }
}

/// One `failed_validation` item per latest failed/errored validation record per
/// `(revision, track, check_name)`, on current heads only. A later `passed`
/// record for the same check clears the card, and so does a later unanimously
/// accepting judgment on the revision (`assessment_subsumes_failure` — Rule B
/// of the judgment-subsumption amendment to ADR-0019); a superseded revision
/// does not report (staleness is the stale-assessment builder's job);
/// `skipped` never reports and never clears. Heads-only + latest-per-check is deliberately
/// stricter than the inspector's `failed_validation_count` rollup, which never
/// clears — the two surfaces will converge on this projection later, not the
/// reverse.
fn failed_validation_items(
    events: &[ShoreEvent],
    supersession: &SupersessionView,
    current_by_revision: &BTreeMap<RevisionId, Vec<AttentionAssessmentRecord>>,
    items: &mut Vec<AttentionItem>,
) -> Result<()> {
    // Dedup by validation_check_id, lowest-event-id representative (mirrors
    // `project_validation_checks`).
    let mut records: BTreeMap<ValidationCheckId, ValidationRecord<'_>> = BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ValidationCheckRecorded)
    {
        let payload: ValidationCheckRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        let track_id =
            event.target.track_id.clone().ok_or_else(|| {
                ShoreError::Message("validation event missing track id".to_owned())
            })?;
        let id = payload.validation_check_id.clone();
        let replace = records
            .get(&id)
            .is_none_or(|record| event.event_id.as_str() < record.event.event_id.as_str());
        if replace {
            records.insert(
                id,
                ValidationRecord {
                    event,
                    payload,
                    track_id,
                },
            );
        }
    }

    // Group by (revision, track, check_name), then decide per group. A single
    // "latest" winner cannot be chosen when two records tie on sort_time: an event
    // id is a content address, not causal order, so letting it break the tie could
    // silently hide a failure behind an equal-time pass. Instead emit every failing
    // record whose sort_time is the group's max — a strictly-later pass clears the
    // card, but a simultaneous pass never does.
    let mut groups: BTreeMap<(RevisionId, TrackId, String), Vec<ValidationRecord<'_>>> =
        BTreeMap::new();
    for record in records.into_values() {
        let ValidationTarget::Revision { revision_id } = &record.payload.target;
        groups
            .entry((
                revision_id.clone(),
                record.track_id.clone(),
                record.payload.check_name.clone(),
            ))
            .or_default()
            .push(record);
    }

    for ((revision_id, _track_id, _check_name), mut group) in groups {
        if !supersession.heads.contains(&revision_id) {
            continue;
        }
        // Skipped records are invisible to the clearing decision: they never
        // report, and a strictly-later skip must not hide an earlier failure.
        let Some(max_time) = group
            .iter()
            .filter(|record| record.payload.status != ValidationStatus::Skipped)
            .map(|record| record.sort_time().to_owned())
            .max()
        else {
            continue;
        };
        let peers = current_by_revision
            .get(&revision_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if assessment_subsumes_failure(peers, &max_time) {
            continue;
        }
        // Deterministic order among tied failing records (the final sort re-orders
        // across kinds, but keep this stable so ties never reorder run to run).
        group.sort_by(|left, right| {
            left.payload
                .validation_check_id
                .as_str()
                .cmp(right.payload.validation_check_id.as_str())
        });
        for record in group {
            if record.sort_time() != max_time {
                continue;
            }
            if !matches!(
                record.payload.status,
                ValidationStatus::Failed | ValidationStatus::Errored
            ) {
                continue;
            }
            let observed_at = record
                .payload
                .completed_at
                .clone()
                .unwrap_or_else(|| record.event.occurred_at.clone());
            let detail = AttentionDetail::FailedValidation {
                validation_check_id: record.payload.validation_check_id.clone(),
                check_name: record.payload.check_name.clone(),
                status: record.payload.status,
                exit_code: record.payload.exit_code,
                track_id: record.track_id.clone(),
                recorded_by: record.event.writer.actor_id.clone(),
                log_artifact_content_hashes: record.payload.log_artifact_content_hashes.clone(),
            };
            items.push(AttentionItem {
                id: format!(
                    "failed_validation:{}",
                    record.payload.validation_check_id.as_str()
                ),
                tier: tier_for(&detail),
                revision_id: Some(revision_id.clone()),
                freshness: freshness_for(supersession, &revision_id),
                observed_at,
                detail,
            });
        }
    }
    Ok(())
}

/// One `stale_assessment` item per current (un-replaced) assessment anchored to
/// a superseded revision — a decision made about state the work has moved past.
/// Suppressed once every thread head has been re-judged
/// (`thread_heads_all_assessed`): a successor still unjudged keeps the
/// moved-on-without-re-deciding nudge; a re-judged successor makes the old
/// decision noise.
/// A superseded revision that is also ambiguous therefore emits both its one
/// `ambiguous_assessment` item and one `stale_assessment` item per current record
/// (multiple simultaneous items per revision is invariant 2 working as intended).
fn stale_assessment_items(
    current_by_revision: &BTreeMap<RevisionId, Vec<AttentionAssessmentRecord>>,
    supersession: &SupersessionView,
    items: &mut Vec<AttentionItem>,
) {
    for (revision_id, peers) in current_by_revision {
        let freshness = freshness_for(supersession, revision_id);
        if freshness.state != AttentionFreshnessState::Superseded {
            continue;
        }
        if thread_heads_all_assessed(supersession, current_by_revision, revision_id) {
            continue;
        }
        for record in peers {
            let detail = AttentionDetail::StaleAssessment {
                assessment_id: record.assessment_id.clone(),
                assessment: record.assessment,
                track_id: record.track_id.clone(),
                recorded_by: record.recorded_by.clone(),
                head_revision_ids: supersession.heads_for(revision_id).into_iter().collect(),
            };
            items.push(AttentionItem {
                id: format!("stale_assessment:{}", record.assessment_id.as_str()),
                tier: tier_for(&detail),
                revision_id: Some(revision_id.clone()),
                freshness: freshness.clone(),
                observed_at: record.recorded_at.clone(),
                detail,
            });
        }
    }
}

/// One `competing_heads` item per supersession thread (connected component) with
/// two or more current heads — the fork surfaced as a peer set, never tie-broken
/// (invariant 2). Thread-scoped: `revision_id` is `None`; the id keys on the
/// lexicographically smallest component member so it is stable across head churn.
fn competing_heads_items(
    supersession: &SupersessionView,
    captured_at: &BTreeMap<RevisionId, String>,
    items: &mut Vec<AttentionItem>,
) {
    for component in &supersession.components {
        let heads: Vec<RevisionId> = component
            .intersection(&supersession.heads)
            .cloned()
            .collect();
        if heads.len() < 2 {
            continue;
        }
        let smallest = component
            .iter()
            .next()
            .expect("a connected component is non-empty");
        let observed_at = heads
            .iter()
            .filter_map(|head| captured_at.get(head).cloned())
            .max()
            .unwrap_or_default();
        let detail = AttentionDetail::CompetingHeads {
            head_revision_ids: heads,
            thread_revision_count: component.len(),
        };
        items.push(AttentionItem {
            id: format!("competing_heads:{}", smallest.as_str()),
            tier: tier_for(&detail),
            revision_id: None,
            freshness: AttentionFreshness::current(),
            observed_at,
            detail,
        });
    }
}

/// The current (un-replaced) assessment records on each revision, as attention
/// records sorted by `recorded_at` then assessment id. Shared by the ambiguous
/// (2.4), stale (2.6), and follow-up (2.8) builders so the current/replaced rule
/// is computed exactly once.
fn current_assessment_records_by_revision(
    events: &[ShoreEvent],
) -> Result<BTreeMap<RevisionId, Vec<AttentionAssessmentRecord>>> {
    let by_revision = collect_assessment_records_by_revision(events)?;
    let mut current: BTreeMap<RevisionId, Vec<AttentionAssessmentRecord>> = BTreeMap::new();
    for (revision_id, records) in by_revision {
        let replaced: BTreeSet<AssessmentId> = records
            .values()
            .flat_map(|record| record.payload.replaces_assessment_ids.iter().cloned())
            .collect();
        let mut peers: Vec<AttentionAssessmentRecord> = records
            .into_values()
            .filter(|record| !replaced.contains(&record.payload.assessment_id))
            .map(|record| AttentionAssessmentRecord {
                assessment_id: record.payload.assessment_id.clone(),
                assessment: record.payload.assessment,
                track_id: record.track_id.clone(),
                recorded_by: record.event.writer.actor_id.clone(),
                recorded_at: record.event.occurred_at.clone(),
                related_observation_ids: sorted_unique(
                    record.payload.related_observation_ids.clone(),
                ),
                related_input_request_ids: sorted_unique(
                    record.payload.related_input_request_ids.clone(),
                ),
                revision_scoped: matches!(record.payload.target, ReviewTargetRef::Revision { .. }),
            })
            .collect();
        peers.sort_by(|left, right| {
            left.recorded_at.cmp(&right.recorded_at).then_with(|| {
                left.assessment_id
                    .as_str()
                    .cmp(right.assessment_id.as_str())
            })
        });
        current.insert(revision_id, peers);
    }
    Ok(current)
}

/// One `ambiguous_assessment` item per revision whose current set has more than
/// one record — including when the values agree (each track is an independent
/// assertion). The peers are carried verbatim; no winner is chosen.
/// Freshness-independent on current heads; on a superseded revision the item is
/// suppressed once every thread head has been re-judged
/// (`thread_heads_all_assessed`).
fn ambiguous_assessment_items(
    current_by_revision: &BTreeMap<RevisionId, Vec<AttentionAssessmentRecord>>,
    supersession: &SupersessionView,
    items: &mut Vec<AttentionItem>,
) {
    for (revision_id, peers) in current_by_revision {
        if peers.len() < 2 {
            continue;
        }
        let freshness = freshness_for(supersession, revision_id);
        if freshness.state == AttentionFreshnessState::Superseded
            && thread_heads_all_assessed(supersession, current_by_revision, revision_id)
        {
            continue;
        }
        let observed_at = peers
            .iter()
            .map(|record| record.recorded_at.clone())
            .max()
            .unwrap_or_default();
        let detail = AttentionDetail::AmbiguousAssessment {
            assessments: peers.clone(),
        };
        items.push(AttentionItem {
            id: format!("ambiguous_assessment:{}", revision_id.as_str()),
            tier: tier_for(&detail),
            revision_id: Some(revision_id.clone()),
            freshness,
            observed_at,
            detail,
        });
    }
}

/// Whether a later, unanimously accepting judgment subsumes a failing
/// validation record (Rule B of the judgment-subsumption amendment to
/// ADR-0019): the current assessment set is non-empty, every current
/// assessment is accepting, and at least one revision-scoped current
/// assessment was recorded strictly later than the failure. Instants are
/// parsed (`unix-ms:` or RFC 3339) — never compared lexically, since the two
/// forms interleave in real stores; an unparseable timestamp on either side
/// can never establish strictly-later, and ties keep the card.
fn assessment_subsumes_failure(peers: &[AttentionAssessmentRecord], failure_time: &str) -> bool {
    if peers.is_empty() {
        return false;
    }
    let all_accepting = peers.iter().all(|record| {
        matches!(
            record.assessment,
            ReviewAssessment::Accepted | ReviewAssessment::AcceptedWithFollowUp
        )
    });
    if !all_accepting {
        return false;
    }
    let Some(failure_instant) = parse_event_instant(failure_time) else {
        return false;
    };
    peers.iter().any(|record| {
        record.revision_scoped
            && parse_event_instant(&record.recorded_at)
                .is_some_and(|instant| instant > failure_instant)
    })
}

/// Whether every current head of the revision's supersession thread carries at
/// least one revision-scoped current assessment — the successor set has been
/// re-judged (Rule A of the judgment-subsumption amendment to ADR-0019). Any
/// assessment value counts as re-judged; attention then flows through the
/// head's own items. Returns false for an empty head set and for any thread
/// containing a supersession cycle: never suppress under a cycle diagnostic.
fn thread_heads_all_assessed(
    supersession: &SupersessionView,
    current_by_revision: &BTreeMap<RevisionId, Vec<AttentionAssessmentRecord>>,
    revision: &RevisionId,
) -> bool {
    if supersession.component_of(revision).is_none_or(|component| {
        component
            .intersection(&supersession.cycle_revisions)
            .count()
            > 0
    }) {
        return false;
    }
    let heads = supersession.heads_for(revision);
    !heads.is_empty()
        && heads.iter().all(|head| {
            current_by_revision
                .get(head)
                .is_some_and(|peers| peers.iter().any(|record| record.revision_scoped))
        })
}

/// Supersession-derived freshness for a revision: current when the revision is a
/// head, superseded (naming every direct superseder, never a single winner) when
/// a later revision supersedes it. v1 staleness is supersession-derived only.
fn freshness_for(supersession: &SupersessionView, revision: &RevisionId) -> AttentionFreshness {
    let superseders = supersession.stale_by_superseding_revision(revision);
    if superseders.is_empty() {
        AttentionFreshness::current()
    } else {
        AttentionFreshness {
            state: AttentionFreshnessState::Superseded,
            superseded_by: superseders.into_iter().collect(),
        }
    }
}

/// One `open_input_request` item per open (un-responded) review-domain request.
/// Task-domain requests carry no subject revision and are excluded; a request
/// with any recorded response is not open.
fn open_input_request_items(
    records: &InputRequestProjectionRecords<'_>,
    supersession: &SupersessionView,
    items: &mut Vec<AttentionItem>,
) -> Result<()> {
    for (input_request_id, record) in &records.request_records {
        if records.responses.contains_key(input_request_id) {
            continue;
        }
        let Some(revision_id) = record.event.subject_revision_id()? else {
            continue;
        };
        let freshness = freshness_for(supersession, &revision_id);
        let detail = AttentionDetail::OpenInputRequest {
            input_request_id: input_request_id.clone(),
            mode: record.event.assertion_mode,
            reason_code: record.payload.reason_code,
            title: record.payload.title.clone(),
            track_id: record.track_id.clone(),
            opened_by: record.event.writer.actor_id.clone(),
        };
        items.push(AttentionItem {
            id: format!("open_input_request:{}", input_request_id.as_str()),
            tier: tier_for(&detail),
            revision_id: Some(revision_id),
            freshness,
            observed_at: record.event.occurred_at.clone(),
            detail,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        EngagementId, InputRequestId, InputRequestResponseId, JournalId, ObjectId, ReviewEndpoint,
        ReviewTargetRef, RevisionSource, TargetRef, TaskTargetRef, ValidationTrigger, WorkObjectId,
        WorktreeCaptureMode,
    };
    use crate::session::event::{
        EventTarget, EventType, GitProvenance, InputRequestOpenedPayload,
        InputRequestRespondedPayload, InputRequestResponseOutcome, ReviewAssessmentRecordedPayload,
        Revision, WorkObjectProposal, WorkObjectProposedPayload, Writer, WriterProducer,
    };

    #[allow(clippy::too_many_arguments)]
    fn validation_event(
        revision: &RevisionId,
        track: &str,
        actor: &str,
        check_id: &str,
        check_name: &str,
        status: ValidationStatus,
        exit_code: Option<i64>,
        completed_at: Option<&str>,
        occurred_at: &str,
        logs: Vec<&str>,
    ) -> ShoreEvent {
        let revision = revision.clone();
        let track_id = TrackId::new(track.to_owned());
        let payload = ValidationCheckRecordedPayload {
            validation_check_id: ValidationCheckId::new(format!("validation:sha256:{check_id}")),
            target: ValidationTarget::Revision {
                revision_id: revision.clone(),
            },
            check_name: check_name.to_owned(),
            command: None,
            status,
            exit_code,
            trigger: ValidationTrigger::Manual,
            source_fingerprint: None,
            summary: None,
            summary_content_type: Default::default(),
            summary_artifact_path: None,
            summary_byte_size: None,
            summary_content_hash: None,
            started_at: None,
            completed_at: completed_at.map(str::to_owned),
            log_artifact_content_hashes: logs.iter().map(|hash| (*hash).to_owned()).collect(),
        };
        let idempotency_key =
            ValidationCheckRecordedPayload::idempotency_key(&revision, &track_id, check_id);
        ShoreEvent::new(
            EventType::ValidationCheckRecorded,
            idempotency_key,
            EventTarget::for_revision(JournalId::new("journal:default"), revision, Some(track_id))
                .unwrap(),
            writer(actor),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn assessment_event(
        revision: &RevisionId,
        track: &str,
        actor: &str,
        assess_id: &str,
        assessment: ReviewAssessment,
        replaces: Vec<&str>,
        related_input_requests: Vec<&str>,
        occurred_at: &str,
    ) -> ShoreEvent {
        assessment_event_with_target(
            revision,
            track,
            actor,
            assess_id,
            assessment,
            replaces,
            related_input_requests,
            occurred_at,
            ReviewTargetRef::Revision {
                revision_id: revision.clone(),
            },
        )
    }

    /// An assessment with an explicit payload target. A file/range/observation
    /// target still groups under its subject revision, but must never act as a
    /// revision-scoped judgment (invariant 3).
    #[allow(clippy::too_many_arguments)]
    fn assessment_event_with_target(
        revision: &RevisionId,
        track: &str,
        actor: &str,
        assess_id: &str,
        assessment: ReviewAssessment,
        replaces: Vec<&str>,
        related_input_requests: Vec<&str>,
        occurred_at: &str,
        target: ReviewTargetRef,
    ) -> ShoreEvent {
        let revision = revision.clone();
        let track_id = TrackId::new(track.to_owned());
        let assessment_id = AssessmentId::new(format!("assess:sha256:{assess_id}"));
        let payload = ReviewAssessmentRecordedPayload {
            assessment_id,
            target,
            assessment,
            summary: None,
            summary_content_type: Default::default(),
            summary_artifact_path: None,
            summary_byte_size: None,
            summary_content_hash: None,
            replaces_assessment_ids: replaces
                .iter()
                .map(|id| AssessmentId::new(format!("assess:sha256:{id}")))
                .collect(),
            related_observation_ids: Vec::new(),
            related_input_request_ids: related_input_requests
                .iter()
                .map(|id| InputRequestId::new(format!("input-request:sha256:{id}")))
                .collect(),
        };
        let idempotency_key =
            ReviewAssessmentRecordedPayload::idempotency_key(&revision, &track_id, assess_id);
        ShoreEvent::new(
            EventType::ReviewAssessmentRecorded,
            idempotency_key,
            EventTarget::for_revision(JournalId::new("journal:default"), revision, Some(track_id))
                .unwrap(),
            writer(actor),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    fn rev(suffix: &str) -> RevisionId {
        RevisionId::new(format!("rev:sha256:{suffix}"))
    }

    fn writer(actor: &str) -> Writer {
        Writer {
            actor_id: ActorId::new(actor.to_owned()),
            producer: WriterProducer {
                name: "shore".to_owned(),
                version: String::new(),
            },
        }
    }

    /// A review-domain revision proposal (`WorkObjectProposed`), optionally
    /// superseding earlier revisions. Mirrors the supersession suite's fixture.
    fn revision_event(suffix: &str, supersedes: Vec<RevisionId>, occurred_at: &str) -> ShoreEvent {
        let revision_id = rev(suffix);
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{}", revision_id.as_str()),
            EventTarget::for_revision(JournalId::new("journal:default"), revision_id.clone(), None)
                .unwrap(),
            writer("actor:human:kevin"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new(format!("engagement:sha256:{suffix}")),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: revision_id,
                        object_id: ObjectId::new(format!("obj:sha256:{suffix}")),
                        git_provenance: Some(GitProvenance {
                            source: RevisionSource::GitWorktree {
                                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                                include_untracked: true,
                                pathspecs: Vec::new(),
                            },
                            base: ReviewEndpoint::GitCommit {
                                commit_oid: "base".to_owned(),
                                tree_oid: "base-tree".to_owned(),
                            },
                            target: ReviewEndpoint::GitWorkingTree {
                                worktree_root: "/repo".to_owned(),
                            },
                        }),
                    },
                    object_artifact_content_hash: format!("sha256:artifact:{suffix}"),
                    supersedes,
                },
            },
            occurred_at,
        )
        .unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn open_request_event(
        revision: &RevisionId,
        track: &str,
        actor: &str,
        request_id: &str,
        reason: InputRequestReasonCode,
        title: &str,
        mode: AssertionMode,
        occurred_at: &str,
    ) -> ShoreEvent {
        let revision = revision.clone();
        let track_id = TrackId::new(track.to_owned());
        let input_request_id = InputRequestId::new(format!("input-request:sha256:{request_id}"));
        let payload = InputRequestOpenedPayload {
            input_request_id: input_request_id.clone(),
            target: ReviewTargetRef::Revision {
                revision_id: revision.clone(),
            },
            task_target: None,
            reason_code: reason,
            title: title.to_owned(),
            body: None,
            body_content_type: Default::default(),
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            target_fingerprint: None,
        };
        let idempotency_key =
            InputRequestOpenedPayload::idempotency_key(&revision, &track_id, request_id);
        let mut event = ShoreEvent::new(
            EventType::InputRequestOpened,
            idempotency_key,
            EventTarget::for_revision(JournalId::new("journal:default"), revision, Some(track_id))
                .unwrap(),
            writer(actor),
            payload,
            occurred_at,
        )
        .unwrap();
        event.assertion_mode = mode;
        event
    }

    fn task_request_event(request_id: &str, occurred_at: &str) -> ShoreEvent {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let input_request_id = InputRequestId::new(format!("input-request:sha256:{request_id}"));
        let payload = InputRequestOpenedPayload {
            input_request_id: input_request_id.clone(),
            target: ReviewTargetRef::Revision {
                revision_id: RevisionId::new("review-unit:placeholder"),
            },
            task_target: Some(TaskTargetRef::TaskAttempt {
                task_attempt_id: task_attempt_id.clone(),
            }),
            reason_code: InputRequestReasonCode::ManualDecisionRequired,
            title: "task ask".to_owned(),
            body: None,
            body_content_type: Default::default(),
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            target_fingerprint: None,
        };
        let idempotency_key = InputRequestOpenedPayload::idempotency_key_for_work_object(
            &task_attempt_id,
            crate::model::WorkObjectType::TaskAttempt,
            request_id,
        );
        let mut event = ShoreEvent::new(
            EventType::InputRequestOpened,
            idempotency_key,
            EventTarget::for_subject(
                JournalId::new("journal:default"),
                TargetRef::Task(TaskTargetRef::TaskAttempt { task_attempt_id }),
                Some(TrackId::new("agent:codex")),
            )
            .unwrap(),
            writer("actor:agent:codex"),
            payload,
            occurred_at,
        )
        .unwrap();
        event.assertion_mode = AssertionMode::Operative;
        event
    }

    fn response_event(request_id: &str, revision: &RevisionId, occurred_at: &str) -> ShoreEvent {
        let input_request_id = InputRequestId::new(format!("input-request:sha256:{request_id}"));
        let response_id =
            InputRequestResponseId::new(format!("input-request-response:sha256:{request_id}"));
        let payload = InputRequestRespondedPayload {
            input_request_response_id: response_id.clone(),
            input_request_id: input_request_id.clone(),
            revision_id: Some(revision.clone()),
            task_target: None,
            outcome: InputRequestResponseOutcome::Approved,
            reason: None,
            reason_content_type: Default::default(),
            reason_artifact_path: None,
            reason_byte_size: None,
            reason_content_hash: None,
            target_fingerprint: None,
        };
        let idempotency_key =
            InputRequestRespondedPayload::idempotency_key(&input_request_id, response_id.as_str());
        ShoreEvent::new(
            EventType::InputRequestResponded,
            idempotency_key,
            EventTarget::for_revision(JournalId::new("journal:default"), revision.clone(), None)
                .unwrap(),
            writer("actor:human:kevin"),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    #[test]
    fn attention_from_events_on_empty_log_is_empty() {
        let projection = attention_from_events(&[], None).expect("empty log projects");
        assert!(projection.items.is_empty());
        assert!(projection.diagnostics.is_empty());
    }

    #[test]
    fn open_requests_become_items_and_responded_requests_do_not() {
        let a = rev("a");
        let events = vec![
            open_request_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "op",
                InputRequestReasonCode::ManualDecisionRequired,
                "Operative gate",
                AssertionMode::Operative,
                "2026-06-04T00:00:00Z",
            ),
            open_request_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "adv",
                InputRequestReasonCode::InsufficientEvidence,
                "Advisory question",
                AssertionMode::Advisory,
                "2026-06-04T00:00:01Z",
            ),
            open_request_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "done",
                InputRequestReasonCode::ManualDecisionRequired,
                "Answered",
                AssertionMode::Operative,
                "2026-06-04T00:00:02Z",
            ),
            response_event("done", &a, "2026-06-04T00:00:03Z"),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        assert_eq!(projection.items.len(), 2);

        let operative = projection
            .items
            .iter()
            .find(|item| item.id == "open_input_request:input-request:sha256:op")
            .expect("operative item");
        assert_eq!(operative.tier, AttentionTier::Primary);
        assert_eq!(operative.revision_id, Some(a.clone()));
        assert_eq!(operative.observed_at, "2026-06-04T00:00:00Z");
        match &operative.detail {
            AttentionDetail::OpenInputRequest {
                input_request_id,
                mode,
                reason_code,
                title,
                track_id,
                opened_by,
            } => {
                assert_eq!(input_request_id.as_str(), "input-request:sha256:op");
                assert_eq!(*mode, AssertionMode::Operative);
                assert_eq!(*reason_code, InputRequestReasonCode::ManualDecisionRequired);
                assert_eq!(title, "Operative gate");
                assert_eq!(track_id.as_str(), "human:kevin");
                assert_eq!(opened_by.as_str(), "actor:human:kevin");
            }
            #[allow(unreachable_patterns)]
            _ => panic!("expected open_input_request detail"),
        }

        let advisory = projection
            .items
            .iter()
            .find(|item| item.id == "open_input_request:input-request:sha256:adv")
            .expect("advisory item");
        assert_eq!(advisory.tier, AttentionTier::Secondary);

        assert!(
            !projection
                .items
                .iter()
                .any(|item| item.id.contains(":done")),
            "responded request must not appear"
        );
    }

    #[test]
    fn task_domain_requests_are_excluded() {
        let a = rev("a");
        let events = vec![
            open_request_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "review",
                InputRequestReasonCode::ManualDecisionRequired,
                "Review ask",
                AssertionMode::Operative,
                "2026-06-04T00:00:00Z",
            ),
            task_request_event("task", "2026-06-04T00:00:01Z"),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        assert_eq!(projection.items.len(), 1);
        assert_eq!(projection.items[0].revision_id, Some(a));
    }

    #[test]
    fn request_on_superseded_revision_reports_superseding_successors() {
        let a = rev("a");
        let b = rev("b");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            revision_event("b", vec![a.clone()], "2026-06-04T00:00:01Z"),
            open_request_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "on-a",
                InputRequestReasonCode::ManualDecisionRequired,
                "Request on A",
                AssertionMode::Operative,
                "2026-06-04T00:00:02Z",
            ),
            open_request_event(
                &b,
                "human:kevin",
                "actor:human:kevin",
                "on-b",
                InputRequestReasonCode::ManualDecisionRequired,
                "Request on B",
                AssertionMode::Operative,
                "2026-06-04T00:00:03Z",
            ),
        ];

        let projection = attention_from_events(&events, None).expect("projects");

        let on_a = projection
            .items
            .iter()
            .find(|item| item.id.contains(":on-a"))
            .expect("item on A");
        assert_eq!(on_a.freshness.state, AttentionFreshnessState::Superseded);
        assert_eq!(on_a.freshness.superseded_by, vec![b.clone()]);

        let on_b = projection
            .items
            .iter()
            .find(|item| item.id.contains(":on-b"))
            .expect("item on B");
        assert_eq!(on_b.freshness.state, AttentionFreshnessState::Current);
        assert!(on_b.freshness.superseded_by.is_empty());
    }

    #[test]
    fn two_unreplaced_assessments_are_ambiguous_even_when_values_agree() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            assessment_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "one",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:01Z",
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "two",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:02Z",
            ),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        let ambiguous: Vec<&AttentionItem> = projection
            .items
            .iter()
            .filter(|item| matches!(item.detail, AttentionDetail::AmbiguousAssessment { .. }))
            .collect();
        assert_eq!(ambiguous.len(), 1);
        let item = ambiguous[0];
        assert_eq!(item.id, format!("ambiguous_assessment:{}", a.as_str()));
        assert_eq!(item.observed_at, "2026-06-04T00:00:02Z");
        match &item.detail {
            AttentionDetail::AmbiguousAssessment { assessments } => {
                assert_eq!(assessments.len(), 2);
                let tracks: Vec<&str> = assessments.iter().map(|r| r.track_id.as_str()).collect();
                assert!(tracks.contains(&"human:kevin"));
                assert!(tracks.contains(&"agent:codex"));
            }
            _ => panic!("expected ambiguous_assessment"),
        }
    }

    #[test]
    fn replaced_assessments_do_not_count_toward_ambiguity() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            assessment_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "x",
                ReviewAssessment::NeedsChanges,
                vec![],
                vec![],
                "2026-06-04T00:00:01Z",
            ),
            assessment_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "y",
                ReviewAssessment::Accepted,
                vec!["x"],
                vec![],
                "2026-06-04T00:00:02Z",
            ),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            !projection
                .items
                .iter()
                .any(|item| matches!(item.detail, AttentionDetail::AmbiguousAssessment { .. })),
            "one current record is not ambiguous"
        );
    }

    #[test]
    fn forked_thread_emits_one_competing_heads_item() {
        let a = rev("a");
        let b = rev("b");
        let c = rev("c");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            revision_event("b", vec![a.clone()], "2026-06-04T00:00:01Z"),
            revision_event("c", vec![a.clone()], "2026-06-04T00:00:02Z"),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        let competing: Vec<&AttentionItem> = projection
            .items
            .iter()
            .filter(|item| matches!(item.detail, AttentionDetail::CompetingHeads { .. }))
            .collect();
        assert_eq!(competing.len(), 1);
        let item = competing[0];
        assert_eq!(item.id, format!("competing_heads:{}", a.as_str()));
        assert_eq!(item.revision_id, None);
        assert_eq!(item.observed_at, "2026-06-04T00:00:02Z");
        match &item.detail {
            AttentionDetail::CompetingHeads {
                head_revision_ids,
                thread_revision_count,
            } => {
                assert_eq!(*head_revision_ids, vec![b.clone(), c.clone()]);
                assert_eq!(*thread_revision_count, 3);
            }
            _ => panic!("expected competing_heads"),
        }
    }

    #[test]
    fn linear_chains_and_isolated_revisions_emit_nothing() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            revision_event("b", vec![a.clone()], "2026-06-04T00:00:01Z"),
            revision_event("d", vec![], "2026-06-04T00:00:02Z"),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            !projection
                .items
                .iter()
                .any(|item| matches!(item.detail, AttentionDetail::CompetingHeads { .. })),
            "a linear chain and an isolated revision have no competing heads"
        );
    }

    #[test]
    fn supersession_diagnostics_pass_through_verbatim() {
        let a = rev("a");
        let b = rev("b");
        let events = vec![
            revision_event("a", vec![b.clone()], "2026-06-04T00:00:00Z"),
            revision_event("b", vec![a.clone()], "2026-06-04T00:00:01Z"),
            revision_event("c", vec![rev("x")], "2026-06-04T00:00:02Z"),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "supersession_cycle"),
            "cycle diagnostic must pass through"
        );
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "supersession_target_missing"),
            "dangling target diagnostic must pass through"
        );
    }

    #[test]
    fn current_assessment_on_superseded_revision_is_stale() {
        let a = rev("a");
        let b = rev("b");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            revision_event("b", vec![a.clone()], "2026-06-04T00:00:01Z"),
            assessment_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:02Z",
            ),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        let stale: Vec<&AttentionItem> = projection
            .items
            .iter()
            .filter(|item| matches!(item.detail, AttentionDetail::StaleAssessment { .. }))
            .collect();
        assert_eq!(stale.len(), 1);
        let item = stale[0];
        assert_eq!(item.id, "stale_assessment:assess:sha256:s1");
        assert_eq!(item.freshness.state, AttentionFreshnessState::Superseded);
        assert_eq!(item.freshness.superseded_by, vec![b.clone()]);
        assert_eq!(item.observed_at, "2026-06-04T00:00:02Z");
        match &item.detail {
            AttentionDetail::StaleAssessment {
                assessment_id,
                assessment,
                track_id,
                recorded_by,
                head_revision_ids,
            } => {
                assert_eq!(assessment_id.as_str(), "assess:sha256:s1");
                assert_eq!(*assessment, ReviewAssessment::Accepted);
                assert_eq!(track_id.as_str(), "human:kevin");
                assert_eq!(recorded_by.as_str(), "actor:human:kevin");
                assert_eq!(head_revision_ids.as_slice(), std::slice::from_ref(&b));
            }
            _ => panic!("expected stale_assessment"),
        }
    }

    #[test]
    fn stale_assessment_names_current_heads_beyond_converged_superseders() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            revision_event("b", vec![a.clone()], "2026-06-04T00:00:01Z"),
            revision_event("c", vec![a.clone()], "2026-06-04T00:00:02Z"),
            revision_event("d", vec![rev("b"), rev("c")], "2026-06-04T00:00:03Z"),
            assessment_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:04Z",
            ),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        let stale = projection
            .items
            .iter()
            .find(|item| matches!(item.detail, AttentionDetail::StaleAssessment { .. }))
            .expect("stale assessment");
        assert_eq!(stale.freshness.superseded_by, vec![rev("b"), rev("c")]);
        match &stale.detail {
            AttentionDetail::StaleAssessment {
                head_revision_ids, ..
            } => assert_eq!(head_revision_ids.as_slice(), [rev("d")]),
            _ => unreachable!(),
        }
        let wire = serde_json::to_value(stale).expect("serializes");
        assert_eq!(wire["headRevisionIds"], serde_json::json!([rev("d")]));
        assert_eq!(
            wire["freshness"]["supersededBy"],
            serde_json::json!([rev("b"), rev("c")])
        );
    }

    #[test]
    fn replaced_or_head_anchored_assessments_are_not_stale_items() {
        let a = rev("a");
        let head = rev("head");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            revision_event("b", vec![a.clone()], "2026-06-04T00:00:01Z"),
            // (a) a superseded-revision assessment later replaced -> not stale.
            assessment_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "old",
                ReviewAssessment::NeedsChanges,
                vec![],
                vec![],
                "2026-06-04T00:00:02Z",
            ),
            assessment_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "new",
                ReviewAssessment::Accepted,
                vec!["old"],
                vec![],
                "2026-06-04T00:00:03Z",
            ),
            // (b) a current assessment on a head -> not stale.
            revision_event("head", vec![], "2026-06-04T00:00:04Z"),
            assessment_event(
                &head,
                "human:kevin",
                "actor:human:kevin",
                "onhead",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:05Z",
            ),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        let stale_ids: Vec<&str> = projection
            .items
            .iter()
            .filter_map(|item| match &item.detail {
                AttentionDetail::StaleAssessment { assessment_id, .. } => {
                    Some(assessment_id.as_str())
                }
                _ => None,
            })
            .collect();
        // Only the un-replaced record on superseded A ("new") is stale.
        assert_eq!(stale_ids, vec!["assess:sha256:new"]);
    }

    #[test]
    fn ambiguous_and_stale_compose_on_a_superseded_revision() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            revision_event("b", vec![a.clone()], "2026-06-04T00:00:01Z"),
            assessment_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "one",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:02Z",
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "two",
                ReviewAssessment::NeedsChanges,
                vec![],
                vec![],
                "2026-06-04T00:00:03Z",
            ),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        let ambiguous = projection
            .items
            .iter()
            .filter(|item| matches!(item.detail, AttentionDetail::AmbiguousAssessment { .. }))
            .count();
        let stale = projection
            .items
            .iter()
            .filter(|item| matches!(item.detail, AttentionDetail::StaleAssessment { .. }))
            .count();
        assert_eq!(ambiguous, 1, "one ambiguity item for the peer set");
        assert_eq!(
            stale, 2,
            "one stale item per current record on the superseded revision"
        );
    }

    #[test]
    fn latest_failed_check_on_a_head_is_an_item() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "fail",
                "cargo test",
                ValidationStatus::Failed,
                Some(101),
                Some("2026-06-04T00:01:00Z"),
                "2026-06-04T00:01:00Z",
                vec!["sha256:log1"],
            ),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "err",
                "cargo build",
                ValidationStatus::Errored,
                None,
                Some("2026-06-04T00:02:00Z"),
                "2026-06-04T00:02:00Z",
                vec![],
            ),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        let failed: Vec<&AttentionItem> = projection
            .items
            .iter()
            .filter(|item| matches!(item.detail, AttentionDetail::FailedValidation { .. }))
            .collect();
        assert_eq!(failed.len(), 2);

        let test_item = failed
            .iter()
            .find(|item| item.id == "failed_validation:validation:sha256:fail")
            .expect("failed cargo test item");
        assert_eq!(test_item.observed_at, "2026-06-04T00:01:00Z");
        match &test_item.detail {
            AttentionDetail::FailedValidation {
                check_name,
                status,
                exit_code,
                track_id,
                recorded_by,
                log_artifact_content_hashes,
                ..
            } => {
                assert_eq!(check_name, "cargo test");
                assert_eq!(*status, ValidationStatus::Failed);
                assert_eq!(*exit_code, Some(101));
                assert_eq!(track_id.as_str(), "agent:codex");
                assert_eq!(recorded_by.as_str(), "actor:agent:codex");
                assert_eq!(log_artifact_content_hashes, &vec!["sha256:log1".to_owned()]);
            }
            _ => panic!("expected failed_validation"),
        }
    }

    #[test]
    fn passing_rerun_clears_and_superseded_revisions_do_not_report() {
        let a = rev("a");
        let head = rev("head");
        let events = vec![
            revision_event("head", vec![], "2026-06-04T00:00:00Z"),
            revision_event("a", vec![], "2026-06-04T00:00:01Z"),
            revision_event("b", vec![a.clone()], "2026-06-04T00:00:02Z"),
            // (a) failed then a later passed for the same (revision, track, check_name).
            validation_event(
                &head,
                "agent:codex",
                "actor:agent:codex",
                "old",
                "cargo test",
                ValidationStatus::Failed,
                Some(101),
                Some("2026-06-04T00:01:00Z"),
                "2026-06-04T00:01:00Z",
                vec![],
            ),
            validation_event(
                &head,
                "agent:codex",
                "actor:agent:codex",
                "new",
                "cargo test",
                ValidationStatus::Passed,
                Some(0),
                Some("2026-06-04T00:02:00Z"),
                "2026-06-04T00:02:00Z",
                vec![],
            ),
            // (b) failed on a superseded revision.
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "stale",
                "cargo clippy",
                ValidationStatus::Failed,
                Some(1),
                Some("2026-06-04T00:03:00Z"),
                "2026-06-04T00:03:00Z",
                vec![],
            ),
            // (c) skipped status on the head.
            validation_event(
                &head,
                "agent:codex",
                "actor:agent:codex",
                "skip",
                "cargo fmt",
                ValidationStatus::Skipped,
                None,
                Some("2026-06-04T00:04:00Z"),
                "2026-06-04T00:04:00Z",
                vec![],
            ),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            !projection
                .items
                .iter()
                .any(|item| matches!(item.detail, AttentionDetail::FailedValidation { .. })),
            "passing rerun clears, superseded does not report, skipped never reports"
        );
    }

    #[test]
    fn equal_time_pass_never_hides_a_simultaneous_fail() {
        let a = rev("a");
        // A pass and a fail for the same (revision, track, check_name) at the SAME
        // completed_at: the pass must not clear the fail (no strictly-later result),
        // and the outcome must not depend on event-id hash order.
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "passing",
                "cargo test",
                ValidationStatus::Passed,
                Some(0),
                Some("2026-06-04T00:01:00Z"),
                "2026-06-04T00:01:00Z",
                vec![],
            ),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "failing",
                "cargo test",
                ValidationStatus::Failed,
                Some(101),
                Some("2026-06-04T00:01:00Z"),
                "2026-06-04T00:01:00Z",
                vec![],
            ),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        let failed: Vec<&str> = projection
            .items
            .iter()
            .filter_map(|item| match &item.detail {
                AttentionDetail::FailedValidation {
                    validation_check_id,
                    ..
                } => Some(validation_check_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(failed, vec!["validation:sha256:failing"]);
    }

    #[test]
    fn a_strictly_later_pass_clears_the_fail() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "failing",
                "cargo test",
                ValidationStatus::Failed,
                Some(101),
                Some("2026-06-04T00:01:00Z"),
                "2026-06-04T00:01:00Z",
                vec![],
            ),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "passing",
                "cargo test",
                ValidationStatus::Passed,
                Some(0),
                Some("2026-06-04T00:02:00Z"),
                "2026-06-04T00:02:00Z",
                vec![],
            ),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            !projection
                .items
                .iter()
                .any(|item| matches!(item.detail, AttentionDetail::FailedValidation { .. })),
            "a strictly-later pass clears the card"
        );
    }

    #[test]
    fn accepted_with_follow_up_and_open_linked_request_is_outstanding() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            open_request_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "fu",
                InputRequestReasonCode::ManualDecisionRequired,
                "Follow up needed",
                AssertionMode::Operative,
                "2026-06-04T00:00:01Z",
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "acc",
                ReviewAssessment::AcceptedWithFollowUp,
                vec![],
                vec!["fu"],
                "2026-06-04T00:00:02Z",
            ),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        let follow_ups: Vec<&AttentionItem> = projection
            .items
            .iter()
            .filter(|item| matches!(item.detail, AttentionDetail::FollowUpOutstanding { .. }))
            .collect();
        assert_eq!(follow_ups.len(), 1);
        let item = follow_ups[0];
        assert_eq!(item.id, "follow_up_outstanding:assess:sha256:acc");
        assert_eq!(item.observed_at, "2026-06-04T00:00:02Z");
        match &item.detail {
            AttentionDetail::FollowUpOutstanding {
                assessment_id,
                track_id,
                recorded_by,
                open_input_request_ids,
            } => {
                assert_eq!(assessment_id.as_str(), "assess:sha256:acc");
                assert_eq!(track_id.as_str(), "agent:codex");
                assert_eq!(recorded_by.as_str(), "actor:agent:codex");
                assert_eq!(
                    open_input_request_ids
                        .iter()
                        .map(|id| id.as_str())
                        .collect::<Vec<_>>(),
                    vec!["input-request:sha256:fu"]
                );
            }
            _ => panic!("expected follow_up_outstanding"),
        }

        // The linked open request also lists as its own item — two facts, two cards.
        assert!(
            projection
                .items
                .iter()
                .any(|item| item.id == "open_input_request:input-request:sha256:fu"),
            "the linked open request is a distinct open_input_request card"
        );
    }

    #[test]
    fn responded_links_and_unlinked_follow_ups_emit_nothing() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            // (a) related request that has been responded.
            open_request_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "resp",
                InputRequestReasonCode::ManualDecisionRequired,
                "Answered ask",
                AssertionMode::Operative,
                "2026-06-04T00:00:01Z",
            ),
            response_event("resp", &a, "2026-06-04T00:00:02Z"),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "linked",
                ReviewAssessment::AcceptedWithFollowUp,
                vec![],
                vec!["resp"],
                "2026-06-04T00:00:03Z",
            ),
            // (b) accepted_with_follow_up with zero related requests.
            assessment_event(
                &rev("b"),
                "agent:codex",
                "actor:agent:codex",
                "unlinked",
                ReviewAssessment::AcceptedWithFollowUp,
                vec![],
                vec![],
                "2026-06-04T00:00:04Z",
            ),
            revision_event("b", vec![], "2026-06-04T00:00:05Z"),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            !projection
                .items
                .iter()
                .any(|item| matches!(item.detail, AttentionDetail::FollowUpOutstanding { .. })),
            "responded links and unlinked follow-ups emit nothing"
        );
    }

    #[test]
    fn items_sort_mixed_timestamps_by_tier_then_instant_then_id() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            // advisory (secondary), newest observed_at.
            open_request_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "adv",
                InputRequestReasonCode::ManualDecisionRequired,
                "Advisory",
                AssertionMode::Advisory,
                "2026-06-04T00:05:00Z",
            ),
            // operative (primary), middle observed_at.
            open_request_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "op2",
                InputRequestReasonCode::ManualDecisionRequired,
                "Operative later",
                AssertionMode::Operative,
                "2026-06-04T00:03:00Z",
            ),
            // operative (primary), oldest observed_at in the other legal form.
            open_request_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "op1",
                InputRequestReasonCode::ManualDecisionRequired,
                "Operative earliest",
                AssertionMode::Operative,
                "unix-ms:0",
            ),
            // operative (primary), malformed and deterministically before legal instants.
            open_request_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "malformed",
                InputRequestReasonCode::ManualDecisionRequired,
                "Operative malformed",
                AssertionMode::Operative,
                "malformed",
            ),
        ];

        let projection = attention_from_events(&events, None).expect("projects");
        let order: Vec<&str> = projection
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert_eq!(
            order,
            vec![
                "open_input_request:input-request:sha256:malformed",
                "open_input_request:input-request:sha256:op1",
                "open_input_request:input-request:sha256:op2",
                "open_input_request:input-request:sha256:adv",
            ],
            "primary before secondary; within tier oldest observed_at first"
        );
    }

    #[test]
    fn revision_scoping_keeps_anchored_items_plus_the_covering_thread() {
        let a = rev("a");
        let b = rev("b");
        let c = rev("c");
        let unrelated = rev("unrelated");
        let events = vec![
            // Forked thread A <- {B, C}.
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            revision_event("b", vec![a.clone()], "2026-06-04T00:00:01Z"),
            revision_event("c", vec![a.clone()], "2026-06-04T00:00:02Z"),
            // Unrelated isolated revision with its own open request.
            revision_event("unrelated", vec![], "2026-06-04T00:00:03Z"),
            open_request_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "on-a",
                InputRequestReasonCode::ManualDecisionRequired,
                "On A",
                AssertionMode::Operative,
                "2026-06-04T00:00:04Z",
            ),
            open_request_event(
                &unrelated,
                "human:kevin",
                "actor:human:kevin",
                "on-unrelated",
                InputRequestReasonCode::ManualDecisionRequired,
                "On unrelated",
                AssertionMode::Operative,
                "2026-06-04T00:00:05Z",
            ),
        ];

        let projection = attention_from_events(&events, Some(&a)).expect("projects");
        let ids: Vec<&str> = projection
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect();

        // The request anchored to A stays.
        assert!(ids.contains(&"open_input_request:input-request:sha256:on-a"));
        // The competing-heads item whose component contains A stays (revision_id None).
        assert!(ids.contains(&format!("competing_heads:{}", a.as_str()).as_str()));
        // The unrelated revision's request drops.
        assert!(!ids.contains(&"open_input_request:input-request:sha256:on-unrelated"));
        // Sanity: B and C are the heads of the covered thread.
        let competing = projection
            .items
            .iter()
            .find_map(|item| match &item.detail {
                AttentionDetail::CompetingHeads {
                    head_revision_ids, ..
                } => Some(head_revision_ids.clone()),
                _ => None,
            })
            .expect("competing heads item");
        assert_eq!(competing, vec![b, c]);
    }

    #[test]
    fn projection_is_deterministic() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            revision_event("b", vec![a.clone()], "2026-06-04T00:00:01Z"),
            revision_event("c", vec![a.clone()], "2026-06-04T00:00:02Z"),
            open_request_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "op",
                InputRequestReasonCode::ManualDecisionRequired,
                "Operative",
                AssertionMode::Operative,
                "2026-06-04T00:00:03Z",
            ),
            assessment_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "one",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:04Z",
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "two",
                ReviewAssessment::NeedsChanges,
                vec![],
                vec![],
                "2026-06-04T00:00:05Z",
            ),
        ];

        let first = attention_from_events(&events, None).expect("projects");
        let second = attention_from_events(&events, None).expect("projects");
        assert_eq!(
            serde_json::to_string(&first.items).unwrap(),
            serde_json::to_string(&second.items).unwrap()
        );
    }

    #[test]
    fn attention_item_wire_shape_is_pinned() {
        let a = rev("a");
        let event = open_request_event(
            &a,
            "human:kevin",
            "actor:human:kevin",
            "req1",
            InputRequestReasonCode::ManualDecisionRequired,
            "Need a decision",
            AssertionMode::Operative,
            "2026-06-04T00:00:00Z",
        );
        let projection = attention_from_events(&[event], None).expect("projects");
        let item = &projection.items[0];

        let expected = serde_json::json!({
            "id": "open_input_request:input-request:sha256:req1",
            "kind": "open_input_request",
            "tier": "primary",
            "revisionId": "rev:sha256:a",
            "freshness": { "state": "current" },
            "observedAt": "2026-06-04T00:00:00Z",
            "inputRequestId": "input-request:sha256:req1",
            "mode": "operative",
            "reasonCode": "manual_decision_required",
            "title": "Need a decision",
            "trackId": "human:kevin",
            "openedBy": "actor:human:kevin",
        });
        assert_eq!(serde_json::to_value(item).unwrap(), expected);
    }

    /// True when no `failed_validation` item survives in the projection.
    fn no_failed_validation_items(projection: &AttentionProjection) -> bool {
        projection
            .items
            .iter()
            .all(|item| !item.id.starts_with("failed_validation:"))
    }

    fn has_failed_validation_item(projection: &AttentionProjection, check_id: &str) -> bool {
        projection
            .items
            .iter()
            .any(|item| item.id == format!("failed_validation:validation:sha256:{check_id}"))
    }

    #[test]
    fn accepting_assessment_after_failure_subsumes_the_card() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v1",
                "red proof",
                ValidationStatus::Failed,
                Some(1),
                None,
                "2026-06-04T00:00:01Z",
                vec![],
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:02Z",
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            no_failed_validation_items(&projection),
            "an accepting judgment recorded after the failure clears the card",
        );
    }

    #[test]
    fn accepted_with_follow_up_subsumes_an_errored_record_too() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v1",
                "flaky probe",
                ValidationStatus::Errored,
                None,
                None,
                "2026-06-04T00:00:01Z",
                vec![],
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::AcceptedWithFollowUp,
                vec![],
                vec![],
                "2026-06-04T00:00:02Z",
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            no_failed_validation_items(&projection),
            "accepted_with_follow_up is accepting and subsumes an errored record",
        );
    }

    #[test]
    fn needs_changes_after_failure_keeps_the_card() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v1",
                "gate",
                ValidationStatus::Failed,
                Some(1),
                None,
                "2026-06-04T00:00:01Z",
                vec![],
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::NeedsChanges,
                vec![],
                vec![],
                "2026-06-04T00:00:02Z",
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(has_failed_validation_item(&projection, "v1"));
    }

    #[test]
    fn needs_clarification_after_failure_keeps_the_card() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v1",
                "gate",
                ValidationStatus::Failed,
                Some(1),
                None,
                "2026-06-04T00:00:01Z",
                vec![],
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::NeedsClarification,
                vec![],
                vec![],
                "2026-06-04T00:00:02Z",
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(has_failed_validation_item(&projection, "v1"));
    }

    #[test]
    fn assessment_recorded_before_the_failure_keeps_the_card() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:01Z",
            ),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v1",
                "post-acceptance gate",
                ValidationStatus::Failed,
                Some(1),
                None,
                "2026-06-04T00:00:02Z",
                vec![],
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            has_failed_validation_item(&projection, "v1"),
            "a failure recorded after acceptance re-raises attention",
        );
    }

    #[test]
    fn assessment_tied_with_the_failure_keeps_the_card() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v1",
                "gate",
                ValidationStatus::Failed,
                Some(1),
                None,
                "2026-06-04T00:00:01Z",
                vec![],
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:01Z",
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            has_failed_validation_item(&projection, "v1"),
            "equal instants keep the card (conservative tie rule)",
        );
    }

    #[test]
    fn mixed_current_assessments_keep_the_card() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v1",
                "gate",
                ValidationStatus::Failed,
                Some(1),
                None,
                "2026-06-04T00:00:01Z",
                vec![],
            ),
            assessment_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:02Z",
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s2",
                ReviewAssessment::NeedsChanges,
                vec![],
                vec![],
                "2026-06-04T00:00:03Z",
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            has_failed_validation_item(&projection, "v1"),
            "any current non-accepting assessment vetoes suppression",
        );
    }

    #[test]
    fn replaced_needs_changes_then_accepted_subsumes() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v1",
                "red proof",
                ValidationStatus::Failed,
                Some(1),
                None,
                "2026-06-04T00:00:01Z",
                vec![],
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::NeedsChanges,
                vec![],
                vec![],
                "2026-06-04T00:00:02Z",
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s2",
                ReviewAssessment::Accepted,
                vec!["s1"],
                vec![],
                "2026-06-04T00:00:03Z",
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            no_failed_validation_items(&projection),
            "the CURRENT set (needs_changes replaced by accepted) is unanimously accepting",
        );
    }

    #[test]
    fn mixed_timestamp_formats_compare_as_instants() {
        // Case A: RFC 3339 failure at 2026-06-04T00:00:01Z (epoch-ms
        // 1780531201000); unix-ms assessment genuinely EARLIER
        // (1780531100000). Lexical order would call the assessment later
        // ('u' > '2') and wrongly suppress; parsed comparison keeps the card.
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v1",
                "gate",
                ValidationStatus::Failed,
                Some(1),
                None,
                "2026-06-04T00:00:01Z",
                vec![],
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "unix-ms:1780531100000",
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            has_failed_validation_item(&projection, "v1"),
            "an earlier unix-ms assessment must not suppress an RFC 3339 failure",
        );

        // Case B: unix-ms failure at 1780531201000 (2026-06-04T00:00:01Z);
        // RFC 3339 assessment genuinely LATER (2026-06-04T00:00:02Z). Lexical
        // order would call the assessment earlier and keep the card; parsed
        // comparison clears it.
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v1",
                "gate",
                ValidationStatus::Failed,
                Some(1),
                None,
                "unix-ms:1780531201000",
                vec![],
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:02Z",
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            no_failed_validation_items(&projection),
            "a genuinely later RFC 3339 assessment subsumes a unix-ms failure",
        );
    }

    #[test]
    fn unparseable_timestamp_keeps_the_card() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v1",
                "gate",
                ValidationStatus::Failed,
                Some(1),
                None,
                "2026-06-04T00:00:01Z",
                vec![],
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "not-a-time",
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            has_failed_validation_item(&projection, "v1"),
            "an unparseable assessment instant can never establish strictly-later",
        );
    }

    #[test]
    fn file_scoped_acceptance_does_not_subsume() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v1",
                "gate",
                ValidationStatus::Failed,
                Some(1),
                None,
                "2026-06-04T00:00:01Z",
                vec![],
            ),
            assessment_event_with_target(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:02Z",
                ReviewTargetRef::File {
                    revision_id: a.clone(),
                    file_path: "src/lib.rs".to_owned(),
                },
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            has_failed_validation_item(&projection, "v1"),
            "a file-scoped acceptance is not a revision-scoped judgment",
        );
    }

    #[test]
    fn later_skipped_record_does_not_hide_an_earlier_failure() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v1",
                "gate",
                ValidationStatus::Failed,
                Some(1),
                None,
                "2026-06-04T00:00:01Z",
                vec![],
            ),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v2",
                "gate",
                ValidationStatus::Skipped,
                None,
                None,
                "2026-06-04T00:00:02Z",
                vec![],
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            has_failed_validation_item(&projection, "v1"),
            "a later skipped run neither reports nor clears",
        );
    }

    #[test]
    fn later_passed_record_still_clears_the_failure() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v1",
                "gate",
                ValidationStatus::Failed,
                Some(1),
                None,
                "2026-06-04T00:00:01Z",
                vec![],
            ),
            validation_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "v2",
                "gate",
                ValidationStatus::Passed,
                Some(0),
                None,
                "2026-06-04T00:00:02Z",
                vec![],
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            no_failed_validation_items(&projection),
            "a strictly-later pass clears the card",
        );
    }

    fn no_stale_assessment_items(projection: &AttentionProjection) -> bool {
        projection
            .items
            .iter()
            .all(|item| !item.id.starts_with("stale_assessment:"))
    }

    fn has_stale_assessment_item(projection: &AttentionProjection, assess_id: &str) -> bool {
        projection
            .items
            .iter()
            .any(|item| item.id == format!("stale_assessment:assess:sha256:{assess_id}"))
    }

    #[test]
    fn stale_assessment_clears_when_every_successor_head_is_assessed() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:01Z",
            ),
            revision_event("b", vec![rev("a")], "2026-06-04T00:00:02Z"),
            assessment_event(
                &rev("b"),
                "agent:codex",
                "actor:agent:codex",
                "s2",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:03Z",
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            no_stale_assessment_items(&projection),
            "a re-judged successor resolves the stale decision",
        );
    }

    #[test]
    fn stale_assessment_stays_when_the_successor_head_is_unassessed() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:01Z",
            ),
            revision_event("b", vec![rev("a")], "2026-06-04T00:00:02Z"),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            has_stale_assessment_item(&projection, "s1"),
            "an unjudged successor keeps the moved-on-without-re-deciding nudge",
        );
    }

    #[test]
    fn stale_assessment_stays_while_any_fork_head_is_unassessed() {
        let a = rev("a");
        let base = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:01Z",
            ),
            revision_event("b", vec![rev("a")], "2026-06-04T00:00:02Z"),
            revision_event("c", vec![rev("a")], "2026-06-04T00:00:03Z"),
            assessment_event(
                &rev("b"),
                "agent:codex",
                "actor:agent:codex",
                "s2",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:04Z",
            ),
        ];
        let projection = attention_from_events(&base, None).expect("projects");
        assert!(
            has_stale_assessment_item(&projection, "s1"),
            "one unassessed fork head keeps the item",
        );

        let mut all_assessed = base;
        all_assessed.push(assessment_event(
            &rev("c"),
            "agent:codex",
            "actor:agent:codex",
            "s3",
            ReviewAssessment::Accepted,
            vec![],
            vec![],
            "2026-06-04T00:00:05Z",
        ));
        let projection = attention_from_events(&all_assessed, None).expect("projects");
        assert!(
            no_stale_assessment_items(&projection),
            "every fork head re-judged resolves the stale decision",
        );
    }

    #[test]
    fn successor_assessed_needs_changes_still_resolves() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:01Z",
            ),
            revision_event("b", vec![rev("a")], "2026-06-04T00:00:02Z"),
            assessment_event(
                &rev("b"),
                "agent:codex",
                "actor:agent:codex",
                "s2",
                ReviewAssessment::NeedsChanges,
                vec![],
                vec![],
                "2026-06-04T00:00:03Z",
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            no_stale_assessment_items(&projection),
            "re-judged is re-judged, whatever the verdict",
        );
    }

    #[test]
    fn file_scoped_head_assessment_does_not_resolve() {
        let a = rev("a");
        let b = rev("b");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:01Z",
            ),
            revision_event("b", vec![a.clone()], "2026-06-04T00:00:02Z"),
            assessment_event_with_target(
                &b,
                "agent:codex",
                "actor:agent:codex",
                "s2",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:03Z",
                ReviewTargetRef::File {
                    revision_id: b.clone(),
                    file_path: "src/lib.rs".to_owned(),
                },
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            has_stale_assessment_item(&projection, "s1"),
            "a file-scoped judgment on the head is not a revision-scoped re-decision",
        );
    }

    #[test]
    fn stale_assessment_stays_inside_a_zero_head_cycle() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![rev("b")], "2026-06-04T00:00:00Z"),
            revision_event("b", vec![rev("a")], "2026-06-04T00:00:01Z"),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:02Z",
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            has_stale_assessment_item(&projection, "s1"),
            "a zero-head cycle never suppresses",
        );
    }

    #[test]
    fn stale_assessment_stays_in_a_cycle_with_an_external_head() {
        // a <-> b cycle plus c superseding b: c is a genuine current head of the
        // component, but the thread contains a cycle, so suppression must refuse
        // even once c is assessed.
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![rev("b")], "2026-06-04T00:00:00Z"),
            revision_event("b", vec![rev("a")], "2026-06-04T00:00:01Z"),
            revision_event("c", vec![rev("b")], "2026-06-04T00:00:02Z"),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:03Z",
            ),
            assessment_event(
                &rev("c"),
                "agent:codex",
                "actor:agent:codex",
                "s2",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:04Z",
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            has_stale_assessment_item(&projection, "s1"),
            "a cycled thread never suppresses, even with an assessed external head",
        );
    }

    fn superseded_ambiguity_fixture(assess_successor: bool) -> Vec<ShoreEvent> {
        let a = rev("a");
        let mut events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            assessment_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:01Z",
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s2",
                ReviewAssessment::NeedsChanges,
                vec![],
                vec![],
                "2026-06-04T00:00:02Z",
            ),
            revision_event("b", vec![rev("a")], "2026-06-04T00:00:03Z"),
        ];
        if assess_successor {
            events.push(assessment_event(
                &rev("b"),
                "agent:codex",
                "actor:agent:codex",
                "s3",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:04Z",
            ));
        }
        events
    }

    #[test]
    fn superseded_ambiguity_clears_when_every_successor_head_is_assessed() {
        let events = superseded_ambiguity_fixture(true);
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            projection
                .items
                .iter()
                .all(|item| !item.id.starts_with("ambiguous_assessment:")),
            "a re-judged successor resolves the superseded ambiguity",
        );
        // Rule A also clears a's two stale_assessment items here (same
        // predicate), so the composed queue is fully empty.
        assert!(projection.items.is_empty());
    }

    #[test]
    fn superseded_ambiguity_stays_when_the_successor_head_is_unassessed() {
        let events = superseded_ambiguity_fixture(false);
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            projection
                .items
                .iter()
                .any(|item| item.id == "ambiguous_assessment:rev:sha256:a"),
            "an unjudged successor keeps the superseded ambiguity",
        );
    }

    #[test]
    fn current_head_ambiguity_always_emits() {
        let a = rev("a");
        let events = vec![
            revision_event("a", vec![], "2026-06-04T00:00:00Z"),
            assessment_event(
                &a,
                "human:kevin",
                "actor:human:kevin",
                "s1",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:01Z",
            ),
            assessment_event(
                &a,
                "agent:codex",
                "actor:agent:codex",
                "s2",
                ReviewAssessment::Accepted,
                vec![],
                vec![],
                "2026-06-04T00:00:02Z",
            ),
        ];
        let projection = attention_from_events(&events, None).expect("projects");
        assert!(
            projection
                .items
                .iter()
                .any(|item| item.id == "ambiguous_assessment:rev:sha256:a"),
            "ambiguity on a current head is always judgment-worthy",
        );
    }
}
