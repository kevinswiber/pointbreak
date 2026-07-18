// The ONE producer of copyable CLI command handoffs in the read-only Review UI.
// Pure command construction: every command is built from authoritative ids the
// view already loaded plus explicit, visible `<placeholder>` tokens the reader
// replaces before running. One shell-quote path guards interpolated values, one
// HTML-escaped renderer paints them, and one clipboard helper copies exactly the
// displayed text. Review shows and copies commands; it never executes them —
// nothing in this module fetches, navigates, or mutates store state.
//
// Attention handoffs are kind-specific and appear only when every authoritative
// field a command needs is present; an unsupported or incomplete item yields no
// command, never a guessed one. `stale_assessment` is deliberately unsupported:
// its resolution is re-judging the thread's heads, which the reader does from
// each head's own page, not from a templated command against the stale anchor.

import { CLASS } from "./classNames";
import { escapeHtml } from "./escape";
import type { AttentionItem } from "./store";

/** One copyable command: a label, the exact command text, and the visible
 * placeholder tokens it contains (in order of appearance). */
export interface WorkflowCommandHandoff {
  label: string;
  command: string;
  placeholders: string[];
}

// The visible placeholder vocabulary. Tokens are literal text in both the
// displayed and the copied command; they are never shell-quoted (quoting would
// present a replace-me token as a runnable value).
const TRACK = "<your-track>";
const OUTCOMES = "<approved|rejected|dismissed|superseded|abandoned>";
const STATUSES = "<passed|failed|errored|skipped>";
const CALLS =
  "<accepted|accepted-with-follow-up|needs-changes|needs-clarification>";

// Shell-safe characters that need no quoting; anything else is single-quoted
// with embedded single quotes escaped. The one quote path for every
// interpolated authoritative value. `=` is deliberately excluded: zsh expands
// a leading-= word to a command path (`=echo` -> /bin/echo), so any value
// containing it is quoted.
const SHELL_SAFE = /^[A-Za-z0-9@%+:,._/-]+$/;

function shellQuote(value: string): string {
  if (SHELL_SAFE.test(value)) return value;
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

/** The zero-revision first-open suggestion: the short path's capture. Offered
 * only for a genuinely empty unfiltered store (the caller gates that). */
export function firstReviewHandoff(): WorkflowCommandHandoff {
  return {
    label: "Capture your first revision",
    command: 'pointbreak capture --summary "<what changed>"',
    placeholders: ["<what changed>"],
  };
}

/** The five stage templates for a loaded revision, in stage order: claim,
 * evidence, question, call, and the same-revision landing association. */
export function revisionHandoffs(revisionId: string): WorkflowCommandHandoff[] {
  if (!revisionId) return [];
  const rev = shellQuote(revisionId);
  return [
    {
      label: "Add a claim (observation)",
      command: `pointbreak observation add --exact-revision ${rev} --track ${TRACK} --title "<claim title>" --body "<why it matters>"`,
      placeholders: [TRACK, "<claim title>", "<why it matters>"],
    },
    {
      label: "Record evidence (validation)",
      command: `pointbreak validation add --exact-revision ${rev} --track ${TRACK} --check-name "<check name>" --status ${STATUSES} --command "<command you ran>" --exit-code <exit-code> --summary "<what the run showed>"`,
      placeholders: [
        TRACK,
        "<check name>",
        STATUSES,
        "<command you ran>",
        "<exit-code>",
        "<what the run showed>",
      ],
    },
    {
      label: "Ask a question (input request)",
      command: `pointbreak input-request open --revision ${rev} --track ${TRACK} --title "<question>" --reason manual-decision-required --mode advisory --body "<what needs an answer>"`,
      placeholders: [TRACK, "<question>", "<what needs an answer>"],
    },
    {
      label: "Make the call (assessment)",
      command: `pointbreak assessment add --exact-revision ${rev} --track ${TRACK} --assessment ${CALLS} --summary "<why this call>"`,
      placeholders: [TRACK, CALLS, "<why this call>"],
    },
    {
      label: "Record a landed commit on this same revision",
      command: `pointbreak association record --revision ${rev} --track ${TRACK} --commit <landed-commit>`,
      placeholders: [TRACK, "<landed-commit>"],
    },
  ];
}

function respondHandoff(
  inputRequestId: string,
  label: string,
): WorkflowCommandHandoff {
  return {
    label,
    command: `pointbreak input-request respond ${shellQuote(inputRequestId)} --outcome ${OUTCOMES} --reason "<answer>"`,
    placeholders: [OUTCOMES, "<answer>"],
  };
}

/** The kind-specific commands for one attention item, or `[]` when the kind is
 * unsupported or any authoritative field a command needs is absent. */
export function attentionHandoffs(
  item: AttentionItem,
): WorkflowCommandHandoff[] {
  switch (item.kind) {
    case "open_input_request": {
      if (!item.inputRequestId) return [];
      return [respondHandoff(item.inputRequestId, "Respond to this request")];
    }
    case "ambiguous_assessment": {
      const candidates = item.assessments ?? [];
      const ids = candidates.map((record) => record.assessmentId ?? "");
      if (!item.revisionId || ids.length < 2 || ids.some((id) => !id))
        return [];
      const replaces = ids
        .map((id) => ` --replaces ${shellQuote(id)}`)
        .join("");
      return [
        {
          label: "Record one current call replacing the competing assessments",
          command: `pointbreak assessment add --exact-revision ${shellQuote(item.revisionId)} --track ${TRACK} --assessment ${CALLS} --summary "<why this call>"${replaces}`,
          placeholders: [TRACK, CALLS, "<why this call>"],
        },
      ];
    }
    case "failed_validation": {
      if (!item.revisionId || !item.trackId || !item.checkName) return [];
      return [
        {
          label: "Record the re-run of this check",
          command: `pointbreak validation add --exact-revision ${shellQuote(item.revisionId)} --track ${shellQuote(item.trackId)} --check-name ${shellQuote(item.checkName)} --status ${STATUSES} --command "<command you ran>" --exit-code <exit-code> --summary "<what the re-run showed>"`,
          placeholders: [
            STATUSES,
            "<command you ran>",
            "<exit-code>",
            "<what the re-run showed>",
          ],
        },
      ];
    }
    case "follow_up_outstanding": {
      const ids = item.openInputRequestIds ?? [];
      if (!ids.length || ids.some((id) => !id)) return [];
      return ids.map((id, index) =>
        respondHandoff(
          id,
          ids.length === 1
            ? "Respond to the open follow-up request"
            : `Respond to open follow-up request ${index + 1} of ${ids.length}`,
        ),
      );
    }
    case "competing_heads": {
      const heads = item.headRevisionIds ?? [];
      if (heads.length < 2 || heads.some((id) => !id)) return [];
      const supersedes = heads
        .map((id) => ` --supersedes ${shellQuote(id)}`)
        .join("");
      return [
        {
          label:
            "Capture a replacement only when genuinely new content replaces every head",
          command: `pointbreak capture --summary "<what changed>"${supersedes}`,
          placeholders: ["<what changed>"],
        },
      ];
    }
    default:
      return [];
  }
}

/** Escape the command for HTML and wrap each placeholder token in a visible
 * marker span. The wrapping never changes the text: the code element's
 * textContent stays byte-identical to `command`. */
function commandHtml(handoff: WorkflowCommandHandoff): string {
  let html = escapeHtml(handoff.command);
  for (const token of new Set(handoff.placeholders)) {
    const escaped = escapeHtml(token);
    html = html
      .split(escaped)
      .join(`<span class="${CLASS.workflowPlaceholder}">${escaped}</span>`);
  }
  return html;
}

/** One handoff block: label, the command as selectable code, and a copy button.
 * The displayed code's textContent is exactly the copied command. */
export function renderWorkflowHandoff(handoff: WorkflowCommandHandoff): string {
  return `<div class="${CLASS.workflowHandoff}" data-workflow-handoff>
    <span class="${CLASS.workflowHandoffLabel}">${escapeHtml(handoff.label)}</span>
    <code class="${CLASS.workflowCommand}" data-workflow-command>${commandHtml(handoff)}</code>
    <button type="button" class="${CLASS.ghost} ${CLASS.workflowCopy}" data-copy-workflow-command aria-label="copy command: ${escapeHtml(handoff.label)}">copy</button>
  </div>`;
}

/** Render a handoff list, or "" when there is nothing to offer. */
export function renderWorkflowHandoffs(
  handoffs: WorkflowCommandHandoff[],
): string {
  return handoffs.map(renderWorkflowHandoff).join("");
}

/**
 * Copy the block's displayed command text to the clipboard. Clipboard-only and
 * advisory: success/failure is reported on the button label and restored; no
 * fetch, navigation, store commit, or write endpoint is ever involved.
 */
export async function copyWorkflowCommand(button: HTMLElement): Promise<void> {
  const text = button
    .closest(`.${CLASS.workflowHandoff}`)
    ?.querySelector<HTMLElement>("[data-workflow-command]")?.textContent;
  if (!text) return;
  const previous = button.textContent ?? "copy";
  try {
    if (!navigator.clipboard?.writeText) {
      throw new Error("clipboard unavailable");
    }
    await navigator.clipboard.writeText(text);
    button.textContent = "copied";
  } catch {
    button.textContent = "copy failed";
  } finally {
    window.setTimeout(() => {
      button.textContent = previous;
    }, 1200);
  }
}
