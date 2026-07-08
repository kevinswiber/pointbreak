use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Args;
use pointbreak::keys::{
    EnrollmentCandidate, EnrollmentCandidateSource, EnrollmentDiscoveryDiagnostic,
    discover_enrollment_candidates,
};
use serde::Serialize;

use crate::cli::output;

#[derive(Debug, Args)]
pub(super) struct DiscoverArgs {
    /// Repository to inspect for advisory Git/OpenSSH signing evidence.
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
    commands: Vec<Vec<String>>,
}

pub(super) fn run(
    args: DiscoverArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let discovery = discover_enrollment_candidates(&args.repo);
    let candidates = discovery
        .candidates
        .into_iter()
        .map(|candidate| render_candidate(&args.repo, candidate))
        .collect();
    let document = DiscoverDocument {
        schema: "pointbreak.key-discover",
        version: 1,
        candidates,
        diagnostics: discovery.diagnostics,
    };
    let format = output::resolve_format(
        args.format_args.explicit(args.pretty),
        output::OutputFormat::Json,
    )?;
    output::write_document_json_fallback(stdout, format, &document)
}

fn render_candidate(repo: &Path, candidate: EnrollmentCandidate) -> DiscoverCandidate {
    let suggested_name = suggested_name(&candidate);
    let signer_id = candidate.signer_id.as_str().to_owned();
    let commands = suggested_commands(
        repo,
        &signer_id,
        candidate.key_argument.as_deref(),
        &suggested_name,
    );

    DiscoverCandidate {
        id: candidate.id,
        source: candidate.source,
        signer_id,
        key_argument: candidate.key_argument,
        suggested_name,
        actor_hints: candidate.actor_hints,
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
    signer_id: &str,
    key_argument: Option<&str>,
    suggested_name: &str,
) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    if let Some(key_argument) = key_argument {
        commands.push(vec![
            "shore".to_owned(),
            "key".to_owned(),
            "use-ssh".to_owned(),
            key_argument.to_owned(),
            "--name".to_owned(),
            suggested_name.to_owned(),
        ]);
    }

    commands.push(vec![
        "shore".to_owned(),
        "key".to_owned(),
        "enroll".to_owned(),
        "--signer".to_owned(),
        signer_id.to_owned(),
        "--actor".to_owned(),
        "<actor>".to_owned(),
        "--repo".to_owned(),
        repo.display().to_string(),
    ]);
    commands
}
