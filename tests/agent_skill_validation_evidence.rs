use std::ffi::OsStr;
use std::path::Path;

/// The complete shipped product workflow set, in handoff order:
/// Author handoff -> Reviewer pass -> Author response.
const PRIMARY_SKILLS: &[&str] = &[
    "pointbreak-author",
    "pointbreak-reviewer",
    "pointbreak-author-response",
];

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

#[test]
fn shipped_primary_skill_set_is_exactly_the_three_workflow_roles() {
    let skills_dir = env::manifest_dir().join("skills");
    let mut shipped: Vec<String> = std::fs::read_dir(&skills_dir)
        .expect("read skills directory")
        .map(|entry| entry.expect("read skills directory entry"))
        .filter(|entry| entry.path().join("SKILL.md").is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    shipped.sort();

    let mut expected: Vec<String> = PRIMARY_SKILLS.iter().map(ToString::to_string).collect();
    expected.sort();

    assert_eq!(
        shipped, expected,
        "skills/ must ship exactly the three primary product workflow skills"
    );
}

#[test]
fn supported_install_command_pins_exactly_the_three_product_skills() {
    // The repository root is the installer's discovery surface and it also
    // contains development-only skills, so the supported install command must
    // select the product set explicitly.
    for skill in PRIMARY_SKILLS {
        assert_contains("skills/README.md", &format!("--skill {skill}"));
    }
    assert_contains("skills/README.md", "development-only skills");

    // Every SKILL.md in the repository lives either in the product
    // distribution directory (exactly the three roles) or under the
    // repository's own development tooling. A skill anywhere else would leak
    // into installer discovery unreviewed.
    let repo_root = env::manifest_dir();
    let mut skill_dirs = Vec::new();
    collect_skill_dirs(&repo_root, &repo_root, &mut skill_dirs);
    for dir in skill_dirs {
        assert!(
            dir.starts_with(".claude/skills/")
                || PRIMARY_SKILLS
                    .iter()
                    .any(|skill| dir == format!("skills/{skill}")),
            "unexpected skill directory in installer discovery surface: {dir}"
        );
    }
}

fn collect_skill_dirs(repo_root: &Path, dir: &Path, found: &mut Vec<String>) {
    for entry in
        std::fs::read_dir(dir).unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
    {
        let entry = entry.expect("read repository entry");
        let path = entry.path();
        let name = entry.file_name();
        if !path.is_dir()
            || [".git", ".direnv", "target", "node_modules"]
                .map(OsStr::new)
                .contains(&&*name)
        {
            continue;
        }
        if path.join("SKILL.md").is_file() {
            let relative = path
                .strip_prefix(repo_root)
                .expect("skill directory under repository root")
                .to_string_lossy()
                .replace('\\', "/");
            found.push(relative);
        }
        collect_skill_dirs(repo_root, &path, found);
    }
}

#[test]
fn skill_distribution_names_all_three_roles_and_no_private_workflow() {
    for skill in PRIMARY_SKILLS {
        assert_contains("skills/README.md", skill);
    }
    assert_order(
        "skills/README.md",
        &["Author handoff", "Reviewer pass", "Author response"],
    );
    assert_contains("skills/README.md", "three product workflow skills");
    assert_contains("skills/README.md", "Installing a skill does not run it");

    for private_workflow in ["pointbreak-loop", "review-loop", "paired-review"] {
        assert_not_contains("skills/README.md", private_workflow);
        assert_not_contains("docs/agent-authoring.md", private_workflow);
        for skill in PRIMARY_SKILLS {
            let path = format!("skills/{skill}/SKILL.md");
            assert_not_contains(&path, private_workflow);
        }
    }
}

#[test]
fn skills_readme_routes_install_into_the_five_stage_contract() {
    assert_order(
        "skills/README.md",
        &[
            "npx skills add withpointbreak/pointbreak",
            "Work -> Claims -> Evidence -> Questions -> Call",
            "`pointbreak-author`",
            "`pointbreak-reviewer`",
            "`pointbreak-author-response`",
        ],
    );
    assert_contains(
        "skills/README.md",
        "never an assessment, decision, task-completion verdict, or merge gate",
    );
}

#[test]
fn author_skill_shows_review_value_before_identity_and_trust() {
    assert_order(
        "skills/pointbreak-author/SKILL.md",
        &[
            "## Capture The Right Revision",
            "pointbreak inspect --open",
            "## Choose your track",
            "POINTBREAK_ACTOR_ID",
            "untrusted",
            ".pointbreak/allowed-signers.json",
            "## Record observations",
            "## Record validation evidence",
            "## Open input requests",
            "## Read back and hand off",
        ],
    );
    assert_contains(
        "skills/pointbreak-author/SKILL.md",
        "You are the coding agent that just authored the change",
    );
    assert_contains(
        "skills/pointbreak-author/SKILL.md",
        "the Call (`assessment`) belongs to the reviewer",
    );
    assert_contains(
        "skills/pointbreak-author/SKILL.md",
        "record checks you did not run",
    );
}

#[test]
fn reviewer_skill_reads_author_facts_before_writing_and_owns_the_call() {
    assert_order(
        "skills/pointbreak-reviewer/SKILL.md",
        &[
            "## Read the author's handoff",
            "## Choose your track",
            "POINTBREAK_ACTOR_ID",
            "untrusted",
            ".pointbreak/allowed-signers.json",
            "## Review independently",
            "## Record reviewer findings",
            "## Record reviewer validation checks",
            "## Respond to operative input requests",
            "## Add exactly one assessment",
        ],
    );
    assert_contains(
        "skills/pointbreak-reviewer/SKILL.md",
        "You are the reviewing agent",
    );
    assert_contains(
        "skills/pointbreak-reviewer/SKILL.md",
        "Never write to the author's track",
    );
    assert_contains("skills/pointbreak-reviewer/SKILL.md", "make the Call");
}

#[test]
fn author_response_skill_reuses_author_identity_and_never_assesses_or_recaptures() {
    assert_order(
        "skills/pointbreak-author-response/SKILL.md",
        &[
            "## Read the reviewer pass",
            "POINTBREAK_ACTOR_ID",
            "untrusted",
            ".pointbreak/allowed-signers.json",
            "## Classify the verdict",
            "## Respond to advisory requests",
            "## Record author response observations",
            "## Record the landing commit",
        ],
    );
    assert_contains(
        "skills/pointbreak-author-response/SKILL.md",
        "You are the agent that authored the change",
    );
    assert_contains(
        "skills/pointbreak-author-response/SKILL.md",
        "same canonical spelling",
    );
    assert_contains(
        "skills/pointbreak-author-response/SKILL.md",
        "Do not run `pointbreak assessment add`",
    );
    assert_contains(
        "skills/pointbreak-author-response/SKILL.md",
        "`pointbreak capture`; this response attaches to the existing revision",
    );
    assert_contains(
        "skills/pointbreak-author-response/SKILL.md",
        "the Call stays the reviewer's",
    );
}

#[test]
fn enrollment_is_optional_staged_and_follows_the_untrusted_explanation() {
    for skill in PRIMARY_SKILLS {
        let path = format!("skills/{skill}/SKILL.md");
        assert_order(&path, &["untrusted", ".pointbreak/allowed-signers.json"]);
        assert_contains(&path, "untrusted does not mean invalid");
        assert_contains(&path, ".pointbreak/allowed-signers.json` for human review");
        assert_contains(&path, "Enrollment is optional");
    }
    assert_contains("docs/agent-authoring.md", "untrusted does not mean invalid");
}

#[test]
fn landing_guidance_associates_the_commit_on_the_same_revision() {
    for path in [
        "skills/pointbreak-author/SKILL.md",
        "skills/pointbreak-author-response/SKILL.md",
        "docs/agent-authoring.md",
    ] {
        assert_contains(path, "pointbreak association record");
        assert_contains(path, "same revision");
    }
    assert_contains(
        "skills/pointbreak-author-response/SKILL.md",
        "`pointbreak capture` is not re-run for the landing",
    );
}

#[test]
fn agent_authoring_routes_roles_through_the_canonical_journey() {
    assert_order(
        "docs/agent-authoring.md",
        &[
            "Work -> Claims -> Evidence -> Questions -> Call",
            "`pointbreak-author`",
            "`pointbreak-reviewer`",
            "`pointbreak-author-response`",
        ],
    );
    assert_contains("docs/agent-authoring.md", "getting-started.md");
}

fn assert_order(relative_path: &str, needles: &[&str]) {
    let path = env::manifest_dir().join(relative_path);
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {relative_path}: {error}"));

    let mut cursor = 0;
    for needle in needles {
        match contents[cursor..].find(needle) {
            Some(offset) => cursor += offset + needle.len(),
            None => panic!(
                "{relative_path} should contain {needle:?} after byte {cursor} \
                 (ordered contract: {needles:?})"
            ),
        }
    }
}

fn assert_not_contains(relative_path: &str, needle: &str) {
    let path = env::manifest_dir().join(relative_path);
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {relative_path}: {error}"));

    assert!(
        !contents.contains(needle),
        "{relative_path} should not contain {needle:?}"
    );
}

fn assert_contains(relative_path: &str, needle: &str) {
    let path = env::manifest_dir().join(relative_path);
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {relative_path}: {error}"));

    assert!(
        contents.contains(needle),
        "{relative_path} should contain {needle:?}"
    );
}

// Runtime-resolved binary/manifest paths for cross-machine (e.g. Windows) archive runs.
#[path = "support/env.rs"]
#[allow(dead_code)]
mod env;
