# Hunk Feedback

Field notes from a multi-hour session using Hunk as a review surface for two agent-driven bug
fixes (issues #303 and #302 in `mmdflux`). Captured as design input for shore.

The session ran a review/implementation loop: a Claude review agent left comments via the Hunk CLI
while a Codex implementation agent made changes in the same worktree. The friction listed below
was real, recurring, and cost work.

## Severities

- **Critical** — caused lost work or required user-visible recovery.
- **High** — silently corrupted review state or forced manual workarounds.
- **Moderate** — surprising behavior with workarounds available.
- **Low** — UX nits.

## 1. Daemon death loses all live comments — no automatic persistence

**Severity: critical**

The session opened with the user noting the daemon "might have died overnight" — `hunk session
comment list` returned nothing. Prior review comments were unrecoverable without re-reading the
conversation transcript. `agent-context.json` was empty (`{}`, 0 bytes); the daemon hadn't written
to it before it died.

The schema for `agent-context.json` was discoverable only by reading
`~/src/hunk/examples/{3,6}/agent-context.json`. There is no schema export, no auto-restore on
daemon restart.

**Root cause:** comments are an in-memory view in the daemon process. The `agent-context.json`
file is written only when the daemon is healthy and chooses to.

**For shore:** persist comments to disk on every mutation. The daemon process is one consumer;
the file is the source of truth. Treat the daemon like a cache, not a database.

## 2. Daemon enters stuck state with raw runtime error

**Severity: critical, recurring**

Hit twice in the session. After mid-session edits cycled rapidly, `hunk session reload --repo .`
started returning:

    hunk: undefined is not an object (evaluating 'changeset.files')

The session **stayed listed** in `session list` (with stale `fileCount`), but:

- `reload` failed
- `review --json` returned the **stale** file list (showed reverted files still present)
- `comment apply` succeeded against stale file paths but refused new ones with
  `No diff file matches`
- The only fix was closing the TUI and relaunching `hunk diff`. After that, even the new TUI
  didn't register a session for several seconds.

**Root cause (guess):** the daemon's reload pipeline panics on some specific edit pattern (file
removed from diff while another is added?). The error message is a JS runtime error surfaced raw,
not a structured failure.

**For shore:** wrap reload in a transaction with rollback on parse error. Surface structured
errors with codes, not raw runtime exceptions. Add a `--force-clear-cache` escape hatch so users
don't have to relaunch the TUI.

## 3. Comments cleared on every successful reload

**Severity: high (data loss in a routine workflow)**

When `reload -- diff` was called to pick up a branch context switch, all live comments
**disappeared**. They survived the daemon when the diff text matched, but any reload that changed
the file set dropped them.

This is not documented as a behavior — it surprised the agent, and the comment batch had to be
re-applied. If a user had hours of review notes and reloaded to pick up a rebase, those would be
gone.

**Root cause:** comments are keyed by the daemon's internal file IDs (e.g.
`/Users/kevin/.../text-metrics:0:src/...`), and reload regenerates IDs.

**For shore:** key comments by `(repo_root, file_path, side, line_number)` — stable across
reloads. If the file no longer exists in the new diff, mark the comment as orphaned but keep it
visible. Don't silently drop.

## 4. Stale comments survived across branches with mismatched code

**Severity: moderate (silent data corruption)**

The session started with two live comments referencing `compact_backward_candidate_is_clear`,
`BACKWARD_NODE_INTRUSION_MARGIN`, `COMPACT_LANE_TOLERANCE` — code that didn't exist in the working
tree. They were from an abandoned branch's review, but the daemon presented them as live against
the current branch's diff. Until the agent noticed and reloaded, those stale comments looked
authoritative.

**Root cause:** session persistence doesn't track which branch/commit comments were authored
against. They're tied to the session, not the code state.

**For shore:** store the commit SHA (or diff hash) at comment-create time. On reload, mark
comments as "stale: code at this position has changed by N lines" or similar. Don't pretend
they're current.

## 5. `agent-context.json` ownership is ambiguous

**Severity: moderate (ambiguity creates conflict)**

The file is daemon-written (it overwrote hand-edited content the moment the daemon recovered).
But it was also empty when the session started — implying a user or external editor could
populate it. The schema lives in `~/src/hunk/examples/`, the daemon's repo, suggesting it's
daemon-private.

The agent edited the file manually when the daemon was dead, which would have been clobbered if
the daemon came back. There's no advisory lock, no last-modified check, no merge.

**For shore:** decide explicitly who owns persistence files.

- If the tool owns: write read-only from the user's perspective; refuse to start if the file was
  edited externally without a `--reconcile` flag.
- If the user owns: the tool only reads, never writes.
- Document the contract in the file header.

## 6. Reload semantics tied to TUI lifecycle

**Severity: high**

The CLI command `hunk session reload --repo . -- diff` operates on a session the **TUI** owns.
If the TUI window closes, the session ends, even though the CLI still has a daemon socket. There
is no headless session mode for CLI-only review (e.g., for an automated reviewer agent that
doesn't need a TUI to be visible).

This forced the user to keep a terminal window dedicated to Hunk for the entire multi-hour
session. If they had accidentally closed it, comments would have evaporated.

**For shore:** decouple session ownership from any specific UI. CLI, TUI, web — all consumers of
the same persistent session. A session ends only when the user explicitly ends it or a TTL
expires.

## 7. Comments-by-line don't survive line shifts

**Severity: moderate (review brittleness)**

When the implementation agent edited a file, hunk-numbered comments stayed (because hunks are
content-anchored), but line-numbered comments could end up referring to lines that now contain
different code. `(filePath, newLine)` is a fragile address when `newLine` drifts on every edit.

The workaround was re-applying full review batches each round. A long-running review where
someone is iterating on the diff would see comments drift off-target silently.

**For shore:** anchor comments to a content fingerprint of the surrounding ±3 lines, not just a
line number. On reload, if the fingerprint moved, follow the line. If the fingerprint changed,
mark the comment as stale.

## 8. No API for "what changed since last comment-apply"

**Severity: low (workflow gap)**

Each round of impl-agent edits, the review agent had to grep file diffs manually to figure out
which hunks were new. Hunk tracks this internally (it knows which hunks were present at last
comment-apply vs now) but doesn't expose it.

**For shore:** `shore session diff-since-last-comment` or similar. Useful for incremental review.

## 9. Sessions list metadata is stale

**Severity: low**

Mid-session, `session list --json` showed `fileCount: 4` for a session whose underlying tree had
6 files — the snapshot was stale. The `updatedAt` field was also stale.

**For shore:** keep metadata fresh on every mutation, or stamp it `as_of: <timestamp>` so
consumers know its age.

## 10. `hunk diff` requires manual relaunch to pick up branch changes

**Severity: low (documented behavior, but cumbersome)**

After `git checkout` and `git pull`, the TUI shows the old diff until the user manually closes
and re-runs. For a multi-issue session like this one (#303 → #302), the user had to remember to
close and relaunch between each issue.

**For shore:** auto-detect HEAD changes via filesystem watcher and offer to reload (with a
banner, not silent reload).

## 11. Tool path discovery

**Severity: trivial**

`hunk` wasn't in `$PATH` when the session started — the binary was at
`/Users/kevin/.local/share/hunk-dev/bin/hunk`. Not Hunk's fault, but worth noting that tools in
`~/.local/share` are awkward.

**For shore:** ship a `shore which` subcommand or a one-liner installer that puts a symlink in
`~/.local/bin`.

## 12. Comment ID format leaks internal detail

**Severity: trivial**

IDs like `mcp:bac82d78-d68f-44aa-a980-2a3eaac3d77e:0` have an `mcp:` prefix that's invisible to
the workflow but hints at the daemon's MCP integration. Just an aesthetic note — agent-facing IDs
should be opaque.

## 13. JSON inspection commands can hang without a TTY

**Severity: moderate (automation trap)**

In Codex's non-TTY shell runner, even basic Hunk commands using the dev binary path hung with no
output:

```bash
/Users/kevin/.local/share/hunk-dev/bin/hunk --help
/Users/kevin/.local/share/hunk-dev/bin/hunk session comment list --repo /Users/kevin/src/shore --json
```

The same commands completed once the tool call allocated a TTY. The live comment-list command then
returned Claude's reviewer notes successfully. This matters because review agents often run through
non-interactive shells, Stop hooks, background monitors, CI jobs, or scripts where allocating a TTY
is either impossible or not obvious.

**For shore:** JSON/status/introspection commands must be non-interactive by default and must not
depend on TTY allocation. If a command truly requires a terminal, detect that immediately and fail
with a clear structured error instead of blocking. Machine-readable commands such as `shore dump`,
future session/comment listing commands, and review mailbox checks should work through ordinary
stdout/stderr pipes.

## General observations

### Three independent stateholders, no atomic handoff

In this session, three things claimed authority over review state: the TUI window, the daemon
process, and `agent-context.json`. The handoff between them was not atomic. Pick one as
authoritative or define a sync protocol.

### "Session" is too small a concept

A session in Hunk means "the diff is currently loaded." When the diff changes, the session
implicitly resets. For a long-running, multi-iteration review, the session should outlive any
specific diff snapshot, and comments should attach to a higher-level concept (PR, branch, issue)
rather than a specific diff load.

In shore terms: a `Review` is the long-lived object; a `DiffSnapshot` is one moment within it;
comments attach to the `Review` and have anchors that resolve against successive snapshots.

### The agent path needs first-class integration

Most of the agent's interaction was through the CLI batch interface (`comment apply --stdin`),
not Hunk's UI. That worked, but:

- Error messages were terse.
- Batch validation only happens at apply time.
- No dry-run mode.
- When the daemon died, comments were stranded with no fallback path.

An agent-first tool would have a simpler "post review markdown to this file, atomic-write to N
comment files" interface — file-system based and inherently durable.

### TUI and CLI have different mental models

The TUI is "navigate a diff, write comments at the cursor." The CLI is "post a JSON batch tagged
by file+line." When the TUI's `selectedHunk` doesn't match a CLI navigate, they diverge silently.

For shore: both views should derive from a single review-stream model (consistent with the
guidance in `AGENTS.md` to build the review stream as a pure headless data layer first).

### Persistence is the biggest gap

The session lost work twice due to daemon issues. Any tool an agent uses for hours should treat
in-memory state as cache and disk as truth, not the other way around.

If a single design rule earns the most leverage in shore, it is this:

> Every comment, navigation event, and session mutation writes to disk before it returns to the
> caller. The disk format is human-readable and authored against by both the tool and the user.
> The daemon is a renderer of that state, not its keeper.
