// Entry projection: the string→string readback that turns event/entry and
// revision records into display labels, chips, and overview HTML. Ported from
// the served app.js projection cluster. All pure (argument-driven, no DOM, no
// state). Imports the pure leaves only (escape, format, refs, types).

import {
  CLASS,
  endorseClass,
  factStatusClass,
  verifyClass,
} from "./classNames";
import { escapeHtml } from "./escape";
import { fmtDateTime } from "./format";
import { shortId, type TargetDisplay, type TargetHead } from "./refs";
import {
  ASSESSMENT_LABELS,
  type CurrentAssessment,
  ENDORSEMENT_LABELS,
  type Endorsement,
  type EntryBase,
  type EntrySummary,
  type HistoryEntry,
  type LatestActivity,
  type Overview,
  type OverviewAttention,
  type OverviewCounts,
  type SearchIndex,
  typeLabel,
  VERIFICATION_LABELS,
} from "./types";

/** A revision as served by `/api/revisions` (the fields the pure layer reads). */
export interface Revision {
  revisionId?: string;
  snapshotId?: string;
  overview?: Overview;
  targetDisplay?: TargetDisplay;
  // The content hash of the captured snapshot payload, used to disambiguate a
  // rebased recapture that shares a stable snapshot id.
  snapshotContentHash?: string;
  // The captured base commit and the capture timestamp, shown in a revision card.
  base?: EntryBase;
  capturedAt?: string;
}

/** A single attention cue: its token, the query that filters to it, and a label. */
export interface AttentionToken {
  token: string;
  query: string;
  label: string;
}

// The typed, type-specific detail of an entry lives in the top-level `summary`
// object; `trackId` is also top-level. `subject` only carries the target ref.
/** The lane an entry belongs to: its explicit track, else its writer's actor id. */
export function entryTrack(e: HistoryEntry): string {
  return e.trackId || e.writer?.actorId || "";
}

// The revision a history entry addresses, read through its subject (every review
// subject variant keys on revisionId), so there is no top-level id to read.
/** The revision id an entry addresses, or "". */
export function entryRevisionId(e: HistoryEntry): string {
  return e.subject?.revisionId || "";
}

// The human label derived client-side from the structured principal object
// (ADR-0010 structured-first). Null unless the agent's principal resolved. The
// lane fallback in entryTrack deliberately never reads e.principal.
/** `<agent> (for <principal>)` for a resolved principal, else null. */
export function principalLabel(e: HistoryEntry): string | null {
  const principal = e.principal;
  if (principal?.status !== "resolved" || !principal.actorId) {
    return null;
  }
  const agent = (e.writer?.actorId || "").replace(/^actor:agent:/, "");
  const principalName = principal.actorId.replace(
    /^actor:git-(email|name):/,
    "",
  );
  return `${agent} (for ${principalName})`;
}

// Reader-relative, advisory signature readback (#171). Render-only: never gates
// a write or changes a verdict, and the same carrier may read differently for
// another reader.
/** An advisory, reader-relative signature-status chip, or "". */
export function verificationChip(status: string): string {
  if (!status) return "";
  const label = VERIFICATION_LABELS[status] || status;
  return `<span class="${verifyClass(escapeHtml(status))}" title="advisory signature readback — reader-relative, never gates a write">${escapeHtml(label)}</span>`;
}

// Strip the actor namespace for display, matching principalLabel's posture.
/** A display name for an endorser, with the git-email/git-name namespace stripped. */
export function endorserDisplay(actorId: string): string {
  return actorId.replace(/^actor:git-(email|name):/, "");
}

/** One endorsement row: its label, endorser, and advisory attributes. */
export function endorsementRow(en: Endorsement): string {
  const cls = en.classification || "";
  const label = ENDORSEMENT_LABELS[cls] || cls;
  const parts = [
    `<span class="${CLASS.endorseLabel}">${escapeHtml(label)}</span>`,
  ];
  if (en.endorser) {
    parts.push(
      `<span class="${CLASS.endorseWho}">${escapeHtml(endorserDisplay(en.endorser))}</span>`,
    );
  }
  const attrs = en.endorserAttributes || {};
  const attrBits: string[] = [];
  if (attrs.kind) attrBits.push(attrs.kind);
  const roles = attrs.roles || [];
  if (roles.length) attrBits.push(roles.join(", "));
  if (attrBits.length) {
    parts.push(
      `<span class="${CLASS.endorseAttrs}">${escapeHtml(attrBits.join(" · "))}</span>`,
    );
  }
  return `<li class="${endorseClass(escapeHtml(cls))}">${parts.join(" ")}</li>`;
}

// Advisory, reader-relative endorsement readback (#171). One row per attestation
// (one per endorsing signer/key) — never collapsed, mirroring the API.
/** The advisory endorsement readback block, or "" when there are none. */
export function endorsementsBlock(
  endorsements: Endorsement[] | null | undefined,
): string {
  const list = endorsements || [];
  if (!list.length) return "";
  const rows = list.map(endorsementRow).join("");
  return `<div class="${CLASS.endorsements}" title="advisory endorsement readback — reader-relative, never gates a write">
    <span class="${CLASS.endorsementsLabel}">endorsements</span>
    <ul class="${CLASS.endorseList}">${rows}</ul>
  </div>`;
}

/** Map an assessment value to its hyphenated display label (pass through unknowns). */
export function assessmentDisplayLabel(value: string): string {
  return ASSESSMENT_LABELS[value] || value || "";
}

/** A human title for a timeline entry, derived from its summary and type. */
export function entryTitle(e: HistoryEntry): string {
  const s: EntrySummary = e.summary || {};
  if (s.title) return s.title;
  if (s.assessment) return assessmentDisplayLabel(s.assessment);
  if (s.outcome) return s.outcome;
  if (s.reasonCode) return s.reasonCode;
  if (e.eventType === "work_object_proposed") {
    const base = s.base?.commitOid || "";
    return base ? `capture · base ${shortId(base)}` : "capture";
  }
  if (e.eventType === "validation_check_recorded") {
    const name = s.checkName || "validation";
    return s.status ? `${name} · ${s.status}` : name;
  }
  return typeLabel(e.eventType);
}

/** The display tags carried by an entry's summary, or []. */
export function entryTags(e: HistoryEntry): string[] {
  const s: EntrySummary = e.summary || {};
  return Array.isArray(s.tags) ? s.tags : [];
}

/** A `file:start-end` anchor for an entry's file target, or "". */
export function entryAnchor(e: HistoryEntry): string {
  const t = e.summary?.target;
  if (!t?.filePath) return "";
  if (t.startLine)
    return `${t.filePath}:${t.startLine}-${t.endLine || t.startLine}`;
  return t.filePath;
}

/** Space an assessment value's underscores for display, or "". */
export function assessmentLabel(value: string): string {
  if (!value) return "";
  return String(value).replaceAll("_", " ");
}

/** The current-assessment cue for a revision overview. */
export function assessmentCue(overview?: Overview | null): string {
  const currentAssessment: CurrentAssessment =
    overview?.currentAssessment || {};
  const status = currentAssessment.status || "unassessed";
  const assessment = currentAssessment.assessment || "";
  const label =
    assessment ||
    (status === "ambiguous"
      ? "ambiguous current assessment"
      : status === "resolved"
        ? "resolved"
        : "unassessed");
  const cls = assessment || status;
  return `<span class="${CLASS.overviewAssessment}"><span>current assessment</span><span class="${factStatusClass(escapeHtml(cls))}">${escapeHtml(assessmentLabel(label))}</span></span>`;
}

/** `N <singular>` / `N <plural>` by count. */
export function plural(
  n: number,
  singular: string,
  pluralLabel = `${singular}s`,
): string {
  return `${n} ${n === 1 ? singular : pluralLabel}`;
}

/** The attention cues an overview surfaces (open requests, validation context, etc.). */
export function attentionTokens(overview?: Overview | null): AttentionToken[] {
  const attention: OverviewAttention = overview?.attention || {};
  const tokens: AttentionToken[] = [];
  if (attention.openInputRequestCount) {
    tokens.push({
      token: "open-request",
      query: "attention:open-request",
      label: plural(attention.openInputRequestCount, "open request"),
    });
  }
  if (attention.unassessed) {
    tokens.push({
      token: "unassessed",
      query: "attention:unassessed",
      label: "unassessed",
    });
  }
  const validationCount =
    (attention.failedValidationCount || 0) +
    (attention.erroredValidationCount || 0);
  if (validationCount) {
    tokens.push({
      token: "validation-context",
      query: "attention:validation-context",
      label: plural(
        validationCount,
        "validation context",
        "validation contexts",
      ),
    });
  }
  if (attention.acceptedWithFollowUp) {
    tokens.push({
      token: "follow-up",
      query: "attention:follow-up",
      label: "follow-up",
    });
  }
  if (attention.staleFactCount) {
    tokens.push({
      token: "stale-fact",
      query: "attention:stale-fact",
      label: plural(attention.staleFactCount, "stale fact"),
    });
  }
  return tokens;
}

/** The attention cues as filter buttons, or a muted placeholder. */
export function attentionCues(overview?: Overview | null): string {
  const tokens = attentionTokens(overview);
  if (!tokens.length)
    return `<span class="${CLASS.overviewMuted}">no attention cues</span>`;
  return tokens
    .map(
      (cue) =>
        `<button class="${CLASS.overviewCue}" type="button" data-attention-query="${escapeHtml(cue.query)}" title="filter ${escapeHtml(cue.query)}">${escapeHtml(cue.label)}</button>`,
    )
    .join("");
}

/** The files/rows/facts stat line for a revision overview. */
export function overviewStats(overview?: Overview | null): string {
  const counts: OverviewCounts = overview?.counts || {};
  const facts =
    (counts.observations || 0) +
    (counts.inputRequests || 0) +
    (counts.assessments || 0) +
    (counts.validationChecks || 0) +
    (counts.adapterNotes || 0);
  const stat = (label: string, value?: number): string =>
    `<span class="${CLASS.overviewStat}"><b>${value ?? 0}</b> ${escapeHtml(label)}</span>`;
  return `<div class="${CLASS.overviewStats}">${stat("files", counts.files)}${stat("rows", counts.rows)}${stat("facts", facts)}</div>`;
}

/** The latest-activity line for a revision overview, or "". */
export function latestActivityLine(overview?: Overview | null): string {
  const latest = overview?.latestActivity;
  if (!latest) return "";
  const title = latest.title || latest.kind || "activity";
  return `<div class="${CLASS.overviewLatest}"><span>latest</span><b>${escapeHtml(title)}</b><span>${escapeHtml(fmtDateTime(latest.at || ""))}</span></div>`;
}

/** A once-per-load search record over a revision. */
export function revisionSearchIndex(r: Revision): SearchIndex {
  const overview: Overview = r.overview || {};
  const currentAssessment: CurrentAssessment = overview.currentAssessment || {};
  const latest: LatestActivity = overview.latestActivity || {};
  const target: TargetDisplay = r.targetDisplay || {};
  const head: TargetHead = target.head || {};
  const cues = attentionTokens(overview);
  const text = [
    r.revisionId,
    r.snapshotId,
    target.label,
    head.label,
    currentAssessment.status,
    currentAssessment.assessment,
    latest.kind,
    latest.title,
    ...cues.map((cue) => cue.label),
    "review cues",
    "attention",
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
  return {
    text,
    type: "revision",
    revision: r.revisionId,
    // The search-index key stays `object` (shared history/query grammar);
    // only the value source is snapshot-named.
    object: r.snapshotId,
    status: currentAssessment.assessment || currentAssessment.status || "",
    attention: cues.map((cue) => cue.token).join(" "),
  };
}

/** The composed review-overview card body for a revision. */
export function renderRevisionOverview(
  r: Revision,
  overview: Overview | null | undefined = r.overview,
): string {
  return `<div class="${CLASS.overviewSummary}">
    <div class="${CLASS.overviewMain}">${assessmentCue(overview)}${overviewStats(overview)}</div>
    <div class="${CLASS.overviewCues}" aria-label="review cues"><span class="${CLASS.overviewLabel}">review cues</span>${attentionCues(overview)}</div>
    ${latestActivityLine(overview)}
  </div>`;
}
