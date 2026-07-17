use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use pointbreak::documents::{observation_add_document, observation_list_document};
use pointbreak::model::{ObservationId, RevisionId};
use pointbreak::session::{
    ObservationAddOptions, ObservationListOptions, ObservationListResult,
    ObservationTargetSelector, list_observations, record_observation,
};

use crate::cli::common::{
    ContentTypeArg, SideArg, clamp_title, count_label, read_body_input, wire_label,
};
use crate::cli::output;

#[derive(Debug, Args)]
pub(super) struct ObservationArgs {
    #[command(subcommand)]
    command: ObservationCommand,
}

#[derive(Debug, Subcommand)]
enum ObservationCommand {
    Add(Box<ObservationAddArgs>),
    List(ObservationListArgs),
}

/// Record an observation for a revision.
#[derive(Debug, Args)]
struct ObservationAddArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Captured revision head seed.
    #[arg(long, conflicts_with = "exact_revision")]
    revision: Option<String>,

    /// Exact captured revision without following supersession.
    #[arg(long)]
    exact_revision: Option<String>,

    /// Review lane that owns this observation.
    #[arg(long)]
    track: String,

    #[arg(long)]
    title: String,

    #[arg(long, group = "observation_body")]
    body: Option<String>,

    #[arg(long, group = "observation_body")]
    body_file: Option<PathBuf>,

    #[arg(long, group = "observation_body")]
    body_stdin: bool,

    #[arg(long, value_enum, default_value = "text/plain")]
    body_content_type: ContentTypeArg,

    #[arg(long)]
    file: Option<String>,

    #[arg(long, value_enum, default_value = "new")]
    side: SideArg,

    #[arg(long)]
    start_line: Option<u32>,

    #[arg(long)]
    end_line: Option<u32>,

    #[arg(long = "tag")]
    tags: Vec<String>,

    #[arg(long, value_enum)]
    confidence: Option<ConfidenceArg>,

    #[arg(long = "supersedes")]
    supersedes: Vec<String>,

    #[arg(long = "responds-to")]
    responds_to: Vec<String>,

    #[arg(long)]
    idempotency_key: Option<String>,

    /// Sign this write with a specific key: a keystore key name or a path to a
    /// key file. Overrides POINTBREAK_SIGNING_KEY. A key that cannot be loaded leaves
    /// the write unsigned (exit 0) with an advisory diagnostic — signing never
    /// blocks.
    #[arg(long)]
    sign_key: Option<String>,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// List observations for a revision.
#[derive(Debug, Args)]
struct ObservationListArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    revision: Option<String>,

    /// Only list observations from this review lane.
    #[arg(long)]
    track: Option<String>,

    #[arg(long)]
    file: Option<String>,

    #[arg(long = "tag")]
    tags: Vec<String>,

    #[arg(long)]
    include_body: bool,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum ConfidenceArg {
    Low,
    Medium,
    High,
}

pub(super) fn run(
    args: ObservationArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        ObservationCommand::Add(args) => {
            let span = tracing::info_span!("shore.review.observation.add");
            let _entered = span.enter();
            tracing::debug!(command = "review.observation.add", "command_start");
            review_observation_add(*args, stdout, stderr)
        }
        ObservationCommand::List(args) => {
            let span = tracing::info_span!("shore.review.observation.list");
            let _entered = span.enter();
            tracing::debug!(command = "review.observation.list", "command_start");
            review_observation_list(args, stdout)
        }
    }
}

fn review_observation_add(
    args: ObservationAddArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let format_explicit = args.format_args.explicit();
    let title = args.title.clone();
    let (options, skip) = observation_add_options(args, stderr)?;
    let result = record_observation(options)?;
    crate::cli::common::surface_best_effort_skip(&skip, stderr);
    let format = output::resolve_format(format_explicit, output::OutputFormat::Json)?;
    // Bespoke text lane: a one-line receipt naming the recorded fact. Rendered
    // before the document builder consumes the result; machine lanes pay nothing.
    let text = matches!(format.format, output::OutputFormat::Text).then(|| {
        format!(
            "recorded observation {} · \"{}\" · track {} · {} created ({} existing)",
            output::short_ref(result.observation_id.as_str()),
            clamp_title(&title),
            result.track_id.as_str(),
            count_label(result.events_created, "event", "events"),
            result.events_existing,
        )
    });
    let document = observation_add_document(result);
    output::write_document(stdout, format, &document, || {
        text.expect("text lane resolves the digest source")
    })
}

fn review_observation_list(
    args: ObservationListArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let format_explicit = args.format_args.explicit();
    let repo = args.repo.clone();
    let result = list_observations(observation_list_options(args)?)?;
    let delegation_map = crate::cli::common::discover_delegation_map(&repo);
    let format = output::resolve_format(format_explicit, output::OutputFormat::Json)?;
    // `observation_list_document` consumes the result by value; render the digest
    // up front on the text lane only, so the machine lanes pay nothing extra.
    let text = matches!(format.format, output::OutputFormat::Text)
        .then(|| render_observation_list_text(&result));
    let document = observation_list_document(result, delegation_map.as_ref());
    output::write_document(stdout, format, &document, || {
        text.expect("text lane resolves the digest source")
    })
}

/// Bespoke text lane for `observation list`: a count headline naming the
/// revision and any active filters, then one scannable line per observation —
/// short id, clamped title, track, status, confidence, tags. An empty listing
/// renders a `no observations` line, never silence.
fn render_observation_list_text(result: &ObservationListResult) -> String {
    let mut scope = format!("on {}", output::short_ref(result.revision_id.as_str()));
    if let Some(track_id) = &result.filters.track_id {
        scope.push_str(&format!(" · track {}", track_id.as_str()));
    }
    if let Some(file) = &result.filters.file {
        scope.push_str(&format!(" · file {file}"));
    }
    if !result.filters.tags.is_empty() {
        scope.push_str(&format!(" · tags {}", result.filters.tags.join(", ")));
    }
    if result.observations.is_empty() {
        return format!("no observations {scope}");
    }
    let mut lines = vec![format!(
        "{} {scope}:",
        count_label(result.observations.len(), "observation", "observations")
    )];
    for view in &result.observations {
        let mut line = format!(
            "  {} · \"{}\" · {} · {}",
            output::short_ref(view.id.as_str()),
            clamp_title(&view.title),
            view.track_id.as_str(),
            wire_label(&view.status),
        );
        if let Some(confidence) = &view.confidence {
            line.push_str(&format!(" · {confidence}"));
        }
        if !view.tags.is_empty() {
            line.push_str(&format!(" · tags {}", view.tags.join(", ")));
        }
        lines.push(line);
    }
    lines.join("\n")
}

fn observation_add_options(
    args: ObservationAddArgs,
    stderr: &mut dyn Write,
) -> Result<(ObservationAddOptions, crate::cli::common::SigningSkip), Box<dyn std::error::Error>> {
    let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
    let target = observation_target(&args);
    let body = read_body_input(
        args.body.as_deref(),
        args.body_file.as_deref(),
        args.body_stdin,
    )?;
    let mut options = ObservationAddOptions::new(&args.repo)
        .with_track(args.track)
        .with_title(args.title)
        .with_target(target);

    if let Some(revision) = &args.revision {
        options = options.with_revision_id(RevisionId::new(ids.rev(revision)?));
    }
    if let Some(exact_revision) = &args.exact_revision {
        options = options.with_exact_revision_id(RevisionId::new(ids.rev(exact_revision)?));
    }
    if let Some(body) = body {
        options = options.with_body(body);
    }
    options = options.with_body_content_type(args.body_content_type.into());
    for tag in args.tags {
        options = options.with_tag(tag);
    }
    if let Some(confidence) = args.confidence {
        options = options.with_confidence(confidence.as_str());
    }
    for supersedes in &args.supersedes {
        options = options.superseding(ObservationId::new(ids.observation(supersedes)?));
    }
    for responds_to in &args.responds_to {
        options = options.responding_to(ObservationId::new(ids.observation(responds_to)?));
    }
    if let Some(idempotency_key) = args.idempotency_key {
        options = options.with_idempotency_key(idempotency_key);
    }
    let mut skip = None;
    if let Some(resolved) =
        crate::cli::common::resolve_and_surface_signer(&args.repo, args.sign_key.as_deref(), stderr)
    {
        let (signed, signer_skip) = crate::cli::common::apply_resolved_signer(options, resolved);
        options = signed;
        skip = signer_skip;
    }

    Ok((options, skip))
}

fn observation_list_options(
    args: ObservationListArgs,
) -> Result<ObservationListOptions, Box<dyn std::error::Error>> {
    let mut options = ObservationListOptions::new(&args.repo)
        .with_include_body(args.include_body)
        .with_trust_set(crate::cli::common::discover_trust_set(&args.repo));
    if let Some(revision) = &args.revision {
        let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
        options = options.with_revision_id(RevisionId::new(ids.rev(revision)?));
    }
    if let Some(track) = args.track {
        options = options.with_track(track);
    }
    if let Some(file) = args.file {
        options = options.with_file(file);
    }
    for tag in args.tags {
        options = options.with_tag(tag);
    }
    Ok(options)
}

fn observation_target(args: &ObservationAddArgs) -> ObservationTargetSelector {
    match (&args.file, args.start_line) {
        (Some(file), Some(start_line)) => ObservationTargetSelector::range(
            file.clone(),
            args.side.into(),
            start_line,
            args.end_line,
        ),
        (Some(file), None) => ObservationTargetSelector::file(file.clone()),
        (None, _) => ObservationTargetSelector::revision(),
    }
}

impl ConfidenceArg {
    fn as_str(self) -> &'static str {
        match self {
            ConfidenceArg::Low => "low",
            ConfidenceArg::Medium => "medium",
            ConfidenceArg::High => "high",
        }
    }
}
