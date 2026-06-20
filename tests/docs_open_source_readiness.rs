#[test]
fn cli_reference_documents_verification_and_endorsement_readback() {
    let cli = std::fs::read_to_string("docs/cli-reference.md").expect("read CLI reference");

    // The per-event verification status + endorsement readback, with the advisory +
    // reader-relative contract, are documented on the read surfaces.
    assert_markdown_section_contains(
        &cli,
        "## `shore review history`",
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
        "## `shore review unit`",
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
        "shore show",
        "shore dump",
        "shore review capture",
        "shore store status",
        "shore store migrate",
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
fn public_docs_cover_the_shared_common_dir_store() {
    let cli = std::fs::read_to_string("docs/cli-reference.md").expect("read CLI reference");
    let storage = std::fs::read_to_string("docs/storage-model.md").expect("read storage model");

    assert_markdown_section_contains(
        &cli,
        "## `shore store`",
        &[
            "shore store status",
            "shore store migrate",
            "shore.store-status",
            "policyOutcome",
            "file:sha256:",
            "hard-blocking policy",
            "review unit list",
            "shared common-dir store",
        ],
    );

    for token in [
        ".git/shore",
        "shared common-dir store",
        "shore store migrate",
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
        "cargo install shoreline",
        "shore review capture",
        "shore review unit show",
        "shore review observation add",
        "shore review input-request open",
        "shore review assessment add",
        ".shore/data/",
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
        storage.contains(".shore/delegates.json"),
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
        cli.contains("SHORE_ACTOR_ID") && cli.contains(".shore/delegates.json"),
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
        "## `shore keys`",
        &[
            "shore keys init",
            "shore keys list",
            "shore keys show",
            "shore keys enroll",
            "shore.keys-init",
            "--sign-key",
        ],
    );
    for token in ["SHORE_SIGNING", "SHORE_SIGNING_KEY", "SHORE_HOME"] {
        assert!(cli.contains(token), "cli-reference documents {token}");
    }

    // Storage: the allowed-signers format (NOT OpenSSH) and the user-level key home.
    for token in [
        ".shore/allowed-signers.json",
        "\"allowedSigners\"",
        "not the OpenSSH",
        "~/.shore/keys/",
    ] {
        assert!(storage.contains(token), "storage-model documents {token}");
    }
    // Keys never live in the repo .shore/ or the store.
    assert!(storage.contains("never") && storage.contains("key home"));

    // Library API: the production signer, CLI-layer resolution, never-gates.
    for token in ["FileEd25519Signer", "signing never gates"] {
        assert!(library_api.contains(token), "library-api documents {token}");
    }

    // Agent-authoring: auto-keygen + enrollment.
    assert!(
        agent_authoring.contains("shore keys enroll"),
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

    // CLI: the use-ssh subcommand + its JSON contract string live under `## `shore keys``.
    assert_markdown_section_contains(
        &cli,
        "## `shore keys`",
        &["shore keys use-ssh", "shore.keys-use-ssh"],
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
        agent_authoring.contains("shore keys use-ssh"),
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

    // As-built decisions captured (feature language, no plan numbers).
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
