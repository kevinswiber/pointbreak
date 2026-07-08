use crate::crypto::SignerId;
use crate::keys::EnrollmentDiscoveryDiagnosticCode;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AllowedSignersParse {
    pub candidates: Vec<AllowedSignerCandidate>,
    pub diagnostics: Vec<AllowedSignersDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllowedSignerCandidate {
    pub line: usize,
    pub principal_hints: Vec<String>,
    pub signer_id: SignerId,
    pub key_argument: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllowedSignersDiagnostic {
    pub line: usize,
    pub code: EnrollmentDiscoveryDiagnosticCode,
    pub message: String,
}

pub fn parse_allowed_signers(input: &str) -> AllowedSignersParse {
    let mut parsed = AllowedSignersParse::default();

    for (index, raw_line) in input.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        parse_allowed_signers_line(line, line_number, &mut parsed);
    }

    parsed
}

fn parse_allowed_signers_line(line: &str, line_number: usize, parsed: &mut AllowedSignersParse) {
    let fields = line.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 3 {
        parsed.diagnostics.push(line_diagnostic(
            line_number,
            EnrollmentDiscoveryDiagnosticCode::GitAllowedSignersLineMalformed,
            "OpenSSH allowed-signers line must include principals, key type, and key blob",
        ));
        return;
    }

    let Some(key_index) = fields.iter().position(|field| looks_like_key_type(field)) else {
        parsed.diagnostics.push(line_diagnostic(
            line_number,
            EnrollmentDiscoveryDiagnosticCode::GitAllowedSignersLineMalformed,
            "OpenSSH allowed-signers line does not include a supported key type token",
        ));
        return;
    };

    if key_index > 1 {
        let options = &fields[1..key_index];
        let code = if options.contains(&"cert-authority") {
            EnrollmentDiscoveryDiagnosticCode::OpensshCertAuthorityUnsupported
        } else {
            EnrollmentDiscoveryDiagnosticCode::GitAllowedSignersLineUnsupported
        };
        parsed.diagnostics.push(line_diagnostic(
            line_number,
            code,
            "OpenSSH allowed-signers options require richer trust semantics and are not enrollment candidates",
        ));
        return;
    }

    let key_type = fields[key_index];
    if key_type != "ssh-ed25519" {
        parsed.diagnostics.push(line_diagnostic(
            line_number,
            EnrollmentDiscoveryDiagnosticCode::GitAllowedSignersLineUnsupported,
            "OpenSSH allowed-signers line uses a non-plain-Ed25519 key type",
        ));
        return;
    }

    let Some(blob) = fields.get(key_index + 1) else {
        parsed.diagnostics.push(line_diagnostic(
            line_number,
            EnrollmentDiscoveryDiagnosticCode::GitAllowedSignersLineMalformed,
            "OpenSSH allowed-signers Ed25519 line is missing its key blob",
        ));
        return;
    };

    let key_argument = format!("key::{key_type} {blob}");
    match super::parse_ssh_ed25519_public_key(&key_argument) {
        Ok(signer_id) => parsed.candidates.push(AllowedSignerCandidate {
            line: line_number,
            principal_hints: principal_hints(fields[0]),
            signer_id,
            key_argument,
        }),
        Err(error) => parsed.diagnostics.push(line_diagnostic(
            line_number,
            EnrollmentDiscoveryDiagnosticCode::GitAllowedSignersLineMalformed,
            format!("OpenSSH allowed-signers Ed25519 key is malformed: {error}"),
        )),
    }
}

fn principal_hints(principals: &str) -> Vec<String> {
    principals
        .split(',')
        .map(str::trim)
        .filter(|principal| !principal.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn looks_like_key_type(field: &str) -> bool {
    field.starts_with("ssh-")
        || field.starts_with("rsa-")
        || field.starts_with("ecdsa-")
        || field.starts_with("sk-ssh-")
}

fn line_diagnostic(
    line: usize,
    code: EnrollmentDiscoveryDiagnosticCode,
    message: impl Into<String>,
) -> AllowedSignersDiagnostic {
    AllowedSignersDiagnostic {
        line,
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SSH_ED25519_PUBKEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAID7lnwK7O5CFXew1hBuUnXz1+zK2pQtYEtxsbRMiOyvP dev@example";
    const SSH_RSA_PUBKEY: &str = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDIruRAxOrjtLtG0Rl4Ez7e0JmAuFFda/QvUwLWt6JucZlgRRfnJDfneTAzDzxQGpB+ok1ff8DovRHozcdn9nXO4bXZgx/8zb0bTqhm0y7Zn2qulvZ8lEBiUuJNRiBjy9pEcPxYYBuMP0dphQzPzSmNVeJvDO00cSvmEgeAdSUPAzIexM9ME3HTSXvt9CsV1QMCo8x/GwnEeJZHCkb2wWEs1oxv9EPrqp2y+dkAB+LFDcoeNMdHBeLzQh3w9pm2WaQsn9KGc6gK4edCeFn7ymcZ8GgNkmAJka4XxRcD+Fg7+3+r98ABtfSdvLuv/ddAQzZjruMP5Z0444anG3qsOtKf test@host";

    fn key_literal() -> String {
        SSH_ED25519_PUBKEY
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ")
            .replacen("ssh-ed25519", "key::ssh-ed25519", 1)
    }

    #[test]
    fn parses_plain_ed25519_allowed_signer_line() {
        let parsed = parse_allowed_signers(&format!("alice@example.com {SSH_ED25519_PUBKEY}\n"));

        assert!(parsed.diagnostics.is_empty(), "{parsed:#?}");
        assert_eq!(parsed.candidates.len(), 1);
        let candidate = &parsed.candidates[0];
        assert_eq!(candidate.line, 1);
        assert_eq!(candidate.principal_hints, vec!["alice@example.com"]);
        assert_eq!(candidate.key_argument, key_literal());
        assert_eq!(
            candidate.signer_id,
            crate::keys::parse_ssh_ed25519_public_key(SSH_ED25519_PUBKEY).unwrap()
        );
    }

    #[test]
    fn rejects_cert_authority_line_as_future_evidence() {
        let parsed = parse_allowed_signers(&format!(
            "alice@example.com cert-authority {SSH_ED25519_PUBKEY}\n"
        ));

        assert!(parsed.candidates.is_empty());
        assert!(
            parsed.diagnostics.iter().any(|diagnostic| {
                diagnostic.code
                    == crate::keys::EnrollmentDiscoveryDiagnosticCode::OpensshCertAuthorityUnsupported
            }),
            "expected cert-authority diagnostic; got {:#?}",
            parsed.diagnostics
        );
    }

    #[test]
    fn skips_comments_blank_lines_and_unsupported_key_types_with_diagnostics() {
        let parsed = parse_allowed_signers(&format!(
            "\n# comment\nalice@example.com {SSH_RSA_PUBKEY}\n\
             bob@example.com ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTY\n\
             carol@example.com sk-ssh-ed25519@openssh.com AAAAGnNrLXNzaC1lZDI1NTE5QG9wZW5zc2guY29t\n"
        ));

        assert!(parsed.candidates.is_empty());
        assert_eq!(parsed.diagnostics.len(), 3);
        assert!(parsed.diagnostics.iter().all(|diagnostic| {
            diagnostic.code
                == crate::keys::EnrollmentDiscoveryDiagnosticCode::GitAllowedSignersLineUnsupported
        }));
    }
}
