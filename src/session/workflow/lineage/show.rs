use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::error::{Result, ShoreError};
use crate::model::{ReviewUnitId, ReviewUnitLineageId, ReviewUnitLineageRoundId};
use crate::session::EventStore;
use crate::session::event::{EventType, ShoreEvent};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::resolve_read_store;

pub const STALE_BY_NEWER_ROUND_CODE: &str = "stale_by_newer_round";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineageShowOptions {
    repo: PathBuf,
    lineage_id: ReviewUnitLineageId,
}

impl LineageShowOptions {
    pub fn new(repo: impl AsRef<Path>, lineage_id: ReviewUnitLineageId) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            lineage_id,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineageShowResult {
    pub event_set_hash: String,
    pub event_count: usize,
    pub lineage_id: ReviewUnitLineageId,
    pub head_review_unit_id: Option<ReviewUnitId>,
    pub rounds: Vec<LineageRoundView>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineageRoundView {
    pub lineage_id: ReviewUnitLineageId,
    pub round_id: ReviewUnitLineageRoundId,
    pub review_unit_id: ReviewUnitId,
    pub predecessor_review_unit_id: Option<ReviewUnitId>,
    pub round_index: Option<usize>,
    pub is_head: bool,
}

pub fn show_lineage(options: LineageShowOptions) -> Result<LineageShowResult> {
    let read_store = resolve_read_store(&options.repo)?;
    let events = EventStore::open(read_store.store_dir()).list_events()?;
    let state = SessionState::from_events(&events)?;
    let projection =
        crate::session::projection::lineage::ReviewUnitLineageProjection::from_events(&events)?;
    let lineage = projection.lineage(&options.lineage_id).ok_or_else(|| {
        ShoreError::Message(format!(
            "unknown ReviewUnit lineage: {}",
            options.lineage_id.as_str()
        ))
    })?;

    let mut diagnostics = lineage.diagnostics.clone();
    append_stale_by_newer_round_diagnostics(&mut diagnostics, &events, lineage);
    let event_set_hash = state
        .event_set_hash
        .expect("SessionState::from_events sets event_set_hash");

    Ok(LineageShowResult {
        event_set_hash,
        event_count: events.len(),
        lineage_id: lineage.lineage_id.clone(),
        head_review_unit_id: lineage.head_review_unit_id.clone(),
        rounds: lineage
            .rounds
            .iter()
            .cloned()
            .map(LineageRoundView::from)
            .collect(),
        diagnostics,
    })
}

fn append_stale_by_newer_round_diagnostics(
    diagnostics: &mut Vec<ProjectionDiagnostic>,
    events: &[ShoreEvent],
    lineage: &crate::session::projection::lineage::ReviewUnitLineageView,
) {
    let Some(head_review_unit_id) = lineage.head_review_unit_id.as_ref() else {
        return;
    };
    let lineage_review_units = lineage
        .rounds
        .iter()
        .map(|round| round.review_unit_id.clone())
        .collect::<BTreeSet<_>>();

    for event in events
        .iter()
        .filter(|event| fact_event_can_be_stale(event.event_type))
    {
        let Some(review_unit_id) = event.target.review_unit_id.as_ref() else {
            continue;
        };
        if review_unit_id == head_review_unit_id || !lineage_review_units.contains(review_unit_id) {
            continue;
        }

        diagnostics.push(ProjectionDiagnostic {
            code: STALE_BY_NEWER_ROUND_CODE.to_owned(),
            message: format!(
                "{} event {} targets older ReviewUnit {} in lineage {}; head is {}",
                event.event_type.as_str(),
                event.event_id.as_str(),
                review_unit_id.as_str(),
                lineage.lineage_id.as_str(),
                head_review_unit_id.as_str()
            ),
        });
    }
}

fn fact_event_can_be_stale(event_type: EventType) -> bool {
    matches!(
        event_type,
        EventType::ReviewObservationRecorded
            | EventType::InputRequestOpened
            | EventType::ReviewAssessmentRecorded
    )
}

impl From<crate::session::projection::lineage::ReviewUnitLineageRoundView> for LineageRoundView {
    fn from(round: crate::session::projection::lineage::ReviewUnitLineageRoundView) -> Self {
        Self {
            lineage_id: round.lineage_id,
            round_id: round.round_id,
            review_unit_id: round.review_unit_id,
            predecessor_review_unit_id: round.predecessor_review_unit_id,
            round_index: round.round_index,
            is_head: round.is_head,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use super::{LineageShowOptions, show_lineage};
    use crate::model::ReviewUnitLineageId;
    use crate::session::{
        CaptureOptions, LineageAttachOptions, ObservationAddOptions, ObservationTargetSelector,
        ReviewUnitShowOptions, attach_review_unit_to_lineage, capture_worktree_review,
        record_observation, show_review_unit,
    };

    #[test]
    fn older_round_fact_is_stale_by_newer_round_in_lineage_view() {
        let repo = modified_repo();
        let lineage_id = review_unit_lineage_id();
        let first = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        attach_review_unit_to_lineage(
            LineageAttachOptions::new(repo.path(), lineage_id.clone())
                .with_review_unit_id(first.review_unit_id.clone()),
        )
        .unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_review_unit_id(first.review_unit_id.clone())
                .with_track("agent:codex")
                .with_title("old fact")
                .with_target(ObservationTargetSelector::review_unit()),
        )
        .unwrap();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
        let second = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        attach_review_unit_to_lineage(
            LineageAttachOptions::new(repo.path(), lineage_id.clone())
                .with_review_unit_id(second.review_unit_id.clone())
                .with_predecessor_review_unit_id(first.review_unit_id.clone()),
        )
        .unwrap();

        let lineage = show_lineage(LineageShowOptions::new(repo.path(), lineage_id)).unwrap();

        assert_eq!(
            lineage.head_review_unit_id.as_ref(),
            Some(&second.review_unit_id)
        );
        assert!(lineage.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "stale_by_newer_round"
                && diagnostic.message.contains(first.review_unit_id.as_str())
        }));

        let exact = show_review_unit(
            ReviewUnitShowOptions::new(repo.path())
                .with_review_unit_id(first.review_unit_id.clone()),
        )
        .unwrap();
        assert!(
            !exact
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "stale_by_newer_round")
        );
    }

    fn review_unit_lineage_id() -> ReviewUnitLineageId {
        ReviewUnitLineageId::new("review-unit-lineage:random:test")
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo
    }

    struct TestRepo {
        root: tempfile::TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("create temp git repository directory");
            let repo = Self { root };

            repo.git(["init"]);
            repo.git(["config", "user.name", "Shore Tests"]);
            repo.git(["config", "user.email", "shore-tests@example.com"]);
            repo.git(["config", "commit.gpgsign", "false"]);

            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
            let path = self.root.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directories");
            }
            fs::write(path, contents).expect("write test repository file");
        }

        fn commit_all(&self, message: &str) {
            self.git(["add", "--all"]);
            self.git(["commit", "-m", message]);
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            let args = args
                .into_iter()
                .map(|arg| arg.as_ref().to_owned())
                .collect::<Vec<_>>();
            let output = Command::new("git")
                .args(&args)
                .current_dir(self.root.path())
                .output()
                .unwrap_or_else(|error| panic!("run git {:?}: {error}", args));

            assert!(
                output.status.success(),
                "git {:?} failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
                args,
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
