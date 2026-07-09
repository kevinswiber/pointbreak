use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use pointbreak::documents::{observation_add_document, observation_list_document};
use pointbreak::model::{ObservationId, RevisionId};
use pointbreak::session::{
    ObservationAddOptions, ObservationListOptions, ObservationTargetSelector, list_observations,
    record_observation,
};

use crate::cli::common::{ContentTypeArg, SideArg, read_body_input};
use crate::cli::output;

#[derive(Debug, Args)]
pub(super) struct ObservationArgs {
    #[command(subcommand)]
    command: ObservationCommand,
}

#[derive(Debug, Subcommand)]
enum ObservationCommand {
    Add(ObservationAddArgs),
    List(ObservationListArgs),
}

/// Record an observation for a revision.
#[derive(Debug, Args)]
struct ObservationAddArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    revision: Option<String>,

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
    /// key file. Overrides SHORE_SIGNING_KEY. A key that cannot be loaded leaves
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
            review_observation_add(args, stdout, stderr)
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
    let (options, skip) = observation_add_options(args, stderr)?;
    let result = record_observation(options)?;
    crate::cli::common::surface_best_effort_skip(&skip, stderr);
    let document = observation_add_document(result);
    let format = output::resolve_format(format_explicit, output::OutputFormat::Json)?;
    output::write_document_json_fallback(stdout, format, &document)
}

fn review_observation_list(
    args: ObservationListArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let format_explicit = args.format_args.explicit();
    let repo = args.repo.clone();
    let result = list_observations(observation_list_options(args)?);
    let delegation_map = crate::cli::common::discover_delegation_map(&repo);
    let document = observation_list_document(result?, delegation_map.as_ref());
    let format = output::resolve_format(format_explicit, output::OutputFormat::Json)?;
    output::write_document_json_fallback(stdout, format, &document)
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
