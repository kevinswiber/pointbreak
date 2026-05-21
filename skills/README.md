# Shoreline Agent Skills

This directory is the distribution source for Shoreline's spec-conformant Agent Skills, installable
on any agentskills.io-conformant agent. It is not an auto-active project-skill path; install the
skills into the agent environment where you want them to run.

The recommended install path is:

```bash
npx skills add kevinswiber/shoreline
```

Install the skill ahead of the work session where it should run, not after the implementation is
already finished.

Manual fallback for agents that read the shared Agent Skills directory:

```bash
git clone https://github.com/kevinswiber/shoreline.git
cd shoreline
mkdir -p ~/.agents/skills
cp -r skills/* ~/.agents/skills/
```

Claude Code currently does not read `~/.agents/skills/`, so copy the same canonical skills there
when using Claude Code:

```bash
mkdir -p ~/.claude/skills
cp -r skills/* ~/.claude/skills/
```

An optional `claude-extras/` overlay with Claude-only conveniences such as tool pre-approval could be
added later. The canonical skills in this directory stay plain Markdown with only `name` and
`description` frontmatter.

CI validates the canonical skill with the upstream Python `skills-ref` validator. Run the same check
locally with:

```bash
uvx --from 'git+https://github.com/agentskills/agentskills#subdirectory=skills-ref' \
  skills-ref validate skills/shoreline-author
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
