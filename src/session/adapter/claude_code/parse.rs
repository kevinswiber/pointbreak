// Parser for Claude Code session JSONL files.
//
// `SessionId` convention: `session:claude:<claude-session-uuid>` where the
// UUID is the hyphenated value Claude Code records under `sessionId` on every
// line. Each JSONL file maps to its own `SessionId`; cross-rollout continuity
// (`--resume`, parentUuid chains) is the explicit `predecessor` concern
// deferred to a later phase.
//
// Strict mode: an unrecognized top-level `type` value fails the parse with
// `ShoreError::UnknownClaudeSessionLineType`. Silent skipping would let the
// adapter drift away from real session content.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Deserialize;

use crate::error::{Result, ShoreError};
use crate::model::SessionId;

/// Decode a Claude Code session JSONL into a typed [`ParsedSession`].
///
/// Returns `Err(ShoreError::UnknownClaudeSessionLineType)` on any unrecognized
/// top-level `type` value, line-numbered from 1.
pub fn parse_session(path: &Path) -> Result<ParsedSession> {
    let file = std::fs::File::open(path)
        .map_err(|e| ShoreError::Message(format!("open {}: {}", path.display(), e)))?;
    let reader = BufReader::new(file);

    let parent_dir_path = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_owned();

    let mut messages: Vec<ParsedMessage> = Vec::new();
    let mut session_uuid: Option<String> = None;
    let mut first_cwd: Option<String> = None;

    for (idx, line) in reader.lines().enumerate() {
        let line_no = idx + 1;
        let line = line.map_err(|e| ShoreError::Message(format!("read line {line_no}: {e}")))?;
        if line.trim().is_empty() {
            continue;
        }

        let header: LineHeader = serde_json::from_str(&line)?;
        if session_uuid.is_none() {
            session_uuid = Some(header.session_id.clone());
        }
        if first_cwd.is_none()
            && let Some(cwd) = header.cwd.as_ref()
            && !cwd.is_empty()
        {
            first_cwd = Some(cwd.clone());
        }

        match header.r#type.as_str() {
            "user" => {
                let decoded = decode_user_line(&line, line_no)?;
                messages.extend(decoded);
            }
            "assistant" => {
                let parsed = decode_assistant_line(&line, line_no)?;
                messages.push(ParsedMessage::Assistant(parsed));
            }
            kind if KNOWN_METADATA_TYPES.contains(&kind) => {
                // Recognized metadata line; consumed but not surfaced as a
                // ParsedMessage. Task 3.3 decides whether to track these.
            }
            other => {
                return Err(ShoreError::UnknownClaudeSessionLineType {
                    line: line_no,
                    kind: other.to_owned(),
                });
            }
        }
    }

    let claude_session_uuid = session_uuid.unwrap_or_default();
    let session_id = SessionId::new(format!("session:claude:{claude_session_uuid}"));
    let project_path = first_cwd.unwrap_or(parent_dir_path);

    pair_tool_uses_with_results(&mut messages);

    Ok(ParsedSession {
        session_id,
        claude_session_uuid,
        project_path,
        messages,
    })
}

/// Known top-level `type` values recognized as benign metadata. Updates here
/// are deliberate: Phase 2 no-migration-shim discipline applies — new line
/// types fail strict mode until added.
const KNOWN_METADATA_TYPES: &[&str] = &[
    "agent-name",
    "agent-setting",
    "ai-title",
    "attachment",
    "bridge-session",
    "file-history-snapshot",
    "last-prompt",
    "permission-mode",
    "pr-link",
    "queue-operation",
    "system",
    "worktree-state",
];

#[derive(Debug, Clone)]
pub struct ParsedSession {
    pub session_id: SessionId,
    pub claude_session_uuid: String,
    pub project_path: String,
    pub messages: Vec<ParsedMessage>,
}

#[derive(Debug, Clone)]
pub enum ParsedMessage {
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolResult(ToolResultMessage),
}

#[derive(Debug, Clone)]
pub struct UserMessage {
    pub uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub timestamp: Option<String>,
    pub line_number: usize,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct AssistantMessage {
    pub uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub timestamp: Option<String>,
    pub line_number: usize,
    pub message_id: String,
    pub text: Vec<TextBlock>,
    pub thinking: Vec<ThinkingBlock>,
    pub tool_uses: Vec<ToolUse>,
}

#[derive(Debug, Clone)]
pub struct ToolResultMessage {
    pub uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub timestamp: Option<String>,
    pub line_number: usize,
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone)]
pub struct ToolUse {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
    pub matching_result: Option<ToolResultRef>,
}

#[derive(Debug, Clone)]
pub struct ToolResultRef {
    pub line_number: usize,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone)]
pub struct TextBlock {
    pub text: String,
}

#[derive(Clone)]
pub struct ThinkingBlock {
    pub thinking: String,
    pub signature: String,
}

impl std::fmt::Debug for ThinkingBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ThinkingBlock {{ len: {} }}", self.thinking.len())
    }
}

#[derive(Debug, Deserialize)]
struct LineHeader {
    #[serde(rename = "type")]
    r#type: String,
    #[serde(rename = "sessionId", default)]
    session_id: String,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserLineRaw {
    #[serde(default)]
    uuid: Option<String>,
    #[serde(rename = "parentUuid", default)]
    parent_uuid: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    message: Option<RawMessage>,
}

#[derive(Debug, Deserialize)]
struct AssistantLineRaw {
    #[serde(default)]
    uuid: Option<String>,
    #[serde(rename = "parentUuid", default)]
    parent_uuid: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    message: RawMessage,
}

#[derive(Debug, Deserialize)]
struct RawMessage {
    #[serde(default)]
    id: Option<String>,
    content: serde_json::Value,
}

fn decode_user_line(line: &str, line_no: usize) -> Result<Vec<ParsedMessage>> {
    let raw: UserLineRaw = serde_json::from_str(line)?;
    let message = match raw.message {
        Some(m) => m,
        None => {
            return Ok(vec![ParsedMessage::User(UserMessage {
                uuid: raw.uuid,
                parent_uuid: raw.parent_uuid,
                timestamp: raw.timestamp,
                line_number: line_no,
                text: String::new(),
            })]);
        }
    };

    if let Some(text) = message.content.as_str() {
        return Ok(vec![ParsedMessage::User(UserMessage {
            uuid: raw.uuid,
            parent_uuid: raw.parent_uuid,
            timestamp: raw.timestamp,
            line_number: line_no,
            text: text.to_owned(),
        })]);
    }

    let blocks = match message.content.as_array() {
        Some(b) => b,
        None => {
            return Ok(vec![ParsedMessage::User(UserMessage {
                uuid: raw.uuid,
                parent_uuid: raw.parent_uuid,
                timestamp: raw.timestamp,
                line_number: line_no,
                text: String::new(),
            })]);
        }
    };

    let mut out: Vec<ParsedMessage> = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    for block in blocks {
        let kind = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "tool_result" => {
                let tool_use_id = block
                    .get("tool_use_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                let is_error = block
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let content = stringify_content(block.get("content"));
                out.push(ParsedMessage::ToolResult(ToolResultMessage {
                    uuid: raw.uuid.clone(),
                    parent_uuid: raw.parent_uuid.clone(),
                    timestamp: raw.timestamp.clone(),
                    line_number: line_no,
                    tool_use_id,
                    content,
                    is_error,
                }));
            }
            "text" => {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    texts.push(t.to_owned());
                }
            }
            _ => {}
        }
    }

    if out.is_empty() {
        out.push(ParsedMessage::User(UserMessage {
            uuid: raw.uuid,
            parent_uuid: raw.parent_uuid,
            timestamp: raw.timestamp,
            line_number: line_no,
            text: texts.join("\n\n"),
        }));
    } else if !texts.is_empty() {
        // Tool-result and free-text in the same user line is uncommon but
        // possible. Preserve both: the User text record sits before the
        // tool_result entries in source order would require finer interleaving,
        // but the message-count check stays accurate.
        out.insert(
            0,
            ParsedMessage::User(UserMessage {
                uuid: raw.uuid,
                parent_uuid: raw.parent_uuid,
                timestamp: raw.timestamp,
                line_number: line_no,
                text: texts.join("\n\n"),
            }),
        );
    }
    Ok(out)
}

fn decode_assistant_line(line: &str, line_no: usize) -> Result<AssistantMessage> {
    let raw: AssistantLineRaw = serde_json::from_str(line)?;
    let mut text: Vec<TextBlock> = Vec::new();
    let mut thinking: Vec<ThinkingBlock> = Vec::new();
    let mut tool_uses: Vec<ToolUse> = Vec::new();

    if let Some(blocks) = raw.message.content.as_array() {
        for block in blocks {
            let kind = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match kind {
                "text" => {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        text.push(TextBlock { text: t.to_owned() });
                    }
                }
                "thinking" => {
                    thinking.push(ThinkingBlock {
                        thinking: block
                            .get("thinking")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned(),
                        signature: block
                            .get("signature")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned(),
                    });
                }
                "tool_use" => {
                    tool_uses.push(ToolUse {
                        id: block
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned(),
                        name: block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned(),
                        input: block
                            .get("input")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                        matching_result: None,
                    });
                }
                _ => {}
            }
        }
    }

    Ok(AssistantMessage {
        uuid: raw.uuid,
        parent_uuid: raw.parent_uuid,
        timestamp: raw.timestamp,
        line_number: line_no,
        message_id: raw.message.id.unwrap_or_default(),
        text,
        thinking,
        tool_uses,
    })
}

fn stringify_content(value: Option<&serde_json::Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if let Some(s) = value.as_str() {
        return s.to_owned();
    }
    if let Some(blocks) = value.as_array() {
        let mut out: Vec<String> = Vec::new();
        for block in blocks {
            if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                out.push(t.to_owned());
            } else {
                out.push(block.to_string());
            }
        }
        return out.join("\n");
    }
    value.to_string()
}

fn pair_tool_uses_with_results(messages: &mut [ParsedMessage]) {
    let mut by_id: HashMap<String, ToolResultRef> = HashMap::new();
    for msg in messages.iter() {
        if let ParsedMessage::ToolResult(tr) = msg {
            by_id.insert(
                tr.tool_use_id.clone(),
                ToolResultRef {
                    line_number: tr.line_number,
                    content: tr.content.clone(),
                    is_error: tr.is_error,
                },
            );
        }
    }
    for msg in messages.iter_mut() {
        if let ParsedMessage::Assistant(a) = msg {
            for tu in a.tool_uses.iter_mut() {
                tu.matching_result = by_id.get(&tu.id).cloned();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::model::SessionId;

    fn fixture_path() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/claude_code_session/a0ce57f0-485d-45b7-98fc-f0f13f467d72.jsonl")
    }

    const FIXTURE_UUID: &str = "a0ce57f0-485d-45b7-98fc-f0f13f467d72";

    #[test]
    fn parse_session_uses_cwd_from_jsonl_for_project_path() {
        let parsed = parse_session(&fixture_path()).expect("fixture parses");
        assert_eq!(
            parsed.project_path, "/Users/kevin/src/boardwalk",
            "project_path must come from the cwd field in the JSONL so identity is stable under file moves"
        );
    }

    #[test]
    fn parse_session_falls_back_to_parent_dir_when_no_cwd_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("no-cwd.jsonl");
        let uuid = "66666666-6666-6666-6666-666666666666";
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"{{"type":"agent-setting","sessionId":"{uuid}"}}"#).unwrap();
        writeln!(
            f,
            r#"{{"type":"user","sessionId":"{uuid}","uuid":"u1","message":{{"role":"user","content":"hi"}}}}"#
        )
        .unwrap();

        let parsed = parse_session(&path).expect("parses");
        let expected = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap()
            .to_owned();
        assert_eq!(parsed.project_path, expected);
    }

    #[test]
    fn parse_session_decodes_chosen_fixture_into_typed_messages() {
        let parsed = parse_session(&fixture_path()).expect("fixture parses");

        assert_eq!(
            parsed.session_id,
            SessionId::new(format!("session:claude:{FIXTURE_UUID}"))
        );
        assert_eq!(parsed.claude_session_uuid, FIXTURE_UUID);
        assert_eq!(parsed.messages.len(), 44);
    }

    #[test]
    fn parse_session_pairs_tool_use_with_tool_result_by_id() {
        let parsed = parse_session(&fixture_path()).expect("fixture parses");

        let mut tool_uses_total = 0usize;
        let mut tool_uses_paired = 0usize;
        for msg in &parsed.messages {
            if let ParsedMessage::Assistant(a) = msg {
                for tu in &a.tool_uses {
                    tool_uses_total += 1;
                    if let Some(r) = &tu.matching_result {
                        tool_uses_paired += 1;
                        assert!(r.line_number > 0, "matching_result line number is recorded");
                    }
                }
            }
        }
        assert_eq!(tool_uses_total, 17, "fixture has 17 tool_uses");
        assert_eq!(
            tool_uses_paired, 17,
            "chosen fixture is cleanly terminated — every tool_use pairs"
        );

        for msg in &parsed.messages {
            if let ParsedMessage::ToolResult(tr) = msg {
                let prior_use = parsed.messages.iter().any(|m| {
                    if let ParsedMessage::Assistant(a) = m {
                        a.tool_uses.iter().any(|tu| tu.id == tr.tool_use_id)
                    } else {
                        false
                    }
                });
                assert!(
                    prior_use,
                    "every tool_result must have a prior tool_use with matching id: {}",
                    tr.tool_use_id
                );
            }
        }
    }

    #[test]
    fn parse_session_preserves_assistant_thinking_blocks() {
        let parsed = parse_session(&fixture_path()).expect("fixture parses");

        let any_thinking = parsed
            .messages
            .iter()
            .filter_map(|m| match m {
                ParsedMessage::Assistant(a) => Some(a),
                _ => None,
            })
            .any(|a| !a.thinking.is_empty());

        assert!(
            any_thinking,
            "at least one assistant message must carry parsed thinking blocks"
        );
    }

    #[test]
    fn parse_session_extracts_assistant_turn_state_mutations() {
        let parsed = parse_session(&fixture_path()).expect("fixture parses");

        let edit_or_write: Vec<_> = parsed
            .messages
            .iter()
            .filter_map(|m| match m {
                ParsedMessage::Assistant(a) => Some(a),
                _ => None,
            })
            .flat_map(|a| a.tool_uses.iter())
            .filter(|t| {
                matches!(
                    t.name.as_str(),
                    "Edit" | "Write" | "MultiEdit" | "NotebookEdit"
                )
            })
            .collect();

        assert!(
            !edit_or_write.is_empty(),
            "fixture should expose at least one Edit/Write tool_use as a typed accessor"
        );
        let sample = edit_or_write.first().unwrap();
        assert!(
            sample.input.is_object(),
            "tool input is preserved as a serde_json::Value"
        );
    }

    #[test]
    fn parse_session_rejects_unknown_line_type_when_strict() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("synthetic.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"agent-setting","sessionId":"11111111-2222-3333-4444-555555555555"}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"future-type-3000","sessionId":"11111111-2222-3333-4444-555555555555"}}"#
        )
        .unwrap();

        let err = parse_session(&path).expect_err("strict mode rejects unknown line type");
        match err {
            ShoreError::UnknownClaudeSessionLineType { line, kind } => {
                assert_eq!(line, 2);
                assert_eq!(kind, "future-type-3000");
            }
            other => panic!("expected UnknownClaudeSessionLineType, got {other:?}"),
        }
    }

    #[test]
    fn parse_session_decodes_each_tool_result_block_in_parallel_user_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("parallel-tool-results.jsonl");
        let uuid = "44444444-4444-4444-4444-444444444444";
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"{{"type":"agent-setting","sessionId":"{uuid}"}}"#).unwrap();
        writeln!(
            f,
            r#"{{"type":"user","sessionId":"{uuid}","uuid":"u1","message":{{"role":"user","content":"please run two things in parallel"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant","sessionId":"{uuid}","uuid":"a1","message":{{"id":"msg_parallel","role":"assistant","content":[{{"type":"tool_use","id":"tu_a","name":"Read","input":{{"file_path":"/tmp/a"}}}},{{"type":"tool_use","id":"tu_b","name":"Read","input":{{"file_path":"/tmp/b"}}}}]}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"user","sessionId":"{uuid}","uuid":"u2","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"tu_a","content":"contents of a"}},{{"type":"tool_result","tool_use_id":"tu_b","content":"contents of b"}}]}}}}"#
        )
        .unwrap();

        let parsed = parse_session(&path).expect("parses");

        let tool_results: Vec<&ToolResultMessage> = parsed
            .messages
            .iter()
            .filter_map(|m| match m {
                ParsedMessage::ToolResult(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(
            tool_results.len(),
            2,
            "each tool_result block in a parallel user line decodes to its own ParsedMessage"
        );
        let ids: std::collections::HashSet<_> = tool_results
            .iter()
            .map(|t| t.tool_use_id.as_str())
            .collect();
        assert!(ids.contains("tu_a"));
        assert!(ids.contains("tu_b"));

        // Pairing must reach both parallel tool uses.
        let mut paired_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for msg in &parsed.messages {
            if let ParsedMessage::Assistant(a) = msg {
                for tu in &a.tool_uses {
                    if tu.matching_result.is_some() {
                        paired_ids.insert(tu.id.clone());
                    }
                }
            }
        }
        assert!(paired_ids.contains("tu_a"));
        assert!(paired_ids.contains("tu_b"));
    }

    #[test]
    fn parse_session_session_id_uses_claude_namespace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("synthetic.jsonl");
        let uuid = "11111111-2222-3333-4444-555555555555";
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"{{"type":"agent-setting","sessionId":"{uuid}"}}"#).unwrap();
        writeln!(
            f,
            r#"{{"type":"system","sessionId":"{uuid}","subtype":"info"}}"#
        )
        .unwrap();

        let parsed = parse_session(&path).expect("parses");

        assert_eq!(
            parsed.session_id,
            SessionId::new(format!("session:claude:{uuid}"))
        );
        assert_eq!(parsed.claude_session_uuid, uuid);
    }
}
