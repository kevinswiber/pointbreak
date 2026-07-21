use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CLOSURE_SCHEMA_V1: &str = "pointbreak.lmdb-proof-build-closure.v1";
const WRAPPER_REPOSITORY: &str = "https://github.com/meilisearch/heed";
const WRAPPER_SOURCE_COMMIT: &str = "14e3e4914ad5128c68f6bbf4ab40ae1de19b342e";
const WRAPPER_CONVERSION_SCRIPT: &str = "convert-to-heed3.sh";
const WRAPPER_CONVERSION_SCRIPT_SHA256: &str =
    "125dbbd3abd5df9481d3bcd8a87a3d73d1e679c4a1e49ed509b85a5e0b3ee949";
const NATIVE_REPOSITORY: &str = "https://github.com/LMDB/lmdb";
const NATIVE_SOURCE_COMMIT: &str = "62e2a60e71cd58e6fdd83a31af3d3c7fe103483d";
const NATIVE_BEHAVIOR_AUTHORITY: &str = "immutable_upstream_git";
const RELEASE_TARGET_MANIFEST: &str = ".github/binary-targets.json";
const ROOT_MANIFEST: &str = "Cargo.toml";
const WRAPPER_ROOT: &str = "vendor/lmdb-proof/heed3";
const SYS_ROOT: &str = "vendor/lmdb-proof/lmdb-master3-sys";
const RELEASE_TARGETS: &[&str] = &[
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
];
const WRAPPER_ADDITIONAL_STEPS: &[&str] = &[
    "copy converted heed/src plus the pinned build.rs, README.md, and LICENSE",
    "use a proof-local manifest with exact registry dependency pins and publish=false",
    "inline the native source from the separately pinned native commit",
];

const REQUIRED_LICENSES: &[&str] = &[
    "vendor/lmdb-proof/heed3/LICENSE",
    "vendor/lmdb-proof/lmdb-master3-sys/LICENSE-APACHE-2.0",
    "vendor/lmdb-proof/lmdb-master3-sys/lmdb/libraries/liblmdb/COPYRIGHT",
    "vendor/lmdb-proof/lmdb-master3-sys/lmdb/libraries/liblmdb/LICENSE",
];
const REQUIRED_GENERATED_INPUTS: &[&str] = &["vendor/lmdb-proof/lmdb-master3-sys/src/bindings.rs"];
const REQUIRED_BUILD_INPUTS: &[&str] = &[
    "vendor/lmdb-proof/heed3/Cargo.toml",
    "vendor/lmdb-proof/heed3/build.rs",
    "vendor/lmdb-proof/lmdb-master3-sys/Cargo.toml",
    "vendor/lmdb-proof/lmdb-master3-sys/build.rs",
    "vendor/lmdb-proof/lmdb-master3-sys/src/lib.rs",
    "vendor/lmdb-proof/lmdb-master3-sys/lmdb/libraries/liblmdb/lmdb.h",
    "vendor/lmdb-proof/lmdb-master3-sys/lmdb/libraries/liblmdb/mdb.c",
    "vendor/lmdb-proof/lmdb-master3-sys/lmdb/libraries/liblmdb/midl.c",
    "vendor/lmdb-proof/lmdb-master3-sys/lmdb/libraries/liblmdb/midl.h",
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LmdbProofClosureDocumentV1 {
    schema: String,
    wrapper: SourceIdentityV1,
    native: NativeSourceIdentityV1,
    features: FeatureClosureV1,
    licenses: Vec<FileIdentityV1>,
    generated_inputs: Vec<FileIdentityV1>,
    build_inputs: Vec<FileIdentityV1>,
    link: LinkClosureV1,
    target_triples: Vec<String>,
    package: PackageClosureV1,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceIdentityV1 {
    repository: String,
    source_commit: String,
    tree_sha256: String,
    materialization: SourceMaterializationV1,
    published_comparison: PublishedComparisonV1,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceMaterializationV1 {
    upstream_script_path: String,
    upstream_script_sha256: String,
    additional_steps: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeSourceIdentityV1 {
    repository: String,
    source_commit: String,
    tree_sha256: String,
    behavior_authority: String,
    published_comparison: PublishedComparisonV1,
    patches: Vec<PatchIdentityV1>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchIdentityV1 {
    id: String,
    path: String,
    preimage_sha256: String,
    postimage_sha256: String,
    purpose: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublishedComparisonV1 {
    package: String,
    version: String,
    checksum: String,
    behavior_authority: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FeatureClosureV1 {
    wrapper: Vec<String>,
    native: Vec<String>,
    encryption: Vec<String>,
    forbidden: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileIdentityV1 {
    path: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LinkClosureV1 {
    static_archive: String,
    dynamic_host_dependencies: bool,
    windows_system_libraries: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PackageClosureV1 {
    excluded_from_default_package: bool,
    source_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedLmdbProofClosureV1 {
    pub target_triples: Vec<String>,
    pub dynamic_host_dependencies: bool,
    pub encryption_features: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LmdbProofOpenCloseReportV1 {
    pub schema: &'static str,
    pub mode: &'static str,
    pub wrapper_source_commit: &'static str,
    pub native_source_commit: &'static str,
    pub lmdb_version: String,
    pub plain: bool,
    pub encrypted: bool,
    pub static_native_archive: &'static str,
    pub dynamic_host_dependencies: bool,
    pub created_files: Vec<String>,
}

pub fn validate_lmdb_proof_closure_v1(
    document: &str,
    repository_root: &Path,
) -> Result<ValidatedLmdbProofClosureV1, String> {
    let (closure, validated) = validate_lmdb_proof_closure_document_v1(document)?;
    let release_targets = release_target_triples(repository_root)?;
    if closure.target_triples != release_targets {
        return Err("proof target matrix does not match the release target manifest".to_owned());
    }
    validate_package_exclusion(repository_root, &closure.package)?;
    validate_file_identities(repository_root, &closure.licenses)?;
    validate_file_identities(repository_root, &closure.generated_inputs)?;
    validate_file_identities(repository_root, &closure.build_inputs)?;
    validate_tree_identity(repository_root, WRAPPER_ROOT, &closure.wrapper.tree_sha256)?;
    validate_tree_identity(repository_root, SYS_ROOT, &closure.native.tree_sha256)?;

    Ok(validated)
}

fn validate_lmdb_proof_closure_document_v1(
    document: &str,
) -> Result<(LmdbProofClosureDocumentV1, ValidatedLmdbProofClosureV1), String> {
    let closure: LmdbProofClosureDocumentV1 = serde_json::from_str(document)
        .map_err(|error| format!("LMDB proof closure is not valid JSON: {error}"))?;

    if closure.schema != CLOSURE_SCHEMA_V1 {
        return Err("LMDB proof closure schema is not supported".to_owned());
    }
    if closure.wrapper.repository != WRAPPER_REPOSITORY {
        return Err("wrapper repository is not the recorded upstream source".to_owned());
    }
    if closure.wrapper.source_commit != WRAPPER_SOURCE_COMMIT {
        return Err("wrapper source commit does not match the reviewed pin".to_owned());
    }
    validate_wrapper_materialization(&closure.wrapper.materialization)?;
    if closure.native.repository != NATIVE_REPOSITORY {
        return Err("native repository is not the recorded upstream source".to_owned());
    }
    if closure.native.source_commit != NATIVE_SOURCE_COMMIT {
        return Err("native source commit does not match the reviewed pin".to_owned());
    }
    if closure.native.behavior_authority != NATIVE_BEHAVIOR_AUTHORITY
        || closure.native.published_comparison.behavior_authority
    {
        return Err(
            "native behavior authority must be the immutable upstream Git source".to_owned(),
        );
    }
    if closure.features.wrapper != Vec::<String>::new() {
        return Err("wrapper feature closure is not the minimum plain set".to_owned());
    }
    if !closure.features.native.is_empty() || !closure.features.encryption.is_empty() {
        return Err("native or encryption features escaped the plain closure".to_owned());
    }
    let forbidden = closure
        .features
        .forbidden
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if forbidden
        != BTreeSet::from([
            "asan",
            "bindgen",
            "fuzzer",
            "fuzzer-no-link",
            "longer-keys",
            "mdb_idl_logn_*",
            "posix-sem",
            "use-valgrind",
        ])
    {
        return Err("forbidden feature inventory is incomplete".to_owned());
    }
    if identity_paths(&closure.licenses) != string_set(REQUIRED_LICENSES) {
        return Err("source license and notice inventory is incomplete".to_owned());
    }
    if identity_paths(&closure.generated_inputs) != string_set(REQUIRED_GENERATED_INPUTS) {
        return Err("generated binding inventory is incomplete".to_owned());
    }
    if identity_paths(&closure.build_inputs) != string_set(REQUIRED_BUILD_INPUTS) {
        return Err("native build input inventory is incomplete".to_owned());
    }
    if closure.link.dynamic_host_dependencies {
        return Err("dynamic host dependencies are forbidden".to_owned());
    }

    if closure
        .target_triples
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        != RELEASE_TARGETS
    {
        return Err("proof target matrix does not match the release target manifest".to_owned());
    }
    if !closure.package.excluded_from_default_package {
        return Err("proof sources must be excluded from the default Cargo package".to_owned());
    }

    validate_link_closure(&closure.link)?;
    validate_published_comparison(
        &closure.wrapper.published_comparison,
        "heed3",
        "0.22.1",
        "62bd3538173398047e263a4cda42e3115aa5c905ac017e9ce72f0e62fd54ffa9",
    )?;
    validate_native_patches(&closure.native.patches)?;
    validate_published_comparison(
        &closure.native.published_comparison,
        "lmdb-master3-sys",
        "0.2.6",
        "78b1c7bad81edaf778b5a1252beae4c52b405bc9fd5492a3e5cde3274f55f525",
    )?;
    let validated = ValidatedLmdbProofClosureV1 {
        target_triples: closure.target_triples.clone(),
        dynamic_host_dependencies: closure.link.dynamic_host_dependencies,
        encryption_features: closure.features.encryption.clone(),
    };
    Ok((closure, validated))
}

fn validate_wrapper_materialization(
    materialization: &SourceMaterializationV1,
) -> Result<(), String> {
    if materialization.upstream_script_path != WRAPPER_CONVERSION_SCRIPT
        || materialization.upstream_script_sha256 != WRAPPER_CONVERSION_SCRIPT_SHA256
        || materialization
            .additional_steps
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != WRAPPER_ADDITIONAL_STEPS
    {
        return Err("wrapper materialization record is not exact".to_owned());
    }
    Ok(())
}

#[cfg(feature = "lmdb-proof")]
pub fn run_lmdb_proof_open_close_v1(root: &Path) -> Result<LmdbProofOpenCloseReportV1, String> {
    let (_, closure) = validate_lmdb_proof_closure_document_v1(include_str!(
        "../../../vendor/lmdb-proof/closure.json"
    ))?;
    fs::create_dir_all(root)
        .map_err(|error| format!("failed to create disposable LMDB root: {error}"))?;

    let mut options = heed3::EnvOpenOptions::new();
    options.map_size(16 * 1024 * 1024).max_dbs(1);
    // SAFETY: this developer-only proof owns a new disposable directory for
    // the lifetime of the environment and does not open it a second time.
    let environment = unsafe { options.open(root) }
        .map_err(|error| format!("plain LMDB open failed: {error}"))?;
    let version = heed3::lmdb_version();
    environment.prepare_for_closing().wait();

    let mut created_files = fs::read_dir(root)
        .map_err(|error| format!("failed to inspect disposable LMDB root: {error}"))?
        .map(|entry| {
            entry.map_err(|error| error.to_string()).and_then(|entry| {
                entry
                    .file_name()
                    .into_string()
                    .map_err(|_| "LMDB created a non-UTF-8 file name".to_owned())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    created_files.sort();

    Ok(LmdbProofOpenCloseReportV1 {
        schema: "pointbreak.lmdb-proof-open-close.v1",
        mode: "plain_open_close",
        wrapper_source_commit: WRAPPER_SOURCE_COMMIT,
        native_source_commit: NATIVE_SOURCE_COMMIT,
        lmdb_version: version.string.to_owned(),
        plain: true,
        encrypted: false,
        static_native_archive: "liblmdb.a",
        dynamic_host_dependencies: closure.dynamic_host_dependencies,
        created_files,
    })
}

fn validate_package_exclusion(
    repository_root: &Path,
    package: &PackageClosureV1,
) -> Result<(), String> {
    if package.source_prefix != "vendor/lmdb-proof/" {
        return Err("proof package source prefix is not exact".to_owned());
    }
    let root_manifest = fs::read_to_string(repository_root.join(ROOT_MANIFEST))
        .map_err(|error| format!("failed to read root Cargo manifest: {error}"))?;
    if !root_manifest.contains("\"vendor/lmdb-proof/**\"")
        || !root_manifest.contains("lmdb-proof = [\"bench\", \"dep:heed3\"]")
        || !root_manifest.contains("path = \"vendor/lmdb-proof/heed3\"")
    {
        return Err(
            "root Cargo manifest does not preserve the source-only proof boundary".to_owned(),
        );
    }
    Ok(())
}

fn validate_link_closure(link: &LinkClosureV1) -> Result<(), String> {
    if link.static_archive != "liblmdb.a" || link.windows_system_libraries != ["advapi32"] {
        return Err("native link closure is not the reviewed static configuration".to_owned());
    }
    Ok(())
}

fn validate_published_comparison(
    comparison: &PublishedComparisonV1,
    package: &str,
    version: &str,
    checksum: &str,
) -> Result<(), String> {
    if comparison.package != package
        || comparison.version != version
        || comparison.checksum != checksum
        || comparison.behavior_authority
    {
        return Err("published native comparison identity is incomplete".to_owned());
    }
    Ok(())
}

fn validate_native_patches(patches: &[PatchIdentityV1]) -> Result<(), String> {
    let [patch] = patches else {
        return Err("native patch inventory is not exact".to_owned());
    };
    if patch.id != "msvc-void-pointer-arithmetic"
        || patch.path != "vendor/lmdb-proof/lmdb-master3-sys/lmdb/libraries/liblmdb/mdb.c"
        || patch.preimage_sha256
            != "9c90127df98846c21dfdc2221a94180bc217b7cbf01b68cd0c60823a7075e464"
        || patch.purpose != "cast byte-address arithmetic explicitly for MSVC"
    {
        return Err("native patch inventory is not exact".to_owned());
    }
    if patch.postimage_sha256 != "508ea872cabb350711eecae33e2df3f3abd74ec27c74652204a5120cbc4632a0"
    {
        return Err("native patch inventory is not exact".to_owned());
    }
    Ok(())
}

fn release_target_triples(repository_root: &Path) -> Result<Vec<String>, String> {
    let targets: serde_json::Value = serde_json::from_slice(
        &fs::read(repository_root.join(RELEASE_TARGET_MANIFEST))
            .map_err(|error| format!("failed to read release target manifest: {error}"))?,
    )
    .map_err(|error| format!("release target manifest is invalid: {error}"))?;
    targets
        .as_array()
        .ok_or_else(|| "release target manifest must be an array".to_owned())?
        .iter()
        .map(|target| {
            target
                .get("rust-target")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| "release target manifest is missing a Rust target".to_owned())
        })
        .collect()
}

fn validate_file_identities(
    repository_root: &Path,
    identities: &[FileIdentityV1],
) -> Result<(), String> {
    for identity in identities {
        validate_relative_path(&identity.path)?;
        validate_sha256_text(&identity.sha256, &identity.path)?;
        let path = repository_root.join(&identity.path);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("missing closure input {}: {error}", identity.path))?;
        if !metadata.file_type().is_file() {
            return Err(format!(
                "closure input is not a regular file: {}",
                identity.path
            ));
        }
        let actual = file_sha256(&path)?;
        if actual != identity.sha256 {
            return Err(format!("closure input hash mismatch: {}", identity.path));
        }
    }
    Ok(())
}

fn validate_tree_identity(
    repository_root: &Path,
    relative_root: &str,
    expected: &str,
) -> Result<(), String> {
    validate_sha256_text(expected, relative_root)?;
    let root = repository_root.join(relative_root);
    let mut files = Vec::new();
    collect_regular_files(&root, &root, &mut files)?;
    files.sort();

    let mut digest = Sha256::new();
    for relative in files {
        let relative_text = normalized_relative_path(&relative, relative_root)?;
        digest.update(relative_text.as_bytes());
        digest.update([0]);
        digest.update(file_sha256(&root.join(&relative))?.as_bytes());
        digest.update(b"\n");
    }
    let actual = format!("{:x}", digest.finalize());
    if actual != expected {
        return Err(format!(
            "closure source tree hash mismatch: {relative_root}"
        ));
    }
    Ok(())
}

fn normalized_relative_path(path: &Path, root_label: &str) -> Result<String, String> {
    path.components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .ok_or_else(|| format!("non-UTF-8 path in closure tree {root_label}"))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|components| components.join("/"))
}

fn collect_regular_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    for entry in fs::read_dir(current).map_err(|error| {
        format!(
            "failed to read closure source tree {}: {error}",
            current.display()
        )
    })? {
        let entry =
            entry.map_err(|error| format!("failed to read closure source entry: {error}"))?;
        let metadata = entry
            .file_type()
            .map_err(|error| format!("failed to inspect closure source entry: {error}"))?;
        let path = entry.path();
        if metadata.is_symlink() {
            return Err(format!(
                "symlinks are forbidden in closure source tree: {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            collect_regular_files(root, &path, files)?;
        } else if metadata.is_file() {
            files.push(
                path.strip_prefix(root)
                    .map_err(|error| format!("invalid closure source path: {error}"))?
                    .to_path_buf(),
            );
        } else {
            return Err(format!(
                "special file is forbidden in closure source tree: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("failed to read closure input {}: {error}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err("closure input path must be a normalized relative path".to_owned());
    }
    Ok(())
}

fn validate_sha256_text(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!("{label} is not a lowercase SHA-256 identity"));
    }
    Ok(())
}

fn identity_paths(identities: &[FileIdentityV1]) -> BTreeSet<&str> {
    identities
        .iter()
        .map(|identity| identity.path.as_str())
        .collect()
}

fn string_set<'a>(values: &'a [&'a str]) -> BTreeSet<&'a str> {
    values.iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::{Value, json};

    use super::validate_lmdb_proof_closure_v1;

    const MANIFEST: &str = include_str!("../../../vendor/lmdb-proof/closure.json");

    fn repository_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    fn mutated_manifest(path: &[&str], value: Value) -> String {
        let mut manifest: Value = serde_json::from_str(MANIFEST).expect("closure fixture parses");
        let mut cursor = &mut manifest;
        for component in &path[..path.len() - 1] {
            cursor = cursor
                .get_mut(*component)
                .unwrap_or_else(|| panic!("missing fixture component {component}"));
        }
        cursor[path[path.len() - 1]] = value;
        serde_json::to_string(&manifest).expect("mutated manifest serializes")
    }

    #[test]
    fn exact_lmdb_proof_closure_is_self_consistent() {
        let closure = validate_lmdb_proof_closure_v1(MANIFEST, repository_root())
            .expect("exact closure validates");

        assert_eq!(closure.target_triples.len(), 8);
        assert!(!closure.dynamic_host_dependencies);
        assert!(closure.encryption_features.is_empty());
    }

    #[test]
    fn published_native_pin_is_not_behavior_authority() {
        let manifest = mutated_manifest(&["native", "behaviorAuthority"], json!("registry"));

        assert_eq!(
            validate_lmdb_proof_closure_v1(&manifest, repository_root()).unwrap_err(),
            "native behavior authority must be the immutable upstream Git source"
        );
    }

    #[test]
    fn overlay_and_native_identity_are_exact() {
        let wrong_overlay = mutated_manifest(
            &["wrapper", "sourceCommit"],
            json!("0000000000000000000000000000000000000000"),
        );
        assert_eq!(
            validate_lmdb_proof_closure_v1(&wrong_overlay, repository_root()).unwrap_err(),
            "wrapper source commit does not match the reviewed pin"
        );

        let wrong_native = mutated_manifest(
            &["native", "sourceCommit"],
            json!("0000000000000000000000000000000000000000"),
        );
        assert_eq!(
            validate_lmdb_proof_closure_v1(&wrong_native, repository_root()).unwrap_err(),
            "native source commit does not match the reviewed pin"
        );

        let wrong_materialization = mutated_manifest(
            &["wrapper", "materialization", "upstreamScriptSha256"],
            json!("0000000000000000000000000000000000000000000000000000000000000000"),
        );
        assert_eq!(
            validate_lmdb_proof_closure_v1(&wrong_materialization, repository_root()).unwrap_err(),
            "wrapper materialization record is not exact"
        );
    }

    #[test]
    fn mutable_sources_and_unexpected_features_are_rejected() {
        let mutable = mutated_manifest(
            &["wrapper", "repository"],
            json!("https://github.com/meilisearch/heed/tree/main"),
        );
        assert_eq!(
            validate_lmdb_proof_closure_v1(&mutable, repository_root()).unwrap_err(),
            "wrapper repository is not the recorded upstream source"
        );

        let unexpected = mutated_manifest(&["features", "wrapper"], json!(["use-valgrind"]));
        assert_eq!(
            validate_lmdb_proof_closure_v1(&unexpected, repository_root()).unwrap_err(),
            "wrapper feature closure is not the minimum plain set"
        );
    }

    #[test]
    fn license_generated_input_and_dynamic_link_gaps_fail_closed() {
        let missing_license = mutated_manifest(&["licenses"], json!([]));
        assert_eq!(
            validate_lmdb_proof_closure_v1(&missing_license, repository_root()).unwrap_err(),
            "source license and notice inventory is incomplete"
        );

        let missing_bindings = mutated_manifest(&["generatedInputs"], json!([]));
        assert_eq!(
            validate_lmdb_proof_closure_v1(&missing_bindings, repository_root()).unwrap_err(),
            "generated binding inventory is incomplete"
        );

        let dynamic = mutated_manifest(&["link", "dynamicHostDependencies"], json!(true));
        assert_eq!(
            validate_lmdb_proof_closure_v1(&dynamic, repository_root()).unwrap_err(),
            "dynamic host dependencies are forbidden"
        );
    }

    #[test]
    fn every_release_target_and_default_exclusion_are_required() {
        let seven_targets = mutated_manifest(
            &["targetTriples"],
            json!([
                "x86_64-apple-darwin",
                "aarch64-apple-darwin",
                "x86_64-unknown-linux-gnu",
                "aarch64-unknown-linux-gnu",
                "x86_64-unknown-linux-musl",
                "aarch64-unknown-linux-musl",
                "x86_64-pc-windows-msvc"
            ]),
        );
        assert_eq!(
            validate_lmdb_proof_closure_v1(&seven_targets, repository_root()).unwrap_err(),
            "proof target matrix does not match the release target manifest"
        );

        let included = mutated_manifest(&["package", "excludedFromDefaultPackage"], json!(false));
        assert_eq!(
            validate_lmdb_proof_closure_v1(&included, repository_root()).unwrap_err(),
            "proof sources must be excluded from the default Cargo package"
        );
    }
}
