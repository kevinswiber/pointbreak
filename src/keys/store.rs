use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::crypto::SignerId;
use crate::error::{Result, ShoreError};
use crate::keys::home::keys_dir;

const KEY_FILE_VERSION: u32 = 1;
const KEY_FILE_ALG: &str = "ed25519";

/// The result of minting (or, in a sibling module, loading) a named keystore
/// key: its derived `did:key` identity plus where its files live on disk. `pub`
/// (with `pub` accessors) because the binary CLI crate consumes it via
/// `shoreline::keys`.
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

/// On-disk private-key document. Internal, forward-compatible: `version` reserves
/// room to migrate; the raw Ed25519 seed is base64-standard encoded.
#[derive(Serialize, Deserialize)]
struct KeyFile {
    version: u32,
    alg: String,
    seed: String,
}

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
    use std::io::Write as _;

    let document = KeyFile {
        version: KEY_FILE_VERSION,
        alg: KEY_FILE_ALG.to_owned(),
        seed: BASE64_STANDARD.encode(seed),
    };
    let bytes = serde_json::to_vec(&document)?;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true); // create_new => fails if the path exists (atomic no-clobber)
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600); // private from creation, not chmod-after
    }
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

/// Read the raw 32-byte Ed25519 seed from a keystore private-key file. Shared by
/// keygen tests here and by the loader in the sibling signer module.
pub(crate) fn read_key_seed(path: &Path) -> Result<[u8; 32]> {
    let bytes = std::fs::read(path)
        .map_err(|error| ShoreError::Message(format!("read key file {}: {error}", path.display())))?;
    let document: KeyFile = serde_json::from_slice(&bytes)?;
    if document.alg != KEY_FILE_ALG {
        return Err(ShoreError::Message(format!(
            "unsupported key algorithm {:?}",
            document.alg
        )));
    }
    let seed = BASE64_STANDARD
        .decode(document.seed.as_bytes())
        .map_err(|error| ShoreError::Message(format!("decode key seed: {error}")))?;
    seed.as_slice()
        .try_into()
        .map_err(|_| ShoreError::Message("key seed is not 32 bytes".to_owned()))
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
        let rederived =
            crate::crypto::SignerId::from_ed25519_public_key(signing_key.verifying_key().to_bytes());

        assert_eq!(rederived, did, "did:key derives deterministically from the seed");
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
        assert_ne!(a.signer_id(), b.signer_id(), "getrandom seeds are independent");
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
}
