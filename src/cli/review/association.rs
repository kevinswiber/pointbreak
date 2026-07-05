use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};
use shoreline::documents::{
    associate_commit_document, associate_ref_document, list_associations_document,
    withdraw_commit_document, withdraw_ref_document,
};
use shoreline::model::{CommitAssociationId, RefAssociationId, RevisionId};
use shoreline::session::{
    AssociateCommitOptions, AssociateRefOptions, AssociationAxis, CommitEdgeSource,
    CommitGraphCondition, ListAssociationsOptions, ListAssociationsResult, RevisionCommitRangeView,
    WithdrawCommitOptions, WithdrawRefOptions, associate_commit, associate_ref, enrich_liveness,
    list_associations, withdraw_commit, withdraw_ref,
};

use crate::cli::output;
use crate::cli::review::common::{SignableOptions, SigningSkip};

#[derive(Debug, Args)]
pub(super) struct AssociationArgs {
    #[command(subcommand)]
    command: AssociationCommand,
}

#[derive(Debug, Subcommand)]
enum AssociationCommand {
    AssociateCommit(AssociateCommitArgs),
    WithdrawCommit(WithdrawCommitArgs),
    AssociateRef(AssociateRefArgs),
    WithdrawRef(WithdrawRefArgs),
    List(ListArgs),
}

/// Associate a revision with a commit.
#[derive(Debug, Args)]
struct AssociateCommitArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    revision: Option<String>,

    #[arg(long)]
    track: String,

    /// The commit rev to associate with this revision (resolved to an OID).
    #[arg(long)]
    commit: String,

    /// Sign this write with a specific key: a keystore key name or a path to a
    /// key file. Overrides SHORE_SIGNING_KEY. A key that cannot be loaded leaves
    /// the write unsigned (exit 0) with an advisory diagnostic — signing never
    /// blocks.
    #[arg(long)]
    sign_key: Option<String>,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// Withdraw a commit association.
#[derive(Debug, Args)]
struct WithdrawCommitArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    revision: Option<String>,

    #[arg(long)]
    track: String,

    /// The commit association id to withdraw.
    #[arg(long)]
    withdraws: String,

    #[arg(long)]
    sign_key: Option<String>,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// Associate a revision with a ref.
#[derive(Debug, Args)]
struct AssociateRefArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    revision: Option<String>,

    #[arg(long)]
    track: String,

    /// The ref to associate; a short branch name is normalized to its full ref.
    #[arg(long = "ref", alias = "branch")]
    ref_name: String,

    /// The head OID the ref points at (explicit, never inferred).
    #[arg(long)]
    head: String,

    #[arg(long)]
    sign_key: Option<String>,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// Withdraw a ref association.
#[derive(Debug, Args)]
struct WithdrawRefArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    revision: Option<String>,

    #[arg(long)]
    track: String,

    /// The ref association id to withdraw.
    #[arg(long)]
    withdraws: String,

    #[arg(long)]
    sign_key: Option<String>,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// List commit and ref associations.
#[derive(Debug, Args)]
struct ListArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    revision: Option<String>,

    /// Restrict to one axis; omit to list both.
    #[arg(long, value_enum)]
    axis: Option<AxisArg>,

    /// Exclude withdrawn associations, showing only what currently holds.
    #[arg(long)]
    current: bool,

    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    #[arg(long)]
    compact: bool,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum AxisArg {
    Commit,
    Ref,
}

impl From<AxisArg> for AssociationAxis {
    fn from(axis: AxisArg) -> Self {
        match axis {
            AxisArg::Commit => AssociationAxis::Commit,
            AxisArg::Ref => AssociationAxis::Ref,
        }
    }
}

pub(super) fn run(
    args: AssociationArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        AssociationCommand::AssociateCommit(args) => {
            let span = tracing::info_span!("shore.review.association.associate-commit");
            let _entered = span.enter();
            associate_commit_run(args, stdout, stderr)
        }
        AssociationCommand::WithdrawCommit(args) => {
            let span = tracing::info_span!("shore.review.association.withdraw-commit");
            let _entered = span.enter();
            withdraw_commit_run(args, stdout, stderr)
        }
        AssociationCommand::AssociateRef(args) => {
            let span = tracing::info_span!("shore.review.association.associate-ref");
            let _entered = span.enter();
            associate_ref_run(args, stdout, stderr)
        }
        AssociationCommand::WithdrawRef(args) => {
            let span = tracing::info_span!("shore.review.association.withdraw-ref");
            let _entered = span.enter();
            withdraw_ref_run(args, stdout, stderr)
        }
        AssociationCommand::List(args) => {
            let span = tracing::info_span!("shore.review.association.list");
            let _entered = span.enter();
            list_run(args, stdout)
        }
    }
}

fn associate_commit_run(
    args: AssociateCommitArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut options = AssociateCommitOptions::new(&args.repo, args.commit).with_track(args.track);
    options = with_selection(options, args.revision);
    let (options, skip) = apply_signer(options, &args.repo, args.sign_key.as_deref(), stderr);
    let result = associate_commit(options)?;
    super::common::surface_best_effort_skip(&skip, stderr);
    let format =
        output::resolve_format(args.format_args.explicit(false), output::OutputFormat::Json)?;
    output::write_document_json_fallback(stdout, format, &associate_commit_document(result))
}

fn withdraw_commit_run(
    args: WithdrawCommitArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut options =
        WithdrawCommitOptions::new(&args.repo, CommitAssociationId::new(args.withdraws))
            .with_track(args.track);
    options = with_selection(options, args.revision);
    let (options, skip) = apply_signer(options, &args.repo, args.sign_key.as_deref(), stderr);
    let result = withdraw_commit(options)?;
    super::common::surface_best_effort_skip(&skip, stderr);
    let format =
        output::resolve_format(args.format_args.explicit(false), output::OutputFormat::Json)?;
    output::write_document_json_fallback(stdout, format, &withdraw_commit_document(result))
}

fn associate_ref_run(
    args: AssociateRefArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut options =
        AssociateRefOptions::new(&args.repo, args.ref_name, args.head).with_track(args.track);
    options = with_selection(options, args.revision);
    let (options, skip) = apply_signer(options, &args.repo, args.sign_key.as_deref(), stderr);
    let result = associate_ref(options)?;
    super::common::surface_best_effort_skip(&skip, stderr);
    let format =
        output::resolve_format(args.format_args.explicit(false), output::OutputFormat::Json)?;
    output::write_document_json_fallback(stdout, format, &associate_ref_document(result))
}

fn withdraw_ref_run(
    args: WithdrawRefArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut options = WithdrawRefOptions::new(&args.repo, RefAssociationId::new(args.withdraws))
        .with_track(args.track);
    options = with_selection(options, args.revision);
    let (options, skip) = apply_signer(options, &args.repo, args.sign_key.as_deref(), stderr);
    let result = withdraw_ref(options)?;
    super::common::surface_best_effort_skip(&skip, stderr);
    let format =
        output::resolve_format(args.format_args.explicit(false), output::OutputFormat::Json)?;
    output::write_document_json_fallback(stdout, format, &withdraw_ref_document(result))
}

fn list_run(args: ListArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty && !args.compact;
    let mut options = ListAssociationsOptions::new(&args.repo).current_only(args.current);
    if let Some(revision) = args.revision {
        options = options.with_revision_id(RevisionId::new(revision));
    }
    if let Some(axis) = args.axis {
        options = options.with_axis(axis.into());
    }
    let result = list_associations(options)?;
    let format = output::resolve_format(
        args.format_args.explicit(pretty),
        output::OutputFormat::Json,
    )?;
    // `list_associations_document` consumes the result by value; the text digest
    // reads the same data, so clone it only when the text lane will render (the
    // machine lanes never pay for the clone).
    let digest_source = matches!(format.format, output::OutputFormat::Text).then(|| result.clone());
    let document = list_associations_document(result);
    output::write_document(stdout, format, &document, || {
        let source = digest_source
            .as_ref()
            .expect("text lane resolves the digest source");
        render_association_text(source, landing_headline(source, &args.repo))
    })
}

/// Best-effort landing headline for the current commit set, derived from the same
/// liveness machinery `review show` uses and mapped exactly like the revision
/// list's `merge_status_for`: `merged|open|orphaned`, and `unknown` on any git
/// failure or withheld headline. Runs only on the text lane (the sole caller is
/// the text renderer) and never propagates a liveness error to the exit code
/// (INV-10). The machine lanes gain nothing — the JSON document is untouched.
fn landing_headline(result: &ListAssociationsResult, repo: &Path) -> &'static str {
    match enrich_liveness(&commit_range_view(result), repo, None) {
        Ok(enrichment) => match enrichment.headline {
            Some(CommitGraphCondition::Merged) => "merged",
            Some(CommitGraphCondition::Live) => "open",
            Some(CommitGraphCondition::Orphaned { .. }) => "orphaned",
            None => "unknown",
        },
        Err(_) => "unknown",
    }
}

/// Adapt the association-list result to the commit-range view the liveness seam
/// consumes. The two structs carry the same lifecycle fields; the view is the
/// shape `enrich_liveness` reads (only `current_commits` and `diagnostics`
/// participate in the headline).
fn commit_range_view(result: &ListAssociationsResult) -> RevisionCommitRangeView {
    RevisionCommitRangeView {
        revision_id: result.revision_id.clone(),
        anchored: result.anchored,
        current_commits: result.current_commits.clone(),
        current_refs: result.current_refs.clone(),
        withdrawn_commits: result.withdrawn_commits.clone(),
        withdrawn_refs: result.withdrawn_refs.clone(),
        diagnostics: result.diagnostics.clone(),
    }
}

/// The projection diagnostic code the digest translates into plain language.
/// Matched by value: the projection's `pub const` is library-private (not
/// re-exported), and the digest keys on the code, never the disposable message.
const DIVERGENT_COMMIT_ASSOCIATION_CODE: &str = "divergent_commit_association";

/// The text digest for `review association list`: the anchored state, current
/// commit/ref associations as short refs, withdrawn counts, a plain-language
/// divergence line, and the best-effort landing headline. Reads only the public
/// `ListAssociationsResult` (INV-12); ids truncate via `output::short_ref`
/// (INV-7); user-controlled ref/head strings are bounded via `clamp_title`.
/// `landing` is the caller-computed headline word; this renderer is pure
/// formatting and makes no git/liveness calls of its own.
fn render_association_text(result: &ListAssociationsResult, landing: &str) -> String {
    let mut lines: Vec<String> = Vec::new();

    let anchor = if result.anchored {
        "anchored"
    } else {
        "not anchored"
    };
    lines.push(format!(
        "{anchor} · {} · {}",
        count_label(
            result.current_commits.len(),
            "current commit association",
            "current commit associations",
        ),
        count_label(
            result.current_refs.len(),
            "current ref association",
            "current ref associations",
        ),
    ));

    for commit in &result.current_commits {
        lines.push(format!(
            "  commit {} ({})",
            output::short_ref(&commit.commit_oid),
            source_label(commit.source),
        ));
    }

    for reference in &result.current_refs {
        lines.push(format!(
            "  ref {} → {}",
            super::common::clamp_title(&reference.ref_name),
            super::common::clamp_title(&output::short_ref(&reference.head_oid)),
        ));
    }

    if !result.withdrawn_commits.is_empty() || !result.withdrawn_refs.is_empty() {
        lines.push(format!(
            "withdrawn: {} · {}",
            count_label(result.withdrawn_commits.len(), "commit", "commits"),
            count_label(result.withdrawn_refs.len(), "ref", "refs"),
        ));
    }

    if result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == DIVERGENT_COMMIT_ASSOCIATION_CODE)
    {
        lines.push(format!(
            "⚠ commit associations diverge: {} \
             (a squash or rebase may have rewritten the tip — withdraw the stale \
             one or associate the landed commit)",
            count_label(
                result.current_commits.len(),
                "current commit association",
                "current commit associations",
            ),
        ));
    }

    lines.push(format!("landing: {landing}"));

    lines.join("\n")
}

/// The current commit edge's provenance, in the result's own vocabulary.
fn source_label(source: CommitEdgeSource) -> &'static str {
    match source {
        CommitEdgeSource::CaptureTarget => "capture_target",
        CommitEdgeSource::Association => "association",
    }
}

/// `N noun`, singular when `count == 1`.
fn count_label(count: usize, singular: &str, plural: &str) -> String {
    let noun = if count == 1 { singular } else { plural };
    format!("{count} {noun}")
}

/// Apply the `--revision` selection shared by the write verbs. The four
/// write-options builders share the method name, so this is generic over a small
/// local trait.
fn with_selection<O: AssociationSelection>(mut options: O, revision: Option<String>) -> O {
    if let Some(revision) = revision {
        options = options.with_revision_id(RevisionId::new(revision));
    }
    options
}

fn apply_signer<O: SignableOptions>(
    mut options: O,
    repo: &Path,
    sign_key: Option<&str>,
    stderr: &mut dyn Write,
) -> (O, SigningSkip) {
    let mut skip = None;
    if let Some(resolved) = super::common::resolve_and_surface_signer(repo, sign_key, stderr) {
        let (signed, signer_skip) = super::common::apply_resolved_signer(options, resolved);
        options = signed;
        skip = signer_skip;
    }
    (options, skip)
}

trait AssociationSelection {
    fn with_revision_id(self, id: RevisionId) -> Self;
}

macro_rules! impl_association_selection {
    ($($ty:ty),+ $(,)?) => {$(
        impl AssociationSelection for $ty {
            fn with_revision_id(self, id: RevisionId) -> Self {
                <$ty>::with_revision_id(self, id)
            }
        }
    )+};
}

impl_association_selection!(
    AssociateCommitOptions,
    WithdrawCommitOptions,
    AssociateRefOptions,
    WithdrawRefOptions,
);

#[cfg(test)]
mod tests {
    use shoreline::model::RevisionId;
    use shoreline::session::{CurrentCommitAssociation, ListAssociationsResult};

    use super::*;

    fn anchored_result(commit_oid: &str) -> ListAssociationsResult {
        ListAssociationsResult {
            revision_id: RevisionId::new("rev:sha256:test"),
            anchored: true,
            current_commits: vec![CurrentCommitAssociation {
                commit_oid: commit_oid.to_owned(),
                tree_oid: format!("{commit_oid}-tree"),
                commit_association_id: None,
                source: CommitEdgeSource::CaptureTarget,
            }],
            current_refs: Vec::new(),
            withdrawn_commits: Vec::new(),
            withdrawn_refs: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn landing_headline_degrades_to_unknown_off_a_repo() {
        // A path that is not a git repository makes the liveness probe fail; the
        // headline degrades to `unknown` rather than propagating the error (INV-10).
        let dir = tempfile::tempdir().expect("create tempdir");
        assert_eq!(
            landing_headline(&anchored_result(&"0".repeat(40)), dir.path()),
            "unknown"
        );
    }
}
