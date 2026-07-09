use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand};
use pointbreak::documents::attention_list_document;
use pointbreak::model::RevisionId;
use pointbreak::session::{
    AttentionDetail, AttentionItem, AttentionListOptions, AttentionListResult, AttentionTier,
    list_attention,
};

use crate::cli::common::clamp_title;
use crate::cli::output;

#[derive(Debug, Args)]
pub(super) struct AttentionArgs {
    #[command(subcommand)]
    command: AttentionCommand,
}

#[derive(Debug, Subcommand)]
enum AttentionCommand {
    List(AttentionListArgs),
}

/// List open asks and unresolved review state that need an actor's judgment.
#[derive(Debug, Args)]
struct AttentionListArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Scope to one revision: its anchored items plus the thread that covers it.
    #[arg(long)]
    revision: Option<String>,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

pub(super) fn run(
    args: AttentionArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        AttentionCommand::List(args) => {
            let span = tracing::info_span!("shore.attention.list");
            let _entered = span.enter();
            tracing::debug!(command = "attention.list", "command_start");
            attention_list(args, stdout)
        }
    }
}

fn attention_list(
    args: AttentionListArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let format_explicit = args.format_args.explicit();
    let format = output::resolve_format(format_explicit, output::OutputFormat::Json)?;

    let mut options = AttentionListOptions::new(&args.repo);
    if let Some(revision) = &args.revision {
        let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
        options = options.with_revision(RevisionId::new(ids.rev(revision)?));
    }

    let result = list_attention(options)?;
    // The text lane reads the same result the document consumes; clone it only
    // when that lane will render (eager-clone rule).
    let text_source = matches!(format.format, output::OutputFormat::Text).then(|| result.clone());
    let document = attention_list_document(result);
    output::write_document(stdout, format, &document, || {
        render_attention_list_text(
            text_source
                .as_ref()
                .expect("text lane resolves the attention source"),
        )
    })
}

/// Bespoke text lane for `attention list` (ADR-0029: text is disposable, never
/// byte-pinned). A count headline, then one scannable line per item — the tier
/// from the document's own field, the kebab kind label, and a shortened anchor
/// id. Items already sort primary-before-secondary, so the lines do too. An empty
/// projection renders a `nothing needs attention` line, never silence.
fn render_attention_list_text(result: &AttentionListResult) -> String {
    if result.items.is_empty() {
        return "nothing needs attention".to_owned();
    }
    let mut lines = vec![format!(
        "attention: {} item(s) need judgment:",
        result.items.len()
    )];
    for item in &result.items {
        lines.push(render_attention_item_line(item));
    }
    lines.join("\n")
}

fn render_attention_item_line(item: &AttentionItem) -> String {
    // The item id is `{kind}:{anchor}`; the kind already labels the line, so only
    // the anchor is shortened. The display kind is kebab (underscore -> hyphen).
    let (kind, anchor) = item.id.split_once(':').unwrap_or((item.id.as_str(), ""));
    let kind = kind.replace('_', "-");
    let anchor = output::short_ref(anchor);
    let tier = match item.tier {
        AttentionTier::Primary => "primary",
        AttentionTier::Secondary => "secondary",
    };
    let mut line = format!("  [{tier}] {kind}  {anchor}");
    if let AttentionDetail::OpenInputRequest { title, .. } = &item.detail {
        line.push_str("  ");
        line.push_str(&clamp_title(title));
    }
    line
}
