use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, io};

use pointbreak::documents::{history_document, revision_show_document};
use pointbreak::model::RevisionId;
use pointbreak::session::event::ShoreEvent;
use pointbreak::session::{
    ArtifactKind, ImportArtifactOptions, IngestEventsOptions, ReviewHistoryOptions,
    RevisionShowOptions, export_artifact, import_artifact, ingest_events, read_events,
    referenced_artifacts, review_history, show_revision,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type PackResult<T> = Result<T, Box<dyn Error>>;

const SCHEMA: &str = "pointbreak.review-example-pack";
const VERSION: u32 = 1;
const NAME: &str = "checkout-refactor";
const CLASSIFICATION: &str = "reproducible_sample_record";
const REVISION: &str =
    "rev:sha256:fa6981d38de12a850da707b69657e7a9153120c92a0dd08f534fbb40394d885f";
const TRACK: &str = "example:marketing-review-proof";
const EVENT_SET_HASH: &str =
    "sha256:cabdabbbdf88ab71b43faee14cc28bf8e407e5c2bfc18d07af4bba126da12243";
const BASE_COMMIT: &str = "f1a8ed1801f669b1b846e482d198092cd6e617df";
const TARGET_COMMIT: &str = "3e7b4b3e1e1e7cccfc14a4c724204ff381b315e4";
const RESPONSE_COMMIT: &str = "c4f50c2dc010f69f9080d0ad6b0999728568c3c1";

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackManifest {
    pub schema: String,
    pub version: u32,
    pub name: String,
    pub classification: String,
    pub producer: ProducerManifest,
    pub record: RecordManifest,
    pub source: SourceManifest,
    pub events: FileManifest,
    pub artifacts: Vec<ArtifactManifest>,
    pub documents: DocumentSetManifest,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProducerManifest {
    pub name: String,
    pub version: String,
    pub commit: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordManifest {
    pub revision: String,
    pub track: String,
    pub selected_assessment: String,
    pub event_set_hash: String,
    pub event_count: usize,
    pub writer_actors: Vec<String>,
    pub verification_status: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceManifest {
    pub bundle_path: String,
    pub bundle_sha256: String,
    pub bundle_ref: String,
    pub base: GitObjectManifest,
    pub target: GitObjectManifest,
    pub response: GitObjectManifest,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitObjectManifest {
    pub commit_oid: String,
    pub tree_oid: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileManifest {
    pub path: String,
    pub count: usize,
    pub sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactManifest {
    pub kind: String,
    pub content_hash: String,
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DocumentSetManifest {
    pub history: DocumentManifest,
    pub revision: DocumentManifest,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentManifest {
    pub path: String,
    pub schema: String,
    pub version: u32,
    pub sha256: String,
    pub filters: serde_json::Value,
}

pub fn export_pack(source_repo: &Path, output: &Path) -> PackResult<()> {
    let source_repo = absolute(source_repo)?;
    let output = absolute(output)?;
    let parent = output
        .parent()
        .ok_or_else(|| invalid("output must have a parent directory"))?;
    fs::create_dir_all(parent)?;
    let stage = parent.join(format!(".{}-stage-{}", NAME, std::process::id()));
    let backup = parent.join(format!(".{}-backup-{}", NAME, std::process::id()));
    remove_if_exists(&stage)?;
    remove_if_exists(&backup)?;

    let result = (|| {
        build_pack(&source_repo, &stage)?;
        verify_pack(&stage)?;

        if output.exists() {
            fs::rename(&output, &backup)?;
        }
        if let Err(error) = fs::rename(&stage, &output) {
            if backup.exists() {
                let _ = fs::rename(&backup, &output);
            }
            return Err(error.into());
        }
        remove_if_exists(&backup)?;
        Ok(())
    })();

    let _ = remove_if_exists(&stage);
    if result.is_err() && backup.exists() && !output.exists() {
        let _ = fs::rename(&backup, &output);
    }
    result
}

pub fn verify_pack(pack: &Path) -> PackResult<()> {
    let manifest = read_manifest(pack)?;
    require(manifest.schema == SCHEMA, "manifest.schema")?;
    require(manifest.version == VERSION, "manifest.version")?;
    require(manifest.name == NAME, "manifest.name")?;
    require(
        manifest.classification == CLASSIFICATION,
        "manifest.classification",
    )?;
    // Checked-in packs retain the producer recorded when they were created;
    // newly exported packs use the current Pointbreak producer name.
    require(
        matches!(manifest.producer.name.as_str(), "shore" | "pointbreak"),
        "producer.name",
    )?;
    require(
        manifest.producer.version.split('.').count() == 3,
        "producer.version",
    )?;
    require(
        manifest.producer.commit.len() == 40
            && manifest
                .producer
                .commit
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        "producer.commit",
    )?;
    require(manifest.record.event_count == 13, "record.eventCount")?;
    require(manifest.record.revision == REVISION, "record.revision")?;
    require(manifest.record.track == TRACK, "record.track")?;
    require(
        manifest.record.selected_assessment == "accepted",
        "record.selectedAssessment",
    )?;
    require(
        manifest.record.event_set_hash == EVENT_SET_HASH,
        "record.eventSetHash",
    )?;
    require(
        manifest.record.verification_status == "unsigned",
        "record.verificationStatus",
    )?;
    require(
        manifest.source.base.commit_oid == BASE_COMMIT,
        "source.base.commitOid",
    )?;
    require(
        manifest.source.target.commit_oid == TARGET_COMMIT,
        "source.target.commitOid",
    )?;
    require(
        manifest.source.response.commit_oid == RESPONSE_COMMIT,
        "source.response.commitOid",
    )?;
    for object in [
        &manifest.source.base,
        &manifest.source.target,
        &manifest.source.response,
    ] {
        require(
            object.tree_oid.len() == 40
                && object.tree_oid.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "source treeOid",
        )?;
    }

    verify_file(pack, &manifest.events.path, &manifest.events.sha256)?;
    let events = read_events_file(&pack.join(&manifest.events.path))?;
    require(events.len() == manifest.events.count, "events.count")?;
    require(
        events.len() == manifest.record.event_count,
        "record.eventCount",
    )?;

    let writers = events
        .iter()
        .map(|event| event.writer.actor_id.as_str().to_owned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    require(
        writers == manifest.record.writer_actors,
        "record.writerActors",
    )?;
    require(
        events.iter().all(|event| event.signature.is_none()),
        "record.verificationStatus",
    )?;

    verify_file(
        pack,
        &manifest.source.bundle_path,
        &manifest.source.bundle_sha256,
    )?;
    verify_file(
        pack,
        &manifest.documents.history.path,
        &manifest.documents.history.sha256,
    )?;
    verify_file(
        pack,
        &manifest.documents.revision.path,
        &manifest.documents.revision.sha256,
    )?;
    verify_documents(pack, &manifest)?;
    let bundle_verify = Command::new("git")
        .args(["bundle", "verify"])
        .arg(pack.join(&manifest.source.bundle_path))
        .output()?;
    require(bundle_verify.status.success(), "source.bundle verify")?;
    let bundle_heads = Command::new("git")
        .args(["bundle", "list-heads"])
        .arg(pack.join(&manifest.source.bundle_path))
        .output()?;
    require(bundle_heads.status.success(), "source.bundle")?;
    require(
        String::from_utf8(bundle_heads.stdout)?
            .lines()
            .any(|line| line == format!("{} {}", RESPONSE_COMMIT, manifest.source.bundle_ref)),
        "source.bundle refs/heads/main",
    )?;

    let refs = referenced_artifacts(&events)?;
    require(refs.len() == manifest.artifacts.len(), "artifacts")?;
    let by_hash = manifest
        .artifacts
        .iter()
        .map(|artifact| (artifact.content_hash.as_str(), artifact))
        .collect::<BTreeMap<_, _>>();
    for artifact in refs {
        let entry = by_hash
            .get(artifact.content_hash())
            .ok_or_else(|| invalid(format!("artifact {} is missing", artifact.content_hash())))?;
        require(
            entry.kind == artifact_kind(artifact.kind()),
            "artifacts.kind",
        )?;
        verify_file(pack, &entry.path, &entry.sha256)?;
    }
    Ok(())
}

pub fn materialize_pack(pack: &Path, output: &Path) -> PackResult<()> {
    verify_pack(pack)?;
    if output.exists() && fs::read_dir(output)?.next().is_some() {
        return Err(invalid(format!(
            "materialization destination is not empty: {}",
            output.display()
        ))
        .into());
    }
    if output.exists() {
        fs::remove_dir(output)?;
    }

    let manifest = read_manifest(pack)?;
    run(
        Command::new("git")
            .arg("clone")
            .arg(pack.join(&manifest.source.bundle_path))
            .arg(output),
        "clone source.bundle",
    )?;

    let events = read_events_file(&pack.join(&manifest.events.path))?;
    ingest_events(IngestEventsOptions::new(output, events.clone()))?;
    let artifacts = manifest
        .artifacts
        .iter()
        .map(|artifact| (artifact.content_hash.as_str(), artifact))
        .collect::<BTreeMap<_, _>>();
    for artifact in referenced_artifacts(&events)? {
        let entry = artifacts
            .get(artifact.content_hash())
            .ok_or_else(|| invalid(format!("artifact {} is missing", artifact.content_hash())))?;
        import_artifact(ImportArtifactOptions::new(
            output,
            artifact,
            fs::read(pack.join(&entry.path))?,
        ))?;
    }

    let (history, revision) = render_documents(output)?;
    require(
        history == fs::read(pack.join(&manifest.documents.history.path))?,
        "documents.history",
    )?;
    require(
        revision == fs::read(pack.join(&manifest.documents.revision.path))?,
        "documents.revision",
    )?;
    Ok(())
}

fn build_pack(source_repo: &Path, stage: &Path) -> PackResult<()> {
    fs::create_dir_all(stage.join("artifacts"))?;
    fs::create_dir_all(stage.join("exports"))?;

    let mut events = read_events(source_repo)?;
    events.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.event_id.as_str().cmp(right.event_id.as_str()))
    });
    require(events.len() == 13, "source event count")?;
    let events_bytes = json_bytes(&events)?;
    fs::write(stage.join("events.json"), &events_bytes)?;

    let bundle_path = stage.join("source.bundle");
    run(
        Command::new("git")
            .arg("-C")
            .arg(source_repo)
            .args(["bundle", "create"])
            .arg(&bundle_path)
            .arg("--all"),
        "create source.bundle",
    )?;

    let (history_bytes, revision_bytes) = render_documents(source_repo)?;
    fs::write(stage.join("exports/history.json"), &history_bytes)?;
    fs::write(stage.join("exports/revision.json"), &revision_bytes)?;

    let mut artifact_manifest = Vec::new();
    for artifact in referenced_artifacts(&events)? {
        let bytes = export_artifact(source_repo, &artifact)?;
        let filename = format!(
            "artifacts/sha256-{}.bin",
            artifact.content_hash().trim_start_matches("sha256:")
        );
        fs::write(stage.join(&filename), &bytes)?;
        artifact_manifest.push(ArtifactManifest {
            kind: artifact_kind(artifact.kind()).to_owned(),
            content_hash: artifact.content_hash().to_owned(),
            path: filename,
            sha256: sha256(&bytes),
        });
    }

    let writer_actors = events
        .iter()
        .map(|event| event.writer.actor_id.as_str().to_owned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let manifest = PackManifest {
        schema: SCHEMA.to_owned(),
        version: VERSION,
        name: NAME.to_owned(),
        classification: CLASSIFICATION.to_owned(),
        producer: ProducerManifest {
            name: "pointbreak".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            commit: git_output(
                Path::new(env!("CARGO_MANIFEST_DIR")),
                &["rev-parse", "HEAD"],
            )?,
        },
        record: RecordManifest {
            revision: REVISION.to_owned(),
            track: TRACK.to_owned(),
            selected_assessment: "accepted".to_owned(),
            event_set_hash: EVENT_SET_HASH.to_owned(),
            event_count: events.len(),
            writer_actors,
            verification_status: "unsigned".to_owned(),
        },
        source: SourceManifest {
            bundle_path: "source.bundle".to_owned(),
            bundle_sha256: sha256(&fs::read(&bundle_path)?),
            bundle_ref: "refs/heads/main".to_owned(),
            base: git_object(source_repo, BASE_COMMIT)?,
            target: git_object(source_repo, TARGET_COMMIT)?,
            response: git_object(source_repo, RESPONSE_COMMIT)?,
        },
        events: FileManifest {
            path: "events.json".to_owned(),
            count: events.len(),
            sha256: sha256(&events_bytes),
        },
        artifacts: artifact_manifest,
        documents: DocumentSetManifest {
            history: DocumentManifest {
                path: "exports/history.json".to_owned(),
                schema: "pointbreak.review-history".to_owned(),
                version: 1,
                sha256: sha256(&history_bytes),
                filters: serde_json::json!({
                    "revisionId": REVISION,
                    "includeBody": true
                }),
            },
            revision: DocumentManifest {
                path: "exports/revision.json".to_owned(),
                schema: "pointbreak.review-revision".to_owned(),
                version: 2,
                sha256: sha256(&revision_bytes),
                filters: serde_json::json!({
                    "revisionId": REVISION,
                    "trackId": TRACK,
                    "includeBody": true
                }),
            },
        },
    };
    fs::write(stage.join("manifest.json"), json_bytes(&manifest)?)?;
    fs::write(
        stage.join("README.md"),
        "# Checkout refactor Review example\n\nThis artifact-complete, unsigned example can be verified and materialized with the repository's `review_example_pack` maintainer tool. `events.json` and the referenced artifact bytes are authoritative; the checked Review documents are derived projections.\n",
    )?;
    Ok(())
}

fn render_documents(repo: &Path) -> PackResult<(Vec<u8>, Vec<u8>)> {
    let revision_id = RevisionId::new(REVISION);
    let history = history_document(review_history(
        ReviewHistoryOptions::new(repo)
            .with_revision_id(revision_id.clone())
            .with_include_body(true),
    )?);
    let revision = revision_show_document(show_revision(
        RevisionShowOptions::new(repo)
            .with_revision_id(revision_id)
            .with_track(TRACK)
            .with_include_body(true),
    )?);
    Ok((json_bytes(&history)?, json_bytes(&revision)?))
}

fn verify_documents(pack: &Path, manifest: &PackManifest) -> PackResult<()> {
    let history: serde_json::Value =
        serde_json::from_slice(&fs::read(pack.join(&manifest.documents.history.path))?)?;
    require(
        history["schema"] == manifest.documents.history.schema,
        "documents.history.schema",
    )?;
    require(
        history["version"] == manifest.documents.history.version,
        "documents.history.version",
    )?;
    require(
        history["eventSetHash"] == EVENT_SET_HASH,
        "documents.history.eventSetHash",
    )?;
    require(history["eventCount"] == 13, "documents.history.eventCount")?;
    require(
        history["historyCount"] == 13,
        "documents.history.historyCount",
    )?;
    require(
        history["filters"] == manifest.documents.history.filters,
        "documents.history.filters",
    )?;

    let revision: serde_json::Value =
        serde_json::from_slice(&fs::read(pack.join(&manifest.documents.revision.path))?)?;
    require(
        revision["schema"] == manifest.documents.revision.schema,
        "documents.revision.schema",
    )?;
    require(
        revision["version"] == manifest.documents.revision.version,
        "documents.revision.version",
    )?;
    require(
        revision["eventSetHash"] == EVENT_SET_HASH,
        "documents.revision.eventSetHash",
    )?;
    require(
        revision["eventCount"] == 13,
        "documents.revision.eventCount",
    )?;
    require(
        revision["filters"] == manifest.documents.revision.filters,
        "documents.revision.filters",
    )?;
    require(
        revision["currentAssessment"]["assessment"] == "accepted",
        "documents.revision.currentAssessment",
    )?;
    require(
        revision["commitRange"].get("liveness").is_none(),
        "documents.revision liveness must be absent",
    )?;

    let assessments = revision["assessments"]
        .as_array()
        .ok_or_else(|| invalid("documents.revision.assessments"))?;
    require(assessments.len() == 2, "documents.revision.assessments")?;
    require(
        assessments[0]["assessment"] == "needs_changes" && assessments[0]["status"] == "replaced",
        "documents.revision needs_changes lifecycle",
    )?;
    require(
        assessments[1]["assessment"] == "accepted"
            && assessments[1]["status"] == "current"
            && assessments[1]["replaces"] == serde_json::json!([assessments[0]["id"].clone()]),
        "documents.revision accepted replacement",
    )?;
    let request = &revision["inputRequests"][0];
    require(
        request["reasonCode"] == "manual_decision_required"
            && request["status"] == "responded"
            && request["responses"][0]["outcome"] == "approved",
        "documents.revision request lifecycle",
    )?;
    Ok(())
}

fn read_manifest(pack: &Path) -> PackResult<PackManifest> {
    let path = pack.join("manifest.json");
    Ok(serde_json::from_slice(&fs::read(&path).map_err(
        |error| invalid(format!("read {}: {error}", path.display())),
    )?)?)
}

fn read_events_file(path: &Path) -> PackResult<Vec<ShoreEvent>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn verify_file(pack: &Path, relative: &str, expected: &str) -> PackResult<()> {
    let relative_path = Path::new(relative);
    require(
        !relative_path.is_absolute()
            && !relative_path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir)),
        format!("unsafe manifest path: {relative}"),
    )?;
    let bytes = fs::read(pack.join(relative_path))?;
    require(
        sha256(&bytes) == expected,
        format!("digest mismatch for {relative}"),
    )
}

fn git_object(repo: &Path, commit: &str) -> PackResult<GitObjectManifest> {
    Ok(GitObjectManifest {
        commit_oid: git_output(repo, &["rev-parse", commit])?,
        tree_oid: git_output(repo, &["rev-parse", &format!("{commit}^{{tree}}")])?,
    })
}

fn git_output(repo: &Path, args: &[&str]) -> PackResult<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(invalid(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn run(command: &mut Command, label: &str) -> PackResult<()> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(invalid(format!(
            "{label} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
        .into());
    }
    Ok(())
}

fn json_bytes(value: &impl Serialize) -> PackResult<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn artifact_kind(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Object => "object",
        ArtifactKind::Body => "body",
    }
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else if path.exists() {
        fs::remove_file(path)
    } else {
        Ok(())
    }
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn require(condition: bool, message: impl Into<String>) -> PackResult<()> {
    if condition {
        Ok(())
    } else {
        Err(invalid(message).into())
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
