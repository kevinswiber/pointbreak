use std::collections::BTreeMap;

use serde::Serialize;

use crate::model::{TrackId, ValidationCheckId, ValidationStatus};
use crate::session::compare_event_instants;
use crate::session::workflow::ValidationCheckView;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
/// How one immutable validation record participates in its check history.
pub enum ValidationCheckDisposition {
    Outstanding,
    Current,
    ResolvedByLaterPass,
    Historical,
    Skipped,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
/// Effective group counts for one revision's validation histories.
pub struct ValidationContinuitySummary {
    pub outstanding_failed_count: usize,
    pub outstanding_errored_count: usize,
    pub recovered_count: usize,
    pub passed_count: usize,
    pub skipped_only_count: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
/// One validation-continuity classification, keyed back to every raw record.
pub struct ValidationContinuityView {
    pub summary: ValidationContinuitySummary,
    pub checks: BTreeMap<ValidationCheckId, ValidationCheckDisposition>,
}

/// Classify exact `(track, check name)` histories without rewriting their facts.
pub fn classify_validation_continuity(checks: &[ValidationCheckView]) -> ValidationContinuityView {
    let mut groups: BTreeMap<(TrackId, String), Vec<&ValidationCheckView>> = BTreeMap::new();
    for check in checks {
        groups
            .entry((check.track_id.clone(), check.check_name.clone()))
            .or_default()
            .push(check);
    }

    let mut view = ValidationContinuityView::default();
    for group in groups.values_mut() {
        group.sort_by(|left, right| {
            compare_effective_times(left, right)
                .then_with(|| left.event_id.as_str().cmp(right.event_id.as_str()))
        });
        classify_group(group, &mut view);
    }
    view
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ValidationGroupState {
    OutstandingFailed,
    OutstandingErrored,
    Recovered,
    Passed,
    SkippedOnly,
}

fn classify_group(group: &[&ValidationCheckView], view: &mut ValidationContinuityView) {
    let latest_time = latest_non_skipped_time(group);
    let latest_pass_time = latest_time_for_status(group, ValidationStatus::Passed);
    let state = group_state(group, latest_time);
    match state {
        ValidationGroupState::OutstandingFailed => view.summary.outstanding_failed_count += 1,
        ValidationGroupState::OutstandingErrored => view.summary.outstanding_errored_count += 1,
        ValidationGroupState::Recovered => view.summary.recovered_count += 1,
        ValidationGroupState::Passed => view.summary.passed_count += 1,
        ValidationGroupState::SkippedOnly => view.summary.skipped_only_count += 1,
    }

    for check in group {
        let disposition = disposition_for(check, state, latest_time, latest_pass_time);
        view.checks.insert(check.id.clone(), disposition);
    }
}

fn group_state(group: &[&ValidationCheckView], latest_time: Option<&str>) -> ValidationGroupState {
    let Some(latest_time) = latest_time else {
        return ValidationGroupState::SkippedOnly;
    };
    let latest = group.iter().copied().filter(|check| {
        check.status != ValidationStatus::Skipped && is_effective_time(check, latest_time)
    });
    let mut has_latest_failed = false;
    let mut has_latest_errored = false;
    for check in latest {
        has_latest_failed |= check.status == ValidationStatus::Failed;
        has_latest_errored |= check.status == ValidationStatus::Errored;
    }

    // When failed and errored records share the latest effective time, classify
    // the group conservatively as errored. This is semantic status precedence,
    // never an event-id tie break.
    if has_latest_errored {
        return ValidationGroupState::OutstandingErrored;
    }
    if has_latest_failed {
        return ValidationGroupState::OutstandingFailed;
    }

    if group.iter().any(|check| {
        matches!(
            check.status,
            ValidationStatus::Failed | ValidationStatus::Errored
        )
    }) {
        ValidationGroupState::Recovered
    } else {
        ValidationGroupState::Passed
    }
}

fn disposition_for(
    check: &ValidationCheckView,
    state: ValidationGroupState,
    latest_time: Option<&str>,
    latest_pass_time: Option<&str>,
) -> ValidationCheckDisposition {
    match check.status {
        ValidationStatus::Skipped => ValidationCheckDisposition::Skipped,
        ValidationStatus::Failed | ValidationStatus::Errored => {
            if latest_pass_time.is_some_and(|pass_time| is_strictly_later_than(pass_time, check)) {
                ValidationCheckDisposition::ResolvedByLaterPass
            } else if matches!(
                state,
                ValidationGroupState::OutstandingFailed | ValidationGroupState::OutstandingErrored
            ) && latest_time.is_some_and(|latest| is_effective_time(check, latest))
            {
                ValidationCheckDisposition::Outstanding
            } else {
                ValidationCheckDisposition::Historical
            }
        }
        ValidationStatus::Passed => {
            if matches!(
                state,
                ValidationGroupState::Passed | ValidationGroupState::Recovered
            ) && latest_time.is_some_and(|latest| is_effective_time(check, latest))
            {
                ValidationCheckDisposition::Current
            } else {
                ValidationCheckDisposition::Historical
            }
        }
    }
}

fn latest_non_skipped_time<'a>(group: &[&'a ValidationCheckView]) -> Option<&'a str> {
    group
        .iter()
        .filter(|check| check.status != ValidationStatus::Skipped)
        .map(|check| effective_time(check))
        .max_by(|left, right| compare_event_instants(left, right))
}

fn latest_time_for_status<'a>(
    group: &[&'a ValidationCheckView],
    status: ValidationStatus,
) -> Option<&'a str> {
    group
        .iter()
        .filter(|check| check.status == status)
        .map(|check| effective_time(check))
        .max_by(|left, right| compare_event_instants(left, right))
}

fn compare_effective_times(
    left: &ValidationCheckView,
    right: &ValidationCheckView,
) -> std::cmp::Ordering {
    compare_event_instants(effective_time(left), effective_time(right))
}

fn is_effective_time(check: &ValidationCheckView, time: &str) -> bool {
    compare_event_instants(effective_time(check), time).is_eq()
}

fn is_strictly_later_than(time: &str, check: &ValidationCheckView) -> bool {
    compare_event_instants(time, effective_time(check)).is_gt()
}

fn effective_time(check: &ValidationCheckView) -> &str {
    check.completed_at.as_deref().unwrap_or(&check.created_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        EventId, RevisionId, TrackId, ValidationStatus, ValidationTarget, ValidationTrigger,
    };
    use crate::session::event::Writer;

    const EARLY: &str = "2026-07-16T10:00:00Z";
    const MIDDLE: &str = "2026-07-16T10:01:00Z";
    const LATE: &str = "2026-07-16T10:02:00Z";

    fn check(
        id: &str,
        event_id: &str,
        track: &str,
        name: &str,
        status: ValidationStatus,
        completed_at: Option<&str>,
        created_at: &str,
    ) -> ValidationCheckView {
        ValidationCheckView {
            id: ValidationCheckId::new(id),
            event_id: EventId::new(event_id),
            track_id: TrackId::new(track),
            target: ValidationTarget::Revision {
                revision_id: RevisionId::new("rev:sha256:fixture"),
            },
            check_name: name.to_owned(),
            command: Some(format!("run {name}")),
            status,
            exit_code: match status {
                ValidationStatus::Passed => Some(0),
                ValidationStatus::Failed | ValidationStatus::Errored => Some(1),
                ValidationStatus::Skipped => None,
            },
            trigger: ValidationTrigger::Manual,
            source_fingerprint: Some("fixture-source".to_owned()),
            summary: Some(format!("{name} {status:?}")),
            summary_content_type: Default::default(),
            summary_content_hash: Some("sha256:summary".to_owned()),
            summary_content_state: Default::default(),
            started_at: None,
            completed_at: completed_at.map(str::to_owned),
            log_artifact_content_hashes: vec!["sha256:log".to_owned()],
            created_at: created_at.to_owned(),
            writer: Writer::shore_local("test"),
            superseded_by_revisions: Default::default(),
        }
    }

    fn one(status: ValidationStatus) -> ValidationCheckView {
        check(
            "validation:sha256:only",
            "evt:sha256:only",
            "agent:author",
            "cargo test",
            status,
            Some(MIDDLE),
            EARLY,
        )
    }

    fn disposition<'a>(
        result: &'a ValidationContinuityView,
        id: &str,
    ) -> &'a ValidationCheckDisposition {
        &result.checks[&ValidationCheckId::new(id)]
    }

    #[test]
    fn validation_continuity_classifies_single_status_groups() {
        for (status, expected_summary, expected_disposition) in [
            (
                ValidationStatus::Passed,
                ValidationContinuitySummary {
                    passed_count: 1,
                    ..Default::default()
                },
                ValidationCheckDisposition::Current,
            ),
            (
                ValidationStatus::Failed,
                ValidationContinuitySummary {
                    outstanding_failed_count: 1,
                    ..Default::default()
                },
                ValidationCheckDisposition::Outstanding,
            ),
            (
                ValidationStatus::Errored,
                ValidationContinuitySummary {
                    outstanding_errored_count: 1,
                    ..Default::default()
                },
                ValidationCheckDisposition::Outstanding,
            ),
            (
                ValidationStatus::Skipped,
                ValidationContinuitySummary {
                    skipped_only_count: 1,
                    ..Default::default()
                },
                ValidationCheckDisposition::Skipped,
            ),
        ] {
            let checks = vec![one(status)];
            let result = classify_validation_continuity(&checks);

            assert_eq!(result.summary, expected_summary, "status {status:?}");
            assert_eq!(
                disposition(&result, "validation:sha256:only"),
                &expected_disposition,
                "status {status:?}"
            );
            assert_eq!(checks.len(), 1, "classification retains raw inputs");
        }
    }

    #[test]
    fn validation_continuity_recovers_only_with_a_strictly_later_pass() {
        for (status, prefix) in [
            (ValidationStatus::Failed, "failed"),
            (ValidationStatus::Errored, "errored"),
        ] {
            let first_id = format!("validation:sha256:{prefix}");
            let pass_id = format!("validation:sha256:{prefix}-pass");
            let checks = vec![
                check(
                    &first_id,
                    &format!("evt:sha256:{prefix}"),
                    "agent:author",
                    "cargo test",
                    status,
                    Some(EARLY),
                    EARLY,
                ),
                check(
                    &pass_id,
                    &format!("evt:sha256:{prefix}-pass"),
                    "agent:author",
                    "cargo test",
                    ValidationStatus::Passed,
                    Some(LATE),
                    MIDDLE,
                ),
            ];

            let result = classify_validation_continuity(&checks);
            assert_eq!(result.summary.recovered_count, 1);
            assert_eq!(result.summary.outstanding_failed_count, 0);
            assert_eq!(result.summary.outstanding_errored_count, 0);
            assert_eq!(
                disposition(&result, &first_id),
                &ValidationCheckDisposition::ResolvedByLaterPass
            );
            assert_eq!(
                disposition(&result, &pass_id),
                &ValidationCheckDisposition::Current
            );
        }
    }

    #[test]
    fn validation_continuity_handles_ordered_pass_failure_and_skip_sequences() {
        let cases = [
            (
                "passed-then-failed",
                vec![
                    (ValidationStatus::Passed, EARLY),
                    (ValidationStatus::Failed, LATE),
                ],
                ValidationContinuitySummary {
                    outstanding_failed_count: 1,
                    ..Default::default()
                },
            ),
            (
                "failed-then-skipped",
                vec![
                    (ValidationStatus::Failed, EARLY),
                    (ValidationStatus::Skipped, LATE),
                ],
                ValidationContinuitySummary {
                    outstanding_failed_count: 1,
                    ..Default::default()
                },
            ),
            (
                "skipped-then-passed",
                vec![
                    (ValidationStatus::Skipped, EARLY),
                    (ValidationStatus::Passed, LATE),
                ],
                ValidationContinuitySummary {
                    passed_count: 1,
                    ..Default::default()
                },
            ),
        ];

        for (name, statuses, expected_summary) in cases {
            let checks = statuses
                .into_iter()
                .enumerate()
                .map(|(index, (status, time))| {
                    check(
                        &format!("validation:sha256:{name}-{index}"),
                        &format!("evt:sha256:{name}-{index}"),
                        "agent:author",
                        "cargo test",
                        status,
                        Some(time),
                        time,
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(
                classify_validation_continuity(&checks).summary,
                expected_summary,
                "case {name}"
            );
        }
    }

    #[test]
    fn validation_continuity_keeps_tracks_and_check_names_independent() {
        let checks = vec![
            check(
                "validation:sha256:author-test-fail",
                "evt:sha256:01",
                "agent:author",
                "cargo test",
                ValidationStatus::Failed,
                Some(EARLY),
                EARLY,
            ),
            check(
                "validation:sha256:author-test-pass",
                "evt:sha256:02",
                "agent:author",
                "cargo test",
                ValidationStatus::Passed,
                Some(LATE),
                LATE,
            ),
            check(
                "validation:sha256:reviewer-test-fail",
                "evt:sha256:03",
                "agent:reviewer",
                "cargo test",
                ValidationStatus::Failed,
                Some(EARLY),
                EARLY,
            ),
            check(
                "validation:sha256:author-clippy-error",
                "evt:sha256:04",
                "agent:author",
                "cargo clippy",
                ValidationStatus::Errored,
                Some(EARLY),
                EARLY,
            ),
        ];

        let result = classify_validation_continuity(&checks);
        assert_eq!(result.summary.recovered_count, 1);
        assert_eq!(result.summary.outstanding_failed_count, 1);
        assert_eq!(result.summary.outstanding_errored_count, 1);
        assert_eq!(result.checks.len(), checks.len());
    }

    #[test]
    fn validation_continuity_equal_time_pass_never_clears_failure() {
        for (pass_event_id, failure_event_id) in [
            ("evt:sha256:00", "evt:sha256:ff"),
            ("evt:sha256:ff", "evt:sha256:00"),
        ] {
            let checks = vec![
                check(
                    "validation:sha256:pass",
                    pass_event_id,
                    "agent:author",
                    "cargo test",
                    ValidationStatus::Passed,
                    Some(MIDDLE),
                    EARLY,
                ),
                check(
                    "validation:sha256:failure",
                    failure_event_id,
                    "agent:author",
                    "cargo test",
                    ValidationStatus::Failed,
                    Some(MIDDLE),
                    EARLY,
                ),
            ];

            let result = classify_validation_continuity(&checks);
            assert_eq!(result.summary.outstanding_failed_count, 1);
            assert_eq!(result.summary.recovered_count, 0);
            assert_eq!(
                disposition(&result, "validation:sha256:failure"),
                &ValidationCheckDisposition::Outstanding
            );
            assert_eq!(
                disposition(&result, "validation:sha256:pass"),
                &ValidationCheckDisposition::Historical
            );
        }
    }

    #[test]
    fn validation_continuity_retains_every_record_with_its_disposition() {
        let checks = vec![
            check(
                "validation:sha256:first-failure",
                "evt:sha256:01",
                "agent:author",
                "cargo test",
                ValidationStatus::Failed,
                Some(EARLY),
                EARLY,
            ),
            check(
                "validation:sha256:recovery-pass",
                "evt:sha256:02",
                "agent:author",
                "cargo test",
                ValidationStatus::Passed,
                Some(MIDDLE),
                MIDDLE,
            ),
            check(
                "validation:sha256:current-failure",
                "evt:sha256:03",
                "agent:author",
                "cargo test",
                ValidationStatus::Failed,
                Some(LATE),
                LATE,
            ),
            check(
                "validation:sha256:later-skip",
                "evt:sha256:04",
                "agent:author",
                "cargo test",
                ValidationStatus::Skipped,
                None,
                "2026-07-16T10:03:00Z",
            ),
        ];
        let original = checks.clone();

        let result = classify_validation_continuity(&checks);

        assert_eq!(
            checks, original,
            "the read projection never rewrites evidence"
        );
        assert_eq!(result.checks.len(), checks.len());
        assert_eq!(
            disposition(&result, "validation:sha256:first-failure"),
            &ValidationCheckDisposition::ResolvedByLaterPass
        );
        assert_eq!(
            disposition(&result, "validation:sha256:recovery-pass"),
            &ValidationCheckDisposition::Historical
        );
        assert_eq!(
            disposition(&result, "validation:sha256:current-failure"),
            &ValidationCheckDisposition::Outstanding
        );
        assert_eq!(
            disposition(&result, "validation:sha256:later-skip"),
            &ValidationCheckDisposition::Skipped
        );
    }
}
