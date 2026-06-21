// Translate a parsed Claude Code session into a deterministic
// `AdapterIntent` stream that Phase 4 will pick up and map onto
// `ShoreEvent`s. The translator is intentionally a translator — not an
// interpreter. Transcript-native checkpoint boundaries only; hook outputs
// surface as observation-on-checkpoint with shared `source_ref`; no
// similarity-based lineage; no payload-level lifting of `assertion_mode`
// or `source_ref` (envelope-only per Phase 2 / Codex Q1).

use super::parse::{AssistantMessage, ParsedMessage, ParsedSession, ToolUse, UserMessage};
use crate::canonical_hash::sha256_bytes_hex;
use crate::model::{ActorId, CheckpointId, LedgerId, TargetRef, TaskTargetRef, WorkObjectId};
use crate::session::event::{AssertionMode, SourceRef, SourceSpeaker, Writer, WriterProducer};

const SOURCE_SYSTEM_CLAUDE_CODE: &str = "claude_code";

/// Translate a parsed session into the deterministic intent stream.
pub fn translate_session(parsed: &ParsedSession) -> Vec<AdapterIntent> {
    let mut intents: Vec<AdapterIntent> = Vec::new();

    let initial_prompt_text = first_user_prompt_text(parsed);
    let initial_prompt_hash = sha256_bytes_hex(initial_prompt_text.as_bytes());

    let task_attempt_material = format!(
        "project_path={}\nsession_uuid={}\ninitial_prompt_hash={}",
        parsed.project_path, parsed.claude_session_uuid, initial_prompt_hash
    );
    let task_attempt_id = WorkObjectId::new(format!(
        "task-attempt:sha256:{}",
        sha256_bytes_hex(task_attempt_material.as_bytes())
    ));

    let first_user_msg: Option<&UserMessage> = parsed.messages.iter().find_map(|m| match m {
        ParsedMessage::User(u) => Some(u),
        _ => None,
    });
    let first_user_ts = first_user_msg
        .and_then(|u| u.timestamp.clone())
        .unwrap_or_default();

    intents.push(AdapterIntent::TaskAttemptCaptured {
        task_attempt_id: task_attempt_id.clone(),
        session_id: parsed.session_id.clone(),
        source_ref: Some(SourceRef::new(
            SOURCE_SYSTEM_CLAUDE_CODE,
            parsed.claude_session_uuid.clone(),
        )),
        assertion_mode: AssertionMode::Advisory,
        writer: writer_user(),
        occurred_at: first_user_ts,
        project_path: parsed.project_path.clone(),
        claude_session_uuid: parsed.claude_session_uuid.clone(),
        initial_prompt_hash,
        predecessor: None,
        source_speaker: SourceSpeaker::User,
    });

    for msg in &parsed.messages {
        let ParsedMessage::Assistant(a) = msg else {
            continue;
        };
        if !assistant_turn_produces_boundary(a) {
            continue;
        }
        let tool_use_ids: Vec<String> = a.tool_uses.iter().map(|t| t.id.clone()).collect();
        let cp_material = format!(
            "session_uuid={}\nassistant_message_id={}\ntool_use_ids={}",
            parsed.claude_session_uuid,
            a.message_id,
            tool_use_ids.join(",")
        );
        let checkpoint_id = CheckpointId::new(format!(
            "checkpoint:sha256:{}",
            sha256_bytes_hex(cp_material.as_bytes())
        ));
        let target = TargetRef::Task(TaskTargetRef::Checkpoint {
            checkpoint_id: checkpoint_id.clone(),
        });
        let occurred_at = a.timestamp.clone().unwrap_or_default();

        intents.push(AdapterIntent::CheckpointCaptured {
            checkpoint_id: checkpoint_id.clone(),
            parent_task_attempt_id: task_attempt_id.clone(),
            target: target.clone(),
            session_id: parsed.session_id.clone(),
            source_ref: Some(SourceRef::new(
                SOURCE_SYSTEM_CLAUDE_CODE,
                format!("{}#assistant:{}", parsed.claude_session_uuid, a.message_id),
            )),
            assertion_mode: AssertionMode::Advisory,
            writer: writer_agent(),
            occurred_at: occurred_at.clone(),
            assistant_message_id: a.message_id.clone(),
            tool_use_ids,
            source_speaker: SourceSpeaker::Agent,
        });

        for tu in &a.tool_uses {
            let Some(result) = &tu.matching_result else {
                continue;
            };
            if !content_carries_hook_output(&result.content) {
                continue;
            }
            intents.push(AdapterIntent::ObservationRecorded {
                parent_task_attempt_id: task_attempt_id.clone(),
                target: target.clone(),
                session_id: parsed.session_id.clone(),
                source_ref: Some(SourceRef::new(
                    SOURCE_SYSTEM_CLAUDE_CODE,
                    format!("{}#tool_result:{}", parsed.claude_session_uuid, tu.id),
                )),
                assertion_mode: AssertionMode::Advisory,
                writer: writer_agent(),
                occurred_at: occurred_at.clone(),
                title: format!("tool_result: {}", tu.name),
                source_speaker: SourceSpeaker::Agent,
            });
        }
    }

    intents
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdapterIntent {
    TaskAttemptCaptured {
        task_attempt_id: WorkObjectId,
        session_id: LedgerId,
        source_ref: Option<SourceRef>,
        assertion_mode: AssertionMode,
        writer: Writer,
        occurred_at: String,
        project_path: String,
        claude_session_uuid: String,
        initial_prompt_hash: String,
        predecessor: Option<WorkObjectId>,
        source_speaker: SourceSpeaker,
    },
    CheckpointCaptured {
        checkpoint_id: CheckpointId,
        parent_task_attempt_id: WorkObjectId,
        target: TargetRef,
        session_id: LedgerId,
        source_ref: Option<SourceRef>,
        assertion_mode: AssertionMode,
        writer: Writer,
        occurred_at: String,
        assistant_message_id: String,
        tool_use_ids: Vec<String>,
        source_speaker: SourceSpeaker,
    },
    ObservationRecorded {
        parent_task_attempt_id: WorkObjectId,
        target: TargetRef,
        session_id: LedgerId,
        source_ref: Option<SourceRef>,
        assertion_mode: AssertionMode,
        writer: Writer,
        occurred_at: String,
        title: String,
        source_speaker: SourceSpeaker,
    },
    /// Reserved variant. The Claude Code session adapter never emits this:
    /// fabricating input-request structure from a transcript would cross from
    /// translator into interpreter (Tripwire 4). Future work may surface
    /// input-request intents from a different write-side signal.
    InputRequestRequested,
}

fn first_user_prompt_text(parsed: &ParsedSession) -> String {
    for msg in &parsed.messages {
        if let ParsedMessage::User(u) = msg
            && !u.text.is_empty()
        {
            return u.text.clone();
        }
    }
    String::new()
}

/// Q1 boundary: assistant turns produce a checkpoint when they either mutate
/// state (file edit, side-effecting tool call) **or** carry verification
/// output (hook tail) on a paired tool_result. Hook output on a read-only
/// turn still counts — it is a structural signal that something verified the
/// turn, not a prose interpretation.
fn assistant_turn_produces_boundary(msg: &AssistantMessage) -> bool {
    assistant_turn_is_state_mutating(msg) || assistant_turn_has_verification_output(msg)
}

fn assistant_turn_is_state_mutating(msg: &AssistantMessage) -> bool {
    msg.tool_uses.iter().any(tool_use_is_state_mutating)
}

fn assistant_turn_has_verification_output(msg: &AssistantMessage) -> bool {
    msg.tool_uses.iter().any(|t| {
        t.matching_result
            .as_ref()
            .is_some_and(|r| content_carries_hook_output(&r.content))
    })
}

fn tool_use_is_state_mutating(tool: &ToolUse) -> bool {
    match tool.name.as_str() {
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => true,
        "Bash" => bash_input_has_side_effect(&tool.input),
        _ => false,
    }
}

/// Phase 3 classifier: shell operators (`>`, `|`, `;`, `&&`, `||`) make a
/// Bash command state-mutating; otherwise the first word of the command must
/// appear in a deliberately short read-only allowlist or the command is
/// classified as mutating. The bias is false-negative on read-only: ambiguous
/// commands count as state-mutating so the translator does not silently drop
/// a checkpoint. `find` is **not** on the allowlist — `find -delete`,
/// `find -exec`, and similar argv forms are mutating. Phase 4/5 may tighten
/// this rule when projection-time evidence accumulates. Do not infer effect
/// from `tool_result` content — that crosses into interpretation.
fn bash_input_has_side_effect(input: &serde_json::Value) -> bool {
    let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
    if command.contains('>')
        || command.contains('|')
        || command.contains(';')
        || command.contains("&&")
        || command.contains("||")
    {
        return true;
    }
    let head = command.split_whitespace().next().unwrap_or("");
    !matches!(
        head,
        "ls" | "cat" | "grep" | "rg" | "head" | "tail" | "wc" | "stat" | "file"
    )
}

fn content_carries_hook_output(content: &str) -> bool {
    content.contains("<system-reminder>")
}

// The speaker fact rides in the task payloads as `sourceSpeaker` (ADR-0007);
// these writers differ only by the synthetic actor id attributing the write.
fn writer_user() -> Writer {
    Writer {
        actor_id: ActorId::new("actor:claude_code:user"),
        producer: WriterProducer {
            name: "claude_code".to_owned(),
            version: String::new(),
        },
    }
}

fn writer_agent() -> Writer {
    Writer {
        actor_id: ActorId::new("actor:claude_code:assistant"),
        producer: WriterProducer {
            name: "claude_code".to_owned(),
            version: String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::super::parse::parse_session;
    use super::*;
    use crate::session::event::SourceSpeaker;

    const FIXTURE_UUID: &str = "a0ce57f0-485d-45b7-98fc-f0f13f467d72";
    const FIRST_USER_PROMPT: &str = "Can we update the README.md to use `boardwalk::transitions!` like the drivers/boardwalk-mock-led/src/lib.rs?";

    fn fixture_path() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/claude_code_session/a0ce57f0-485d-45b7-98fc-f0f13f467d72.jsonl")
    }

    #[test]
    fn translate_emits_task_attempt_captured_intent_first() {
        let parsed = parse_session(&fixture_path()).expect("parses");
        let intents = translate_session(&parsed);

        let initial_prompt_hash = sha256_bytes_hex(FIRST_USER_PROMPT.as_bytes());
        let material = format!(
            "project_path={}\nsession_uuid={}\ninitial_prompt_hash={}",
            parsed.project_path, FIXTURE_UUID, initial_prompt_hash
        );
        let expected_id = WorkObjectId::new(format!(
            "task-attempt:sha256:{}",
            sha256_bytes_hex(material.as_bytes())
        ));

        match &intents[0] {
            AdapterIntent::TaskAttemptCaptured {
                task_attempt_id,
                session_id,
                source_ref,
                assertion_mode,
                claude_session_uuid,
                predecessor,
                ..
            } => {
                assert_eq!(task_attempt_id, &expected_id);
                assert_eq!(
                    session_id,
                    &LedgerId::new(format!("session:claude:{FIXTURE_UUID}"))
                );
                assert_eq!(
                    source_ref,
                    &Some(SourceRef::new("claude_code", FIXTURE_UUID))
                );
                assert_eq!(*assertion_mode, AssertionMode::Advisory);
                assert_eq!(claude_session_uuid, FIXTURE_UUID);
                assert_eq!(predecessor, &None);
            }
            other => panic!("intents[0] must be TaskAttemptCaptured, got {other:?}"),
        }
    }

    #[test]
    fn translate_emits_checkpoint_captured_at_state_mutating_assistant_turns() {
        let parsed = parse_session(&fixture_path()).expect("parses");
        let intents = translate_session(&parsed);

        let task_attempt_id = match &intents[0] {
            AdapterIntent::TaskAttemptCaptured {
                task_attempt_id, ..
            } => task_attempt_id.clone(),
            _ => panic!("expected TaskAttemptCaptured first"),
        };

        let target_state_mutating = parsed.messages.iter().find_map(|m| match m {
            ParsedMessage::Assistant(a) if assistant_turn_is_state_mutating(a) => Some(a.clone()),
            _ => None,
        });
        let target_a = target_state_mutating.expect("fixture has a state-mutating turn");
        let tool_use_ids: Vec<String> = target_a.tool_uses.iter().map(|t| t.id.clone()).collect();
        let material = format!(
            "session_uuid={}\nassistant_message_id={}\ntool_use_ids={}",
            FIXTURE_UUID,
            target_a.message_id,
            tool_use_ids.join(",")
        );
        let expected_checkpoint_id = CheckpointId::new(format!(
            "checkpoint:sha256:{}",
            sha256_bytes_hex(material.as_bytes())
        ));

        let matching = intents
            .iter()
            .filter_map(|i| match i {
                AdapterIntent::CheckpointCaptured {
                    checkpoint_id,
                    parent_task_attempt_id,
                    target,
                    assistant_message_id,
                    ..
                } if assistant_message_id == &target_a.message_id => {
                    Some((checkpoint_id, parent_task_attempt_id, target))
                }
                _ => None,
            })
            .next();
        let (cp_id, parent, target) =
            matching.expect("matching CheckpointCaptured intent for the state-mutating turn");
        assert_eq!(cp_id, &expected_checkpoint_id);
        assert_eq!(parent, &task_attempt_id);

        let target_json = serde_json::to_value(target).unwrap();
        assert_eq!(target_json["task"]["kind"], "checkpoint");
        assert_eq!(
            target_json["task"]["checkpointId"],
            expected_checkpoint_id.as_str()
        );
    }

    #[test]
    fn translate_does_not_emit_checkpoint_at_non_state_mutating_assistant_turns() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("synthetic-readonly.jsonl");
        let uuid = "22222222-2222-2222-2222-222222222222";
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"{{"type":"agent-setting","sessionId":"{uuid}"}}"#).unwrap();
        writeln!(
            f,
            r#"{{"type":"user","sessionId":"{uuid}","uuid":"u1","timestamp":"t1","message":{{"role":"user","content":"please inspect"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant","sessionId":"{uuid}","uuid":"a1","timestamp":"t2","message":{{"id":"msg_readonly","role":"assistant","content":[{{"type":"tool_use","id":"tu_r1","name":"Read","input":{{"file_path":"/tmp/x"}}}}]}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"user","sessionId":"{uuid}","uuid":"u2","timestamp":"t3","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"tu_r1","content":"file contents"}}]}}}}"#
        )
        .unwrap();

        let parsed = parse_session(&path).expect("parses");
        let intents = translate_session(&parsed);

        let has_checkpoint = intents
            .iter()
            .any(|i| matches!(i, AdapterIntent::CheckpointCaptured { .. }));
        assert!(
            !has_checkpoint,
            "a turn whose only tool_use is Read must not produce a checkpoint"
        );
    }

    #[test]
    fn translate_emits_observation_recorded_for_hook_output() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("synthetic-hook.jsonl");
        let uuid = "33333333-3333-3333-3333-333333333333";
        let tu_id = "tu_hook_1";
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"{{"type":"agent-setting","sessionId":"{uuid}"}}"#).unwrap();
        writeln!(
            f,
            r#"{{"type":"user","sessionId":"{uuid}","uuid":"u1","timestamp":"t1","message":{{"role":"user","content":"please update the file"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant","sessionId":"{uuid}","uuid":"a1","timestamp":"t2","message":{{"id":"msg_edit","role":"assistant","content":[{{"type":"tool_use","id":"{tu_id}","name":"Edit","input":{{"file_path":"/tmp/x","old_string":"a","new_string":"b"}}}}]}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"user","sessionId":"{uuid}","uuid":"u2","timestamp":"t3","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"{tu_id}","content":"edit applied\n<system-reminder>formatter ran</system-reminder>"}}]}}}}"#
        )
        .unwrap();

        let parsed = parse_session(&path).expect("parses");
        let intents = translate_session(&parsed);

        let checkpoint_id = intents
            .iter()
            .find_map(|i| match i {
                AdapterIntent::CheckpointCaptured { checkpoint_id, .. } => {
                    Some(checkpoint_id.clone())
                }
                _ => None,
            })
            .expect("checkpoint should be emitted for the Edit turn");

        let observation = intents
            .iter()
            .find_map(|i| match i {
                AdapterIntent::ObservationRecorded {
                    target,
                    source_ref,
                    assertion_mode,
                    ..
                } => Some((target.clone(), source_ref.clone(), *assertion_mode)),
                _ => None,
            })
            .expect("hook output emits an ObservationRecorded");

        assert_eq!(
            observation.0,
            TargetRef::Task(TaskTargetRef::Checkpoint { checkpoint_id })
        );
        assert_eq!(
            observation.1,
            Some(SourceRef::new(
                "claude_code",
                format!("{uuid}#tool_result:{tu_id}")
            ))
        );
        assert_eq!(observation.2, AssertionMode::Advisory);
    }

    #[test]
    fn translate_emits_checkpoint_and_observation_for_verification_output_on_read_only_turn() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("synthetic-verification.jsonl");
        let uuid = "55555555-5555-5555-5555-555555555555";
        let tu_id = "tu_verify_1";
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"{{"type":"agent-setting","sessionId":"{uuid}"}}"#).unwrap();
        writeln!(
            f,
            r#"{{"type":"user","sessionId":"{uuid}","uuid":"u1","timestamp":"t1","message":{{"role":"user","content":"please inspect"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant","sessionId":"{uuid}","uuid":"a1","timestamp":"t2","message":{{"id":"msg_readonly_verified","role":"assistant","content":[{{"type":"tool_use","id":"{tu_id}","name":"Read","input":{{"file_path":"/tmp/x"}}}}]}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"user","sessionId":"{uuid}","uuid":"u2","timestamp":"t3","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"{tu_id}","content":"contents\n<system-reminder>hook ran</system-reminder>"}}]}}}}"#
        )
        .unwrap();

        let parsed = parse_session(&path).expect("parses");
        let intents = translate_session(&parsed);

        let checkpoint_id = intents
            .iter()
            .find_map(|i| match i {
                AdapterIntent::CheckpointCaptured { checkpoint_id, .. } => {
                    Some(checkpoint_id.clone())
                }
                _ => None,
            })
            .expect("verification output is a Q1 boundary even on a read-only tool");

        let observation = intents.iter().find_map(|i| match i {
            AdapterIntent::ObservationRecorded {
                target, source_ref, ..
            } => Some((target.clone(), source_ref.clone())),
            _ => None,
        });
        let (target, source_ref) = observation.expect("observation emitted for the hook output");
        assert_eq!(
            target,
            TargetRef::Task(TaskTargetRef::Checkpoint { checkpoint_id })
        );
        assert_eq!(
            source_ref,
            Some(SourceRef::new(
                "claude_code",
                format!("{uuid}#tool_result:{tu_id}")
            ))
        );
    }

    #[test]
    fn translate_attaches_distinct_writer_actors_to_each_intent() {
        let parsed = parse_session(&fixture_path()).expect("parses");
        let intents = translate_session(&parsed);

        let task_writer = match &intents[0] {
            AdapterIntent::TaskAttemptCaptured { writer, .. } => writer.clone(),
            _ => panic!("first intent is TaskAttemptCaptured"),
        };
        let assistant_writer = intents
            .iter()
            .find_map(|i| match i {
                AdapterIntent::CheckpointCaptured { writer, .. } => Some(writer.clone()),
                _ => None,
            })
            .expect("at least one CheckpointCaptured intent");

        assert_eq!(task_writer.actor_id.as_str(), "actor:claude_code:user");
        assert_eq!(
            assistant_writer.actor_id.as_str(),
            "actor:claude_code:assistant"
        );
        assert_ne!(task_writer.actor_id, assistant_writer.actor_id);
    }

    #[test]
    fn translate_records_source_speaker_on_each_intent() {
        let parsed = parse_session(&fixture_path()).expect("parses");
        let intents = translate_session(&parsed);

        match &intents[0] {
            AdapterIntent::TaskAttemptCaptured { source_speaker, .. } => {
                assert_eq!(*source_speaker, SourceSpeaker::User);
            }
            other => panic!("first intent must be TaskAttemptCaptured, got {other:?}"),
        }
        let mut saw_assistant_intent = false;
        for intent in &intents[1..] {
            match intent {
                AdapterIntent::CheckpointCaptured { source_speaker, .. }
                | AdapterIntent::ObservationRecorded { source_speaker, .. } => {
                    saw_assistant_intent = true;
                    assert_eq!(*source_speaker, SourceSpeaker::Agent);
                }
                _ => {}
            }
        }
        assert!(saw_assistant_intent, "fixture yields assistant intents");
    }

    #[test]
    fn translate_does_not_emit_input_request_intents() {
        let parsed = parse_session(&fixture_path()).expect("parses");
        let intents = translate_session(&parsed);

        let any_input_request = intents
            .iter()
            .any(|i| matches!(i, AdapterIntent::InputRequestRequested));
        assert!(
            !any_input_request,
            "adapter never fabricates input-request structure from a transcript (Tripwire 4)"
        );
    }

    #[test]
    fn translate_does_not_set_predecessor_by_similarity() {
        let parsed = parse_session(&fixture_path()).expect("parses");
        let intents_a = translate_session(&parsed);
        let intents_b = translate_session(&parsed);

        for intent in intents_a.iter().chain(intents_b.iter()) {
            if let AdapterIntent::TaskAttemptCaptured { predecessor, .. } = intent {
                assert!(
                    predecessor.is_none(),
                    "adapter never sets predecessor by similarity (Q4 / Tripwire 5)"
                );
            }
        }
    }

    #[test]
    fn translate_is_deterministic() {
        let parsed = parse_session(&fixture_path()).expect("parses");
        let first = translate_session(&parsed);
        let second = translate_session(&parsed);
        assert_eq!(first, second);
    }
}
