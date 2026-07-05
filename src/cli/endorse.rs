use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use serde::Serialize;
use shoreline::model::{ActorId, EventId};
use shoreline::session::{
    EventSignatureRecordOptions, EventSignatureRecordResult, record_event_signature,
    resolve_writer_actor_id,
};

use crate::cli::common::resolve_signer;
use crate::cli::json::DiagnosticDocument;
use crate::cli::output;

/// Record a detached co-signature (an endorsement) over an existing event.
#[derive(Debug, Args)]
pub(super) struct EndorseArgs {
    /// The event id to endorse: a full id, a prefixed short id (`evt:<hex>`), or
    /// a bare hex fragment inferred as an event id (any recorded event).
    target: String,
    /// Signing key (name in the keystore, or a path). Honors `SHORE_SIGNING_KEY`
    /// and the user-default key. UNLIKE ordinary writes, an endorsement has NO
    /// unsigned degrade: if no signer resolves, the command fails — the signature
    /// IS the endorsement's content.
    #[arg(long)]
    sign_key: Option<String>,
    /// Attribute the carrier's envelope writer to an explicit actor (defaults to the
    /// resolved writing actor — the endorser's own identity).
    #[arg(long)]
    actor: Option<String>,
    /// Repository root or a path inside it.
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    /// Pretty-print the JSON response.
    #[arg(long)]
    pretty: bool,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EndorseBody {
    event_id: String,
    target_event_id: String,
    target_event_record_hash: String,
    attesting_signer: String,
    actor_id: String,
    events_created: usize,
    events_existing: usize,
}

pub(super) fn run(
    args: EndorseArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    // The endorser's own actor: explicit --actor wins, else standard writer resolution. INV-D.
    let explicit = args.actor.as_deref().map(ActorId::new);
    let actor = resolve_writer_actor_id(&args.repo, explicit.as_ref());

    // Resolve the signer FIRST (so unsigned is a hard error regardless of target).
    // Reuse the shared CLI seam; surface its advisory diagnostic, then REQUIRE a signer.
    let resolution = resolve_signer(&args.repo, &actor, args.sign_key.as_deref());
    if let Some(diagnostic) = resolution.diagnostic.as_deref() {
        let _ = writeln!(stderr, "{diagnostic}");
    }
    let signer = resolution.signer.ok_or_else(|| -> Box<dyn std::error::Error> {
        "no signing key resolved: an endorsement has no unsigned form (the signature is its content). \
         Set --sign-key / SHORE_SIGNING_KEY, or run `shore keys init` and `shore keys enroll`."
            .into()
    })?;

    // Resolve the target only after the signer is in hand, so an invalid/ambiguous
    // fragment never masks the "no signer" hard error when both conditions hold.
    let ids = crate::cli::idresolve::IdResolver::new(&args.repo);
    let target = ids.event(&args.target)?;

    // The producer is used AS-IS: the resolved boxed signer is the attesting signer
    // (it composes through the blanket `impl EventSigner for Box<dyn EventSigner …>`);
    // the carrier's envelope writer is the endorser's own actor. INV-D. `mode`
    // (strict/best-effort) is irrelevant here: a sign-time failure propagates from
    // `record_event_signature` via `?` and becomes the same hard error — never a
    // silent degrade.
    let result: EventSignatureRecordResult = record_event_signature(
        EventSignatureRecordOptions::new(&args.repo, EventId::new(&target), signer)
            .with_actor_id(actor.clone()),
    )?;

    let body = EndorseBody {
        event_id: result.event_id.as_str().to_owned(),
        target_event_id: result.target_event_id.as_str().to_owned(),
        target_event_record_hash: result.target_event_record_hash,
        attesting_signer: result.attesting_signer.as_str().to_owned(),
        actor_id: actor.as_str().to_owned(),
        events_created: result.events_created,
        events_existing: result.events_existing,
    };
    let document = DiagnosticDocument::new("shore.review-endorse", body, Vec::new());
    let format = output::resolve_format(
        args.format_args.explicit(args.pretty),
        output::OutputFormat::Json,
    )?;
    output::write_document_json_fallback(stdout, format, &document)
}
