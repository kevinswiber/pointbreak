use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::crypto::SignerId;

const USER_SIGNING_KEY_CONFIG: &str = "user.signingKey";
const GPG_FORMAT_CONFIG: &str = "gpg.format";
const ALLOWED_SIGNERS_FILE_CONFIG: &str = "gpg.ssh.allowedSignersFile";
const GIT_USER_SIGNING_KEY_CANDIDATE_ID: &str = "git-user-signing-key";
const GIT_SIGNING_KEY_SUGGESTED_NAME: &str = "git-signing-key";

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentDiscovery {
    pub candidates: Vec<EnrollmentCandidate>,
    pub diagnostics: Vec<EnrollmentDiscoveryDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentCandidate {
    pub id: String,
    pub source: EnrollmentCandidateSource,
    pub signer_id: SignerId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_argument: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_name: Option<String>,
    pub actor_hints: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnrollmentCandidateSource {
    GitUserSigningKey,
    GitAllowedSignersFile { path: PathBuf, line: usize },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentDiscoveryDiagnostic {
    pub code: EnrollmentDiscoveryDiagnosticCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<EnrollmentDiscoveryDiagnosticSource>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnrollmentDiscoveryDiagnosticSource {
    GitRepository {
        path: PathBuf,
    },
    GitConfig {
        #[serde(rename = "gitConfigKey")]
        git_config_key: String,
    },
    File {
        path: PathBuf,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentDiscoveryDiagnosticCode {
    GitRepositoryUnavailable,
    GitSshSigningNotConfigured,
    GitUserSigningKeyMissing,
    GitSigningKeyPublicKeyMissing,
    GitSigningKeyUnsupported,
    GitAllowedSignersFileUnreadable,
    GitAllowedSignersLineMalformed,
    GitAllowedSignersLineUnsupported,
    OpensshCertAuthorityUnsupported,
}

pub fn discover_enrollment_candidates(repo: &Path) -> EnrollmentDiscovery {
    let worktree_root = match crate::git::git_worktree_root(repo) {
        Ok(path) => path,
        Err(error) => {
            return EnrollmentDiscovery {
                candidates: Vec::new(),
                diagnostics: vec![diagnostic(
                    EnrollmentDiscoveryDiagnosticCode::GitRepositoryUnavailable,
                    format!("{} is not a usable Git worktree: {error}", repo.display()),
                    Some(EnrollmentDiscoveryDiagnosticSource::GitRepository {
                        path: repo.to_path_buf(),
                    }),
                )],
            };
        }
    };

    let mut discovery = EnrollmentDiscovery::default();
    let gpg_format = crate::git::git_config_get(&worktree_root, GPG_FORMAT_CONFIG);
    let user_signing_key = crate::git::git_config_get(&worktree_root, USER_SIGNING_KEY_CONFIG);
    let _allowed_signers_file =
        crate::git::git_config_get(&worktree_root, ALLOWED_SIGNERS_FILE_CONFIG);

    if !gpg_format
        .as_deref()
        .is_some_and(|format| format.eq_ignore_ascii_case("ssh"))
    {
        discovery.diagnostics.push(diagnostic(
            EnrollmentDiscoveryDiagnosticCode::GitSshSigningNotConfigured,
            "Git SSH signing is not configured; set gpg.format to ssh".to_owned(),
            Some(git_config_source(GPG_FORMAT_CONFIG)),
        ));
        return discovery;
    }

    let Some(signing_key) = user_signing_key else {
        discovery.diagnostics.push(diagnostic(
            EnrollmentDiscoveryDiagnosticCode::GitUserSigningKeyMissing,
            "Git SSH signing is enabled, but user.signingKey is not configured".to_owned(),
            Some(git_config_source(USER_SIGNING_KEY_CONFIG)),
        ));
        return discovery;
    };

    discover_user_signing_key(&worktree_root, &signing_key, &mut discovery);
    discovery
}

fn discover_user_signing_key(
    worktree_root: &Path,
    signing_key: &str,
    discovery: &mut EnrollmentDiscovery,
) {
    let trimmed = signing_key.trim();
    if trimmed.starts_with("key::") {
        match super::parse_ssh_ed25519_public_key(trimmed) {
            Ok(signer_id) => {
                discovery.candidates.push(git_user_signing_key_candidate(
                    signer_id,
                    trimmed.to_owned(),
                ));
            }
            Err(error) => discovery.diagnostics.push(diagnostic(
                EnrollmentDiscoveryDiagnosticCode::GitSigningKeyUnsupported,
                format!("Git user.signingKey is not a supported Ed25519 SSH key: {error}"),
                Some(git_config_source(USER_SIGNING_KEY_CONFIG)),
            )),
        }
        return;
    }

    if looks_like_unprefixed_ssh_key(trimmed) {
        discovery.diagnostics.push(diagnostic(
            EnrollmentDiscoveryDiagnosticCode::GitSigningKeyUnsupported,
            "Git user.signingKey inline SSH keys must use the key::ssh-ed25519 literal form"
                .to_owned(),
            Some(git_config_source(USER_SIGNING_KEY_CONFIG)),
        ));
        return;
    }

    let configured_path = resolve_config_path(worktree_root, trimmed);
    let public_path = if is_pub_path(&configured_path) {
        configured_path
    } else {
        public_companion_path(&configured_path)
    };

    let public_key = match std::fs::read_to_string(&public_path) {
        Ok(contents) => contents,
        Err(error) => {
            discovery.diagnostics.push(diagnostic(
                EnrollmentDiscoveryDiagnosticCode::GitSigningKeyPublicKeyMissing,
                format!(
                    "Git user.signingKey public key {} is not readable: {error}",
                    public_path.display()
                ),
                Some(file_source(public_path, None)),
            ));
            return;
        }
    };

    match super::parse_ssh_ed25519_public_key(&public_key) {
        Ok(signer_id) => discovery.candidates.push(git_user_signing_key_candidate(
            signer_id,
            public_path.display().to_string(),
        )),
        Err(error) => discovery.diagnostics.push(diagnostic(
            EnrollmentDiscoveryDiagnosticCode::GitSigningKeyUnsupported,
            format!(
                "Git user.signingKey public key {} is not a supported Ed25519 SSH key: {error}",
                public_path.display()
            ),
            Some(file_source(public_path, None)),
        )),
    }
}

fn git_user_signing_key_candidate(
    signer_id: SignerId,
    key_argument: String,
) -> EnrollmentCandidate {
    EnrollmentCandidate {
        id: GIT_USER_SIGNING_KEY_CANDIDATE_ID.to_owned(),
        source: EnrollmentCandidateSource::GitUserSigningKey,
        signer_id,
        key_argument: Some(key_argument),
        suggested_name: Some(GIT_SIGNING_KEY_SUGGESTED_NAME.to_owned()),
        actor_hints: Vec::new(),
    }
}

fn diagnostic(
    code: EnrollmentDiscoveryDiagnosticCode,
    message: String,
    source: Option<EnrollmentDiscoveryDiagnosticSource>,
) -> EnrollmentDiscoveryDiagnostic {
    EnrollmentDiscoveryDiagnostic {
        code,
        message,
        source,
    }
}

fn git_config_source(key: &str) -> EnrollmentDiscoveryDiagnosticSource {
    EnrollmentDiscoveryDiagnosticSource::GitConfig {
        git_config_key: key.to_owned(),
    }
}

fn file_source(path: PathBuf, line: Option<usize>) -> EnrollmentDiscoveryDiagnosticSource {
    EnrollmentDiscoveryDiagnosticSource::File { path, line }
}

fn resolve_config_path(worktree_root: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        worktree_root.join(path)
    }
}

fn is_pub_path(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "pub")
}

fn public_companion_path(path: &Path) -> PathBuf {
    let mut companion = path.as_os_str().to_os_string();
    companion.push(".pub");
    PathBuf::from(companion)
}

fn looks_like_unprefixed_ssh_key(value: &str) -> bool {
    value.starts_with("ssh-") || value.starts_with("ecdsa-") || value.starts_with("sk-ssh-")
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;
    use std::process::Command;

    use tempfile::TempDir;

    use super::*;

    const SSH_ED25519_PUBKEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAID7lnwK7O5CFXew1hBuUnXz1+zK2pQtYEtxsbRMiOyvP dev@example";

    struct TestRepo {
        root: TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let repo = Self {
                root: TempDir::new().expect("create temp git repository directory"),
            };
            repo.git(["init"]);
            repo.git(["symbolic-ref", "HEAD", "refs/heads/main"]);
            repo.git(["config", "user.name", "Pointbreak Tests"]);
            repo.git(["config", "user.email", "pointbreak-tests@example.com"]);
            repo.git(["config", "commit.gpgsign", "false"]);
            repo.git(["config", "gpg.format", ""]);
            repo.git(["config", "user.signingKey", ""]);
            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn config(&self, key: &str, value: &str) {
            self.git(["config", key, value]);
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            let args = args
                .into_iter()
                .map(|arg| arg.as_ref().to_owned())
                .collect::<Vec<_>>();
            let output = Command::new("git")
                .args(&args)
                .current_dir(self.path())
                .output()
                .unwrap_or_else(|error| {
                    panic!("run git {:?} in {}: {error}", args, self.path().display())
                });
            assert!(
                output.status.success(),
                "git {:?} failed in {}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
                args,
                self.path().display(),
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    fn git_literal() -> String {
        SSH_ED25519_PUBKEY
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ")
            .replacen("ssh-ed25519", "key::ssh-ed25519", 1)
    }

    fn assert_diagnostic(
        discovery: &EnrollmentDiscovery,
        expected: EnrollmentDiscoveryDiagnosticCode,
    ) {
        assert!(
            discovery
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == expected),
            "expected diagnostic {expected:?}; got {:#?}",
            discovery.diagnostics
        );
    }

    #[test]
    fn non_git_repo_reports_diagnostic_not_failure() {
        let dir = TempDir::new().expect("create non-git directory");

        let discovery = discover_enrollment_candidates(dir.path());

        assert!(discovery.candidates.is_empty());
        assert_diagnostic(
            &discovery,
            EnrollmentDiscoveryDiagnosticCode::GitRepositoryUnavailable,
        );
    }

    #[test]
    fn missing_ssh_signing_config_reports_diagnostic_not_failure() {
        let repo = TestRepo::new();

        let discovery = discover_enrollment_candidates(repo.path());

        assert!(discovery.candidates.is_empty());
        assert_diagnostic(
            &discovery,
            EnrollmentDiscoveryDiagnosticCode::GitSshSigningNotConfigured,
        );
    }

    #[test]
    fn missing_user_signing_key_reports_diagnostic_not_failure() {
        let repo = TestRepo::new();
        repo.config("gpg.format", "ssh");

        let discovery = discover_enrollment_candidates(repo.path());

        assert!(discovery.candidates.is_empty());
        assert_diagnostic(
            &discovery,
            EnrollmentDiscoveryDiagnosticCode::GitUserSigningKeyMissing,
        );
    }

    #[test]
    fn discovers_git_user_signing_key_literal_when_gpg_format_is_ssh() {
        let repo = TestRepo::new();
        let literal = git_literal();
        repo.config("gpg.format", "ssh");
        repo.config("user.signingKey", &literal);

        let discovery = discover_enrollment_candidates(repo.path());

        assert!(discovery.diagnostics.is_empty(), "{discovery:#?}");
        assert_eq!(discovery.candidates.len(), 1);
        let candidate = &discovery.candidates[0];
        assert_eq!(
            candidate.source,
            EnrollmentCandidateSource::GitUserSigningKey
        );
        assert_eq!(
            candidate.signer_id,
            crate::keys::parse_ssh_ed25519_public_key(&literal).unwrap()
        );
        assert_eq!(candidate.key_argument.as_deref(), Some(literal.as_str()));
    }

    #[test]
    fn discovers_pub_companion_for_private_user_signing_key_path() {
        let repo = TestRepo::new();
        let key_dir = TempDir::new().expect("create key directory");
        let private_path = key_dir.path().join("id_ed25519");
        let public_path = key_dir.path().join("id_ed25519.pub");
        std::fs::write(&private_path, "private key material is never read").unwrap();
        std::fs::write(&public_path, SSH_ED25519_PUBKEY).unwrap();
        repo.config("gpg.format", "ssh");
        repo.config("user.signingKey", private_path.to_str().unwrap());

        let discovery = discover_enrollment_candidates(repo.path());

        assert!(discovery.diagnostics.is_empty(), "{discovery:#?}");
        assert_eq!(discovery.candidates.len(), 1);
        let candidate = &discovery.candidates[0];
        assert_eq!(
            candidate.key_argument.as_deref(),
            Some(public_path.to_str().unwrap())
        );
        assert_ne!(
            candidate.key_argument.as_deref(),
            Some(private_path.to_str().unwrap())
        );
        assert_eq!(
            candidate.signer_id,
            crate::keys::parse_ssh_ed25519_public_key(SSH_ED25519_PUBKEY).unwrap()
        );
    }

    #[test]
    fn missing_pub_companion_is_a_diagnostic_not_a_failure() {
        let repo = TestRepo::new();
        let key_dir = TempDir::new().expect("create key directory");
        let private_path = key_dir.path().join("id_ed25519");
        repo.config("gpg.format", "ssh");
        repo.config("user.signingKey", private_path.to_str().unwrap());

        let discovery = discover_enrollment_candidates(repo.path());

        assert!(discovery.candidates.is_empty());
        assert_diagnostic(
            &discovery,
            EnrollmentDiscoveryDiagnosticCode::GitSigningKeyPublicKeyMissing,
        );
    }

    #[test]
    fn serializes_discovery_model_with_stable_field_names() {
        let literal = git_literal();
        let signer_id = crate::keys::parse_ssh_ed25519_public_key(&literal).unwrap();
        let discovery = EnrollmentDiscovery {
            candidates: vec![EnrollmentCandidate {
                id: "git-user-signing-key".to_owned(),
                source: EnrollmentCandidateSource::GitUserSigningKey,
                signer_id: signer_id.clone(),
                key_argument: Some(literal.clone()),
                suggested_name: Some("git-signing-key".to_owned()),
                actor_hints: Vec::new(),
            }],
            diagnostics: vec![EnrollmentDiscoveryDiagnostic {
                code: EnrollmentDiscoveryDiagnosticCode::GitUserSigningKeyMissing,
                message: "missing signing key".to_owned(),
                source: Some(EnrollmentDiscoveryDiagnosticSource::GitConfig {
                    git_config_key: "user.signingKey".to_owned(),
                }),
            }],
        };

        let value = serde_json::to_value(discovery).unwrap();

        assert_eq!(value["candidates"][0]["id"], "git-user-signing-key");
        assert_eq!(
            value["candidates"][0]["source"]["kind"],
            "git_user_signing_key"
        );
        assert_eq!(value["candidates"][0]["signerId"], signer_id.as_str());
        assert_eq!(value["candidates"][0]["keyArgument"], literal);
        assert_eq!(value["candidates"][0]["suggestedName"], "git-signing-key");
        assert_eq!(
            value["diagnostics"][0]["code"],
            "git_user_signing_key_missing"
        );
        assert_eq!(value["diagnostics"][0]["source"]["kind"], "git_config");
        assert_eq!(
            value["diagnostics"][0]["source"]["gitConfigKey"],
            "user.signingKey"
        );
    }
}
