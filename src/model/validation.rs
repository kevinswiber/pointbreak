use serde::{Deserialize, Serialize};

use super::RevisionId;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Passed,
    Failed,
    Errored,
    Skipped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationTrigger {
    Manual,
    Push,
    PullRequest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ValidationTarget {
    Revision { revision_id: RevisionId },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_status_serializes_four_variants_in_snake_case() {
        for (variant, wire) in [
            (ValidationStatus::Passed, "\"passed\""),
            (ValidationStatus::Failed, "\"failed\""),
            (ValidationStatus::Errored, "\"errored\""),
            (ValidationStatus::Skipped, "\"skipped\""),
        ] {
            assert_eq!(serde_json::to_string(&variant).unwrap(), wire);
            let back: ValidationStatus = serde_json::from_str(wire).unwrap();
            assert_eq!(back, variant);
        }
    }

    #[test]
    fn validation_trigger_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&ValidationTrigger::Manual).unwrap(),
            "\"manual\""
        );
        assert_eq!(
            serde_json::to_string(&ValidationTrigger::Push).unwrap(),
            "\"push\""
        );
        assert_eq!(
            serde_json::to_string(&ValidationTrigger::PullRequest).unwrap(),
            "\"pull_request\""
        );
    }

    #[test]
    fn validation_target_revision_round_trips_with_kind_tag_and_is_path_free() {
        let target = ValidationTarget::Revision {
            revision_id: RevisionId::new("rev:sha256:def"),
        };

        let value = serde_json::to_value(&target).unwrap();
        assert_eq!(value["kind"], "revision");
        assert_eq!(value["revisionId"], "rev:sha256:def");

        let back: ValidationTarget = serde_json::from_value(value).unwrap();
        assert_eq!(back, target);
    }
}
