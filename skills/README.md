# Pointbreak Agent Skills

This directory is the distribution source for Pointbreak's three product workflow skills:
portable, spec-conformant Agent Skills that any agentskills.io-conformant coding agent can run.
They are model-agnostic prompts around the ordinary `pointbreak` CLI — not a scheduler, a hosted
service, or a vendor-specific agent runtime.

## Install the skills

The supported route installs all three skills into the agent environment where they should run:

```bash
npx skills add withpointbreak/pointbreak \
  --skill pointbreak-author \
  --skill pointbreak-reviewer \
  --skill pointbreak-author-response
```

The repository also carries development-only skills under `.claude/skills/` for working on
Pointbreak itself; they are not part of the product path, and the pinned `--skill` flags keep the
supported install to exactly the three workflow skills.

Install ahead of the work session where a skill should trigger, not after the implementation is
already finished. Installing a skill does not run it: each skill triggers from its own description
when the matching review moment arrives.

Manual fallback for agents that read the shared Agent Skills directory:

```bash
git clone https://github.com/withpointbreak/pointbreak.git
cd pointbreak
mkdir -p ~/.agents/skills
cp -r skills/* ~/.agents/skills/
```

Claude Code currently does not read `~/.agents/skills/`, so copy the same canonical skills there
when using Claude Code:

```bash
mkdir -p ~/.claude/skills
cp -r skills/* ~/.claude/skills/
```

## The three workflow roles

The skills write into the same five stages Pointbreak Review renders, in plain language:
`Work -> Claims -> Evidence -> Questions -> Call`. Each skill owns one workflow role in the paired
review loop, on its own review track:

| Skill | Workflow role | Use it when |
| --- | --- | --- |
| `pointbreak-author` | Author handoff | A coherent implementation change is ready to hand off. Captures the revision, records claims and validation evidence for checks actually run, and opens genuine questions. Never assesses its own work. |
| `pointbreak-reviewer` | Reviewer pass | Another actor left a captured handoff. Reads the author's facts first, reviews independently on a separate reviewer track, responds to operative questions, and records exactly one current assessment. |
| `pointbreak-author-response` | Author response | Reviewer state exists on the same revision. Reads it, answers advisory questions, makes fixes only when the call is actionable, and records response context. Never assesses and never recaptures unchanged content. |

The roles describe review lanes, not headcount or a required agent vendor. The complete
human-readable walkthrough of the same loop lives in
[docs/getting-started.md](../docs/getting-started.md); the agent-side command discipline lives in
[docs/agent-authoring.md](../docs/agent-authoring.md).

Validation is evidence — never an assessment, decision, task-completion verdict, or merge gate.
Signing is automatic and advisory: a skill's first write may mint a key and report
signed-but-untrusted, and untrusted does not mean invalid. When a human chooses to trust a writer,
`pointbreak key enroll <name>` stages the key in the committed `.pointbreak/allowed-signers.json`
for human review; enrollment is optional and never required to see Review value.

## Development

The canonical skills stay plain Markdown with only `name` and `description` frontmatter. An
optional `claude-extras/` overlay with Claude-only conveniences such as tool pre-approval could be
added later.

CI validates the canonical skills with the upstream Python `skills-ref` validator. Run the same
check locally with:

```bash
just skills-validate
```

Link the canonical skills into another project's agent-specific skill directories with:

```bash
just skills-link --project /path/to/project claude agents opencode
```

To intentionally link into user-level skill directories instead, pass `--user`:

```bash
just skills-link --user claude
```

Remove those symlinks by passing the same target and agents:

```bash
just skills-unlink --project /path/to/project claude agents opencode
```

Use the `agents` target for the shared `.agents/skills` convention. Codex scans that directory, so
`codex` is accepted as an alias for `agents`; `codex-legacy` links into `.codex/skills` when you
need the older Codex-specific location.
