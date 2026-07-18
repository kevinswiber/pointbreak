use std::collections::BTreeMap;

use serde::Serialize;

use super::DiagnosticDocument;

pub const VERSION_SCHEMA: &str = "pointbreak.version";
pub const VERSION_DISPLAY: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("POINTBREAK_BUILD_DESCRIBE"),
    ")"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildSourceV1 {
    Git,
    Package,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildIdentityV1 {
    pub source: BuildSourceV1,
    pub commit: Option<String>,
    pub describe: String,
    pub dirty: bool,
}

impl BuildIdentityV1 {
    pub fn current() -> Self {
        let source = match env!("POINTBREAK_BUILD_SOURCE") {
            "git" => BuildSourceV1::Git,
            "package" => BuildSourceV1::Package,
            value => unreachable!("build.rs emitted unsupported build source {value:?}"),
        };
        let commit = match source {
            BuildSourceV1::Git => Some(env!("POINTBREAK_BUILD_COMMIT").to_owned()),
            BuildSourceV1::Package => None,
        };
        let dirty = match env!("POINTBREAK_BUILD_DIRTY") {
            "true" => true,
            "false" => false,
            value => unreachable!("build.rs emitted unsupported dirty state {value:?}"),
        };
        Self {
            source,
            commit,
            describe: env!("POINTBREAK_BUILD_DESCRIBE").to_owned(),
            dirty,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionBody {
    pub cli_version: String,
    pub build: BuildIdentityV1,
    pub documents: BTreeMap<&'static str, u32>,
}

impl VersionBody {
    pub fn current() -> Self {
        Self {
            cli_version: env!("CARGO_PKG_VERSION").to_owned(),
            build: BuildIdentityV1::current(),
            documents: super::document_registry().iter().copied().collect(),
        }
    }

    pub fn display_version(&self) -> String {
        format!("{} ({})", self.cli_version, self.build.describe)
    }
}

pub fn version_document() -> DiagnosticDocument<VersionBody> {
    DiagnosticDocument::new(VERSION_SCHEMA, VersionBody::current(), Vec::new())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    use super::*;

    #[test]
    fn registry_covers_the_extension_required_documents() {
        let registry = crate::documents::document_registry();
        for (schema, version) in [
            ("pointbreak.version", 1),
            ("pointbreak.attention-list", 1),
            ("pointbreak.identity-whoami", 1),
            ("pointbreak.review-revision-list", 1),
            ("pointbreak.review-capture", 1),
            ("pointbreak.review-observation-add", 1),
            ("pointbreak.store-status", 1),
            ("pointbreak.review-revision", 2),
            ("pointbreak.review-snapshot", 1),
            ("pointbreak.inspect-freshness", 1),
            ("pointbreak.inspect-startup", 1),
        ] {
            assert_eq!(
                registry
                    .iter()
                    .find(|(candidate, _)| *candidate == schema)
                    .map(|(_, version)| *version),
                Some(version),
                "registry missing or mis-versioned: {schema}"
            );
        }
    }

    #[test]
    fn version_body_serializes_camel_case_with_sorted_documents() {
        let body = VersionBody::current();
        let value = serde_json::to_value(&body).unwrap();
        assert_eq!(value["cliVersion"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["build"]["source"], env!("POINTBREAK_BUILD_SOURCE"));
        match env!("POINTBREAK_BUILD_SOURCE") {
            "git" => assert_eq!(value["build"]["commit"].as_str().unwrap().len(), 40),
            "package" => assert!(value["build"]["commit"].is_null()),
            source => panic!("unexpected build source {source:?}"),
        }
        assert!(value["build"]["describe"].is_string());
        assert!(value["build"]["dirty"].is_boolean());
        let documents = value["documents"].as_object().unwrap();
        assert!(documents.len() >= 5);
        let keys: Vec<_> = documents.keys().cloned().collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(
            keys, sorted,
            "documents map must be deterministically ordered"
        );
    }

    #[test]
    fn naming_cutover_version_v1_bytes_are_frozen() {
        let mut actual = serde_json::to_value(version_document()).unwrap();
        actual["cliVersion"] = serde_json::Value::String("0.6.0".to_owned());
        assert!(
            actual.as_object_mut().unwrap().remove("build").is_some(),
            "current v1 adds build without rewriting the historical v1 fixture"
        );
        if let Some(store_paths) = actual["documents"]
            .as_object_mut()
            .unwrap()
            .remove("pointbreak.store-paths")
        {
            assert_eq!(store_paths, 1, "the approved registry addition stays v1");
        }
        let expected: serde_json::Value = serde_json::from_slice(
            &crate::test_fixtures::naming_cutover_contract_bytes("protocol/version-v1.json"),
        )
        .unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn registry_is_cli_documents_plus_the_exact_promoted_inspect_set() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut emitted = BTreeSet::from([VERSION_SCHEMA.to_owned()]);
        collect_schema_literals(&manifest_dir.join("src/cli"), &mut emitted);
        collect_schema_literals(&manifest_dir.join("src/documents"), &mut emitted);

        let cli_registered = crate::documents::cli_document_registry()
            .iter()
            .map(|(schema, _)| (*schema).to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(emitted, cli_registered);

        let promoted = crate::documents::promoted_inspect_document_registry()
            .iter()
            .map(|(schema, _)| (*schema).to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            promoted,
            BTreeSet::from([
                "pointbreak.review-snapshot".to_owned(),
                "pointbreak.inspect-freshness".to_owned(),
                "pointbreak.inspect-startup".to_owned(),
            ])
        );

        let registered = crate::documents::document_registry()
            .iter()
            .map(|(schema, _)| (*schema).to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            registered,
            cli_registered.union(&promoted).cloned().collect()
        );
        assert!(!registered.contains("pointbreak.inspect-attention"));
        assert!(!registered.contains("pointbreak.inspect-identity"));
    }

    fn collect_schema_literals(path: &Path, schemas: &mut BTreeSet<String>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "inspect") {
                    continue;
                }
                collect_schema_literals(&path, schemas);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && !path.ends_with("src/documents/mod.rs")
                && !path.ends_with("src/documents/version.rs")
                && !path.ends_with("src/documents/inspect.rs")
            {
                let source = fs::read_to_string(path).unwrap();
                for schema in schema_literals(&source) {
                    schemas.insert(schema);
                }
            }
        }
    }

    fn schema_literals(source: &str) -> Vec<String> {
        source
            .match_indices("\"pointbreak.")
            .filter_map(|(start, _)| {
                let schema = &source[start + 1..];
                let end = schema
                    .find(|character: char| {
                        !(character.is_ascii_lowercase()
                            || character.is_ascii_digit()
                            || matches!(character, '.' | '-'))
                    })
                    .unwrap_or(schema.len());
                (schema.as_bytes().get(end) == Some(&b'\"')).then(|| schema[..end].to_owned())
            })
            .collect()
    }
}
