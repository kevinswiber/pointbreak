// Id short-forms, reference classification, and linkification.
// Ported from the served app.js refs/linkify cluster. Imports escape only.

import { CLASS, REF_ID_PREFIXES, refClass } from "./classNames";
import { escapeHtml } from "./escape";

/** Classification of a reference token: its kind and whether it navigates. */
export interface RefInfo {
  kind: string;
  clickable: boolean;
}

/** The captured base commit head shown in a target's display badge. */
export interface TargetHead {
  label?: string;
  liveBranch?: string;
  commitOidShort?: string;
}

/** Server-derived, path-private display metadata for a revision's target. */
export interface TargetDisplay {
  label?: string;
  head?: TargetHead | null;
  kind?: string;
  pathPrivate?: boolean;
}

/** The last colon segment of an id, truncated to 12 chars. */
export function shortId(id: unknown): string {
  if (!id) return "";
  const tail = String(id).split(":").pop() || "";
  return tail.length > 12 ? tail.slice(0, 12) : tail;
}

/** Git-style short form keeping the kind prefix: `rev:sha256:1ace…` -> `rev:1ace028b`. */
export function shortRef(id: unknown): string {
  const value = String(id);
  let match = value.match(
    /^([a-z][a-z-]*):(?:git:|worktree:)?sha256:([0-9a-f]{6,})$/i,
  );
  if (match) return `${match[1]}:${match[2].slice(0, 8)}`;
  match = value.match(/^sha256:([0-9a-f]{8,})$/i);
  if (match) return `sha256:${match[1].slice(0, 8)}`;
  if (/^[0-9a-f]{40}$/i.test(value)) return value.slice(0, 10);
  return value;
}

/** Path-private target label from `targetDisplay`, floored to "working tree" (escaped). */
export function targetDisplayLabel(
  td: TargetDisplay | null | undefined,
): string {
  if (!td) return "working tree";
  return escapeHtml(td.label || "working tree");
}

/** Ready-to-insert (escaped) head badge for the captured base commit, or "". */
export function targetHeadBadge(td: TargetDisplay | null | undefined): string {
  const head = td?.head;
  if (!head?.label) return "";
  let inner = `@ ${escapeHtml(head.label)}`;
  if (head.liveBranch) inner += ` · ${escapeHtml(head.liveBranch)} (current)`;
  return ` <span class="${CLASS.badge}">${inner}</span>`;
}

// Linkified content-id kinds with no `resolveRef` route: they render as a
// non-clickable chip (styled + tooltip) rather than a dead `role="link"`. Keeps
// linkification a display-only concern — no chip promises navigation it can't do.
const NON_CLICKABLE_KINDS = new Set([
  "validation",
  "obj",
  "engagement",
  "checkpoint",
  "task-attempt",
  "assoc-commit",
  "assoc-ref",
  "withdraw-commit",
  "withdraw-ref",
]);

/** Classify a token as a navigable ref, a non-navigable hash/commit, or a track. */
export function refInfo(token: string): RefInfo | null {
  const match = token.match(
    /^([a-z][a-z-]*):(?:git:|worktree:)?sha256:[0-9a-f]+$/i,
  );
  if (match) {
    const kind = match[1].toLowerCase();
    return { kind, clickable: !NON_CLICKABLE_KINDS.has(kind) };
  }
  if (/^sha256:[0-9a-f]+$/i.test(token))
    return { kind: "hash", clickable: false };
  if (/^[0-9a-f]{40}$/i.test(token))
    return { kind: "commit", clickable: false };
  if (/^(agent|human):[a-z0-9][a-z0-9_-]*$/i.test(token)) {
    return { kind: "track", clickable: true };
  }
  return null;
}

export const REF_RE = new RegExp(
  `\\b(?:${REF_ID_PREFIXES.join("|")}):(?:git:|worktree:)?sha256:[0-9a-f]{6,}\\b` +
    // A bare content hash, but NOT the `sha256:` tail of a prefixed id whose
    // prefix is not linkified (preceded by `:`) — those render whole as plain
    // text (e.g. retired `review-unit:`/`snap:` ids), not a partial hash chip.
    "|(?<!:)\\bsha256:[0-9a-f]{16,}\\b" +
    "|\\b[0-9a-f]{40}\\b" +
    "|\\b(?:agent|human):[a-z0-9][a-z0-9_-]*\\b",
  "gi",
);

/** Replace embedded ids in already-escaped text with truncated reference chips. */
export function linkifyEscaped(escaped: string): string {
  return escaped.replace(REF_RE, (token) => {
    const info = refInfo(token);
    if (!info) return token;
    const display = escapeHtml(shortRef(token));
    if (!info.clickable) {
      return `<span class="${refClass(info.kind)}" title="${escapeHtml(token)}">${display}</span>`;
    }
    return `<span class="${refClass(info.kind)}" role="link" tabindex="0" data-ref-kind="${info.kind}" data-ref-id="${escapeHtml(token)}" title="${escapeHtml(token)}">${display}</span>`;
  });
}

/** Escape then linkify free text. */
export function linkify(text: unknown): string {
  return linkifyEscaped(escapeHtml(String(text ?? "")));
}

/** Whether a body content type selects markdown rendering. */
export function isMarkdownContentType(
  contentType: string | undefined,
): boolean {
  return contentType === "text/markdown";
}

/** Allow only http(s)/mailto/fragment hrefs (escaped); reject everything else. */
export function safeMarkdownHref(href: unknown): string {
  const raw = String(href ?? "").trim();
  if (/^(https?:|mailto:)/i.test(raw) || raw.startsWith("#"))
    return escapeHtml(raw);
  return "";
}
