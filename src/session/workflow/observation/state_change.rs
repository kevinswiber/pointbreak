use std::path::{Path, PathBuf};

use super::{
    ObservationAddResult, ObservationWriteInput, resolve_review_unit, write_observation_event,
};
use crate::error::{Result, ShoreError};
use crate::model::{ReviewTargetRef, ReviewUnitId};
use crate::session::EventStore;
use crate::session::store_init::ShoreStorePaths;

const TAG_PREFIX_STATE_CHANGE: &str = "state-change:";
const TAG_PREFIX_OVERRIDE_TARGET: &str = "override-target:";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateChangeOptions {
    repo: PathBuf,
    track: Option<String>,
    title: Option<String>,
    body: Option<String>,
    target: Option<ReviewTargetRef>,
    override_targets: Vec<ReviewTargetRef>,
    extra_tags: Vec<String>,
    idempotency_key: Option<String>,
}

impl StateChangeOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            track: None,
            title: None,
            body: None,
            target: None,
            override_targets: Vec::new(),
            extra_tags: Vec::new(),
            idempotency_key: None,
        }
    }

    pub fn with_track(mut self, track: impl Into<String>) -> Self {
        self.track = Some(track.into());
        self
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn with_target(mut self, target: ReviewTargetRef) -> Self {
        self.target = Some(target);
        self
    }

    pub fn overriding(mut self, target: ReviewTargetRef) -> Self {
        self.override_targets.push(target);
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.extra_tags.push(tag.into());
        self
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

pub fn record_deferred_state_change(
    mut options: StateChangeOptions,
) -> Result<ObservationAddResult> {
    let extra_tags = std::mem::take(&mut options.extra_tags);
    let mut tags = vec![format!("{TAG_PREFIX_STATE_CHANGE}deferred")];
    tags.extend(extra_tags);
    delegate_to_observation(options, tags)
}

pub fn record_split_out_state_change(
    mut options: StateChangeOptions,
) -> Result<ObservationAddResult> {
    let extra_tags = std::mem::take(&mut options.extra_tags);
    let mut tags = vec![format!("{TAG_PREFIX_STATE_CHANGE}split-out")];
    tags.extend(extra_tags);
    delegate_to_observation(options, tags)
}

pub fn record_superseded_state_change(
    mut options: StateChangeOptions,
) -> Result<ObservationAddResult> {
    if !matches!(
        options.target.as_ref(),
        Some(ReviewTargetRef::Assessment { .. })
    ) {
        return Err(ShoreError::WorkflowInputInvalid {
            reason: "superseded state change requires an assessment target".to_owned(),
        });
    }

    let extra_tags = std::mem::take(&mut options.extra_tags);
    let mut tags = vec![format!("{TAG_PREFIX_STATE_CHANGE}superseded")];
    tags.extend(extra_tags);
    delegate_to_observation(options, tags)
}

pub fn record_overridden_state_change(
    mut options: StateChangeOptions,
) -> Result<ObservationAddResult> {
    if options
        .body
        .as_deref()
        .is_none_or(|body| body.trim().is_empty())
    {
        return Err(ShoreError::WorkflowInputInvalid {
            reason: "summary is required for overridden state change".to_owned(),
        });
    }
    if options.override_targets.is_empty() {
        return Err(ShoreError::WorkflowInputInvalid {
            reason: "at least one override target is required for overridden state change"
                .to_owned(),
        });
    }

    let mut tags = vec![format!("{TAG_PREFIX_STATE_CHANGE}overridden")];
    for target in &options.override_targets {
        tags.push(format_override_target_tag(target)?);
    }
    let extra_tags = std::mem::take(&mut options.extra_tags);
    tags.extend(extra_tags);
    delegate_to_observation(options, tags)
}

fn delegate_to_observation(
    options: StateChangeOptions,
    tags: Vec<String>,
) -> Result<ObservationAddResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let events = EventStore::open(paths.shore_dir()).list_events()?;
    let (resolved, target) = match options.target {
        Some(target) => {
            let review_unit_id = review_unit_id_for_target(&target);
            (resolve_review_unit(&events, Some(review_unit_id))?, target)
        }
        None => {
            let resolved = resolve_review_unit(&events, None)?;
            let target = ReviewTargetRef::ReviewUnit {
                review_unit_id: resolved.review_unit_id.clone(),
            };
            (resolved, target)
        }
    };

    write_observation_event(ObservationWriteInput {
        repo: options.repo,
        resolved,
        target,
        track: options.track,
        title: options
            .title
            .unwrap_or_else(|| default_state_change_title_for(&tags)),
        body: options.body,
        tags,
        confidence: None,
        supersedes_observation_ids: Vec::new(),
        idempotency_key: options.idempotency_key,
    })
}

fn default_state_change_title_for(tags: &[String]) -> String {
    tags.iter()
        .find_map(|tag| tag.strip_prefix(TAG_PREFIX_STATE_CHANGE))
        .map(|kind| format!("State change: {kind}"))
        .unwrap_or_else(|| "State change".to_owned())
}

fn format_override_target_tag(target: &ReviewTargetRef) -> Result<String> {
    match target {
        ReviewTargetRef::Observation { observation_id, .. } => Ok(format!(
            "{TAG_PREFIX_OVERRIDE_TARGET}observation:{}",
            observation_id.as_str()
        )),
        ReviewTargetRef::Intervention {
            intervention_id, ..
        } => Ok(format!(
            "{TAG_PREFIX_OVERRIDE_TARGET}intervention:{}",
            intervention_id.as_str()
        )),
        ReviewTargetRef::Assessment { assessment_id, .. } => Ok(format!(
            "{TAG_PREFIX_OVERRIDE_TARGET}assessment:{}",
            assessment_id.as_str()
        )),
        _ => Err(ShoreError::WorkflowInputInvalid {
            reason: "override target must be observation, intervention, or assessment".to_owned(),
        }),
    }
}

fn review_unit_id_for_target(target: &ReviewTargetRef) -> &ReviewUnitId {
    match target {
        ReviewTargetRef::ReviewUnit { review_unit_id }
        | ReviewTargetRef::File { review_unit_id, .. }
        | ReviewTargetRef::Range { review_unit_id, .. }
        | ReviewTargetRef::Observation { review_unit_id, .. }
        | ReviewTargetRef::Intervention { review_unit_id, .. }
        | ReviewTargetRef::Disposition { review_unit_id, .. }
        | ReviewTargetRef::Assessment { review_unit_id, .. }
        | ReviewTargetRef::Event { review_unit_id, .. } => review_unit_id,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::error::ShoreError;
    use crate::model::ReviewTargetRef;
    use crate::session::event::{EventType, ReviewAssessment, ReviewObservationRecordedPayload};
    use crate::session::{
        AssessmentAddOptions, CaptureOptions, EventStore, ObservationAddOptions,
        capture_worktree_review, record_assessment, record_observation,
    };

    #[test]
    fn record_deferred_state_change_writes_observation_with_state_change_deferred_tag() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = record_deferred_state_change(
            StateChangeOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Defer until next sprint")
                .with_body("Out of scope for current cycle"),
        )
        .unwrap();

        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let payload = find_observation_payload(&events, &result.observation_id);

        assert!(
            payload
                .tags
                .iter()
                .any(|tag| tag == "state-change:deferred")
        );
        assert_eq!(payload.title, "Defer until next sprint");
    }

    #[test]
    fn record_split_out_state_change_writes_observation_with_state_change_split_out_tag() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = record_split_out_state_change(
            StateChangeOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Split out follow-up")
                .with_body("Move to a smaller review unit"),
        )
        .unwrap();

        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let payload = find_observation_payload(&events, &result.observation_id);

        assert!(
            payload
                .tags
                .iter()
                .any(|tag| tag == "state-change:split-out")
        );
        assert_eq!(payload.title, "Split out follow-up");
    }

    #[test]
    fn record_superseded_state_change_writes_observation_with_state_change_superseded_tag_and_target_assessment_ref()
     {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let assessment = record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::NeedsChanges)
                .with_summary("fix this"),
        )
        .unwrap();

        let result = record_superseded_state_change(
            StateChangeOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Superseded")
                .with_body("Newer assessment replaces this")
                .with_target(ReviewTargetRef::Assessment {
                    review_unit_id: capture.review_unit_id,
                    assessment_id: assessment.assessment_id.clone(),
                }),
        )
        .unwrap();

        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let payload = find_observation_payload(&events, &result.observation_id);

        assert!(
            payload
                .tags
                .iter()
                .any(|tag| tag == "state-change:superseded")
        );
        assert!(matches!(
            payload.target,
            ReviewTargetRef::Assessment { assessment_id, .. } if assessment_id == assessment.assessment_id
        ));
    }

    #[test]
    fn record_overridden_state_change_writes_observation_with_state_change_overridden_tag_and_override_target_tags()
     {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let observation = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Concern")
                .with_body("This needs a closer look"),
        )
        .unwrap();

        let result = record_overridden_state_change(
            StateChangeOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Override")
                .with_body("Manual decision")
                .overriding(ReviewTargetRef::Observation {
                    review_unit_id: capture.review_unit_id,
                    observation_id: observation.observation_id.clone(),
                }),
        )
        .unwrap();

        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let payload = find_observation_payload(&events, &result.observation_id);

        assert!(
            payload
                .tags
                .iter()
                .any(|tag| tag == "state-change:overridden")
        );
        assert!(payload.tags.iter().any(|tag| {
            tag == &format!(
                "override-target:observation:{}",
                observation.observation_id.as_str()
            )
        }));
    }

    #[test]
    fn record_overridden_state_change_requires_summary_and_at_least_one_override_target() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let missing_body = record_overridden_state_change(
            StateChangeOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("override"),
        );
        assert!(matches!(
            missing_body,
            Err(ShoreError::WorkflowInputInvalid { .. })
        ));

        let missing_override_target = record_overridden_state_change(
            StateChangeOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("override")
                .with_body("rationale"),
        );
        assert!(matches!(
            missing_override_target,
            Err(ShoreError::WorkflowInputInvalid { .. })
        ));
    }

    #[test]
    fn record_deferred_state_change_defaults_target_to_current_review_unit_when_none() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = record_deferred_state_change(
            StateChangeOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Defer until next sprint")
                .with_body("Out of scope for current cycle"),
        )
        .unwrap();

        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let payload = find_observation_payload(&events, &result.observation_id);
        assert!(matches!(payload.target, ReviewTargetRef::ReviewUnit { .. }));
    }

    #[test]
    fn record_superseded_state_change_rejects_none_target() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let err = record_superseded_state_change(
            StateChangeOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Superseded"),
        )
        .expect_err("superseded must require an assessment target");
        assert!(matches!(err, ShoreError::WorkflowInputInvalid { .. }));
    }

    fn find_observation_payload(
        events: &[crate::session::event::ShoreEvent],
        observation_id: &crate::model::ObservationId,
    ) -> ReviewObservationRecordedPayload {
        events
            .iter()
            .filter(|event| event.event_type == EventType::ReviewObservationRecorded)
            .filter_map(|event| serde_json::from_value(event.payload.clone()).ok())
            .find(|payload: &ReviewObservationRecordedPayload| {
                &payload.observation_id == observation_id
            })
            .unwrap()
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    1\n}\n");
        repo.git(&["add", "src/lib.rs"]);
        repo.git(&["commit", "-m", "base"]);
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    2\n}\n");
        repo
    }

    struct TestRepo {
        root: tempfile::TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("create temp git repository directory");
            let repo = Self { root };

            repo.git(&["init"]);
            repo.git(&["config", "user.name", "Shore Tests"]);
            repo.git(&["config", "user.email", "shore-tests@example.com"]);
            repo.git(&["config", "commit.gpgsign", "false"]);

            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn write(&self, path: &str, contents: &str) {
            let path = self.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }

        fn git(&self, args: &[&str]) {
            let output = Command::new("git")
                .args(args)
                .current_dir(self.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
                args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
