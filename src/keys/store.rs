use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::crypto::SignerId;
use crate::error::{Result, ShoreError};
use crate::keys::home::keys_dir;
use crate::keys::signer::FileEd25519Signer;

const KEY_FILE_VERSION: u32 = 1;
const KEY_FILE_ALG: &str = "ed25519";

/// The result of minting (or, in a sibling module, loading) a named keystore
/// key: its derived `did:key` identity plus where its files live on disk. `pub`
/// (with `pub` accessors) because the binary CLI crate consumes it via
/// `pointbreak::keys`.
#[derive(Clone, Debug)]
pub struct KeyHandle {
    name: String,
    signer_id: SignerId,
    private_key_path: PathBuf,
    public_key_path: PathBuf,
}

impl KeyHandle {
    pub fn signer_id(&self) -> &SignerId {
        &self.signer_id
    }
    pub fn private_key_path(&self) -> &Path {
        &self.private_key_path
    }
    pub fn public_key_path(&self) -> &Path {
        &self.public_key_path
    }
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// A keystore key's public identity for listing: its name, derived `did:key`, and
/// custody. `pub` (with `pub` accessors) so the binary CLI consumes it via
/// `pointbreak::keys`.
#[derive(Clone, Debug)]
pub struct KeyInfo {
    name: String,
    signer_id: SignerId,
    custody: KeyCustody,
}

impl KeyInfo {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn signer_id(&self) -> &SignerId {
        &self.signer_id
    }
    pub fn custody(&self) -> KeyCustody {
        self.custody
    }
}

/// Whether a keystore key holds its private seed on disk (`File`) or delegates
/// custody to ssh-agent and stores only the public key (`Agent`). `pub` so the
/// binary CLI's `keys list` reports it via `pointbreak::keys`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCustody {
    File,
    Agent,
}

/// The custody-tagged material recovered from a keystore key: either the raw seed
/// (→ a local `FileEd25519Signer`) or the public key of an agent-backed reference
/// (→ an `SshAgentSigner` built at resolve time). `pub`: the binary CLI's resolve
/// layer consumes it via `pointbreak::keys`.
#[derive(Clone, Debug)]
pub enum KeyMaterial {
    Seed([u8; 32]),
    AgentBacked { public_key: [u8; 32] },
}

/// On-disk key document. Internal, forward-compatible (`version` reserves room to
/// migrate). Two custodies share this shape (additive — a pre-existing seed file
/// is unchanged on disk):
///   - file custody:  `{version, alg, seed}` (a base64 raw Ed25519 seed)
///   - agent custody: `{version, alg, custody:"agent", publicKey}` (a base64 raw
///     32-byte public key; the private key lives in ssh-agent, never on disk)
#[derive(Serialize, Deserialize)]
struct KeyFile {
    version: u32,
    alg: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    custody: Option<String>,
    #[serde(rename = "publicKey", skip_serializing_if = "Option::is_none")]
    public_key: Option<String>,
}

const KEY_CUSTODY_AGENT: &str = "agent";

/// A validated, path-safe keystore key name. A key name becomes a filename under
/// `keys_dir()`, so it MUST NOT contain path separators, `..`, a leading dot, or
/// control characters — otherwise `--name ../../id_ed25519` could escape the
/// keystore and a `--name` could clobber an unrelated file. Allowed charset:
/// ASCII alphanumerics plus `-`, `_`, `.` (never leading), bounded length.
pub struct KeyName(String);

impl KeyName {
    pub fn parse(value: &str) -> Result<Self> {
        let ok = !value.is_empty()
            && value.len() <= 64
            && !value.starts_with('.')
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if !ok {
            return Err(ShoreError::WorkflowInputInvalid {
                reason: format!(
                    "invalid key name {value:?}: use ASCII letters, digits, '-', '_', '.' (no path \
                     separators, no leading dot), 1..=64 chars"
                ),
            });
        }
        Ok(Self(value.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Mint a new Ed25519 key named `name`, write its private-key file (`0600` on
/// Unix) and `did:key` sidecar, and return its handle. Validates the name first,
/// and atomically refuses to overwrite an existing named key (`create_new`).
///
/// `generate_key` is a thin wrapper over the root-injecting `generate_key_in`.
/// Unit tests call `generate_key_in` with a `tempdir` root and never set
/// `SHORE_HOME`; only the production path and subprocess CLI tests read the env.
pub fn generate_key(name: &str) -> Result<KeyHandle> {
    generate_key_in(&keys_dir()?, &KeyName::parse(name)?)
}

/// Root-injecting keygen: write the named key under `dir`. `pub` because the
/// binary CLI crate's resolver tests inject a `tempdir` root through this variant
/// (the env-reading `generate_key` wrapper is the production path).
pub fn generate_key_in(dir: &Path, name: &KeyName) -> Result<KeyHandle> {
    let private_key_path = dir.join(name.as_str());
    let public_key_path = dir.join(format!("{}.pub", name.as_str()));

    let mut seed = [0_u8; 32];
    getrandom::fill(&mut seed).map_err(|error| {
        ShoreError::Message(format!("generate key {:?}: {error}", name.as_str()))
    })?;

    let signing_key = SigningKey::from_bytes(&seed);
    let signer_id = SignerId::from_ed25519_public_key(signing_key.verifying_key().to_bytes());

    // Atomic no-clobber create with the intended mode set AT creation (no
    // exists()->write()->chmod TOCTOU window where the key is briefly world-readable).
    write_key_file(&private_key_path, &seed)?;
    std::fs::write(&public_key_path, format!("{}\n", signer_id.as_str()))
        .map_err(|error| ShoreError::Message(format!("write public sidecar: {error}")))?;

    Ok(KeyHandle {
        name: name.as_str().to_owned(),
        signer_id,
        private_key_path,
        public_key_path,
    })
}

fn write_key_file(path: &Path, seed: &[u8; 32]) -> Result<()> {
    let document = KeyFile {
        version: KEY_FILE_VERSION,
        alg: KEY_FILE_ALG.to_owned(),
        seed: Some(BASE64_STANDARD.encode(seed)),
        custody: None,
        public_key: None,
    };
    write_key_document(path, &document, /* private = */ true)
}

/// Write a key document with the atomic no-clobber `create_new` policy. `private`
/// applies `0600` (Unix) at creation for seed files; an agent reference holds only
/// the public key (not secret), so it is written world-readable like the `.pub`
/// sidecar — refuse-to-clobber still applies, the mode does not.
fn write_key_document(path: &Path, document: &KeyFile, private: bool) -> Result<()> {
    use std::io::Write as _;
    let bytes = serde_json::to_vec(document)?;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true); // create_new => fails if the path exists (atomic no-clobber)
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600); // private from creation, not chmod-after
    }
    // On platforms without Unix mode bits, `private` has no on-disk effect (the
    // directory ACL governs); the no-clobber create policy above still applies.
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options.open(path).map_err(|error| {
        ShoreError::Message(format!(
            "create key file {} (refusing to overwrite an existing key): {error}",
            path.display()
        ))
    })?;
    file.write_all(&bytes).map_err(|error| {
        ShoreError::Message(format!("write key file {}: {error}", path.display()))
    })?;
    Ok(())
}

/// Persist an agent-backed reference: a custody-tagged key file carrying only the
/// public key (no seed), plus the `<name>.pub` did:key sidecar. The `did:key`
/// derives from `public_key` with no agent and no private key, so the reference is
/// enroll/list/show-able offline. Refuse-to-clobber via `create_new`.
/// `pub`: `shore key use-ssh` consumes it via `pointbreak::keys`.
pub fn write_agent_reference(name: &str, public_key: [u8; 32]) -> Result<KeyHandle> {
    write_agent_reference_in(&keys_dir()?, &KeyName::parse(name)?, public_key)
}

/// Root-injecting variant: write the reference under `dir`. `pub` so unit tests
/// inject a `tempdir` root and never set `SHORE_HOME`, like `generate_key_in`.
pub fn write_agent_reference_in(
    dir: &Path,
    name: &KeyName,
    public_key: [u8; 32],
) -> Result<KeyHandle> {
    let private_key_path = dir.join(name.as_str());
    let public_key_path = dir.join(format!("{}.pub", name.as_str()));
    let signer_id = SignerId::from_ed25519_public_key(public_key);

    let document = KeyFile {
        version: KEY_FILE_VERSION,
        alg: KEY_FILE_ALG.to_owned(),
        seed: None,
        custody: Some(KEY_CUSTODY_AGENT.to_owned()),
        public_key: Some(BASE64_STANDARD.encode(public_key)),
    };
    // A reference holds only the public key (not secret), so `0600` is deliberately
    // skipped; refuse-to-clobber via `create_new` still applies.
    write_key_document(&private_key_path, &document, /* private = */ false)?;
    std::fs::write(&public_key_path, format!("{}\n", signer_id.as_str()))
        .map_err(|error| ShoreError::Message(format!("write public sidecar: {error}")))?;

    Ok(KeyHandle {
        name: name.as_str().to_owned(),
        signer_id,
        private_key_path,
        public_key_path,
    })
}

/// Load a named keystore key as a production signer: read its file from
/// `keys_dir()`, reconstruct the `SigningKey`, and re-derive the `SignerId`.
/// All fallible work (resolve + read + decode) lives here, ahead of signing.
/// `pub`: the binary CLI consumes it via `pointbreak::keys::load_signer`.
pub fn load_signer(name: &str) -> Result<FileEd25519Signer> {
    load_signer_in(&keys_dir()?, name)
}

/// Root-injecting loader: the resolver CLI tests pass a `tempdir` root so they
/// never mutate `SHORE_HOME`. `pub` for the same reason `generate_key_in` is.
///
/// `name` is validated via [`KeyName::parse`] before it becomes a filename, so a
/// keystore-name lookup can never traverse outside `dir` (e.g. `../outside-key`).
/// Load an explicit key file by path with [`load_signer_from_path`] instead.
pub fn load_signer_in(dir: &Path, name: &str) -> Result<FileEd25519Signer> {
    let name = KeyName::parse(name)?;
    let seed = read_key_seed(&dir.join(name.as_str()))?;
    Ok(FileEd25519Signer::from_seed(seed))
}

/// Load a signer from an explicit key-file path (the `--sign-key <path>` form).
/// Unlike `load_signer_in`, the caller supplies the full path deliberately, so no
/// keystore-name validation applies. `pub`: the binary CLI consumes it.
pub fn load_signer_from_path(path: &Path) -> Result<FileEd25519Signer> {
    let seed = read_key_seed(path)?;
    Ok(FileEd25519Signer::from_seed(seed))
}

/// Enumerate the keys in the user-level keystore, each with its derived
/// `did:key`. `pub`: the binary CLI consumes it via `pointbreak::keys::list_keys`.
pub fn list_keys() -> Result<Vec<KeyInfo>> {
    list_keys_in(&keys_dir()?)
}

/// Root-injecting enumerator: list the keys under `dir`, skipping `.pub`
/// sidecars and non-files. `pub` so unit tests inject a `tempdir` root. A missing
/// directory is an empty keystore, not an error.
pub fn list_keys_in(dir: &Path) -> Result<Vec<KeyInfo>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ShoreError::Message(format!(
                "read keystore {}: {error}",
                dir.display()
            )));
        }
    };

    let mut keys = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| ShoreError::Message(format!("read keystore entry: {error}")))?;
        if !entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if name.ends_with(".pub") {
            continue; // public sidecar, not a private key
        }
        let (signer_id, custody) = match read_key_material(&entry.path())? {
            KeyMaterial::Seed(seed) => (
                SignerId::from_ed25519_public_key(
                    SigningKey::from_bytes(&seed).verifying_key().to_bytes(),
                ),
                KeyCustody::File,
            ),
            KeyMaterial::AgentBacked { public_key } => (
                SignerId::from_ed25519_public_key(public_key),
                KeyCustody::Agent,
            ),
        };
        keys.push(KeyInfo {
            name: name.into_owned(),
            signer_id,
            custody,
        });
    }
    keys.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(keys)
}

/// Load a named key's custody-tagged material. A `seed` file loads as `Seed`; an
/// agent reference loads as `AgentBacked`. `pub`: the binary CLI's resolve layer
/// consumes it via `pointbreak::keys`.
pub fn load_key_material(name: &str) -> Result<KeyMaterial> {
    load_key_material_in(&keys_dir()?, name)
}

/// Root-injecting variant for hermetic tests (like `load_signer_in`). `name` is
/// validated via `KeyName::parse` before it becomes a filename (no traversal).
pub fn load_key_material_in(dir: &Path, name: &str) -> Result<KeyMaterial> {
    let name = KeyName::parse(name)?;
    read_key_material(&dir.join(name.as_str()))
}

/// Resolve a named key's `did:key` (`SignerId`) **without** needing its private
/// key. A file key's id derives from its seed; an agent-backed reference's id
/// derives from its stored public key — no agent, no seed. `pub`: the binary CLI's
/// `enroll` consumes it so an agent-backed key enrolls offline. (Distinct from
/// [`load_signer`], which must reconstruct a usable signer and so reads the seed.)
pub fn load_signer_id(name: &str) -> Result<SignerId> {
    load_signer_id_in(&keys_dir()?, name)
}

/// Root-injecting variant for hermetic tests (like `load_signer_in`). `name` is
/// validated via `KeyName::parse` inside `load_key_material_in` (no traversal).
pub fn load_signer_id_in(dir: &Path, name: &str) -> Result<SignerId> {
    Ok(match load_key_material_in(dir, name)? {
        KeyMaterial::Seed(seed) => SignerId::from_ed25519_public_key(
            SigningKey::from_bytes(&seed).verifying_key().to_bytes(),
        ),
        KeyMaterial::AgentBacked { public_key } => SignerId::from_ed25519_public_key(public_key),
    })
}

/// Read the raw 32-byte Ed25519 seed from a keystore private-key file. Shared by
/// keygen tests here and by the loader in the sibling signer module. A non-seed
/// (agent-backed) reference has no private seed, so this is an error for it.
pub(crate) fn read_key_seed(path: &Path) -> Result<[u8; 32]> {
    match read_key_material(path)? {
        KeyMaterial::Seed(seed) => Ok(seed),
        KeyMaterial::AgentBacked { .. } => Err(ShoreError::Message(format!(
            "key file {} is an agent-backed reference with no private seed",
            path.display()
        ))),
    }
}

/// Parse a key file into its custody-tagged material. Seed present ⇒ `Seed`;
/// `custody:"agent"` + `publicKey` (no seed) ⇒ `AgentBacked`. A malformed file
/// (neither shape) is a typed error.
fn read_key_material(path: &Path) -> Result<KeyMaterial> {
    let document = read_key_file(path)?;
    if document.alg != KEY_FILE_ALG {
        return Err(ShoreError::Message(format!(
            "unsupported key algorithm {:?}",
            document.alg
        )));
    }
    if let Some(seed) = document.seed.as_deref() {
        return Ok(KeyMaterial::Seed(decode_32(seed, "key seed")?));
    }
    if document.custody.as_deref() == Some(KEY_CUSTODY_AGENT) {
        let public = document.public_key.as_deref().ok_or_else(|| {
            ShoreError::Message("agent-backed reference is missing publicKey".to_owned())
        })?;
        return Ok(KeyMaterial::AgentBacked {
            public_key: decode_32(public, "public key")?,
        });
    }
    Err(ShoreError::Message(format!(
        "key file {} has neither a seed nor an agent-backed publicKey",
        path.display()
    )))
}

fn read_key_file(path: &Path) -> Result<KeyFile> {
    let bytes = std::fs::read(path).map_err(|error| {
        ShoreError::Message(format!("read key file {}: {error}", path.display()))
    })?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn decode_32(b64: &str, what: &str) -> Result<[u8; 32]> {
    let bytes = BASE64_STANDARD
        .decode(b64.as_bytes())
        .map_err(|error| ShoreError::Message(format!("decode {what}: {error}")))?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| ShoreError::Message(format!("{what} is not 32 bytes")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(s: &str) -> KeyName {
        KeyName::parse(s).unwrap()
    }

    #[test]
    fn generated_key_round_trips_seed_to_stable_did_key() {
        let root = tempfile::tempdir().unwrap();
        let handle = generate_key_in(root.path(), &name("default")).unwrap();
        let did = handle.signer_id().clone();

        // Reload the raw seed from disk and re-derive the public key / did:key.
        let seed = read_key_seed(handle.private_key_path()).unwrap();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
        let rederived = crate::crypto::SignerId::from_ed25519_public_key(
            signing_key.verifying_key().to_bytes(),
        );

        assert_eq!(
            rederived, did,
            "did:key derives deterministically from the seed"
        );
    }

    #[test]
    fn did_key_derives_from_the_public_key() {
        let root = tempfile::tempdir().unwrap();
        let handle = generate_key_in(root.path(), &name("default")).unwrap();
        let public = handle.signer_id().ed25519_public_key().unwrap();
        assert_eq!(public.len(), 32);
        assert!(handle.signer_id().as_str().starts_with("did:key:z6Mk"));
    }

    #[cfg(unix)]
    #[test]
    fn private_key_file_is_0600_on_unix() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        let handle = generate_key_in(root.path(), &name("default")).unwrap();
        let mode = std::fs::metadata(handle.private_key_path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "private key must not be group/world readable"
        );
    }

    #[test]
    fn regenerating_an_existing_name_does_not_clobber() {
        let root = tempfile::tempdir().unwrap();
        let first = generate_key_in(root.path(), &name("default")).unwrap();
        let first_did = first.signer_id().clone();

        // create_new makes the collision an OS-level atomic failure, not a TOCTOU race.
        let err = generate_key_in(root.path(), &name("default")).unwrap_err();
        assert!(err.to_string().contains("refusing to overwrite"));

        // The original key file is untouched.
        let still = read_key_seed(first.private_key_path()).unwrap();
        let reloaded = crate::crypto::SignerId::from_ed25519_public_key(
            ed25519_dalek::SigningKey::from_bytes(&still)
                .verifying_key()
                .to_bytes(),
        );
        assert_eq!(reloaded, first_did, "existing key survived the collision");
    }

    #[test]
    fn pub_sidecar_records_the_did_key() {
        let root = tempfile::tempdir().unwrap();
        let handle = generate_key_in(root.path(), &name("default")).unwrap();
        let recorded = std::fs::read_to_string(handle.public_key_path()).unwrap();
        assert_eq!(recorded.trim(), handle.signer_id().as_str());
    }

    #[test]
    fn two_generated_keys_differ() {
        let root = tempfile::tempdir().unwrap();
        let a = generate_key_in(root.path(), &name("a")).unwrap();
        let b = generate_key_in(root.path(), &name("b")).unwrap();
        assert_ne!(
            a.signer_id(),
            b.signer_id(),
            "getrandom seeds are independent"
        );
    }

    #[test]
    fn load_signer_reconstructs_a_generated_key() {
        let root = tempfile::tempdir().unwrap();
        let key = KeyName::parse("default").unwrap();
        let generated = generate_key_in(root.path(), &key).unwrap();
        let loaded = load_signer_in(root.path(), "default").unwrap();

        // The loaded signer's identity equals the generated key's did:key.
        use crate::crypto::EventSigner as _;
        assert_eq!(loaded.signer_id(), generated.signer_id());
    }

    #[test]
    fn load_signer_for_missing_name_errors() {
        let root = tempfile::tempdir().unwrap();
        let result = load_signer_in(root.path(), "nope");
        assert!(result.is_err());
    }

    #[test]
    fn load_signer_in_rejects_path_traversal_name() {
        let root = tempfile::tempdir().unwrap();
        let keys = root.path().join("keys");
        std::fs::create_dir_all(&keys).unwrap();
        // Plant a valid key as a sibling of keys/, reachable only via traversal.
        generate_key_in(&keys, &name("planted")).unwrap();
        std::fs::copy(keys.join("planted"), root.path().join("outside")).unwrap();

        // A traversal name from keys/ would reach ../outside; it must be rejected,
        // not silently load a key file outside the keystore directory.
        assert!(load_signer_in(&keys, "../outside").is_err());
        assert!(load_signer_in(&keys, "a/b").is_err());
    }

    #[test]
    fn list_keys_enumerates_keys_skipping_sidecars_sorted() {
        let root = tempfile::tempdir().unwrap();
        let work = generate_key_in(root.path(), &name("work")).unwrap();
        let default = generate_key_in(root.path(), &name("default")).unwrap();

        let listed = list_keys_in(root.path()).unwrap();
        let names: Vec<&str> = listed.iter().map(KeyInfo::name).collect();
        assert_eq!(names, ["default", "work"], "sorted, no .pub sidecars");

        let default_entry = listed.iter().find(|k| k.name() == "default").unwrap();
        assert_eq!(default_entry.signer_id(), default.signer_id());
        let work_entry = listed.iter().find(|k| k.name() == "work").unwrap();
        assert_eq!(work_entry.signer_id(), work.signer_id());
    }

    #[test]
    fn list_keys_in_missing_dir_is_empty() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("absent");
        assert!(list_keys_in(&missing).unwrap().is_empty());
    }

    #[test]
    fn rejects_path_unsafe_key_names() {
        // A key name becomes a filename under the keystore; reject anything that
        // could escape it or clobber an unrelated file.
        for bad in [
            "../../id_ed25519",
            "a/b",
            "..",
            ".hidden",
            "",
            "name with space",
            "x\u{0}y",
        ] {
            assert!(KeyName::parse(bad).is_err(), "{bad:?} must be rejected");
        }
        for good in ["default", "agent-claude-code", "ci_key.1", "me"] {
            assert!(KeyName::parse(good).is_ok(), "{good:?} must be accepted");
        }
    }

    // A real public key derived from a fixed seed so the did:key is a valid
    // z6Mk… string; the test only needs determinism, not a live agent key.
    fn sample_public_key() -> [u8; 32] {
        let seed = [7_u8; 32];
        ed25519_dalek::SigningKey::from_bytes(&seed)
            .verifying_key()
            .to_bytes()
    }

    #[test]
    fn agent_reference_round_trips_public_key_to_signer_id() {
        let root = tempfile::tempdir().unwrap();
        let public = sample_public_key();
        let expected = crate::crypto::SignerId::from_ed25519_public_key(public);

        // Write the agent-backed reference (no seed on disk).
        let handle = write_agent_reference_in(root.path(), &name("default"), public).unwrap();
        assert_eq!(
            handle.signer_id(),
            &expected,
            "did:key derives from the public key, no agent"
        );

        // Load it back: custody is AgentBacked carrying the same public key.
        match load_key_material_in(root.path(), "default").unwrap() {
            KeyMaterial::AgentBacked { public_key } => {
                assert_eq!(public_key, public);
                let rederived = crate::crypto::SignerId::from_ed25519_public_key(public_key);
                assert_eq!(
                    rederived, expected,
                    "did:key offline from stored public material"
                );
            }
            other => panic!("expected AgentBacked, got {other:?}"),
        }
    }

    #[test]
    fn agent_reference_writes_a_did_key_sidecar() {
        let root = tempfile::tempdir().unwrap();
        let handle =
            write_agent_reference_in(root.path(), &name("default"), sample_public_key()).unwrap();
        let recorded = std::fs::read_to_string(handle.public_key_path()).unwrap();
        assert_eq!(recorded.trim(), handle.signer_id().as_str());
    }

    #[test]
    fn agent_reference_has_no_seed_on_disk() {
        let root = tempfile::tempdir().unwrap();
        write_agent_reference_in(root.path(), &name("default"), sample_public_key()).unwrap();
        let raw = std::fs::read_to_string(root.path().join("default")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["custody"], "agent");
        assert!(value.get("publicKey").is_some(), "publicKey present");
        assert!(
            value.get("seed").is_none(),
            "no private seed in an agent reference"
        );
    }

    #[test]
    fn existing_seed_key_file_still_loads_as_seed_custody() {
        // An existing {version, alg, seed} file is unchanged — it loads as Seed.
        let root = tempfile::tempdir().unwrap();
        let generated = generate_key_in(root.path(), &name("default")).unwrap();

        match load_key_material_in(root.path(), "default").unwrap() {
            KeyMaterial::Seed(seed) => {
                let rederived = crate::crypto::SignerId::from_ed25519_public_key(
                    ed25519_dalek::SigningKey::from_bytes(&seed)
                        .verifying_key()
                        .to_bytes(),
                );
                assert_eq!(
                    &rederived,
                    generated.signer_id(),
                    "seed re-derives the file key's did:key"
                );
            }
            other => panic!("a seed file must load as Seed, got {other:?}"),
        }
    }

    #[test]
    fn list_keys_reports_custody_for_file_and_agent_keys() {
        let root = tempfile::tempdir().unwrap();
        generate_key_in(root.path(), &name("filekey")).unwrap();
        write_agent_reference_in(root.path(), &name("agentkey"), sample_public_key()).unwrap();

        let listed = list_keys_in(root.path()).unwrap();
        let file = listed.iter().find(|k| k.name() == "filekey").unwrap();
        let agent = listed.iter().find(|k| k.name() == "agentkey").unwrap();
        assert_eq!(file.custody(), KeyCustody::File);
        assert_eq!(agent.custody(), KeyCustody::Agent);
    }

    #[test]
    fn load_signer_id_for_a_file_key_matches_load_signer() {
        let root = tempfile::tempdir().unwrap();
        let generated = generate_key_in(root.path(), &name("default")).unwrap();
        // The did:key resolved without a signer equals the seed-loaded signer's id.
        let id = load_signer_id_in(root.path(), "default").unwrap();
        assert_eq!(&id, generated.signer_id());
    }

    #[test]
    fn load_signer_id_for_an_agent_reference_derives_did_key_with_no_seed() {
        // An agent-backed reference has NO seed; its did:key derives from the stored
        // public key with no agent and no private key.
        let root = tempfile::tempdir().unwrap();
        let public = sample_public_key();
        let handle = write_agent_reference_in(root.path(), &name("default"), public).unwrap();

        // load_signer would fail here (there is no seed to read); load_signer_id must not.
        assert!(
            load_signer_in(root.path(), "default").is_err(),
            "no seed to load a signer from"
        );
        let id = load_signer_id_in(root.path(), "default").unwrap();
        assert_eq!(
            &id,
            handle.signer_id(),
            "did:key offline from public material"
        );
        assert_eq!(id, crate::crypto::SignerId::from_ed25519_public_key(public));
    }

    #[test]
    fn agent_reference_refuses_to_clobber_a_colliding_name() {
        let root = tempfile::tempdir().unwrap();
        write_agent_reference_in(root.path(), &name("default"), sample_public_key()).unwrap();

        // create_new => an existing name is an atomic OS-level failure, same as file keys.
        let err = write_agent_reference_in(root.path(), &name("default"), sample_public_key())
            .unwrap_err();
        assert!(err.to_string().contains("refusing to overwrite"));
    }
}
