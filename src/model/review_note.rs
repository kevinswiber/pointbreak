use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Old,
    New,
}

pub(crate) fn push_normalized_line(payload: &mut String, line: &str) {
    payload.push_str(&line.replace("\r\n", "\n").replace('\r', "\n"));
    payload.push('\n');
}

pub(crate) fn sha256_prefixed(payload: &str) -> String {
    let digest = Sha256::digest(payload.as_bytes());
    format!("sha256:{digest:x}")
}
