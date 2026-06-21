use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};
use shoreline::documents::{
    associate_commit_document, associate_ref_document, list_associations_document,
    withdraw_commit_document, withdraw_ref_document,
};
use shoreline::model::{CommitAssociationId, RefAssociationId, ReviewUnitLineageId, RevisionId};
use shoreline::session::{
    AssociateCommitOptions, AssociateRefOptions, AssociationAxis, ListAssociationsOptions,
    WithdrawCommitOptions, WithdrawRefOptions, associate_commit, associate_ref, list_associations,
    withdraw_commit, withdraw_ref,
};

use crate::cli::json;
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

#[derive(Debug, Args)]
struct AssociateCommitArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    review_unit: Option<String>,

    #[arg(long)]
    lineage: Option<String>,

    #[arg(long)]
    track: String,

    /// The commit rev to associate with this review unit (resolved to an OID).
    #[arg(long)]
    commit: String,

    /// Sign this write with a specific key: a keystore key name or a path to a
    /// key file. Overrides SHORE_SIGNING_KEY. A key that cannot be loaded leaves
    /// the write unsigned (exit 0) with an advisory diagnostic — signing never
    /// blocks.
    #[arg(long)]
    sign_key: Option<String>,
}

#[derive(Debug, Args)]
struct WithdrawCommitArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    review_unit: Option<String>,

    #[arg(long)]
    lineage: Option<String>,

    #[arg(long)]
    track: String,

    /// The commit association id to withdraw.
    #[arg(long)]
    withdraws: String,

    #[arg(long)]
    sign_key: Option<String>,
}

#[derive(Debug, Args)]
struct AssociateRefArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    review_unit: Option<String>,

    #[arg(long)]
    lineage: Option<String>,

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
}

#[derive(Debug, Args)]
struct WithdrawRefArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    review_unit: Option<String>,

    #[arg(long)]
    lineage: Option<String>,

    #[arg(long)]
    track: String,

    /// The ref association id to withdraw.
    #[arg(long)]
    withdraws: String,

    #[arg(long)]
    sign_key: Option<String>,
}

#[derive(Debug, Args)]
struct ListArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    review_unit: Option<String>,

    #[arg(long)]
    lineage: Option<String>,

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
    options = with_selection(options, args.review_unit, args.lineage);
    let (options, skip) = apply_signer(options, &args.repo, args.sign_key.as_deref(), stderr);
    let result = associate_commit(options)?;
    super::common::surface_best_effort_skip(&skip, stderr);
    json::write_json(stdout, &associate_commit_document(result), false)
}

fn withdraw_commit_run(
    args: WithdrawCommitArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut options =
        WithdrawCommitOptions::new(&args.repo, CommitAssociationId::new(args.withdraws))
            .with_track(args.track);
    options = with_selection(options, args.review_unit, args.lineage);
    let (options, skip) = apply_signer(options, &args.repo, args.sign_key.as_deref(), stderr);
    let result = withdraw_commit(options)?;
    super::common::surface_best_effort_skip(&skip, stderr);
    json::write_json(stdout, &withdraw_commit_document(result), false)
}

fn associate_ref_run(
    args: AssociateRefArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut options =
        AssociateRefOptions::new(&args.repo, args.ref_name, args.head).with_track(args.track);
    options = with_selection(options, args.review_unit, args.lineage);
    let (options, skip) = apply_signer(options, &args.repo, args.sign_key.as_deref(), stderr);
    let result = associate_ref(options)?;
    super::common::surface_best_effort_skip(&skip, stderr);
    json::write_json(stdout, &associate_ref_document(result), false)
}

fn withdraw_ref_run(
    args: WithdrawRefArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut options = WithdrawRefOptions::new(&args.repo, RefAssociationId::new(args.withdraws))
        .with_track(args.track);
    options = with_selection(options, args.review_unit, args.lineage);
    let (options, skip) = apply_signer(options, &args.repo, args.sign_key.as_deref(), stderr);
    let result = withdraw_ref(options)?;
    super::common::surface_best_effort_skip(&skip, stderr);
    json::write_json(stdout, &withdraw_ref_document(result), false)
}

fn list_run(args: ListArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty && !args.compact;
    let mut options = ListAssociationsOptions::new(&args.repo).current_only(args.current);
    if let Some(review_unit) = args.review_unit {
        options = options.with_review_unit_id(RevisionId::new(review_unit));
    }
    if let Some(lineage) = args.lineage {
        options = options.with_lineage_id(ReviewUnitLineageId::new(lineage));
    }
    if let Some(axis) = args.axis {
        options = options.with_axis(axis.into());
    }
    let result = list_associations(options)?;
    json::write_json(stdout, &list_associations_document(result), pretty)
}

/// Apply the `--review-unit` / `--lineage` selection shared by the write verbs.
/// The four write-options builders share these method names, so this is generic
/// over a small local trait.
fn with_selection<O: AssociationSelection>(
    mut options: O,
    review_unit: Option<String>,
    lineage: Option<String>,
) -> O {
    if let Some(review_unit) = review_unit {
        options = options.with_review_unit_id(RevisionId::new(review_unit));
    }
    if let Some(lineage) = lineage {
        options = options.with_lineage_id(ReviewUnitLineageId::new(lineage));
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
    fn with_review_unit_id(self, id: RevisionId) -> Self;
    fn with_lineage_id(self, id: ReviewUnitLineageId) -> Self;
}

macro_rules! impl_association_selection {
    ($($ty:ty),+ $(,)?) => {$(
        impl AssociationSelection for $ty {
            fn with_review_unit_id(self, id: RevisionId) -> Self {
                <$ty>::with_review_unit_id(self, id)
            }
            fn with_lineage_id(self, id: ReviewUnitLineageId) -> Self {
                <$ty>::with_lineage_id(self, id)
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
