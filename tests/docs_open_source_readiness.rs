#[test]
fn adr_0007_is_landed_and_writer_identity_docs_are_role_free() {
    let adr = std::fs::read_to_string("docs/adr/adr-0007-writer-act-vocabulary.md")
        .expect("ADR-0007 is landed in docs/adr/");
    assert!(adr.contains("**Status:** Accepted"));

    let adr4 =
        std::fs::read_to_string("docs/adr/adr-0004-event-signatures.md").expect("read ADR-0004");
    assert!(
        !adr4.contains("`role`") && !adr4.contains("writer.role"),
        "ADR-0004's to-be-signed description carries no role field"
    );

    let library_api = std::fs::read_to_string("docs/library-api.md").expect("read library API");
    assert!(
        !library_api.contains("writer role"),
        "library API signed-fields sentence carries no writer role"
    );
}

#[test]
fn cli_reference_exists_and_covers_current_commands() {
    let cli = std::fs::read_to_string("docs/cli-reference.md").expect("read CLI reference");

    for command in [
        "shore show",
        "shore dump",
        "shore review capture",
        "shore store status",
        "shore store link",
        "shore review observation add",
        "shore review input-request open",
        "shore review assessment add",
        "shore review history",
        "shore review unit show",
        "shore notes apply",
    ] {
        assert!(
            cli.contains(command),
            "missing command reference for {command}"
        );
    }

    assert!(cli.contains("shore.review-capture"));
    assert!(cli.contains("shore.review-unit"));
    assert!(cli.contains("eventSetHash"));

    assert_markdown_section_contains(
        &cli,
        "## `shore review observation`",
        &[
            "--review-unit <review-unit-id>",
            "--include-body",
            "--pretty",
            "--compact",
        ],
    );
    assert_markdown_section_contains(
        &cli,
        "## `shore review input-request`",
        &[
            "--review-unit <review-unit-id>",
            "--track <track-id>",
            "--mode operative|advisory",
            "--file <path>",
            "--include-body",
            "--pretty",
            "--compact",
        ],
    );
    assert_markdown_section_contains(
        &cli,
        "## `shore review assessment`",
        &[
            "--review-unit <review-unit-id>",
            "--include-summary",
            "--pretty",
            "--compact",
        ],
    );
}

#[test]
fn public_docs_cover_clone_local_store_behavior() {
    let cli = std::fs::read_to_string("docs/cli-reference.md").expect("read CLI reference");
    let storage = std::fs::read_to_string("docs/storage-model.md").expect("read storage model");

    assert_markdown_section_contains(
        &cli,
        "## `shore store`",
        &[
            "shore store status",
            "shore store link",
            "shore.store-status",
            "shore.store-link",
            "policyOutcome",
            "file:sha256:",
            "hard-blocking policy",
            "clone_local_capture_batch_only",
            "review unit list",
        ],
    );

    for token in [
        "clone-local store",
        "batch-only",
        "sensitivity scan",
        "hard-blocking policy",
        "inventory",
        "opaque store, clone, and repository-family refs",
    ] {
        assert!(
            storage.contains(token),
            "storage model missing clone-local behavior: {token}"
        );
    }

    for forbidden in ["Plan 0050", "Task 5", "Phase 5"] {
        assert!(!cli.contains(forbidden));
        assert!(!storage.contains(forbidden));
    }
}

#[test]
fn getting_started_walks_through_first_review() {
    let guide = std::fs::read_to_string("docs/getting-started.md").expect("read getting started");
    let normalized_guide = guide.replace("\r\n", "\n");

    for required in [
        "cargo install shoreline",
        "shore review capture",
        "shore review unit show",
        "shore review observation add",
        "shore review input-request open",
        "shore review assessment add",
        ".shore/",
    ] {
        assert!(
            guide.contains(required),
            "missing getting-started step: {required}"
        );
    }

    assert!(
        !guide.contains("<<"),
        "getting-started shell snippets should avoid heredocs"
    );
    assert!(
        !guide.contains("TMP=$(mktemp -d)"),
        "getting-started shell snippets should avoid POSIX-only assignment syntax"
    );
    assert!(
        guide.contains("printf '%s\\n'"),
        "getting-started should create sample files with shell-portable printf"
    );
    assert!(normalized_guide.contains(
        "--start-line 6 \\\n  --body \"The fallback value is visible user-facing behavior"
    ));
}

#[test]
fn contributor_docs_cover_local_development_flow() {
    let contributing = std::fs::read_to_string("CONTRIBUTING.md").expect("read contributing");

    for required in [
        "just setup-hooks",
        "just check",
        "just lint",
        "just test",
        "cog check",
        "upstream/main..HEAD",
        "unscoped commit",
        "feat/",
        "fix/",
    ] {
        assert!(
            contributing.contains(required),
            "missing contributor guidance: {required}"
        );
    }

    assert!(!contributing.contains("cog check origin/main..HEAD"));
}

#[test]
fn community_health_files_carry_required_guidance() {
    let security = std::fs::read_to_string("SECURITY.md").expect("read security policy");
    let pull_request_template = std::fs::read_to_string(".github/pull_request_template.md")
        .expect("read pull request template");
    let bug_report_template = std::fs::read_to_string(".github/ISSUE_TEMPLATE/bug_report.md")
        .expect("read bug report template");

    assert!(security.contains("kevin@swiber.dev"));
    assert!(security.contains("Please do not file security-sensitive reports"));
    assert!(pull_request_template.contains("just check"));
    assert!(pull_request_template.contains("unscoped"));
    assert!(bug_report_template.contains("## Reproduction"));
    assert!(bug_report_template.contains("security policy"));
}

#[test]
fn readme_is_concise_and_routes_to_deeper_docs() {
    let readme = std::fs::read_to_string("README.md").expect("read README");
    let line_count = readme.lines().count();

    assert!(
        line_count <= 220,
        "README should be a concise landing page, got {line_count} lines"
    );

    for required in [
        "cargo install shoreline",
        "docs/getting-started.md",
        "docs/cli-reference.md",
        "CONTRIBUTING.md",
        "docs/releasing.md",
        "docs/review-workflow.md",
    ] {
        assert!(
            readme.contains(required),
            "README missing route to {required}"
        );
    }

    assert!(!readme.contains("substrate-language"));
    assert!(!readme.contains("substrate-thesis-summary"));
    assert!(!readme.contains("internal architecture language"));
}

#[test]
fn release_docs_are_current_after_v0_1_publish() {
    let releasing = std::fs::read_to_string("docs/releasing.md").expect("read releasing docs");

    assert!(!releasing.contains("Before the first v0.1.0 publish"));
    assert!(!releasing.contains("Cargo package preflight currently passes, but warns"));
    assert!(releasing.contains("shoreline"));
    assert!(releasing.contains("Release Plan"));
    assert!(releasing.contains("Release"));
    assert!(releasing.contains("Apache-2.0"));
}

#[test]
fn adr_0010_is_landed_and_accepted() {
    let adr = std::fs::read_to_string("docs/adr/adr-0010-actor-identity-and-delegation.md")
        .expect("read ADR-0010");
    assert!(adr.contains("**Status:** Accepted"));
    // No private research/planning pointers in public docs.
    assert!(!adr.contains("implementation plan"));
    assert!(!adr.contains("research 0009"));
    // The hard prerequisite link resolves.
    assert!(
        std::path::Path::new("docs/adr/adr-0009-resumption-binding-trust-source.md").exists(),
        "ADR-0009 (composition target) must be landed"
    );
}

#[test]
fn docs_cover_actor_identity_and_delegation() {
    let storage = std::fs::read_to_string("docs/storage-model.md").expect("read storage model");
    assert!(
        storage.contains("## Legacy Writer Tool Events"),
        "storage-model documents the writer.tool hard break"
    );
    assert!(
        storage.contains(".shoreline/delegates"),
        "storage-model documents the delegates file"
    );
    assert!(
        !storage.contains("actor/tool provenance"),
        "storage-model uses producer vocabulary, not tool"
    );
    assert!(
        !storage.contains("WriterTool"),
        "no residual WriterTool in storage-model"
    );

    let agent_authoring =
        std::fs::read_to_string("docs/agent-authoring.md").expect("read agent authoring");
    assert!(
        agent_authoring.contains("actor:agent:") && agent_authoring.contains("SHORE_ACTOR_ID"),
        "agent-authoring documents the agent actor-id scheme"
    );

    let library_api = std::fs::read_to_string("docs/library-api.md").expect("read library API");
    for token in ["DelegationMap", "with_delegation_map", "PrincipalPolicy"] {
        assert!(library_api.contains(token), "library-api documents {token}");
    }

    let cli = std::fs::read_to_string("docs/cli-reference.md").expect("read CLI reference");
    assert!(
        cli.contains("SHORE_ACTOR_ID") && cli.contains(".shoreline/delegates"),
        "cli-reference documents agent identity and delegates discovery"
    );
}

fn assert_markdown_section_contains(markdown: &str, heading: &str, required: &[&str]) {
    let start = markdown
        .find(heading)
        .unwrap_or_else(|| panic!("missing section heading: {heading}"));
    let tail = &markdown[start..];
    let end = tail[heading.len()..]
        .find("\n## ")
        .map(|idx| heading.len() + idx)
        .unwrap_or(tail.len());
    let section = &tail[..end];

    for token in required {
        assert!(
            section.contains(token),
            "section {heading} missing token: {token}"
        );
    }
}
