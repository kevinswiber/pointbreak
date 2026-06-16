use std::io::{Read, Write};
use std::path::Path;

use clap::ValueEnum;
use shoreline::keys::{
    FileEd25519Signer, KeyHandle, KeyName, generate_key, generate_key_in, load_signer,
    load_signer_from_path, load_signer_in,
};
use shoreline::model::{ActorId, Side};
use shoreline::session::{DelegationMap, TrustSet, is_agent_actor_id, resolve_writer_actor_id};

/// Discover the layered delegation map under `<worktree-root>/.shore/`.
///
/// Two files compose, git-config style: the committed shared default
/// `.shore/delegates.json` and a locally-excluded private override
/// `.shore/delegates.local.json`. The local file's records for an agent fully
/// replace the committed records for that agent (via
/// [`DelegationMap::with_local_override`]); agents absent from the local file
/// inherit the committed map; either file may exist alone.
///
/// `repo` may be the worktree root or any path inside it, matching what the read
/// commands accept — so discovery resolves the worktree root first (the same root
/// the store resolves against) before joining the delegates paths; a non-git
/// context (e.g. an exported bundle) falls back to `repo` as given, per the ADR's
/// "works in any non-git context".
///
/// Presence is per-file: when **neither** file exists, returns `None` (zero-setup
/// stores see zero change). A malformed file is **advisory** — a one-line warning
/// to stderr names the parse error and that file is treated as absent, so a bad
/// local override never poisons the committed default (ADR-0003). Shared by every
/// review read command and the inspector server.
pub(crate) fn discover_delegation_map(repo: &Path) -> Option<DelegationMap> {
    let worktree_root =
        shoreline::git::git_worktree_root(repo).unwrap_or_else(|_| repo.to_path_buf());
    let committed = load_optional_delegates(&worktree_root.join(".shore/delegates.json"));
    let local = load_optional_delegates(&worktree_root.join(".shore/delegates.local.json"));
    match (committed, local) {
        (None, None) => None,
        (committed, local) => Some(
            committed
                .unwrap_or_default()
                .with_local_override(local.unwrap_or_default()),
        ),
    }
}

/// Load a delegates file if present; a malformed file is advisory — warn once to
/// stderr and treat it as absent (per-file, so a bad local never poisons the
/// committed default).
fn load_optional_delegates(path: &Path) -> Option<DelegationMap> {
    if !path.exists() {
        return None;
    }
    match DelegationMap::from_delegates_file(path) {
        Ok(map) => Some(map),
        Err(error) => {
            eprintln!("warning: ignoring {}: {error}", path.display());
            None
        }
    }
}

/// Discover the committed signature allow-list under `<worktree-root>/.shore/`.
///
/// Symmetric to [`discover_delegation_map`]: `repo` may be the worktree root or
/// any path inside it, so discovery resolves the worktree root first (a non-git
/// context falls back to `repo` as given), then loads `.shore/allowed-signers.json`
/// — the custom Shoreline JSON allow-list (`{"allowedSigners": {...}}`), not the
/// OpenSSH `allowed_signers` format. An absent file yields the empty
/// `TrustSet::default()` (zero-setup stores see no change); a malformed file is
/// advisory — a one-line stderr warning, then the empty default (a bad allow-list
/// never aborts a read). Shared by the verifying read commands so an enrolled
/// signer's events render `valid` rather than `untrusted_key`. This is
/// reader-supplied trust config, never store content.
pub(crate) fn discover_trust_set(repo: &Path) -> TrustSet {
    let worktree_root =
        shoreline::git::git_worktree_root(repo).unwrap_or_else(|_| repo.to_path_buf());
    load_optional_trust_set(&worktree_root.join(".shore/allowed-signers.json")).unwrap_or_default()
}

/// Load the allow-list if present; a malformed file is advisory — warn once to
/// stderr and treat it as absent (returns `None`, so the caller uses the empty
/// default).
fn load_optional_trust_set(path: &Path) -> Option<TrustSet> {
    if !path.exists() {
        return None;
    }
    match TrustSet::from_allowed_signers_file(path) {
        Ok(trust) => Some(trust),
        Err(error) => {
            eprintln!("warning: ignoring {}: {error}", path.display());
            None
        }
    }
}

/// Environment override selecting signing mode: `off` disables signing entirely,
/// `auto` (the default) resolves a signer where possible.
const SHORE_SIGNING_ENV: &str = "SHORE_SIGNING";
/// Environment override naming the signing key (a keystore key name or a path).
const SHORE_SIGNING_KEY_ENV: &str = "SHORE_SIGNING_KEY";

// Diagnostic codes — single-sourced so the write-path stderr surfacing and the
// docs reference one spelling. Each diagnostic embeds its code as the leading
// token so callers can assert on `.contains("<code>")`.
const SIGNING_KEY_UNREADABLE: &str = "signing_key_unreadable";
const SIGNING_KEY_UNSUPPORTED_ALGORITHM: &str = "signing_key_unsupported_algorithm";
const SIGNING_KEY_HOME_UNREADABLE: &str = "signing_key_home_unreadable";
const SIGNING_MODE_UNRECOGNIZED: &str = "signing_mode_unrecognized";

/// What [`resolve_signer`] decided: an optional already-loaded production signer
/// plus an optional one-line diagnostic naming the reason no signer resolved (or
/// the configured-but-broken key). Never carries an error — a failure to resolve
/// is a `None` signer with a diagnostic, never an `Err`.
pub(crate) struct SignerResolution {
    pub(crate) signer: Option<FileEd25519Signer>,
    pub(crate) diagnostic: Option<String>,
}

/// CLI-layer signer resolution. **Never returns `Err`** — every failure degrades
/// to no signer plus a named diagnostic, so a write can proceed unsigned (exit
/// 0). All fallible key work happens here, before any `.sign_with`, which is why
/// signing never gates a write: the library signing seam
/// (`sign_event_if_requested`) propagates errors via `?`, so resolution must be
/// done — and only ever yield a known-good signer — ahead of the signing call.
///
/// Precedence: per-call `--sign-key` > `SHORE_SIGNING_KEY` env > agent-context
/// auto-keygen > user-default `default` key > none. `SHORE_SIGNING=off`
/// short-circuits to none. An **explicitly** selected key (`--sign-key` /
/// `SHORE_SIGNING_KEY`) that fails to load is terminal — `None` + diagnostic,
/// never a silent fall-through to a different identity.
pub(crate) fn resolve_signer(
    repo: &Path,
    actor: &ActorId,
    sign_key: Option<&str>,
) -> SignerResolution {
    resolve_signer_with_env(
        repo,
        actor,
        sign_key,
        std::env::var(SHORE_SIGNING_ENV).ok().as_deref(),
        std::env::var(SHORE_SIGNING_KEY_ENV).ok().as_deref(),
        None, // production: key lookups resolve keys_dir(); tests inject Some(tempdir)
    )
}

/// Resolve the signer for a write and surface any advisory diagnostic to
/// `stderr`, returning the signer to apply (or `None` for an unsigned write).
/// The single integration point the six write builders share: it resolves the
/// writing actor exactly as the library will attribute it, picks a signer, prints
/// the broken-key / agent-keygen notice advisorily, and never affects the exit
/// code — signing never gates a write.
pub(crate) fn resolve_and_surface_signer(
    repo: &Path,
    sign_key: Option<&str>,
    stderr: &mut dyn Write,
) -> Option<FileEd25519Signer> {
    let actor = resolve_writer_actor_id(repo, None);
    let resolution = resolve_signer(repo, &actor, sign_key);
    if let Some(diagnostic) = resolution.diagnostic.as_deref() {
        let _ = writeln!(stderr, "{diagnostic}");
    }
    resolution.signer
}

/// Pure resolution seam (env values AND the keystore root threaded in for
/// testability, so unit tests never mutate the process environment).
fn resolve_signer_with_env(
    repo: &Path,
    actor: &ActorId,
    sign_key: Option<&str>,
    shore_signing: Option<&str>,
    shore_signing_key: Option<&str>,
    keys_root: Option<&Path>,
) -> SignerResolution {
    // Mode override first: `off` is an explicit opt-out.
    if let Some(mode) = shore_signing
        && mode.eq_ignore_ascii_case("off")
    {
        return SignerResolution {
            signer: None,
            diagnostic: None,
        };
    }

    // Rung 1/2: the highest-precedence EXPLICIT key selection (flag first, then
    // env). An explicit selection is TERMINAL: if it fails to load, return None +
    // the named diagnostic and STOP — never fall through to agent-keygen or the
    // default key, which would sign under a different identity than the one named.
    if let Some(candidate) = [sign_key, shore_signing_key].into_iter().flatten().next() {
        return match load_configured_signer(candidate, keys_root) {
            Ok(signer) => SignerResolution {
                signer: Some(signer),
                diagnostic: mode_note(shore_signing),
            },
            Err(diagnostic) => SignerResolution {
                signer: None,
                diagnostic: Some(diagnostic),
            },
        };
    }

    // Rung 3 (only reached when NO explicit key was selected): agent-context
    // auto-keygen hook (filled by the auto-keygen task). Threads `keys_root` so
    // that task can test keygen against an injected root.
    if is_agent_actor_id(actor.as_str())
        && let Some(resolution) = resolve_agent_signer(repo, actor, keys_root)
    {
        return resolution;
    }

    // Rung 4: user-default keystore key ("default"), if present. A missing default
    // is the clean unsigned path, not a failure.
    SignerResolution {
        signer: load_default_signer(keys_root),
        diagnostic: mode_note(shore_signing),
    }
}

/// Load a key named by the flag/env: a keystore name, or a filesystem path. An
/// unsupported algorithm or an unreadable/malformed file is a named, non-fatal
/// failure (`signing_key_unsupported_algorithm` / `signing_key_unreadable`). When
/// `keys_root` is `Some`, keystore-name lookups use the injected root (tests).
fn load_configured_signer(
    candidate: &str,
    keys_root: Option<&Path>,
) -> std::result::Result<FileEd25519Signer, String> {
    let path = Path::new(candidate);
    // A candidate that contains a path separator, or exists as a file on disk,
    // loads by explicit path; otherwise it is a keystore key name (validated by the
    // keystore loader so a bare name can never traverse outside the key home).
    let by_path =
        candidate.contains('/') || candidate.contains(std::path::MAIN_SEPARATOR) || path.is_file();
    let loaded = if by_path {
        load_signer_from_path(path)
    } else {
        match keys_root {
            Some(root) => load_signer_in(root, candidate),
            None => load_signer(candidate),
        }
    };
    loaded.map_err(|error| classify_signing_key_error(&error.to_string(), candidate))
}

/// Classify a keystore load failure into a code-bearing diagnostic. A key file
/// whose `alg` field is not `ed25519` is `signing_key_unsupported_algorithm`;
/// every other failure (missing/unreadable/malformed) is `signing_key_unreadable`.
fn classify_signing_key_error(message: &str, candidate: &str) -> String {
    if message.contains("unsupported key algorithm") {
        format!("{SIGNING_KEY_UNSUPPORTED_ALGORITHM}: signing key {candidate:?}: {message}")
    } else {
        format!("{SIGNING_KEY_UNREADABLE}: signing key {candidate:?}: {message}")
    }
}

/// Load the user-default ("default") keystore key, honoring an injected root. A
/// missing default is the clean unsigned path (`None`), not a failure.
fn load_default_signer(keys_root: Option<&Path>) -> Option<FileEd25519Signer> {
    match keys_root {
        Some(root) => load_signer_in(root, "default").ok(),
        None => load_signer("default").ok(),
    }
}

/// Agent-context auto-keygen: when an agent actor has no signer resolved through
/// the explicit-key rungs, load (or on first write, silently generate) a
/// passphrase-less per-machine key named from the actor slug and return it as the
/// signer. A freshly generated key carries a one-line enrollment notice (the
/// write path prints it to stderr); a reused key is silent. Any keygen/load
/// failure degrades to no signer plus an advisory diagnostic — never an error —
/// keeping the write unsigned and exit 0. Threads `keys_root` so tests inject a
/// root (None = production `keys_dir()`).
fn resolve_agent_signer(
    _repo: &Path,
    actor: &ActorId,
    keys_root: Option<&Path>,
) -> Option<SignerResolution> {
    let name = agent_key_name(actor.as_str());

    // Reuse an existing per-machine key without re-minting or re-notifying.
    if let Ok(signer) = load_agent_key(keys_root, &name) {
        return Some(SignerResolution {
            signer: Some(signer),
            diagnostic: None,
        });
    }

    // First write for this agent: mint silently, then load.
    match generate_agent_key(keys_root, &name) {
        Ok(handle) => {
            let did = handle.signer_id().as_str().to_owned();
            match load_agent_key(keys_root, &name) {
                Ok(signer) => Some(SignerResolution {
                    signer: Some(signer),
                    // The write path surfaces this as the one-line stderr notice.
                    diagnostic: Some(format!(
                        "shore: generated signing key for {} ({did}); \
                         run `shore keys enroll` to stage trust",
                        actor.as_str()
                    )),
                }),
                Err(_) => Some(SignerResolution {
                    signer: None,
                    diagnostic: Some(SIGNING_KEY_HOME_UNREADABLE.to_owned()),
                }),
            }
        }
        // Read-only / unresolvable key home, or any I/O error: unsigned, never an error.
        Err(_) => Some(SignerResolution {
            signer: None,
            diagnostic: Some(SIGNING_KEY_HOME_UNREADABLE.to_owned()),
        }),
    }
}

/// Derive a stable, filesystem-safe per-machine key name from an agent actor id.
/// `actor:agent:claude-code` -> `agent-claude-code`. Non-`[a-z0-9_]` runs collapse
/// to a single `-` and edges are trimmed, so the name can never escape the
/// keystore directory.
fn agent_key_name(actor: &str) -> String {
    let slug = actor.strip_prefix("actor:agent:").unwrap_or(actor);
    let mut safe = String::with_capacity(slug.len());
    let mut last_dash = false;
    for character in slug.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            safe.push(character);
            last_dash = false;
        } else if !last_dash {
            safe.push('-');
            last_dash = true;
        }
    }
    format!("agent-{}", safe.trim_matches('-'))
}

/// Load an agent key honoring an injected root (None = production `keys_dir()`).
fn load_agent_key(
    keys_root: Option<&Path>,
    name: &str,
) -> std::result::Result<FileEd25519Signer, String> {
    let loaded = match keys_root {
        Some(root) => load_signer_in(root, name),
        None => load_signer(name),
    };
    loaded.map_err(|error| error.to_string())
}

/// Generate an agent key honoring an injected root. `agent_key_name` already
/// produced a safe slug; `KeyName::parse` re-validates it.
fn generate_agent_key(
    keys_root: Option<&Path>,
    name: &str,
) -> std::result::Result<KeyHandle, String> {
    let key_name = KeyName::parse(name).map_err(|error| error.to_string())?;
    let generated = match keys_root {
        Some(root) => generate_key_in(root, &key_name),
        None => generate_key(name),
    };
    generated.map_err(|error| error.to_string())
}

/// An unrecognized `SHORE_SIGNING` value is advisory, never an error: it is
/// treated as `auto` with a `signing_mode_unrecognized` note.
fn mode_note(shore_signing: Option<&str>) -> Option<String> {
    match shore_signing {
        Some(mode) if mode.eq_ignore_ascii_case("auto") || mode.eq_ignore_ascii_case("off") => None,
        Some(other) => Some(format!(
            "{SIGNING_MODE_UNRECOGNIZED}: {other:?} treated as auto"
        )),
        None => None,
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub(super) enum SideArg {
    Old,
    New,
}

pub(crate) fn read_body_input(
    inline: Option<&str>,
    file: Option<&Path>,
    stdin: bool,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    if let Some(inline) = inline {
        return Ok(Some(inline.to_owned()));
    }
    if let Some(path) = file {
        return Ok(Some(std::fs::read_to_string(path)?));
    }
    if stdin {
        let mut body = String::new();
        std::io::stdin().read_to_string(&mut body)?;
        return Ok(Some(body));
    }
    Ok(None)
}

impl From<SideArg> for Side {
    fn from(value: SideArg) -> Self {
        match value {
            SideArg::Old => Side::Old,
            SideArg::New => Side::New,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use shoreline::model::ActorId;
    use shoreline::session::PrincipalResolution;

    // Minimal committed map: claude-code -> KEVIN.
    const COMMITTED: &str = r#"{"delegates":{"actor:agent:claude-code":[
      {"principal":"actor:git-email:kevin@swiber.dev","validFrom":"2026-06-10T00:00:00Z","validUntil":null}]}}"#;
    // Local override: claude-code -> ALICE.
    const LOCAL: &str = r#"{"delegates":{"actor:agent:claude-code":[
      {"principal":"actor:git-email:alice@example.com","validFrom":"2026-06-10T00:00:00Z","validUntil":null}]}}"#;

    #[test]
    fn read_body_input_prefers_inline_then_file_then_stdin_false() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let body_path = dir.path().join("body.txt");
        std::fs::write(&body_path, "from file").expect("write body file");

        let body = super::read_body_input(Some("from inline"), Some(&body_path), false)
            .expect("body input resolves");

        assert_eq!(body, Some("from inline".to_string()));
    }

    #[test]
    fn discovers_committed_delegates_json() {
        let repo = git_repo();
        write(&repo, ".shore/delegates.json", COMMITTED);
        let map = super::discover_delegation_map(repo.path()).expect("committed map discovered");
        assert!(matches!(
            map.resolve(&ActorId::new("actor:agent:claude-code"), "2026-06-12T00:00:00Z"),
            PrincipalResolution::Resolved(p) if p.as_str() == "actor:git-email:kevin@swiber.dev"));
    }

    #[test]
    fn local_override_layers_over_committed() {
        let repo = git_repo();
        write(&repo, ".shore/delegates.json", COMMITTED);
        write(&repo, ".shore/delegates.local.json", LOCAL);
        let map = super::discover_delegation_map(repo.path()).expect("layered map");
        assert!(matches!(
            map.resolve(&ActorId::new("actor:agent:claude-code"), "2026-06-12T00:00:00Z"),
            PrincipalResolution::Resolved(p) if p.as_str() == "actor:git-email:alice@example.com"));
    }

    #[test]
    fn local_alone_is_used_when_committed_absent() {
        let repo = git_repo();
        write(&repo, ".shore/delegates.local.json", LOCAL);
        assert!(super::discover_delegation_map(repo.path()).is_some());
    }

    #[test]
    fn neither_file_present_returns_none() {
        let repo = git_repo();
        assert!(super::discover_delegation_map(repo.path()).is_none());
    }

    const ALLOWED: &str = r#"{"allowedSigners":{
      "actor:git-email:alice@example.com":["did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd"]}}"#;

    #[test]
    fn discover_trust_set_loads_committed_allowed_signers() {
        use shoreline::crypto::SignerId;

        let repo = git_repo();
        write(&repo, ".shore/allowed-signers.json", ALLOWED);
        let trust = super::discover_trust_set(repo.path());
        let actor = ActorId::new("actor:git-email:alice@example.com");
        let signer =
            SignerId::parse("did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd").unwrap();
        assert!(trust.authorizes(&actor, &signer, "2026-06-16T00:00:00Z"));
    }

    #[test]
    fn discover_trust_set_absent_file_is_empty_default_without_error() {
        let repo = git_repo();
        let trust = super::discover_trust_set(repo.path());
        assert_eq!(trust, shoreline::session::TrustSet::default());
    }

    #[test]
    fn discover_trust_set_malformed_is_advisory_and_falls_back_to_default() {
        let repo = git_repo();
        write(&repo, ".shore/allowed-signers.json", "{ not json");
        // Malformed is advisory: a one-line warning, then the empty default.
        let trust = super::discover_trust_set(repo.path());
        assert_eq!(trust, shoreline::session::TrustSet::default());
    }

    #[test]
    fn malformed_local_is_advisory_and_falls_back_to_committed() {
        let repo = git_repo();
        write(&repo, ".shore/delegates.json", COMMITTED);
        write(&repo, ".shore/delegates.local.json", "{ not json");
        // Malformed local is advisory (ADR-0003): the committed default still applies.
        let map =
            super::discover_delegation_map(repo.path()).expect("committed survives bad local");
        assert!(matches!(
            map.resolve(&ActorId::new("actor:agent:claude-code"), "2026-06-12T00:00:00Z"),
            PrincipalResolution::Resolved(p) if p.as_str() == "actor:git-email:kevin@swiber.dev"));
    }

    fn git_repo() -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("create temp git repository directory");
        let output = Command::new("git")
            .arg("init")
            .current_dir(repo.path())
            .output()
            .expect("run git init");
        assert!(output.status.success(), "git init failed");
        repo
    }

    fn write(repo: &tempfile::TempDir, rel: &str, contents: &str) {
        let path = repo.path().join(rel);
        if let Some(parent) = Path::new(&path).parent() {
            std::fs::create_dir_all(parent).expect("create parent dirs");
        }
        std::fs::write(path, contents).expect("write file");
    }
}

#[cfg(test)]
mod resolve_signer_tests {
    use std::path::Path;

    use shoreline::crypto::EventSigner as _;
    use shoreline::keys::{KeyName, generate_key_in};
    use shoreline::model::ActorId;

    use super::resolve_signer_with_env;

    fn human_actor() -> ActorId {
        ActorId::new("actor:git-email:dev@example.com")
    }

    fn agent_actor() -> ActorId {
        ActorId::new("actor:agent:claude-code")
    }

    fn repo() -> &'static Path {
        Path::new(".")
    }

    /// Mint a key into the injected keystore root and return its did:key string.
    fn generate_into(root: &Path, name: &str) -> String {
        let handle = generate_key_in(root, &KeyName::parse(name).unwrap()).unwrap();
        handle.signer_id().as_str().to_owned()
    }

    #[test]
    fn sign_key_flag_takes_precedence_over_env_and_default() {
        let root = tempfile::tempdir().unwrap();
        let flag_did = generate_into(root.path(), "flagkey");
        let _env_did = generate_into(root.path(), "envkey");
        let _default_did = generate_into(root.path(), "default");

        let resolution = resolve_signer_with_env(
            repo(),
            &human_actor(),
            Some("flagkey"),
            None,
            Some("envkey"),
            Some(root.path()),
        );

        let signer = resolution.signer.expect("flag key resolves");
        assert_eq!(signer.signer_id().as_str(), flag_did);
    }

    #[test]
    fn env_signing_key_wins_over_default_when_no_flag() {
        let root = tempfile::tempdir().unwrap();
        let env_did = generate_into(root.path(), "envkey");
        let _default_did = generate_into(root.path(), "default");

        let resolution = resolve_signer_with_env(
            repo(),
            &human_actor(),
            None,
            None,
            Some("envkey"),
            Some(root.path()),
        );

        let signer = resolution.signer.expect("env key resolves");
        assert_eq!(signer.signer_id().as_str(), env_did);
    }

    #[test]
    fn user_default_key_is_used_when_no_flag_and_no_env() {
        let root = tempfile::tempdir().unwrap();
        let default_did = generate_into(root.path(), "default");

        let resolution =
            resolve_signer_with_env(repo(), &human_actor(), None, None, None, Some(root.path()));

        let signer = resolution.signer.expect("default key resolves");
        assert_eq!(signer.signer_id().as_str(), default_did);
    }

    #[test]
    fn no_key_anywhere_resolves_to_none_with_no_diagnostic() {
        let root = tempfile::tempdir().unwrap();
        let resolution =
            resolve_signer_with_env(repo(), &human_actor(), None, None, None, Some(root.path()));
        assert!(resolution.signer.is_none());
        assert!(resolution.diagnostic.is_none());
    }

    #[test]
    fn signing_off_short_circuits_to_none_even_with_a_default_key() {
        let root = tempfile::tempdir().unwrap();
        let _default_did = generate_into(root.path(), "default");

        let resolution = resolve_signer_with_env(
            repo(),
            &human_actor(),
            None,
            Some("off"),
            None,
            Some(root.path()),
        );
        assert!(resolution.signer.is_none());
    }

    #[test]
    fn signing_off_is_case_insensitive() {
        let root = tempfile::tempdir().unwrap();
        let _default_did = generate_into(root.path(), "default");

        for value in ["off", "OFF", "Off"] {
            let resolution = resolve_signer_with_env(
                repo(),
                &human_actor(),
                None,
                Some(value),
                None,
                Some(root.path()),
            );
            assert!(
                resolution.signer.is_none(),
                "SHORE_SIGNING={value} must disable signing"
            );
        }
    }

    #[test]
    fn explicitly_selected_broken_key_does_not_fall_through_to_default() {
        let root = tempfile::tempdir().unwrap();
        // A "default" key ALSO exists — resolution must NOT substitute it.
        let default_did = generate_into(root.path(), "default");

        let resolution = resolve_signer_with_env(
            repo(),
            &human_actor(),
            None,
            None,
            Some("nonexistent"),
            Some(root.path()),
        );

        assert!(
            resolution.signer.is_none(),
            "an explicitly selected broken key must not substitute the default identity"
        );
        let diagnostic = resolution.diagnostic.expect("a named diagnostic");
        assert!(
            diagnostic.contains("signing_key_unreadable"),
            "diagnostic names the failure: {diagnostic}"
        );
        assert!(
            !diagnostic.contains(&default_did),
            "the default identity is never substituted"
        );
    }

    #[test]
    fn unsupported_algorithm_key_path_resolves_to_none_with_named_diagnostic() {
        let root = tempfile::tempdir().unwrap();
        let rsa_path = root.path().join("rsa-key");
        // A valid key file whose algorithm is not ed25519.
        std::fs::write(
            &rsa_path,
            r#"{"version":1,"alg":"rsa","seed":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="}"#,
        )
        .unwrap();

        let resolution = resolve_signer_with_env(
            repo(),
            &human_actor(),
            Some(rsa_path.to_str().unwrap()),
            None,
            None,
            Some(root.path()),
        );

        assert!(resolution.signer.is_none());
        assert!(
            resolution
                .diagnostic
                .as_deref()
                .is_some_and(|d| d.contains("signing_key_unsupported_algorithm")),
            "diagnostic: {:?}",
            resolution.diagnostic
        );
    }

    #[test]
    fn unrecognized_signing_mode_is_advisory_not_an_error() {
        let root = tempfile::tempdir().unwrap();
        let _default_did = generate_into(root.path(), "default");

        let resolution = resolve_signer_with_env(
            repo(),
            &human_actor(),
            None,
            Some("banana"),
            None,
            Some(root.path()),
        );

        assert!(resolution.signer.is_some(), "still resolves the default");
        assert!(
            resolution
                .diagnostic
                .as_deref()
                .is_some_and(|d| d.contains("signing_mode_unrecognized")),
            "diagnostic: {:?}",
            resolution.diagnostic
        );
    }

    #[cfg(unix)]
    #[test]
    fn agent_actor_with_no_key_generates_a_0600_key_and_signs() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().unwrap();
        let resolution =
            resolve_signer_with_env(repo(), &agent_actor(), None, None, None, Some(root.path()));

        let signer = resolution
            .signer
            .expect("agent actor auto-keygens and signs");
        assert!(!signer.signer_id().as_str().is_empty());

        let key_path = root.path().join("agent-claude-code");
        assert!(key_path.exists(), "the agent key file was generated");
        let mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "auto-keygen key is private");
    }

    #[test]
    fn agent_keygen_emits_enrollment_notice_on_first_write_only() {
        let root = tempfile::tempdir().unwrap();
        let first =
            resolve_signer_with_env(repo(), &agent_actor(), None, None, None, Some(root.path()));
        let notice = first
            .diagnostic
            .expect("first write carries the enrollment notice");
        assert!(notice.contains("generated signing key"));
        assert!(notice.contains("shore keys enroll"));

        let second =
            resolve_signer_with_env(repo(), &agent_actor(), None, None, None, Some(root.path()));
        assert!(
            second.diagnostic.is_none(),
            "the reused key emits no fresh notice"
        );
    }

    #[test]
    fn agent_key_is_reused_on_the_next_write_no_second_keygen() {
        let root = tempfile::tempdir().unwrap();
        let first =
            resolve_signer_with_env(repo(), &agent_actor(), None, None, None, Some(root.path()));
        let first_did = first
            .signer
            .expect("first keygen")
            .signer_id()
            .as_str()
            .to_owned();

        let second =
            resolve_signer_with_env(repo(), &agent_actor(), None, None, None, Some(root.path()));
        let second_did = second
            .signer
            .expect("reuse")
            .signer_id()
            .as_str()
            .to_owned();

        assert_eq!(first_did, second_did, "the same per-machine key is reused");
    }

    #[test]
    fn human_actor_with_no_key_does_not_auto_keygen_and_stays_unsigned() {
        let root = tempfile::tempdir().unwrap();
        let resolution =
            resolve_signer_with_env(repo(), &human_actor(), None, None, None, Some(root.path()));
        assert!(resolution.signer.is_none());
        // No key file was created under the injected root.
        let entries = std::fs::read_dir(root.path())
            .map(|dir| dir.count())
            .unwrap_or(0);
        assert_eq!(entries, 0, "a human actor never auto-keygens");
    }

    #[test]
    fn agent_keygen_failure_degrades_to_none_never_an_error() {
        // A keys_root that is actually a file, so the key cannot be written.
        let file_root = tempfile::NamedTempFile::new().unwrap();
        let resolution = resolve_signer_with_env(
            repo(),
            &agent_actor(),
            None,
            None,
            None,
            Some(file_root.path()),
        );
        assert!(
            resolution.signer.is_none(),
            "keygen failure must not yield a signer"
        );
        // Structurally there is no Err — SignerResolution is returned by value.
    }

    #[test]
    fn actor_slug_maps_to_a_filesystem_safe_key_name() {
        use shoreline::keys::KeyName;

        assert_eq!(
            super::agent_key_name("actor:agent:claude-code"),
            "agent-claude-code"
        );
        assert_eq!(
            super::agent_key_name("actor:agent:weird/../name"),
            "agent-weird-name"
        );
        // Every derived name is a valid, path-safe keystore key name.
        for actor in ["actor:agent:claude-code", "actor:agent:weird/../name"] {
            let name = super::agent_key_name(actor);
            assert!(!name.contains('/'), "{name} has no path separator");
            assert!(KeyName::parse(&name).is_ok(), "{name} is a valid key name");
        }
    }
}
