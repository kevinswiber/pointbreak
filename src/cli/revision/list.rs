use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use pointbreak::documents::revision_list_document;
use pointbreak::session::{
    OrphanVisibility, QueryDiagnosticCode, QuerySurface, RefFilterMode, RevisionListOptions,
    RevisionOverviewsOptions, RevisionRecordInputs, SupersessionView, build_revision_search_record,
    list_revisions, matches_query, parse_search_query_for, read_events_for_display,
    revision_supersession_classification, show_revision_overviews,
};

use crate::cli::common::{count_label, endpoint_label};
use crate::cli::output;

/// List captured revisions.
#[derive(Debug, Args)]
pub(super) struct RevisionListArgs {
    /// Repository root or a path inside the repository.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// List only the revisions sharing this content object id (a listing lens —
    /// coincident content may span threads; never a head-selector).
    #[arg(long)]
    object: Option<String>,

    /// Filter with the review filter grammar (e.g. `is:superseded tag:state-change`).
    /// When set, builds per-revision overviews + supersession classification for the
    /// listed revisions; the flagless listing pays no such cost.
    #[arg(long)]
    filter: Option<String>,

    /// Filter to revisions associated with this ref; a short branch name is
    /// normalized to its full ref before matching.
    #[arg(long = "ref", alias = "branch")]
    ref_name: Option<String>,

    /// How `--ref` matches: by the recorded label (offline) or by reachability
    /// from the ref's live tip.
    #[arg(long, value_enum, default_value = "label")]
    by: RefFilterByArg,

    /// Show every recorded revision. This is the default; the flag is retained
    /// for command compatibility.
    #[arg(long = "all")]
    _all: bool,

    /// Show only orphaned revisions (every anchored commit unreachable). Takes
    /// precedence over `--all`.
    #[arg(long)]
    orphans: bool,

    /// Reachability target for the "merged" status: a revision is merged only when
    /// an ancestor of this ref (equality counts). Defaults to the repository's
    /// detected default branch (`origin/HEAD`, else local `main`/`master`), so the
    /// status answers "did this land on the default branch?"; when no default
    /// branch is detected it falls back to broad reachability (any live tip).
    #[arg(long = "integration-ref")]
    integration_ref: Option<String>,

    /// Scope the listing to captures belonging to the worktree at this path.
    #[arg(long)]
    worktree: Option<PathBuf>,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum RefFilterByArg {
    #[default]
    Label,
    Liveness,
}

impl From<RefFilterByArg> for RefFilterMode {
    fn from(by: RefFilterByArg) -> Self {
        match by {
            RefFilterByArg::Label => RefFilterMode::Label,
            RefFilterByArg::Liveness => RefFilterMode::Liveness,
        }
    }
}

pub(super) fn run(
    args: RevisionListArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.revision.list");
    let _entered = span.enter();
    tracing::debug!(command = "revision.list", "command_start");

    let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
    let object = match &args.object {
        Some(object) => Some(ids.object(object)?),
        None => None,
    };

    let mut options = RevisionListOptions::new(&args.repo).with_read_for_display(true);
    if let Some(ref_name) = args.ref_name {
        options = options.with_ref_filter(ref_name, args.by.into());
    }
    // `--all` is now the default and remains accepted for compatibility.
    // `--orphans` is the only CLI reachability filter and keeps precedence when
    // both flags are supplied.
    let visibility = if args.orphans {
        OrphanVisibility::OrphansOnly
    } else {
        OrphanVisibility::All
    };
    options = options.with_orphan_visibility(visibility);
    if let Some(integration_ref) = args.integration_ref {
        options = options.with_integration_ref(integration_ref);
    }
    if let Some(worktree) = args.worktree {
        options = options.with_worktree_scope(worktree);
    }
    let mut result = list_revisions(options)?;

    // `--object` is a listing lens: filter to revisions over the same content
    // object id (coincident content, which may span threads). It never resolves a
    // head and never force-disambiguates.
    if let Some(object) = object.as_deref() {
        result
            .entries
            .retain(|entry| entry.object_id.as_str() == object);
        result.revision_count = result.entries.len();
    }

    apply_revision_filter(&args.repo, args.filter.as_deref(), &mut result)?;

    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    // `revision_list_document` consumes the result by value; render the digest
    // up front on the text lane only, so the machine lanes pay nothing extra.
    let text = matches!(format.format, output::OutputFormat::Text)
        .then(|| render_revision_list_text(&result));
    let document = revision_list_document(result);
    output::write_document(stdout, format, &document, || {
        text.expect("text lane resolves the digest source")
    })
}

/// Bespoke text lane for `revision list`: a count headline, then one scannable
/// line per listed row — short id, endpoints, merge status, capture time, and
/// the member count for a grouped row. An empty listing renders a
/// `no revisions` line, never silence.
fn render_revision_list_text(result: &pointbreak::session::RevisionListResult) -> String {
    if result.entries.is_empty() {
        return "no revisions".to_owned();
    }
    let mut lines = vec![format!(
        "{}:",
        count_label(result.revision_count, "revision", "revisions")
    )];
    for entry in &result.entries {
        let mut line = format!(
            "  {} · base {} → {} · {} · {}",
            output::short_ref(entry.revision_id.as_str()),
            endpoint_label(&entry.base),
            endpoint_label(&entry.target),
            entry.merge_status,
            entry.captured_at,
        );
        let members = entry.grouped_revision_ids.len();
        if members > 1 {
            line.push_str(&format!(
                " · stands for {}",
                count_label(members, "revision", "revisions")
            ));
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// Apply the review filter grammar on the revision surface, in place. A no-op
/// unless `--filter` is set, so the flagless listing pays no new cost and stays
/// byte-identical: the per-revision overviews and supersession classification are
/// built only inside this branch. Overviews are keyed by the representative
/// `revision_id`, so a grouped row filters on its representative's
/// overview/classification (the row stands for the group).
fn apply_revision_filter(
    repo: &Path,
    filter: Option<&str>,
    result: &mut pointbreak::session::RevisionListResult,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(filter) = filter else {
        return Ok(());
    };

    let overviews = show_revision_overviews(
        RevisionOverviewsOptions::new(repo)
            .with_revisions(result.entries.iter().map(|e| e.revision_id.clone()))
            .with_trust_set(crate::cli::common::discover_trust_set(repo))
            .with_read_for_display(true),
    )?;
    let (events, _) = read_events_for_display(repo)?;
    let classification =
        revision_supersession_classification(&SupersessionView::from_events(&events)?);

    // Parse on the revision surface. A known-but-unsupported qualifier or value is
    // a usage error (non-zero exit carrying the message); a deprecated qualifier
    // keeps running behind a stderr hint.
    let parsed = parse_search_query_for(filter, QuerySurface::Revision);
    for diagnostic in &parsed.diagnostics {
        match diagnostic.code {
            QueryDiagnosticCode::UnsupportedQualifier | QueryDiagnosticCode::UnsupportedValue => {
                return Err(diagnostic.message.clone().into());
            }
            QueryDiagnosticCode::DeprecatedQualifier => eprintln!("hint: {}", diagnostic.message),
        }
    }

    result.entries.retain(|entry| {
        let Some(overview) = overviews.get(&entry.revision_id) else {
            return false;
        };
        let facet = classification.get(&entry.revision_id);
        let record = build_revision_search_record(RevisionRecordInputs {
            entry,
            overview,
            classification_state: facet.map(|f| f.state).unwrap_or("isolated"),
            competing: facet.is_some_and(|f| f.competing),
        });
        matches_query(&record.0, &parsed.clauses)
    });
    result.revision_count = result.entries.len();
    Ok(())
}
