// Pure diff/file/annotation renderers and classifiers. Ported from the served
// app.js diff cluster. Every function is argument-driven (no DOM, no state, no
// global reads or writes): the one shape change from app.js is that `renderDiff`
// returns `{ html, ctx }` instead of writing the `diffCtx`/cursor globals. The
// transient globals live in the diff controller, which passes them in.
//
// Imports the pure leaves only (escape, highlight, markdown, query, refs,
// types). This module owns the diff-artifact/annotation view types and the
// `DiffCtx` seam (plus the file-scope query matcher, `matchDiffFiles`).

import {
  annoContainerClass,
  annoKindClass,
  CLASS,
  dfileClass,
  diffStatusClass,
  drowClass,
} from "./classNames";
import { escapeHtml } from "./escape";
import { type EmphSpan, highlightRowText, type TokenSpan } from "./highlight";
import { renderBodyContent } from "./markdown";
import { type QueryDiagnostic, tokenizeQuery } from "./query";
import { linkify } from "./refs";
import { DEFAULT_OPEN_FILES, LARGE_FILE_ROWS } from "./types";

// ---------------------------------------------------------------------------
// Wire view types
//
// A view over the `/api/snapshots/{id}` snapshot artifact and the review-fact records the
// diff renderer reads — only the fields the renderers touch, optional where they
// tolerate absence. The controller imports these to type the values it threads
// into `renderDiff`.
// ---------------------------------------------------------------------------

/** One row of a captured hunk: a context/added/removed line with its line numbers. */
export interface DiffRow {
  kind: string;
  old_line: number | null;
  new_line: number | null;
  text: string;
  /** Syntax tokens over `text` (UTF-16 offsets); absent when the row is unhighlighted. */
  tokens?: TokenSpan[];
  /** Intraline emphasis spans over `text` (UTF-16 offsets); absent when unemphasized. */
  emphasis?: EmphSpan[];
}

/** One captured hunk: its header and rows. */
export interface DiffHunk {
  header: string;
  rows?: DiffRow[];
}

/** A non-line metadata row surfaced inside a file body (e.g. a mode note). */
export interface DiffMetadataRow {
  text: string;
}

/** A captured file in the snapshot diff (the fields the renderer reads). */
export interface DiffFile {
  status?: string;
  old_path?: string | null;
  new_path?: string | null;
  hunks?: DiffHunk[];
  metadata_rows?: DiffMetadataRow[];
  is_binary?: boolean;
  is_mode_only?: boolean;
  similarity?: number | null;
}

/** The captured snapshot: the file list the diff renders. */
export interface DiffSnapshot {
  files?: DiffFile[];
}

/** A captured object artifact carrying a snapshot diff. */
export interface DiffArtifact {
  snapshot?: DiffSnapshot;
}

/** A file/line target a review fact addresses, when it has one. */
export interface AnnotationTarget {
  kind?: string;
  filePath?: string;
  startLine?: number;
  endLine?: number;
  side?: string;
}

/** A review fact rendered against the diff (observation/input-request/assessment). */
export interface Annotation {
  id: string;
  kind: string;
  title: string;
  track: string;
  body?: string;
  bodyContentType?: string;
  tags?: string[];
  target?: AnnotationTarget;
}

/**
 * The render context the diff controller's delegated listeners read to fill a
 * lazy file body or scroll to a fact. `renderDiff` returns this instead of
 * writing it to a global. (NOT the `state.diff` route string — that stays the
 * snapshot-id the route grammar serializes.)
 */
export interface DiffCtx {
  snapshotId: string;
  files: DiffFile[];
  anchored: Annotation[];
  unanchored: Annotation[];
  filePaths: Set<string>;
}

/** The file/fact/unanchored counts the navigator summarizes. */
export interface DiffNavSummary {
  fileCount: number;
  factCount: number;
  unanchoredCount: number;
}

// The display path for a diff file (a rename shows both sides).
/** The display path for a diff file (a rename shows both sides). */
export function filePathLabel(f: DiffFile): string {
  const oldp = f.old_path;
  const newp = f.new_path;
  return oldp && newp && oldp !== newp
    ? `${oldp} → ${newp}`
    : newp || oldp || "(unknown path)";
}

/** The total captured rows across a file's hunks. */
export function fileRowCount(f: DiffFile): number {
  return (f.hunks ?? []).reduce((n, h) => n + (h.rows ? h.rows.length : 0), 0);
}

// Classify a file that carries no (or low) reviewable content, returning the
// reason string used both as the default-collapse signal and the collapsed
// one-line summary. `null` means a normal content-bearing file.
/** The low-signal reason (binary/mode-only/rename/large), or null for a normal file. */
export function classifyLowSignal(f: DiffFile): string | null {
  if (f.is_binary) return "binary";
  if (f.is_mode_only) return "mode change only";
  const hunks = f.hunks ?? [];
  const renamed =
    f.status === "renamed" ||
    (!!f.old_path && !!f.new_path && f.old_path !== f.new_path);
  if (renamed && !hunks.length) {
    return f.similarity != null ? `rename ${f.similarity}%` : "rename";
  }
  if (fileRowCount(f) > LARGE_FILE_ROWS) return "large file";
  return null;
}

// The anchored facts (range + file-level) that belong to one file. The single
// source of the per-file count the header badge and navigator both read.
/** How many anchored facts target either side of this file. */
export function fileFactCount(f: DiffFile, anchored: Annotation[]): number {
  const oldp = f.old_path;
  const newp = f.new_path;
  let n = 0;
  for (const a of anchored) {
    const p = a.target?.filePath;
    if (p === newp || p === oldp) n += 1;
  }
  return n;
}

/** The file a fact's path addresses (either side), or null. */
export function fileForFact(
  files: DiffFile[],
  filePath: string,
): DiffFile | null {
  return (
    files.find((f) => f.new_path === filePath || f.old_path === filePath) ??
    null
  );
}

/** Whether a range fact's line span overlaps any captured row of the file. */
export function rangeTouchesCapturedRows(
  a: Annotation,
  file: DiffFile | null,
): boolean {
  if (!file) return false;
  const t = a.target ?? {};
  if (t.kind !== "range" || t.startLine == null) return true;
  const start = t.startLine;
  const side = t.side === "old" ? "old" : "new";
  const end = t.endLine ?? start;
  for (const h of file.hunks ?? []) {
    for (const r of h.rows ?? []) {
      const line = side === "old" ? r.old_line : r.new_line;
      if (line != null && line >= start && line <= end) return true;
    }
  }
  return false;
}

/** One review fact rendered as an annotation card (optionally with its location). */
export function renderAnnotation(a: Annotation, showLocation: boolean): string {
  const tags = (a.tags ?? [])
    .map((t) => `<span class="${CLASS.badge}">${escapeHtml(t)}</span>`)
    .join(" ");
  const body = renderBodyContent(a.body, a.bodyContentType);
  const t = a.target ?? {};
  const loc =
    showLocation && t.filePath
      ? `<span class="${CLASS.annoLoc}">${escapeHtml(t.filePath)}${t.startLine ? `:${t.startLine}-${t.endLine || t.startLine}` : ""}</span>`
      : "";
  return `<div class="${annoContainerClass(a.kind)}" data-anno="${escapeHtml(a.id)}">
    <div class="${CLASS.annoHead}"><span class="${annoKindClass(a.kind)}">${a.kind}</span><span class="${CLASS.annoTrack}">${escapeHtml(a.track)}</span><span class="${CLASS.annoTitle}">${linkify(a.title)}</span> ${tags} ${loc}</div>${body}</div>`;
}

// The fact-vicinity summary an annotated large file mounts before full rows: its
// facts first, plus a direct affordance to hydrate the remaining rows.
/** The fact-vicinity summary for an annotated large file. */
export function renderDiffFactVicinity(
  f: DiffFile,
  anchored: Annotation[],
): string {
  const facts = anchored.filter((a) => {
    const p = a.target?.filePath;
    return p === f.new_path || p === f.old_path;
  });
  return `<div class="${CLASS.diffFactVicinity}" data-fact-vicinity="true">
    <p>Large annotated file: showing review facts first.</p>
    <button type="button" data-render-diff-file="true">Render all rows</button>
    ${facts.map((a) => renderAnnotation(a, true)).join("")}
  </div>`;
}

// The eager file header: status + path + fact-count badge. It is the disclosure
// control (carries the authoritative aria-expanded); the section keeps only
// internal render state.
/** The eager, interactive file header (status, path, low-signal summary, badge). */
export function renderDiffFileHeader(
  f: DiffFile,
  anchored: Annotation[],
  reason: string | null,
  open: boolean,
): string {
  const n = fileFactCount(f, anchored);
  const summary = reason
    ? `<span class="${CLASS.dfileSummary}">${escapeHtml(reason)}</span>`
    : "";
  return `<header class="${CLASS.dfileHead}" role="button" tabindex="0" aria-expanded="${open}">
    <span class="${diffStatusClass(escapeHtml(f.status))}">${escapeHtml(f.status)}</span>
    <span class="${CLASS.dpath}">${escapeHtml(filePathLabel(f))}</span>${summary}
    ${n ? `<span class="${CLASS.dfileNotes}">${n} note${n === 1 ? "" : "s"}</span>` : ""}</header>`;
}

// The lazy file body: file-level facts, metadata rows, and hunks/rows with their
// inline annotations. Each body owns its own `emitted` Set — a fact belongs to
// exactly one file (fileFacts filters by path), so cross-file de-dup is not
// load-bearing.
/** The file body: file-level facts, metadata rows, and hunks with inline facts. */
export function renderDiffFileBody(
  f: DiffFile,
  anchored: Annotation[],
): string {
  const oldp = f.old_path;
  const newp = f.new_path;
  const fileFacts = anchored.filter((a) => {
    const p = a.target?.filePath;
    return p === newp || p === oldp;
  });
  const rangeFacts = fileFacts.filter((a) => a.target?.kind === "range");
  const fileLevelFacts = fileFacts.filter((a) => a.target?.kind === "file");

  const emitted = new Set<string>();
  let html = "";
  for (const a of fileLevelFacts) {
    html += renderAnnotation(a, false);
    emitted.add(a.id);
  }
  for (const m of f.metadata_rows ?? []) {
    html += `<div class="${CLASS.drow} ${CLASS.drowMeta}"><span class="${CLASS.dtext}">${escapeHtml(m.text)}</span></div>`;
  }

  // Bucket range facts by the (side, line) they anchor to, once per file —
  // O(facts) instead of an O(rows × facts) re-scan inside the row loop. A fact on
  // the "old" side keys against old_line, otherwise against new_line, across its
  // inclusive [startLine, endLine] span.
  const factsByLine = new Map<string, Annotation[]>();
  for (const a of rangeFacts) {
    const t = a.target ?? {};
    if (t.startLine == null) continue;
    const start = t.startLine;
    const side = t.side === "old" ? "old" : "new";
    const end = t.endLine ?? start;
    for (let line = start; line <= end; line++) {
      const key = `${side}:${line}`;
      const bucket = factsByLine.get(key);
      if (bucket) bucket.push(a);
      else factsByLine.set(key, [a]);
    }
  }

  const hunks = f.hunks ?? [];
  for (const h of hunks) {
    html += `<div class="${CLASS.dhunk}">${escapeHtml(h.header)}</div>`;
    for (const r of h.rows ?? []) {
      // Look up this row's facts in O(1): a row matches a range fact on the
      // captured side whose line falls in [startLine, endLine].
      const matching: Annotation[] = [];
      const seen = new Set<string>();
      const collect = (key: string): void => {
        const bucket = factsByLine.get(key);
        if (!bucket) return;
        for (const a of bucket) {
          if (!seen.has(a.id)) {
            seen.add(a.id);
            matching.push(a);
          }
        }
      };
      if (r.old_line != null) collect(`old:${r.old_line}`);
      if (r.new_line != null) collect(`new:${r.new_line}`);
      const sign = r.kind === "added" ? "+" : r.kind === "removed" ? "-" : " ";
      // An annotated row is a clickable gutter marker linking to its annotation.
      const noted = matching.length > 0;
      const notedAttrs = noted
        ? ` data-anno="${escapeHtml(matching[0].id)}" tabindex="0" role="button"`
        : "";
      html += `<div class="${drowClass(escapeHtml(r.kind), noted)}"${notedAttrs}>
        <span class="${CLASS.ln}">${r.old_line ?? ""}</span>
        <span class="${CLASS.ln}">${r.new_line ?? ""}</span>
        <span class="${CLASS.sign}">${sign}</span>
        <span class="${CLASS.dtext}">${highlightRowText(r.text, r.tokens, r.emphasis)}</span></div>`;
      for (const a of matching) {
        if (!emitted.has(a.id)) {
          html += renderAnnotation(a, false);
          emitted.add(a.id);
        }
      }
    }
  }

  // Safety fallback: if a range fact was classified as anchored but no rendered
  // row emitted it, surface it inside the file instead of dropping it.
  for (const a of rangeFacts) {
    if (!emitted.has(a.id)) {
      html += renderAnnotation(a, true);
      emitted.add(a.id);
    }
  }
  if (!hunks.length && !(f.metadata_rows ?? []).length) {
    // The collapsed header already surfaces any low-signal reason; in the body
    // only note files with no classifiable reason (e.g. an empty added file), so
    // the reason text is not double-printed.
    if (!classifyLowSignal(f)) {
      html += `<div class="${CLASS.drow} ${CLASS.drowMeta}"><span class="${CLASS.dtext}">(no captured content)</span></div>`;
    }
  }
  return html;
}

// File-by-file accordion. Every header renders eagerly; a file's hunks/rows
// render lazily on first expand. Annotated files open by default, then a small
// budget of the rest, so the live DOM stays bounded on a large changeset.
// Low-signal files collapse by default — unless they carry a fact, which always
// wins so the fact is visible. Returns `{ html, ctx }`; the controller assigns
// the cursor/filter globals from `ctx`.
/** Render the whole diff overlay body, returning the html plus its render context. */
export function renderDiff(
  snapshotId: string,
  artifact: DiffArtifact,
  annotations: Annotation[],
): { html: string; ctx: DiffCtx } {
  const annos = annotations ?? [];
  const files = artifact.snapshot?.files ?? [];
  const filePaths = new Set<string>();
  for (const f of files) {
    if (f.new_path) filePaths.add(f.new_path);
    if (f.old_path) filePaths.add(f.old_path);
  }
  const anchored: Annotation[] = [];
  const unanchored: Annotation[] = [];
  for (const a of annos) {
    const t = a.target ?? {};
    if (
      (t.kind === "range" || t.kind === "file") &&
      t.filePath &&
      filePaths.has(t.filePath)
    ) {
      const file = fileForFact(files, t.filePath);
      if (t.kind === "range" && !rangeTouchesCapturedRows(a, file)) {
        unanchored.push(a);
      } else {
        anchored.push(a);
      }
    } else {
      unanchored.push(a);
    }
  }

  const ctx: DiffCtx = { snapshotId, files, anchored, unanchored, filePaths };

  const counts: Record<string, number> = {};
  for (const a of annos) {
    counts[a.kind] = (counts[a.kind] ?? 0) + 1;
  }
  const breakdown = Object.entries(counts)
    .map(([k, n]) => `${n} ${k}${n === 1 ? "" : "s"}`)
    .join(", ");
  let html = `<div class="${CLASS.annoSummary}">${annos.length} review fact${annos.length === 1 ? "" : "s"} on this revision${
    breakdown ? ` · ${breakdown}` : ""
  }${unanchored.length ? ` · ${unanchored.length} not anchored to a diff line` : ""}</div>`;
  if (unanchored.length) {
    html += `<div class="${CLASS.annoGroup}">${unanchored.map((a) => renderAnnotation(a, true)).join("")}</div>`;
  }
  if (!files.length) {
    return {
      html: `${html}<p class="${CLASS.empty}">No files captured in this snapshot.</p>`,
      ctx,
    };
  }

  let openBudget = DEFAULT_OPEN_FILES;
  html += files
    .map((f, i) => {
      const reason = classifyLowSignal(f);
      const annotated = fileFactCount(f, anchored) > 0;
      const annotatedLarge = annotated && fileRowCount(f) > LARGE_FILE_ROWS;
      const open =
        (annotated && !annotatedLarge) || (reason ? false : openBudget-- > 0);
      const expanded = annotatedLarge || open;
      const body = annotatedLarge
        ? renderDiffFactVicinity(f, anchored)
        : open
          ? renderDiffFileBody(f, anchored)
          : "";
      const lowAttr = reason ? ` data-lowsignal="${escapeHtml(reason)}"` : "";
      const bodyAttr = annotatedLarge
        ? ` data-fact-vicinity="true"`
        : open
          ? ` data-rendered="1"`
          : "";
      return `<section class="${dfileClass(!!reason)}" data-dfile="${i}" data-expanded="${expanded}"${lowAttr}>${renderDiffFileHeader(f, anchored, reason, expanded)}<div class="${CLASS.dfileBody}" data-dfile-body="${i}"${bodyAttr}>${body}</div></section>`;
    })
    .join("");
  return { html, ctx };
}

/** The navigator's file/fact/unanchored summary row. */
export function renderDiffNavSummary(summary: DiffNavSummary): string {
  return `<div class="${CLASS.diffNavSummary}" aria-label="diff summary">
    <span><b>${summary.fileCount}</b> files</span>
    <span><b>${summary.factCount}</b> facts</span>
    <span><b>${summary.unanchoredCount}</b> unanchored</span>
  </div>`;
}

// Categorize why a fact did not anchor to a captured diff line, for the
// navigator's unanchored panel.
/** The reason a fact is unanchored (broad/revision-level/missing file/outside rows). */
export function unanchoredReason(
  a: Annotation,
  filePaths: Set<string>,
): string {
  const t = a.target ?? {};
  if (a.kind === "assessment") return "broad assessment";
  if (t.kind === "revision" || !t.filePath) return "revision-level";
  if (t.kind === "range" && filePaths.has(t.filePath)) {
    return "line outside captured rows";
  }
  if (!filePaths.has(t.filePath)) return "file missing from snapshot";
  return "not anchored to a diff line";
}

/** The diff file-search grammar's supported keys — a client-only mini-grammar,
 * distinct from and never re-minting the event/revision surfaces' key sets. */
export const DIFF_FILE_QUERY_KEYS = ["path", "change", "has", "is"] as const;
export type DiffFileQueryKey = (typeof DIFF_FILE_QUERY_KEYS)[number];

// The real DiffFile.status enum (src/model/file.rs FileStatus, #[serde(rename_all =
// "snake_case")]) — NOT the classifyLowSignal reason string ("binary"/"mode change
// only"/"rename N%"/"large file"), which is a different, UI-only vocabulary. `change:`
// binds to the file's actual change status; low-signal reasons have no qualifier here.
const DIFF_FILE_CHANGE_VALUES = [
  "added",
  "deleted",
  "modified",
  "renamed",
  "copied",
] as const;
const DIFF_FILE_HAS_VALUES = ["facts"] as const;
const DIFF_FILE_IS_VALUES = ["unanchored"] as const;

interface DiffFileClause {
  field: DiffFileQueryKey;
  value: string;
}

interface ParsedDiffFileQuery {
  clauses: DiffFileClause[];
  freeText: string[];
  diagnostics: QueryDiagnostic[];
}

// Deliberately narrower than the shared parseSearchQueryFor: no negation, no
// per-surface key-set import. `status:` is special-cased because it must never
// resolve here even though it is a recognized key elsewhere in the app; every
// other unrecognized key (including keys valid on the event/revision surfaces,
// e.g. `track:`) falls through as free text rather than importing the global
// key-set union just to detect them — this surface's grammar is four keys, not
// the whole app's.
function parseDiffFileQuery(query: string): ParsedDiffFileQuery {
  const clauses: DiffFileClause[] = [];
  const freeText: string[] = [];
  const diagnostics: QueryDiagnostic[] = [];
  for (const tok of tokenizeQuery(query || "")) {
    const colon = tok.indexOf(":");
    const field = colon > 0 ? tok.slice(0, colon).toLowerCase() : "";
    const rawValue =
      colon > 0
        ? tok
            .slice(colon + 1)
            .replace(/^"|"$/g, "")
            .toLowerCase()
        : "";
    if (field === "status") {
      diagnostics.push({
        code: "unsupported-qualifier",
        key: "status",
        message:
          "status: isn't valid in the diff file search — use change: (added, deleted, modified, renamed, copied)",
      });
      continue;
    }
    if ((DIFF_FILE_QUERY_KEYS as readonly string[]).includes(field)) {
      const key = field as DiffFileQueryKey;
      if (
        key === "change" &&
        !(DIFF_FILE_CHANGE_VALUES as readonly string[]).includes(rawValue)
      ) {
        diagnostics.push({
          code: "unsupported-value",
          key: "change",
          message: `change: has no value "${rawValue}" — expected one of ${DIFF_FILE_CHANGE_VALUES.join(", ")}`,
        });
        continue;
      }
      if (
        key === "has" &&
        !(DIFF_FILE_HAS_VALUES as readonly string[]).includes(rawValue)
      ) {
        diagnostics.push({
          code: "unsupported-value",
          key: "has",
          message: `has: has no value "${rawValue}" — expected "facts"`,
        });
        continue;
      }
      if (
        key === "is" &&
        !(DIFF_FILE_IS_VALUES as readonly string[]).includes(rawValue)
      ) {
        diagnostics.push({
          code: "unsupported-value",
          key: "is",
          message: `is: has no value "${rawValue}" — expected "unanchored"`,
        });
        continue;
      }
      clauses.push({ field: key, value: rawValue });
      continue;
    }
    const term = tok.replace(/^"|"$/g, "").toLowerCase();
    if (term) freeText.push(term);
  }
  return { clauses, freeText, diagnostics };
}

/** The result of {@link matchDiffFiles}: the matching files (in `ctx.files` order) plus
 * any diagnostics for known-but-unsupported qualifiers — never silent-empty. */
export interface DiffFileMatch {
  files: DiffFile[];
  diagnostics: QueryDiagnostic[];
}

/**
 * Filter a diff's files by the file-scope query grammar: `path:` (substring on
 * the file path label), `change:` (exact on the real change-status enum), `has:facts`
 * (at least one anchored fact), `is:unanchored` (targeted by a fact classified
 * unanchored — the same anchored/unanchored partition `renderDiff` already computes),
 * and free text (path substring). `status:` is never valid here and yields a
 * pointed diagnostic; any other unrecognized key is free text. Pure — reads
 * only `ctx`, writes nothing.
 */
export function matchDiffFiles(ctx: DiffCtx, query: string): DiffFileMatch {
  const { clauses, freeText, diagnostics } = parseDiffFileQuery(query);
  const files = ctx.files.filter((f) => {
    const label = filePathLabel(f).toLowerCase();
    for (const term of freeText) {
      if (!label.includes(term)) return false;
    }
    for (const c of clauses) {
      if (c.field === "path" && !label.includes(c.value)) return false;
      if (c.field === "change" && f.status !== c.value) return false;
      if (c.field === "has" && fileFactCount(f, ctx.anchored) <= 0)
        return false;
      if (c.field === "is" && fileFactCount(f, ctx.unanchored) <= 0)
        return false;
    }
    return true;
  });
  return { files, diagnostics };
}
