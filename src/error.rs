pub type Result<T> = std::result::Result<T, ShoreError>;

#[derive(Debug, thiserror::Error)]
pub enum ShoreError {
    #[error("{0}")]
    Message(String),

    #[error("json parse failed: {0}")]
    Json(#[from] serde_json::Error),

    #[error(
        "git command failed: {command}\nstatus: {status}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    )]
    GitCommand {
        command: String,
        status: String,
        stdout: String,
        stderr: String,
    },

    #[error("invalid event: {message}")]
    InvalidEvent { message: String },

    #[error("unsupported event schema/version: {schema} v{version}")]
    UnsupportedEventSchemaVersion { schema: String, version: u32 },

    #[error("unsupported event type {event_type}: {migration_hint}")]
    UnsupportedEventType {
        event_type: String,
        migration_hint: String,
    },

    #[error("unsupported state schema/version: {schema} v{version}")]
    UnsupportedStateSchemaVersion { schema: String, version: u32 },

    #[error("{reason}")]
    WorkflowInputInvalid { reason: String },

    #[error("unknown claude code session line type `{kind}` at line {line}")]
    UnknownClaudeSessionLineType { line: usize, kind: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_input_invalid_preserves_display_text() {
        let err = ShoreError::WorkflowInputInvalid {
            reason: "track is required".to_owned(),
        };
        assert_eq!(err.to_string(), "track is required");
    }
}
