use std::path::Path;

#[test]
fn agent_skills_and_docs_adopt_validation_evidence_workflow() {
    assert_contains(
        "skills/shoreline-author/SKILL.md",
        "shore review validation add",
    );
    assert_contains(
        "skills/shoreline-author/SKILL.md",
        "That pre-change failure did not run against the captured ReviewUnit",
    );
    assert_not_contains(
        "skills/shoreline-author/SKILL.md",
        "Initial red run failed before the parser change",
    );
    assert_contains(
        "skills/shoreline-author/SKILL.md",
        "shore review validation list",
    );
    assert_contains(
        "skills/shoreline-reviewer/SKILL.md",
        "shore review validation list",
    );
    assert_contains(
        "skills/shoreline-reviewer/SKILL.md",
        "shore review validation add",
    );
    assert_contains(
        "skills/shoreline-author-response/SKILL.md",
        "shore review validation list",
    );
    assert_contains("docs/agent-authoring.md", "shore review validation add");
    assert_contains("docs/agent-authoring.md", "shore review validation list");
    assert_contains("skills/README.md", "validation evidence");
}

#[test]
fn agent_skills_document_automatic_signing_and_enrollment() {
    for skill in [
        "skills/shoreline-author/SKILL.md",
        "skills/shoreline-reviewer/SKILL.md",
        "skills/shoreline-author-response/SKILL.md",
    ] {
        // Auto-keygen + enrollment pointer is present in every shipped skill.
        assert_contains(skill, "shore keys enroll");
        // The opt-out escape is documented.
        assert_contains(skill, "SHORE_SIGNING=off");
        // The existing agent actor-id export is unchanged.
        assert_contains(skill, "export SHORE_ACTOR_ID=\"actor:agent:${agent_name}\"");
        // No private plan labels leak into shipped skills.
        assert_not_contains(skill, "Phase 5");
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
