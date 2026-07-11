// Pure fact-card renderers: the observation/input-request/assessment/validation/
// the verdict badge/summary, the target label, and the
// generic fact section. Ported from the served app.js card cluster. Every
// function is an argument-driven HTML emitter (no DOM, no state). The DOM
// orchestrators that mount these (renderDetail/renderUnitPage/openUnit) and the
// state-bound staleFactSectionContext stay in a later plan.
//
// Imports the pure leaves + projection. This module owns the fact-document view
// types the cards read; a later plan's detail page imports them from here.

import {
  annoContainerClass,
  annoKindClass,
  bodyClass,
  CLASS,
  factStatusClass,
  verdictClass,
} from "./classNames";
import { escapeHtml } from "./escape";
import { fmtDateTime } from "./format";
import { renderBodyContent, renderContentHtml } from "./markdown";
import type { ThreadLayout } from "./model";
import {
  assessmentDisplayLabel,
  endorsementsBlock,
  verificationChip,
} from "./projection";
import { isMarkdownContentType, linkify } from "./refs";
import { renderSupersessionSvg } from "./supersession";
import type { Endorsement } from "./types";

// ---------------------------------------------------------------------------
// Wire view types
//
// A view over the `/api/revisions/{id}` fact documents the cards read — only the
// fields each renderer touches, optional where it tolerates absence. A later
// plan's detail page imports these to type the records it threads in.
// ---------------------------------------------------------------------------

/** A target a fact addresses (a file/line range, the revision, or another fact). */
export interface CardTarget {
  kind?: string;
  filePath?: string;
  startLine?: number;
  endLine?: number;
  side?: string;
  observationId?: string;
  inputRequestId?: string;
  assessmentId?: string;
  eventId?: string;
}

/** The current-assessment rollup the verdict badge/summary read. */
export interface VerdictAssessment {
  status?: string;
  assessment?: string;
  assessmentId?: string;
  candidates?: unknown[];
}

/** A recorded observation fact. */
export interface Observation {
  id?: string;
  trackId?: string;
  title?: string;
  status?: string;
  target?: CardTarget;
  tags?: string[];
  body?: string;
  bodyContentType?: string;
  bodyContentState?: string;
  createdAt?: string;
  verificationStatus?: string;
  endorsements?: Endorsement[];
  supersedes?: string[];
}

/** One response recorded against an input request. */
export interface InputRequestResponse {
  outcome?: string;
  reason?: string;
  reasonContentType?: string;
  reasonContentState?: string;
  verificationStatus?: string;
  endorsements?: Endorsement[];
}

/** A recorded input-request fact. */
export interface InputRequest {
  id?: string;
  trackId?: string;
  title?: string;
  status?: string;
  target?: CardTarget;
  mode?: string;
  reasonCode?: string;
  body?: string;
  bodyContentType?: string;
  bodyContentState?: string;
  createdAt?: string;
  verificationStatus?: string;
  endorsements?: Endorsement[];
  responses?: InputRequestResponse[];
}

/** A recorded assessment fact. */
export interface Assessment {
  id?: string;
  trackId?: string;
  assessment?: string;
  status?: string;
  target?: CardTarget;
  summary?: string;
  summaryContentType?: string;
  summaryContentState?: string;
  createdAt?: string;
  verificationStatus?: string;
  endorsements?: Endorsement[];
  replaces?: string[];
  relatedObservations?: string[];
  relatedInputRequests?: string[];
}

/** A recorded validation-evidence fact (advisory; never a verdict). */
export interface ValidationCheck {
  trackId?: string;
  checkName?: string;
  status?: string;
  target?: CardTarget;
  trigger?: string;
  exitCode?: number;
  summary?: string;
  summaryContentType?: string;
  summaryContentState?: string;
  completedAt?: string;
  createdAt?: string;
  verificationStatus?: string;
  endorsements?: Endorsement[];
  command?: string;
  logArtifactContentHashes?: string[];
}

/** The fields the verdict badge/summary read off the `/api/revisions/{id}` payload. */
export interface RevisionDetail {
  currentAssessment?: VerdictAssessment;
  assessments?: Assessment[];
}

/** The shared options a fact card renders from. */
export interface FactCardOptions {
  track?: string;
  title?: string;
  status?: string;
  /** Already-rendered target label HTML (from `targetLabel`). */
  target?: string;
  tags?: Array<string | null | undefined>;
  body?: string;
  bodyContentType?: string;
  /** The removed-body state (`suppressed_present` | `physically_removed`);
   *  when set, the card renders the content-removed cue instead of a body. */
  bodyContentState?: string;
  createdAt?: string;
  /** Already-rendered advisory signature chip HTML (from `verificationChip`). */
  verify?: string;
  /** Already-rendered endorsement readback HTML (from `endorsementsBlock`). */
  endorsements?: string;
  /** Already-rendered relation/response HTML appended after the body. */
  extra?: string;
}

/** The advisory current-assessment badge (a recorded judgement, never a gate). */
export function verdictBadge(ca: VerdictAssessment | null | undefined): string {
  const status = ca?.status || "unassessed";
  let value: string;
  let cls: string;
  if (status === "resolved") {
    const assessment = ca?.assessment ?? "";
    value = assessmentDisplayLabel(assessment);
    cls = verdictClass(assessment);
  } else if (status === "ambiguous") {
    value = `ambiguous (${(ca?.candidates ?? []).length} candidates)`;
    cls = verdictClass("ambiguous");
  } else {
    value = "unassessed";
    cls = verdictClass("unassessed");
  }
  return `<div class="${cls}"><span class="${CLASS.verdictStatus}">current assessment</span><span class="${CLASS.verdictValue}">${escapeHtml(value)}</span></div>`;
}

/** The summary body for the resolved (or ambiguous) current assessment, or "". */
export function currentAssessmentSummary(d: RevisionDetail): string {
  const ca = d.currentAssessment || {};
  if (ca.status === "resolved" && ca.assessmentId) {
    const a = (d.assessments || []).find((x) => x.id === ca.assessmentId);
    if (a?.summary) {
      const cls = bodyClass(
        "verdict-summary",
        isMarkdownContentType(a.summaryContentType),
      );
      return `<div class="${cls}">${renderContentHtml(a.summary, a.summaryContentType)}</div>`;
    }
    // Mirror the muted removed-body cue the Assessments card below shows, so a
    // removed current-assessment summary does not leave the rollup silently blank.
    const cue = removedBodyCue(a?.summaryContentState);
    if (cue) return cue;
  }
  if (ca.status === "ambiguous") {
    return `<div class="${CLASS.verdictSummary}">${(ca.candidates || []).length} unreplaced assessments — see Assessments below.</div>`;
  }
  return "";
}

/** A human-readable label for a fact's target (file/line, revision, or a ref). */
export function targetLabel(t: CardTarget | null | undefined): string {
  const tt = t ?? {};
  switch (tt.kind) {
    case "range":
      return `${escapeHtml(tt.filePath)}:${tt.startLine}-${tt.endLine ?? tt.startLine} (${escapeHtml(tt.side || "new")})`;
    case "file":
      return escapeHtml(tt.filePath || "");
    case "revision":
      return "whole revision";
    case "observation":
      return `→ ${linkify(tt.observationId)}`;
    case "input_request":
      return `→ ${linkify(tt.inputRequestId)}`;
    case "assessment":
      return `→ ${linkify(tt.assessmentId)}`;
    case "event":
      return `→ ${linkify(tt.eventId)}`;
    default:
      return escapeHtml(tt.kind || "");
  }
}

// The shared card shape every fact kind renders into: a kinded head (kind/track/
// title/status/target/tags/verify/time) plus the body, endorsement readback, and
// any kind-specific extra. The advisory signature/endorsement readback flows in
// pre-rendered (reader-relative, never a gate).
/** The muted content-removed cue for a removed body state, or null. */
function removedBodyCue(state: string | undefined): string | null {
  if (state !== "suppressed_present" && state !== "physically_removed") {
    return null;
  }
  const title =
    state === "suppressed_present"
      ? "removal recorded; bytes still stored until compact"
      : "removed; bytes swept from the store";
  return `<div class="${CLASS.factBodyRemoved}" title="${title}">content removed</div>`;
}

/** The shared fact-card HTML for a given kind and options. */
export function factCard(kind: string, opts: FactCardOptions): string {
  const tags = (opts.tags || [])
    .filter(Boolean)
    .map((t) => `<span class="${CLASS.badge}">${escapeHtml(t)}</span>`)
    .join(" ");
  const body =
    removedBodyCue(opts.bodyContentState) ??
    renderBodyContent(opts.body, opts.bodyContentType);
  return `<div class="${annoContainerClass(kind)}">
    <div class="${CLASS.annoHead}">
      <span class="${annoKindClass(kind)}">${kind}</span>
      <span class="${CLASS.annoTrack}">${escapeHtml(opts.track || "")}</span>
      <span class="${CLASS.annoTitle}">${linkify(opts.title || "")}</span>
      ${opts.status ? `<span class="${factStatusClass(escapeHtml(opts.status))}">${escapeHtml(opts.status)}</span>` : ""}
      ${opts.target ? `<span class="${CLASS.annoLoc}">${opts.target}</span>` : ""}
      ${tags}
      ${opts.verify || ""}
      ${opts.createdAt ? `<span class="${CLASS.annoTime}" title="${escapeHtml(opts.createdAt)}">${escapeHtml(fmtDateTime(opts.createdAt))}</span>` : ""}
    </div>
    ${body}
    ${opts.endorsements || ""}
    ${opts.extra || ""}</div>`;
}

/** An observation card, noting any superseded observations. */
export function renderObservationCard(o: Observation): string {
  const supersedes = o.supersedes ?? [];
  const extra = supersedes.length
    ? `<div class="${CLASS.factRel}">supersedes ${supersedes.map(linkify).join(", ")}</div>`
    : "";
  return factCard("observation", {
    track: o.trackId,
    title: o.title,
    status: o.status,
    target: targetLabel(o.target),
    tags: o.tags,
    body: o.body,
    bodyContentType: o.bodyContentType,
    bodyContentState: o.bodyContentState,
    createdAt: o.createdAt,
    verify: verificationChip(o.verificationStatus ?? ""),
    endorsements: endorsementsBlock(o.endorsements),
    extra,
  });
}

/** An input-request card, with its responses' advisory readback. */
export function renderInputRequestCard(ir: InputRequest): string {
  const responses = (ir.responses ?? [])
    .map(
      (r) =>
        `<div class="${CLASS.factResponse}"><span class="${CLASS.outcome}">${escapeHtml(r.outcome)}</span>${removedBodyCue(r.reasonContentState) ?? (r.reason ? renderBodyContent(r.reason, r.reasonContentType) : "")} ${verificationChip(r.verificationStatus ?? "")}${endorsementsBlock(r.endorsements)}</div>`,
    )
    .join("");
  return factCard("input-request", {
    track: ir.trackId,
    title: ir.title,
    status: ir.status,
    target: targetLabel(ir.target),
    tags: [ir.mode, ir.reasonCode],
    body: ir.body,
    bodyContentType: ir.bodyContentType,
    bodyContentState: ir.bodyContentState,
    createdAt: ir.createdAt,
    verify: verificationChip(ir.verificationStatus ?? ""),
    endorsements: endorsementsBlock(ir.endorsements),
    extra: responses
      ? `<div class="${CLASS.factResponses}">${responses}</div>`
      : "",
  });
}

/** An assessment card, noting replaced and related facts. */
export function renderAssessmentCard(a: Assessment): string {
  const rel: string[] = [];
  const replaces = a.replaces ?? [];
  const relatedObservations = a.relatedObservations ?? [];
  const relatedInputRequests = a.relatedInputRequests ?? [];
  if (replaces.length) rel.push(`replaces ${replaces.map(linkify).join(", ")}`);
  if (relatedObservations.length) {
    rel.push(`re ${relatedObservations.map(linkify).join(", ")}`);
  }
  if (relatedInputRequests.length) {
    rel.push(`re ${relatedInputRequests.map(linkify).join(", ")}`);
  }
  return factCard("assessment", {
    track: a.trackId,
    title: assessmentDisplayLabel(a.assessment ?? ""),
    status: a.status,
    target: targetLabel(a.target),
    body: a.summary,
    bodyContentType: a.summaryContentType,
    bodyContentState: a.summaryContentState,
    createdAt: a.createdAt,
    verify: verificationChip(a.verificationStatus ?? ""),
    endorsements: endorsementsBlock(a.endorsements),
    extra: rel.length
      ? `<div class="${CLASS.factRel}">${rel.join(" · ")}</div>`
      : "",
  });
}

// Validation evidence is advisory: it renders with the shared factCard shape
// (status maps to .fact-status.<status>) but never as a verdict aggregate.
/** A validation-check card (advisory evidence; never a verdict). */
export function renderValidationCheckCard(v: ValidationCheck): string {
  const rel: string[] = [];
  const logs = v.logArtifactContentHashes ?? [];
  if (v.command) rel.push(escapeHtml(v.command));
  if (logs.length) rel.push(`logs ${logs.map(linkify).join(", ")}`);
  return factCard("validation", {
    track: v.trackId,
    title: v.checkName,
    status: v.status, // passed | failed | errored | skipped → .fact-status.<status>
    target: targetLabel(v.target),
    tags: [v.trigger, v.exitCode != null ? `exit ${v.exitCode}` : null],
    body: v.summary || "",
    bodyContentType: v.summaryContentType,
    bodyContentState: v.summaryContentState,
    createdAt: v.completedAt || v.createdAt,
    verify: verificationChip(v.verificationStatus ?? ""),
    endorsements: endorsementsBlock(v.endorsements),
    extra: rel.length
      ? `<div class="${CLASS.factRel}">${rel.join(" · ")}</div>`
      : "",
  });
}

/** A counted section over a list of facts, with an optional context note. */
export function factSection<T>(
  title: string,
  items: T[] | null | undefined,
  render: (item: T) => string,
  context = "",
): string {
  const list = items ?? [];
  const body = list.length
    ? list.map(render).join("")
    : `<p class="${CLASS.upEmpty}">none</p>`;
  return `<section><h2>${escapeHtml(title)} (${list.length})</h2>${context}${body}</section>`;
}

/** One fact type's laid-out supersession graph, from `/api/revisions/{id}`. */
export interface FactSupersessionGraph {
  laidOut?: ThreadLayout;
}

/** The inspector-private fork-gated fact graphs on the composite document. */
export interface FactSupersession {
  assessments?: FactSupersessionGraph;
  observations?: FactSupersessionGraph;
}

/**
 * The fork-gated fact supersession DAG for one section, or "" when absent/empty.
 * A read-time legibility readback of the already-computed `replaces`/`supersedes`
 * shape — advisory, non-interactive, additive (the cards keep their own chips).
 */
export function renderFactSupersessionBlock(
  graph: FactSupersessionGraph | null | undefined,
  noun: "assessment" | "observation",
): string {
  const laid = graph?.laidOut;
  if (!laid || !(laid.nodes ?? []).length) return "";
  const svg = renderSupersessionSvg(laid, {
    idAttr: "data-fact-id",
    ariaNoun: noun,
    interactive: false,
    isSelected: () => false,
  });
  if (!svg) return "";
  const heads = (laid.nodes ?? []).filter((n) => n.isHead).length;
  const caption = `${noun} supersession${heads > 1 ? ` — ${heads} competing` : ""}`;
  return `<figure class="${CLASS.factDag}"><figcaption>${escapeHtml(caption)}</figcaption>${svg}</figure>`;
}
