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

    #[error("unsupported state schema/version: {schema} v{version}")]
    UnsupportedStateSchemaVersion { schema: String, version: u32 },
}
