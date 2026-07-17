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
import { normalizeTimeSlot, RANGE_ANCHOR_FIELD } from "./query";
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
  type ProjectionDiagnostic,
  REVISION_ATTENTION_VALUES,
  type SearchIndex,
  typeLabel,
  VERIFICATION_LABELS,
} from "./types";

/** A revision as served by `/api/revisions` (the fields the pure layer reads). */
export interface Revision {
  revisionId?: string;
  summary?: string;
  snapshotId?: string;
  overview?: Overview;
  targetDisplay?: TargetDisplay;
  // The content hash of the captured snapshot payload, used to disambiguate a
  // rebased recapture that shares a stable snapshot id.
  snapshotContentHash?: string;
  // The captured base commit and the capture timestamp, shown in a revision card.
  base?: EntryBase;
  capturedAt?: string;
  // The structural landing status from git reachability: `merged`, `open`,
  // `unreachable` (no live ref reaches any anchored commit), or `unknown`.
  mergeStatus?: string;
  diagnostics?: ProjectionDiagnostic[];
}

const SNAPSHOT_CONTENT_UNAVAILABLE = "snapshot_content_unavailable";

interface RevisionDiagnosticsCarrier {
  diagnostics?: ProjectionDiagnostic[];
}

/** Whether this revision's captured snapshot cannot currently be read. */
export function revisionSnapshotUnavailable(
  r: RevisionDiagnosticsCarrier,
): boolean {
  return (r.diagnostics ?? []).some(
    (diagnostic) => diagnostic.code === SNAPSHOT_CONTENT_UNAVAILABLE,
  );
}

/** The diagnostics scoped to one revision, rendered on that revision's card. */
export function revisionDiagnostics(r: RevisionDiagnosticsCarrier): string {
  const diagnostics = r.diagnostics ?? [];
  return diagnostics
    .map(
      (diagnostic) =>
        `<div class="${CLASS.revisionDiagnostic}" role="status"><b>${escapeHtml(diagnostic.code)}</b><span>${escapeHtml(diagnostic.message)}</span></div>`,
    )
    .join("");
}

/** A single attention cue: its token, the query that filters to it, and a label. */
export interface AttentionToken {
  token: string;
  query: string;
  label: string;
}

// The typed, type-specific detail of an entry lives in the top-level `summary`
// object; `trackId` is also top-level. `subject` only carries the target ref.
/** The lane an entry belongs to: its explicit track only ("" when absent) — the
 *  writer actor is a separate slot now (`entryActor`). */
export function entryTrack(e: HistoryEntry): string {
  return e.trackId || "";
}

/** The writer actor id an entry carries, the actor slot split out of `track`. */
export function entryActor(e: HistoryEntry): string {
  return e.writer?.actorId || "";
}

// The revision a history entry addresses, read through its subject (every review
// subject variant keys on revisionId), so there is no top-level id to read.
/** The revision id an entry addresses, or "". */
export function entryRevisionId(e: HistoryEntry): string {
  return e.subject?.revisionId || "";
}

// The human label derived client-side from the structured principal object
// (ADR-0010 structured-first). Null unless the agent's principal resolved.
// entryTrack/entryActor deliberately never read e.principal.
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
    if (s.summary) return s.summary;
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

// The token/query spellings come from the shared REVISION_ATTENTION_VALUES
// constant (the attention: value vocabulary), destructured in its declared
// order; the human-facing labels stay local — they are display strings, not
// query tokens.
const [
  ATTENTION_OPEN_REQUEST,
  ATTENTION_UNASSESSED,
  ATTENTION_VALIDATION_CONTEXT,
  ATTENTION_FOLLOW_UP,
  ATTENTION_STALE_FACT,
] = REVISION_ATTENTION_VALUES;

/** The attention cues an overview surfaces (open requests, validation context, etc.). */
export function attentionTokens(overview?: Overview | null): AttentionToken[] {
  const attention: OverviewAttention = overview?.attention || {};
  const tokens: AttentionToken[] = [];
  if (attention.openInputRequestCount) {
    tokens.push({
      token: ATTENTION_OPEN_REQUEST,
      query: `attention:${ATTENTION_OPEN_REQUEST}`,
      label: plural(attention.openInputRequestCount, "open request"),
    });
  }
  if (attention.unassessed) {
    tokens.push({
      token: ATTENTION_UNASSESSED,
      query: `attention:${ATTENTION_UNASSESSED}`,
      label: "unassessed",
    });
  }
  const validationCount =
    (attention.failedValidationCount || 0) +
    (attention.erroredValidationCount || 0);
  if (validationCount) {
    tokens.push({
      token: ATTENTION_VALIDATION_CONTEXT,
      query: `attention:${ATTENTION_VALIDATION_CONTEXT}`,
      label: plural(
        validationCount,
        "validation context",
        "validation contexts",
      ),
    });
  }
  if (attention.acceptedWithFollowUp) {
    tokens.push({
      token: ATTENTION_FOLLOW_UP,
      query: `attention:${ATTENTION_FOLLOW_UP}`,
      label: "follow-up",
    });
  }
  if (attention.staleFactCount) {
    tokens.push({
      token: ATTENTION_STALE_FACT,
      query: `attention:${ATTENTION_STALE_FACT}`,
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
    (counts.validationChecks || 0);
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

/** The classification facts `is:contested`/`is:superseded` need — passed by the
 *  caller (which reads state), never read here. */
export interface RevisionClassificationInput {
  state?: string;
  competing?: boolean;
}

// Lowercased at build time, mirroring the Rust record builders: the
// set-membership match kind compares the field to an already-lowercased query
// value with no compare-time lowercasing of its own, so a mixed-case source id
// would otherwise never match.
function tokenSet(values: string[]): string {
  return values.length
    ? ` ${values.map((v) => v.toLowerCase()).join(" ")} `
    : "";
}

// The tag dual index: each tag contributes BOTH its full string and its
// first-colon key, lowercased and deduplicated.
function tagTokenSet(tags: string[]): string {
  const tokens = new Set<string>();
  for (const tag of tags) {
    if (!tag) continue;
    const lowered = tag.toLowerCase();
    const colon = lowered.indexOf(":");
    if (colon > 0) tokens.add(lowered.slice(0, colon));
    tokens.add(lowered);
  }
  return tokens.size ? ` ${[...tokens].join(" ")} ` : "";
}

/** A once-per-load search record over a revision. */
export function revisionSearchIndex(
  r: Revision,
  classification: RevisionClassificationInput | null = null,
): SearchIndex {
  const overview: Overview = r.overview || {};
  const currentAssessment: CurrentAssessment = overview.currentAssessment || {};
  const attention: OverviewAttention = overview.attention || {};
  const latest: LatestActivity = overview.latestActivity || {};
  const target: TargetDisplay = r.targetDisplay || {};
  const head: TargetHead = target.head || {};
  const cues = attentionTokens(overview);
  const text = [
    r.revisionId,
    r.snapshotId,
    r.summary,
    target.label,
    target.workLabel?.text,
    head.label,
    currentAssessment.status,
    currentAssessment.assessment,
    latest.kind,
    latest.title,
    ...(r.diagnostics ?? []).flatMap((diagnostic) => [
      diagnostic.code,
      diagnostic.message,
    ]),
    ...cues.map((cue) => cue.label),
    "review cues",
    "attention",
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();

  const isTokens: string[] = [];
  if ((attention.openInputRequestCount ?? 0) > 0) isTokens.push("open");
  // Responded-only, mirroring the Rust record: an ambiguous request counts in
  // the total but is neither open nor answered, so total-minus-open would
  // over-match.
  if ((attention.respondedInputRequestCount ?? 0) > 0)
    isTokens.push("answered");
  if (attention.unassessed) isTokens.push("unassessed");
  if ((attention.staleFactCount ?? 0) > 0) isTokens.push("stale");
  if (attention.acceptedWithFollowUp) isTokens.push("follow-up");
  if (classification?.competing) isTokens.push("contested");
  if (classification?.state === "superseded") isTokens.push("superseded");

  return {
    text,
    type: "revision",
    revision: r.revisionId,
    // The search-index key is `snapshot` (grammar renamed from `object`, #334);
    // the value is the revision's snapshot/content-object id.
    snapshot: r.snapshotId,
    // The revision grammar's assessment: field. Resolved-only, mirroring the
    // Rust revision-record builder: the wire value ONLY when the current
    // assessment is resolved; unassessed and ambiguous both emit "" — an
    // ambiguous revision can carry a stale assessment value that must not
    // leak through here.
    assessment:
      currentAssessment.status === "resolved"
        ? currentAssessment.assessment || ""
        : "",
    // The attention token set in the space-wrapped membership encoding.
    attention: cues.length ? ` ${cues.map((cue) => cue.token).join(" ")} ` : "",
    track: tokenSet(overview.tracks ?? []),
    actor: tokenSet(overview.actors ?? []),
    tag: tagTokenSet(overview.tags ?? []),
    is: tokenSet(isTokens),
    // The range anchor: the revision's capturedAt, normalized to the shared
    // fixed-width form under the one canonical occurred_at key.
    [RANGE_ANCHOR_FIELD]: normalizeTimeSlot(r.capturedAt),
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
    ${revisionDiagnostics(r)}
    ${latestActivityLine(overview)}
  </div>`;
}
