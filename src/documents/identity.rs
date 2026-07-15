use crate::model::ActorId;

pub const IDENTITY_WHOAMI_SCHEMA: &str = "pointbreak.identity-whoami";

/// The complete v1 body for the writer-identity preview document.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityWhoamiBody {
    actor_id: String,
}

impl IdentityWhoamiBody {
    pub fn actor_id(&self) -> &str {
        &self.actor_id
    }
}

/// Exact v1 envelope for `shore identity whoami`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityWhoamiDocument {
    schema: &'static str,
    version: u32,
    #[serde(flatten)]
    body: IdentityWhoamiBody,
}

impl IdentityWhoamiDocument {
    pub fn body(&self) -> &IdentityWhoamiBody {
        &self.body
    }
}

pub fn identity_whoami_document(actor_id: ActorId) -> IdentityWhoamiDocument {
    IdentityWhoamiDocument {
        schema: IDENTITY_WHOAMI_SCHEMA,
        version: 1,
        body: IdentityWhoamiBody {
            actor_id: actor_id.as_str().to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_is_the_exact_v1_shape() {
        let document = identity_whoami_document(ActorId::new("actor:local"));
        assert_eq!(
            serde_json::to_string(&document).unwrap(),
            r#"{"schema":"pointbreak.identity-whoami","version":1,"actorId":"actor:local"}"#
        );
    }
}
