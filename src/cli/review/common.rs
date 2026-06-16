use std::io::Read;
use std::path::Path;

use clap::ValueEnum;
use shoreline::keys::{FileEd25519Signer, load_signer, load_signer_in};
use shoreline::model::{ActorId, Side};
use shoreline::session::{DelegationMap, is_agent_actor_id};

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
    // loads by path; otherwise it is a keystore key name.
    let by_path =
        candidate.contains('/') || candidate.contains(std::path::MAIN_SEPARATOR) || path.is_file();
    let loaded = if by_path {
        // Reuse the keystore loader by splitting the path into directory + file.
        let dir = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        match path.file_name().and_then(|name| name.to_str()) {
            Some(file) => load_signer_in(dir, file),
            None => {
                return Err(format!(
                    "{SIGNING_KEY_UNREADABLE}: signing key {candidate:?} is not a key file path"
                ));
            }
        }
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

/// Hook for agent-context auto-keygen. The keygen behavior is implemented in the
/// auto-keygen task; here it resolves to nothing so the ladder falls through to
/// the user-default rung. Threads `keys_root` so the auto-keygen task can test
/// keygen against an injected root without mutating `SHORE_HOME`.
fn resolve_agent_signer(
    _repo: &Path,
    _actor: &ActorId,
    _keys_root: Option<&Path>,
) -> Option<SignerResolution> {
    None
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

    #[test]
    fn agent_rung_is_a_no_op_hook_until_auto_keygen_lands() {
        let root = tempfile::tempdir().unwrap();
        let resolution =
            resolve_signer_with_env(repo(), &agent_actor(), None, None, None, Some(root.path()));
        assert!(resolution.signer.is_none());
    }
}
