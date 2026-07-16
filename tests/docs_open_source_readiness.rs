#[test]
fn cli_reference_documents_verification_and_endorsement_readback() {
    let cli = std::fs::read_to_string("docs/cli-reference.md").expect("read CLI reference");

    // The per-event verification status + endorsement readback, with the advisory +
    // reader-relative contract, are documented on the read surfaces.
    assert_markdown_section_contains(
        &cli,
        "## `pointbreak history`",
        &[
            "verificationStatus",
            "untrusted_key",
            "endorsements",
            "endorsement-trusted",
            "unknown_endorser",
            "ambiguous_endorser",
            "endorserAttributes",
            "reader-relative",
            "advisory",
        ],
    );
    assert_markdown_section_contains(
        &cli,
        "## `pointbreak revision show`",
        &["verificationStatus", "endorsements", "endorserAttributes"],
    );
}

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
        "pointbreak capture",
        "pointbreak store status",
        "pointbreak store paths",
        "pointbreak store migrate",
        "pointbreak observation add",
        "pointbreak input-request open",
        "pointbreak assessment add",
        "pointbreak history",
        "pointbreak revision show",
    ] {
        assert!(
            cli.contains(command),
            "missing command reference for {command}"
        );
    }

    assert!(cli.contains("pointbreak.review-capture"));
    assert!(cli.contains("pointbreak.review-revision"));
    assert!(cli.contains("eventSetHash"));

    let workflow =
        std::fs::read_to_string("docs/review-workflow.md").expect("read review workflow");
    assert!(
        workflow.contains("pointbreak.review-input-request-open` / `-list` / `-show` / `-respond"),
        "review workflow documents the reminted input-request show schema"
    );
    assert!(
        !workflow.contains("pointbreak.review-input-request-open` / `-list` / `-fetch`"),
        "review workflow must not describe a pointbreak.* fetch schema"
    );

    assert_markdown_section_contains(
        &cli,
        "## `pointbreak observation`",
        &[
            "--revision <revision-id>",
            "--include-body",
            "--format <fmt>",
        ],
    );
    assert_markdown_section_contains(
        &cli,
        "## `pointbreak input-request`",
        &[
            "--revision <revision-id>",
            "--track <track-id>",
            "--mode operative|advisory",
            "--file <path>",
            "--include-body",
            "--format <fmt>",
        ],
    );
    assert_markdown_section_contains(
        &cli,
        "## `pointbreak assessment`",
        &[
            "--revision <revision-id>",
            "--include-summary",
            "--format <fmt>",
        ],
    );
}

#[test]
fn public_docs_cover_the_shared_common_dir_store() {
    let cli = std::fs::read_to_string("docs/cli-reference.md").expect("read CLI reference");
    let storage = std::fs::read_to_string("docs/storage-model.md").expect("read storage model");

    assert_markdown_section_contains(
        &cli,
        "## `pointbreak store`",
        &[
            "pointbreak store status",
            "pointbreak store paths",
            "pointbreak store migrate",
            "pointbreak.store-status",
            "policyOutcome",
            "file:sha256:",
            "hard-blocking policy",
            "revision list",
            "shared common-dir store",
        ],
    );

    assert!(
        cli.contains("The per-worktree `.pointbreak/store.local.json` file is mode-only"),
        "CLI reference must match the Git-common-dir binding authority"
    );
    assert!(
        !cli.contains("legacy per-worktree `.pointbreak/store.local.json` binding"),
        "CLI reference must not claim that retired per-worktree bindings still resolve"
    );

    for token in [
        "<git-common-dir>/pointbreak",
        "pointbreak.link.json",
        "shared common-dir store",
        "pointbreak store paths",
        "ephemeral",
        "sensitivity scan",
        "inventory",
        "opaque refs",
    ] {
        assert!(
            storage.contains(token),
            "storage model missing shared common-dir store behavior: {token}"
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
        "cargo install pointbreak",
        "pointbreak capture",
        "pointbreak revision show",
        "pointbreak observation add",
        "pointbreak input-request open",
        "pointbreak assessment add",
        "<git-common-dir>/pointbreak",
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
        "cargo install pointbreak",
        "docs/installation.md",
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
    assert!(releasing.contains("pointbreak"));
    assert!(releasing.contains("Release Plan"));
    assert!(releasing.contains("Release"));
    assert!(releasing.contains("Apache-2.0"));
    assert!(releasing.contains("NOTICE"));
    assert!(releasing.contains("TRADEMARKS.md"));
}

#[test]
fn license_and_trademark_notices_are_documented() {
    let readme = std::fs::read_to_string("README.md").expect("read README");
    let notice = std::fs::read_to_string("NOTICE").expect("read NOTICE");
    let trademarks = std::fs::read_to_string("TRADEMARKS.md").expect("read trademark policy");

    assert!(readme.contains("Apache-2.0"));
    assert!(readme.contains("NOTICE"));
    assert!(readme.contains("TRADEMARKS.md"));
    assert!(notice.contains("Pointbreak"));
    assert!(notice.contains("Apache License, Version 2.0"));
    assert!(notice.contains("does not grant permission"));
    assert!(notice.contains("trademarks"));
    assert!(trademarks.contains("Pointbreak Review"));
    assert!(trademarks.contains("modified distribution"));
    assert!(trademarks.contains("logo"));
}

#[test]
fn adr_0010_is_landed_and_accepted() {
    let adr = std::fs::read_to_string("docs/adr/adr-0010-actor-identity-and-delegation.md")
        .expect("read ADR-0010");
    assert!(adr.contains("**Status:** Accepted"));
    // The local-override layer is documented (amendment for the single-.shore layout).
    assert!(
        adr.contains(".shore/delegates.local.json"),
        "ADR-0010 documents the delegates local-override layer"
    );
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
fn adr_0015_and_0016_are_landed_and_accepted() {
    let adr_0015 = std::fs::read_to_string("docs/adr/adr-0015-single-common-dir-store.md")
        .expect("ADR-0015 is landed in docs/adr/");
    let adr_0016 = std::fs::read_to_string(
        "docs/adr/adr-0016-content-targeted-artifact-removal-and-compaction.md",
    )
    .expect("ADR-0016 is landed in docs/adr/");

    for adr in [&adr_0015, &adr_0016] {
        assert!(adr.contains("**Status:** Accepted"));
        // No private plan/research labels leak into the public ADRs.
        for forbidden in [
            "0075",
            "research 0011",
            "implementation plan",
            "Facet",
            "B2",
            "SF1",
            "SF2",
            "~0075",
        ] {
            assert!(
                !adr.contains(forbidden),
                "no private label {forbidden} in the landed store-topology ADRs"
            );
        }
    }

    // The two ADRs cross-reference each other as landed in-repo neighbors.
    assert!(adr_0015.contains("./adr-0016-content-targeted-artifact-removal-and-compaction.md"));
    assert!(adr_0016.contains("./adr-0015-single-common-dir-store.md"));
}

#[test]
fn docs_cover_actor_identity_and_delegation() {
    let storage = std::fs::read_to_string("docs/storage-model.md").expect("read storage model");
    assert!(
        storage.contains("## Legacy Writer Tool Events"),
        "storage-model documents the writer.tool hard break"
    );
    assert!(
        storage.contains("## Legacy Writer Role Events"),
        "storage-model documents the writer.role hard break (both read_event anchors stay valid)"
    );
    assert!(
        storage.contains(".pointbreak/delegates.json"),
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
        agent_authoring.contains("actor:agent:") && agent_authoring.contains("POINTBREAK_ACTOR_ID"),
        "agent-authoring documents the agent actor-id scheme"
    );

    let library_api = std::fs::read_to_string("docs/library-api.md").expect("read library API");
    for token in ["DelegationMap", "with_delegation_map", "PrincipalPolicy"] {
        assert!(library_api.contains(token), "library-api documents {token}");
    }

    let cli = std::fs::read_to_string("docs/cli-reference.md").expect("read CLI reference");
    assert!(
        cli.contains("POINTBREAK_ACTOR_ID") && cli.contains(".pointbreak/delegates.json"),
        "cli-reference documents agent identity and delegates discovery"
    );
}

#[test]
fn adr_0010_amendment_records_key_custody_landing() {
    let adr = std::fs::read_to_string("docs/adr/adr-0010-actor-identity-and-delegation.md")
        .expect("read ADR-0010");

    // Status is unchanged — the amendment is a landing record, not a re-decision.
    assert!(adr.contains("**Status:** Accepted"));

    // The amendment section is present.
    assert!(
        adr.contains("## Amendment: Key Custody Landing"),
        "ADR-0010 records the key-custody landing amendment"
    );

    // As-built decisions captured (feature language, no plan numbers).
    for token in [
        ".shore/allowed-signers.json",
        "~/.shore/keys/",
        "signing never gates",
        "use-ssh",                     // ssh-agent fast-follow named
        "is_valid_principal_actor_id", // principal-validator distinction
    ] {
        assert!(adr.contains(token), "ADR-0010 amendment records {token}");
    }

    // The allowed-signers-is-not-OpenSSH note is present.
    assert!(adr.contains("not the OpenSSH") || adr.contains("not OpenSSH"));

    // No private plan labels leak into the public ADR.
    for forbidden in ["Phase 5", "Task 5.3", "plan 0066", "0066"] {
        assert!(
            !adr.contains(forbidden),
            "no private plan label {forbidden} in ADR-0010"
        );
    }
    // The existing private-pointer pins still hold.
    assert!(!adr.contains("research 0009"));
    assert!(!adr.contains("implementation plan"));
}

#[test]
fn docs_cover_key_custody_and_signing_ux() {
    let cli = std::fs::read_to_string("docs/cli-reference.md").expect("read CLI reference");
    let storage = std::fs::read_to_string("docs/storage-model.md").expect("read storage model");
    let library_api = std::fs::read_to_string("docs/library-api.md").expect("read library API");
    let agent_authoring =
        std::fs::read_to_string("docs/agent-authoring.md").expect("read agent authoring");

    // CLI: the keys family, the sign-key flag, and the new env vars.
    assert_markdown_section_contains(
        &cli,
        "## `pointbreak key`",
        &[
            "pointbreak key init",
            "pointbreak key list",
            "pointbreak key show",
            "pointbreak key enroll",
            "pointbreak key discover",
            "pointbreak.key-init",
            "pointbreak.key-discover",
            "--sign-key",
        ],
    );
    assert!(
        cli.contains("discovery does not authorize keys"),
        "cli-reference documents that discovery is advisory"
    );
    for token in [
        "POINTBREAK_SIGNING",
        "POINTBREAK_SIGNING_KEY",
        "POINTBREAK_HOME",
    ] {
        assert!(cli.contains(token), "cli-reference documents {token}");
    }

    // Storage: the allowed-signers format (NOT OpenSSH) and the user-level key home.
    for token in [
        ".pointbreak/allowed-signers.json",
        "\"allowedSigners\"",
        "not the OpenSSH",
        "OpenSSH allowed-signers files are evidence inputs",
        ".pointbreak/allowed-signers.json remains the committed trust file",
        "~/.pointbreak/keys/",
    ] {
        assert!(storage.contains(token), "storage-model documents {token}");
    }
    // Keys never live in the repo .shore/ or the store.
    assert!(storage.contains("never") && storage.contains("key home"));

    // Library API: the production signer, CLI-layer resolution, never-gates.
    for token in [
        "FileEd25519Signer",
        "discover_enrollment_candidates",
        "signing never gates",
    ] {
        assert!(library_api.contains(token), "library-api documents {token}");
    }

    // Agent-authoring: auto-keygen + enrollment.
    assert!(
        agent_authoring.contains("pointbreak key enroll"),
        "agent-authoring documents the enrollment pointer"
    );

    // Signing-UX page exists and carries the ladder.
    let signing_ux = std::fs::read_to_string("docs/signing-ux.md").expect("read signing UX");
    for rung in ["unsigned", "untrusted_key", "valid"] {
        assert!(
            signing_ux.contains(rung),
            "signing-ux documents the {rung} rung"
        );
    }
    for token in ["pointbreak key discover", "review candidate"] {
        assert!(signing_ux.contains(token), "signing-ux documents {token}");
    }

    // No private plan labels in any touched doc.
    for doc in [&cli, &storage, &library_api, &agent_authoring, &signing_ux] {
        for forbidden in ["Phase 5", "Task 5.2", "plan 0066", "0066"] {
            assert!(
                !doc.contains(forbidden),
                "no private plan label {forbidden} in docs"
            );
        }
    }
}

#[test]
fn docs_cover_ssh_agent_use_ssh_signing() {
    let cli = std::fs::read_to_string("docs/cli-reference.md").expect("read CLI reference");
    let signing_ux = std::fs::read_to_string("docs/signing-ux.md").expect("read signing UX");
    let storage = std::fs::read_to_string("docs/storage-model.md").expect("read storage model");
    let agent_authoring =
        std::fs::read_to_string("docs/agent-authoring.md").expect("read agent authoring");

    // CLI: the use-ssh subcommand + its JSON contract string live under `## `pointbreak key``.
    assert_markdown_section_contains(
        &cli,
        "## `pointbreak key`",
        &["pointbreak key use-ssh", "pointbreak.key-use-ssh"],
    );

    // Signing UX: the developer parallel, custody, the exclusions, and the agent never-gates modes.
    for token in [
        "gpg.format=ssh",
        "ssh-ed25519",
        "ed25519-sk",
        "signing_agent_unavailable",
        "signing_agent_key_absent",
        "signing_agent_sign_failed",
        "signing_key_unsupported_algorithm",
        "openssh-ssh-agent",
    ] {
        assert!(signing_ux.contains(token), "signing-ux documents {token}");
    }

    // Storage: the agent-backed (public-key-only, no-seed) keystore reference.
    assert!(
        storage.contains("agent-backed") && storage.contains("public") && storage.contains("seed"),
        "storage-model documents the agent-backed keystore reference (public key only, no seed)"
    );

    // Agent-authoring: the human use-ssh note (agents still auto-keygen).
    assert!(
        agent_authoring.contains("pointbreak key use-ssh"),
        "agent-authoring notes the human use-ssh path"
    );

    // No private plan labels in any touched doc.
    for doc in [&cli, &signing_ux, &storage, &agent_authoring] {
        for forbidden in [
            "Phase 5",
            "Task 5.1",
            "plan 0067",
            "0067",
            "plan 0066",
            "0066",
        ] {
            assert!(
                !doc.contains(forbidden),
                "no private plan label {forbidden} in docs"
            );
        }
    }
}

#[test]
fn adr_0010_second_amendment_records_ssh_agent_custody() {
    let adr = std::fs::read_to_string("docs/adr/adr-0010-actor-identity-and-delegation.md")
        .expect("read ADR-0010");

    // Status is unchanged — this is a landing record, not a re-decision.
    assert!(adr.contains("**Status:** Accepted"));

    // BOTH amendments are present (the first stays; this one is appended).
    assert!(
        adr.contains("## Amendment: Key Custody Landing"),
        "the first (key-custody) amendment stays present"
    );
    assert!(
        adr.contains("## Amendment: ssh-agent Custody Landing"),
        "ADR-0010 records the ssh-agent custody landing amendment"
    );

    // As-built decisions captured (feature language, no plan numbers). The ADR
    // body is historical — it records the surface as it was, so the needle keeps
    // the old `keys` spelling on purpose.
    for token in [
        "shore keys use-ssh",
        "ssh-agent",
        "ssh-ed25519",
        "ed25519-sk",
        "signing_agent_unavailable",
        "signing_agent_sign_failed",
        "Box<dyn EventSigner",
        "pre-flight",
        "sign_event_if_requested",
        "openssh-ssh-agent",
    ] {
        assert!(
            adr.contains(token),
            "ADR-0010 second amendment records {token}"
        );
    }

    // The resolve→sign window is recorded as closed by the sign-time degrade.
    assert!(
        adr.contains("TOCTOU") || adr.contains("resolve-to-sign") || adr.contains("resolve→sign"),
        "the resolve→sign window and its sign-time-degrade closure are documented"
    );

    // No private plan labels leak into the public ADR.
    for forbidden in [
        "Phase 5",
        "Task 5.2",
        "plan 0067",
        "0067",
        "plan 0066",
        "0066",
    ] {
        assert!(
            !adr.contains(forbidden),
            "no private plan label {forbidden} in ADR-0010"
        );
    }
    assert!(!adr.contains("research 0009"));
    assert!(!adr.contains("implementation plan"));
}

#[test]
fn adr_0026_is_landed_and_free_of_private_planning_references() {
    let adr = std::fs::read_to_string("docs/adr/adr-0026-fact-to-fact-response-relationship.md")
        .expect("ADR-0026 is landed in docs/adr/");
    assert!(adr.contains("**Status:** Accepted"));
    for forbidden in [".gumbo", "plan-create", "adr-drafts", "0097"] {
        assert!(
            !adr.contains(forbidden),
            "landed ADR-0026 leaks private token: {forbidden}"
        );
    }
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

#[test]
fn narrative_docs_carry_no_stale_review_surface_spellings() {
    // Historical records (ADR bodies, CHANGELOG.md) are exempt: they describe
    // the surface as it was. The `~/.shore/keys/` keystore path never matches
    // (`.shore/keys/` has no space), so it needs no carve-out.
    let retired_patterns = [
        "shore review capture",
        "shore review show",
        "shore review revisions",
        "shore review observation",
        "shore review assessment",
        "shore review validation",
        "shore review input-request",
        "shore review association",
        "shore review history",
        "shore review endorse",
        "shore keys ",
        "shore identity enroll",
    ];
    for path in [
        "docs/review-workflow.md",
        "docs/manual-testing.md",
        "docs/agent-authoring.md",
        "docs/storage-model.md",
        "docs/getting-started.md",
        "README.md",
        "docs/cli-reference.md",
    ] {
        let text = std::fs::read_to_string(path).expect("read doc");
        for pattern in retired_patterns {
            assert!(
                !text.contains(pattern),
                "{path} still contains a stale spelling: {pattern:?}"
            );
        }
    }
}

#[test]
fn adr_0031_is_landed_with_an_accepted_status() {
    let adr = std::fs::read_to_string("docs/adr/adr-0031-review-surface-grammar.md")
        .expect("ADR-0031 is landed in docs/adr/");
    assert!(adr.contains("**Status:** Accepted"));
    assert!(
        !adr.contains("pending in-repo landing"),
        "a landed ADR carries no pending-landing status text"
    );
    assert!(!adr.contains("DRAFT"));
}
