use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Args;
use pointbreak::crypto::SignerId;
use pointbreak::keys::{
    EnrollmentCandidate, EnrollmentCandidateSource, EnrollmentDiscoveryDiagnostic,
    EnrollmentDiscoveryDiagnosticCode, KeyCustody, agent_has_key, discover_enrollment_candidates,
    list_keys,
};
use pointbreak::model::ActorId;
use serde::Serialize;

use crate::cli::common::{count_label, discover_trust_set};
use crate::cli::output;

#[derive(Debug, Args)]
pub(super) struct DiscoverArgs {
    /// Repository to inspect for advisory Git/OpenSSH signing evidence.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoverDocument {
    schema: &'static str,
    version: u32,
    candidates: Vec<DiscoverCandidate>,
    diagnostics: Vec<EnrollmentDiscoveryDiagnostic>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoverCandidate {
    id: String,
    source: EnrollmentCandidateSource,
    signer_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_argument: Option<String>,
    suggested_name: String,
    actor_hints: Vec<String>,
    local_keys: Vec<DiscoverLocalKey>,
    enrolled_actors: Vec<String>,
    resolved_actor: String,
    commands: Vec<Vec<String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoverLocalKey {
    name: String,
    default: bool,
    custody: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_loaded: Option<bool>,
}

struct LocalKeyState {
    signer_id: SignerId,
    output: DiscoverLocalKey,
}

struct DiscoverRenderContext {
    resolved_actor: ActorId,
    trust_set: pointbreak::session::TrustSet,
    local_keys: Vec<LocalKeyState>,
}

pub(super) fn run(
    args: DiscoverArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let discovery = discover_enrollment_candidates(&args.repo);
    let has_candidates = !discovery.candidates.is_empty();
    let context = DiscoverRenderContext::new(&args.repo);
    let candidates = discovery
        .candidates
        .into_iter()
        .map(|candidate| render_candidate(&args.repo, candidate, &context))
        .collect();
    let document = DiscoverDocument {
        schema: "pointbreak.key-discover",
        version: 1,
        candidates,
        diagnostics: filtered_diagnostics(discovery.diagnostics, has_candidates),
    };
    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    output::write_document(stdout, format, &document, || {
        render_key_discover_text(&document)
    })
}

/// Bespoke text lane for `key discover`: a count headline, one line per
/// candidate — signer, where the evidence came from, the suggested key name,
/// and its enrollment state — then the advisory diagnostics as hints. No
/// candidates renders a `no enrollment candidates` line, never silence.
fn render_key_discover_text(document: &DiscoverDocument) -> String {
    let mut lines = Vec::new();
    if document.candidates.is_empty() {
        lines.push("no enrollment candidates".to_owned());
    } else {
        lines.push(format!(
            "{}:",
            count_label(
                document.candidates.len(),
                "enrollment candidate",
                "enrollment candidates"
            )
        ));
        for candidate in &document.candidates {
            let mut line = format!(
                "  {} · {} · suggested name {}",
                candidate.signer_id,
                candidate_source_label(&candidate.source),
                candidate.suggested_name,
            );
            if candidate.enrolled_actors.is_empty() {
                line.push_str(" · not enrolled");
            } else {
                line.push_str(&format!(
                    " · enrolled for {}",
                    candidate.enrolled_actors.join(", ")
                ));
            }
            lines.push(line);
        }
    }
    for diagnostic in &document.diagnostics {
        lines.push(format!("hint: {}", diagnostic.message));
    }
    lines.join("\n")
}

/// Short readable label for where a candidate's signing evidence was found.
fn candidate_source_label(source: &EnrollmentCandidateSource) -> String {
    match source {
        EnrollmentCandidateSource::GitUserSigningKey => "git user.signingKey".to_owned(),
        EnrollmentCandidateSource::GitAllowedSignersFile { path, line } => {
            format!("allowed-signers {}:{line}", path.display())
        }
    }
}

impl DiscoverRenderContext {
    fn new(repo: &Path) -> Self {
        Self {
            resolved_actor: pointbreak::session::resolve_writer_actor_id(repo, None),
            trust_set: discover_trust_set(repo),
            local_keys: discover_local_key_state(),
        }
    }
}

fn render_candidate(
    repo: &Path,
    candidate: EnrollmentCandidate,
    context: &DiscoverRenderContext,
) -> DiscoverCandidate {
    let suggested_name = suggested_name(&candidate);
    let signer_id = candidate.signer_id.as_str().to_owned();
    let local_keys = matching_local_keys(&context.local_keys, &candidate.signer_id);
    let enrolled_actors = context
        .trust_set
        .actors_for_signer(&candidate.signer_id)
        .into_iter()
        .map(|actor| actor.as_str().to_owned())
        .collect::<Vec<_>>();
    let commands = suggested_commands(
        repo,
        &candidate.signer_id,
        candidate.key_argument.as_deref(),
        &suggested_name,
        &context.resolved_actor,
        !local_keys.is_empty(),
        context
            .trust_set
            .authorizes(&context.resolved_actor, &candidate.signer_id, ""),
    );

    DiscoverCandidate {
        id: candidate.id,
        source: candidate.source,
        signer_id,
        key_argument: candidate.key_argument,
        suggested_name,
        actor_hints: candidate.actor_hints,
        local_keys,
        enrolled_actors,
        resolved_actor: context.resolved_actor.as_str().to_owned(),
        commands,
    }
}

fn suggested_name(candidate: &EnrollmentCandidate) -> String {
    candidate
        .suggested_name
        .clone()
        .unwrap_or_else(|| match &candidate.source {
            EnrollmentCandidateSource::GitUserSigningKey => "git-signing-key".to_owned(),
            EnrollmentCandidateSource::GitAllowedSignersFile { line, .. } => {
                format!("allowed-signer-line-{line}")
            }
        })
}

fn suggested_commands(
    repo: &Path,
    signer_id: &SignerId,
    key_argument: Option<&str>,
    suggested_name: &str,
    resolved_actor: &ActorId,
    has_local_key: bool,
    resolved_actor_authorized: bool,
) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    if !has_local_key && let Some(key_argument) = key_argument {
        commands.push(vec![
            "pointbreak".to_owned(),
            "key".to_owned(),
            "use-ssh".to_owned(),
            key_argument.to_owned(),
            "--name".to_owned(),
            suggested_name.to_owned(),
        ]);
    }

    if !resolved_actor_authorized {
        commands.push(vec![
            "pointbreak".to_owned(),
            "key".to_owned(),
            "enroll".to_owned(),
            "--signer".to_owned(),
            signer_id.as_str().to_owned(),
            "--actor".to_owned(),
            resolved_actor.as_str().to_owned(),
            "--repo".to_owned(),
            repo.display().to_string(),
        ]);
    }
    commands
}

fn discover_local_key_state() -> Vec<LocalKeyState> {
    list_keys()
        .unwrap_or_default()
        .into_iter()
        .map(|info| {
            let signer_id = info.signer_id().clone();
            let (custody, agent_loaded) = match info.custody() {
                KeyCustody::File => ("file", None),
                KeyCustody::Agent => (
                    "agent",
                    signer_id.ed25519_public_key().ok().and_then(agent_has_key),
                ),
            };
            LocalKeyState {
                signer_id,
                output: DiscoverLocalKey {
                    name: info.name().to_owned(),
                    default: info.name() == "default",
                    custody,
                    agent_loaded,
                },
            }
        })
        .collect()
}

fn matching_local_keys(
    local_keys: &[LocalKeyState],
    signer_id: &SignerId,
) -> Vec<DiscoverLocalKey> {
    local_keys
        .iter()
        .filter(|key| key.signer_id == *signer_id)
        .map(|key| DiscoverLocalKey {
            name: key.output.name.clone(),
            default: key.output.default,
            custody: key.output.custody,
            agent_loaded: key.output.agent_loaded,
        })
        .collect()
}

fn filtered_diagnostics(
    diagnostics: Vec<EnrollmentDiscoveryDiagnostic>,
    has_candidates: bool,
) -> Vec<EnrollmentDiscoveryDiagnostic> {
    diagnostics
        .into_iter()
        .filter(|diagnostic| {
            !is_redundant_allowed_signers_option_diagnostic(diagnostic, has_candidates)
        })
        .collect()
}

fn is_redundant_allowed_signers_option_diagnostic(
    diagnostic: &EnrollmentDiscoveryDiagnostic,
    has_candidates: bool,
) -> bool {
    has_candidates
        && diagnostic.code == EnrollmentDiscoveryDiagnosticCode::GitAllowedSignersLineUnsupported
        && diagnostic
            .message
            .contains("OpenSSH allowed-signers options require richer trust semantics")
}
