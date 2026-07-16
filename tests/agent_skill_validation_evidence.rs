use std::path::Path;

#[test]
fn agent_skills_and_docs_adopt_validation_evidence_workflow() {
    assert_contains(
        "skills/pointbreak-author/SKILL.md",
        "pointbreak validation add",
    );
    assert_contains(
        "skills/pointbreak-author/SKILL.md",
        "That pre-change failure did not run against the captured revision",
    );
    assert_not_contains(
        "skills/pointbreak-author/SKILL.md",
        "Initial red run failed before the parser change",
    );
    assert_contains(
        "skills/pointbreak-author/SKILL.md",
        "pointbreak validation list",
    );
    assert_contains(
        "skills/pointbreak-reviewer/SKILL.md",
        "pointbreak validation list",
    );
    assert_contains(
        "skills/pointbreak-reviewer/SKILL.md",
        "pointbreak validation add",
    );
    assert_contains(
        "skills/pointbreak-author-response/SKILL.md",
        "pointbreak validation list",
    );
    assert_contains("docs/agent-authoring.md", "pointbreak validation add");
    assert_contains("docs/agent-authoring.md", "pointbreak validation list");
    assert_contains("skills/README.md", "validation evidence");
}

#[test]
fn repo_owned_agent_skills_use_only_the_flat_pointbreak_cli() {
    for skill in [
        "skills/pointbreak-author/SKILL.md",
        "skills/pointbreak-reviewer/SKILL.md",
        "skills/pointbreak-author-response/SKILL.md",
    ] {
        assert_not_contains(skill, "shore ");
        assert_not_contains(skill, "SHORE_");
        assert_not_contains(skill, ".shore");
        assert_not_contains(skill, "pointbreak review");
    }
}

#[test]
fn agent_skills_document_automatic_signing_and_enrollment() {
    for skill in [
        "skills/pointbreak-author/SKILL.md",
        "skills/pointbreak-reviewer/SKILL.md",
        "skills/pointbreak-author-response/SKILL.md",
    ] {
        // Auto-keygen + enrollment pointer is present in every shipped skill.
        assert_contains(skill, "pointbreak key enroll");
        // The opt-out escape is documented.
        assert_contains(skill, "POINTBREAK_SIGNING=off");
        // The canonical agent actor-id export is documented.
        assert_contains(
            skill,
            "export POINTBREAK_ACTOR_ID=\"actor:agent:${agent_name}\"",
        );
        // No private plan labels leak into shipped skills.
        assert_not_contains(skill, "Phase 5");
        assert_not_contains(skill, "0066");
    }
}

#[test]
fn agent_skills_note_human_use_ssh_path() {
    for skill in [
        "skills/pointbreak-author/SKILL.md",
        "skills/pointbreak-reviewer/SKILL.md",
        "skills/pointbreak-author-response/SKILL.md",
    ] {
        // Humans can reuse an existing SSH key; agents still auto-keygen (note stays).
        assert_contains(skill, "pointbreak key use-ssh");
        assert_contains(skill, "pointbreak key enroll");
        // No private plan labels leak into shipped skills.
        assert_not_contains(skill, "0067");
        assert_not_contains(skill, "0066");
    }
}

fn assert_not_contains(relative_path: &str, needle: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {relative_path}: {error}"));

    assert!(
        !contents.contains(needle),
        "{relative_path} should not contain {needle:?}"
    );
}

fn assert_contains(relative_path: &str, needle: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {relative_path}: {error}"));

    assert!(
        contents.contains(needle),
        "{relative_path} should contain {needle:?}"
    );
}
