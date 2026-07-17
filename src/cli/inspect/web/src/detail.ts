// The detail pane: the `#detail-body` projection of the single selection (the
// pane's persistent chrome — the `.detail-head` ghost controls — lives outside
// the projection target and survives every repaint). It paints
// the event detail (identity table + readback + body + debug raw event block),
// the revision composite page (the on-demand `/api/revisions/{id}` fetch mounting
// the pure `cards` renderers), and the state-bound `staleFactSectionContext` fed
// into the pure `cards.factSection`. Ported from the served app.js detail cluster
// (`renderDetail` / `renderUnitPage` → `renderRevisionPage` / `openUnit` →
// `openRevision` / `showComposite` / `eventBodyBlock` /
// `staleFactSectionContext`), in the revision vocabulary.
//
// Two structural moves preserve the served behaviour behind the new boundaries:
//   - Detail mutates `#detail-body` and reads state / `http`; it never calls render
//     (the store subscriber repaints on commit) and never imports the ref-chip
//     resolution module. The "show in timeline" affordance is therefore emitted as
//     a `data-reveal-revision` dataset for the navigation delegate to resolve. The
//     one imperative exception is the supersession DAG's node wiring
//     (`wireDagInteractions`): hover tracing cannot delegate, so its nodes route
//     through `router.navigate` directly, as the DAG's lens host used to.
//   - The per-render diff-button listeners become one delegated `#detail` click
//     handler (installed once by the composition root): the buttons carry
//     `data-open-diff` / `data-diff-hash` / `data-diff-focus`, and the delegate
//     opens the diff through `diff/controller.openDiff`. That import is met by
//     one deliberate counter-edge: the diff page consumes this module's exported
//     `ensureRevisionComposite`/`compositeAnnotations` (the shared composite
//     payload seam), so detail and diff/controller reference each other — safe
//     because both edges are called-at-event-time, never at module init.

import {
  currentAssessmentSummary,
  type FactSupersession,
  factSection,
  type InputRequest,
  type Observation,
  type RevisionDetail,
  renderAssessmentCard,
  renderFactSupersessionBlock,
  renderInputRequestCard,
  renderObservationCard,
  renderValidationCheckCard,
  type ValidationCheck,
  verdictBadge,
} from "./cards";
import { CLASS } from "./classNames";
import { DIFF_ROUTE_CLEARED, openDiff } from "./diff/controller";
import type { Annotation } from "./diff/render";
import { $ } from "./dom";
import { escapeHtml } from "./escape";
import { fmtDateTime } from "./format";
import { fetchJSON } from "./http";
import { anchorRevision, askLabel } from "./lenses/attention";
import { renderBodyContent } from "./markdown";
import {
  eventForId,
  selectedEventId,
  snapshotContentHashForRevision,
  snapshotIdForRevision,
  supersededByRevision,
  supersedesRevision,
  supersessionBadge,
  type Thread,
} from "./model";
import {
  assessmentDisplayLabel,
  endorsementsBlock,
  entryActor,
  entryRevisionId,
  entryTitle,
  entryTrack,
  principalLabel,
  revisionDiagnostics,
  revisionSnapshotUnavailable,
  verificationChip,
} from "./projection";
import {
  actorChip,
  linkify,
  shortId,
  shortRef,
  type TargetDisplay,
  targetDisplayLabel,
  targetHeadBadge,
  workLabelText,
} from "./refs";
import { navigate } from "./router";
import type { AttentionDoc, AttentionItem } from "./store";
import { getState } from "./store";
import { renderSupersessionSvg } from "./supersession";
import {
  type EntryBase,
  type EntrySource,
  type EntryTarget,
  type HistoryEntry,
  type ProjectionDiagnostic,
  typeLabel,
} from "./types";

// ---------------------------------------------------------------------------
// The /api/revisions/{id} composite-page document (the fields the page reads)
// ---------------------------------------------------------------------------

/** The revision record on the `/api/revisions/{id}` document (keyed on `id`, not `revisionId`). */
interface RevisionPageRevision {
  id?: string;
  summary?: string;
  base?: EntryBase;
  // Shared `pointbreak.review-revision` vocabulary: the member doc keeps `objectId`
  // and `objectArtifactContentHash` on the wire, unlike the snapshot-named
  // `/api/revisions` list entries.
  objectId?: string;
  objectArtifactContentHash?: string;
  targetDisplay?: TargetDisplay;
}

/** The per-revision counts the composite-page summary stats read. */
interface RevisionPageSummary {
  fileCount?: number;
  rowCount?: number;
  observationCount?: number;
  inputRequestCount?: number;
  assessmentCount?: number;
  validationCheckCount?: number;
}

interface CommitCondition {
  // `unreachable` (object present, no live ref reaches it) and `missing`
  // (object gone) are distinct first-class conditions — never `orphaned`.
  condition?: "merged" | "live" | "unreachable" | "missing";
}

interface CommitLiveness extends CommitCondition {
  commitOid?: string;
  liveBranch?: string;
  // For an unreachable commit: `reflog` while a reflog entry retains the
  // object, `none` after expiry; absent when the probe cannot answer.
  retention?: "reflog" | "none";
}

/** One recorded ref association's continuity against the ref's live state. */
interface RefContinuityEntry {
  refName?: string;
  recordedHeadOid?: string;
  currentTipOid?: string;
  // `current`, `advanced`, `rewritten`, `moved`, `deleted`, or `unknown`.
  continuity?: string;
  rewriteAction?: string;
  sameTree?: boolean;
}

interface CommitRangeLiveness {
  perCommit?: CommitLiveness[];
  headline?: CommitCondition;
  refContinuity?: RefContinuityEntry[];
}

interface CurrentCommitAssociation {
  commitOid?: string;
  source?: "capture_target" | "association";
  commitAssociationId?: string;
}

interface CurrentRefAssociation {
  refName?: string;
  headOid?: string;
  refAssociationId?: string;
}

interface WithdrawnCommitAssociation extends CurrentCommitAssociation {
  commitWithdrawalId?: string;
}

interface WithdrawnRefAssociation extends CurrentRefAssociation {
  refWithdrawalId?: string;
}

interface RevisionCommitRange {
  anchored?: boolean;
  currentCommits?: CurrentCommitAssociation[];
  currentRefs?: CurrentRefAssociation[];
  withdrawnCommits?: WithdrawnCommitAssociation[];
  withdrawnRefs?: WithdrawnRefAssociation[];
  liveness?: CommitRangeLiveness;
}

/** The `/api/revisions/{id}` composite document the revision page projects. */
export interface RevisionPageDoc extends RevisionDetail {
  revision?: RevisionPageRevision;
  summary?: RevisionPageSummary;
  observations?: Observation[];
  inputRequests?: InputRequest[];
  validationChecks?: ValidationCheck[];
  diagnostics?: ProjectionDiagnostic[];
  commitRange?: RevisionCommitRange;
  factSupersession?: FactSupersession;
  // Fork-gated: present only when the revision's supersession component has more
  // than one member (same `Thread` wire shape `/api/threads` round-trips).
  revisionSupersession?: Thread;
}

function commitConditionLabel(condition: CommitCondition | undefined): string {
  return condition?.condition ?? "unknown";
}

/** The per-commit condition plus its reflog-retention qualifier. */
function commitLivenessLabel(item: CommitLiveness | undefined): string {
  let label = commitConditionLabel(item);
  if (item?.retention === "reflog") label += " (reflog-retained)";
  else if (item?.retention === "none") label += " (reflog expired)";
  return label;
}

/** The continuity readout for one recorded ref association, or "". */
function refContinuityLabel(entry: RefContinuityEntry | undefined): string {
  if (!entry?.continuity) return "";
  let label = entry.continuity;
  if (entry.continuity === "rewritten" && entry.rewriteAction)
    label += ` by ${entry.rewriteAction}`;
  if (entry.sameTree === true) label += " (same tree)";
  if (entry.currentTipOid && entry.currentTipOid !== entry.recordedHeadOid)
    label += ` → ${shortId(entry.currentTipOid)}`;
  return label;
}

function shortGitRef(reference: string | undefined): string {
  return (reference || "")
    .replace(/^refs\/heads\//, "")
    .replace(/^refs\/remotes\//, "");
}

function associationRows(label: string, rows: string[]): string {
  const content = rows.length
    ? `<ul>${rows.join("")}</ul>`
    : `<p class="${CLASS.upEmpty}">none</p>`;
  return `<div><h3>${escapeHtml(label)}</h3>${content}</div>`;
}

/** Bounded readback of event-borne associations plus repository-borne liveness. */
export function renderAssociationAndLanding(
  commitRange: RevisionCommitRange | null | undefined,
  diagnostics: ProjectionDiagnostic[] | null | undefined,
): string {
  const range = commitRange ?? {};
  const currentCommits = range.currentCommits ?? [];
  const currentRefs = range.currentRefs ?? [];
  const withdrawnCommits = range.withdrawnCommits ?? [];
  const withdrawnRefs = range.withdrawnRefs ?? [];
  const livenessByCommit = new Map(
    (range.liveness?.perCommit ?? []).map((item) => [item.commitOid, item]),
  );
  const hasLanding = currentCommits.some(
    (commit) => commit.source === "association",
  );
  const divergent = (diagnostics ?? []).some(
    (diagnostic) => diagnostic.code === "divergent_commit_association",
  );

  let headline: string;
  let captureQualifier = "";
  if (!currentCommits.length) {
    headline = "floating revision — no landing commit association recorded";
  } else if (hasLanding) {
    if (divergent) {
      headline = "landing ambiguous";
    } else if (range.liveness) {
      headline = `landing ${commitConditionLabel(range.liveness.headline)}`;
    } else {
      headline = "landing unknown — Git reachability unavailable";
    }
  } else {
    headline = range.liveness
      ? `anchored capture target ${commitConditionLabel(range.liveness.headline)}`
      : "anchored capture target unknown — Git reachability unavailable";
    captureQualifier = `<p class="${CLASS.advisoryNote}">no landing commit association recorded</p>`;
  }

  const commitRow = (commit: CurrentCommitAssociation): string => {
    const liveness = livenessByCommit.get(commit.commitOid);
    const association = commit.commitAssociationId
      ? ` ${linkify(commit.commitAssociationId)}`
      : "";
    return `<li>${linkify(commit.commitOid)} <span class="${CLASS.factStatus}">${escapeHtml(commit.source || "unknown")}</span>${association} <span>${escapeHtml(commitLivenessLabel(liveness))}</span></li>`;
  };
  // Continuity entries key on the recorded (refName, headOid) pair, so a ref
  // re-associated at a new head reads each recorded state independently.
  const continuityByRef = new Map(
    (range.liveness?.refContinuity ?? []).map((entry) => [
      `${entry.refName} ${entry.recordedHeadOid}`,
      entry,
    ]),
  );
  const refRow = (reference: CurrentRefAssociation): string => {
    const continuity = refContinuityLabel(
      continuityByRef.get(`${reference.refName} ${reference.headOid}`),
    );
    const continuityCell = continuity
      ? ` <span class="${CLASS.factStatus}">${escapeHtml(continuity)}</span>`
      : "";
    return `<li>${escapeHtml(shortGitRef(reference.refName))} @ ${linkify(reference.headOid)}${continuityCell} ${linkify(reference.refAssociationId)}</li>`;
  };
  const withdrawnCommitRow = (commit: WithdrawnCommitAssociation): string =>
    `<li>${linkify(commit.commitOid)} ${linkify(commit.commitAssociationId)} ${linkify(commit.commitWithdrawalId)}</li>`;
  const withdrawnRefRow = (reference: WithdrawnRefAssociation): string =>
    `<li>${escapeHtml(shortGitRef(reference.refName))} @ ${linkify(reference.headOid)} ${linkify(reference.refAssociationId)} ${linkify(reference.refWithdrawalId)}</li>`;

  return `<section><h2>Association and landing</h2>
    <dl class="${CLASS.upIdentity}"><dt>anchored</dt><dd>${range.anchored ? "yes" : "no"}</dd><dt>state</dt><dd>${escapeHtml(headline)}</dd></dl>
    ${captureQualifier}
    ${associationRows("current commits", currentCommits.map(commitRow))}
    ${associationRows("current refs", currentRefs.map(refRow))}
    ${associationRows("withdrawn commits", withdrawnCommits.map(withdrawnCommitRow))}
    ${associationRows("withdrawn refs", withdrawnRefs.map(withdrawnRefRow))}
  </section>`;
}

// The revision whose composite is currently shown, so a re-render with an
// unchanged revision selection does not re-fetch. Transient view-cache — never on
// the store.
let shownCompositeId: string | null = null;

// The composite documents fetched this session, keyed by revision id and stamped
// with the history event-set hash they were fetched under: a freshness reload
// that moves the hash invalidates the entry, so revisiting a revision after the
// record moved re-fetches instead of serving a pinned document. Failures are
// never cached. Transient view-cache — never on the store.
interface CompositeCacheEntry {
  doc: RevisionPageDoc;
  eventSetHash: string | undefined;
}
const compositeCache = new Map<string, CompositeCacheEntry>();
const compositeInFlight = new Map<string, Promise<RevisionPageDoc | null>>();

/**
 * The one composite fetch path (the detail page and the diff page both consume
 * it): fetch `/api/revisions/{id}` — the entity-primary read that resolves
 * grouped-away and cold ids exactly — deduplicating in-flight calls and caching
 * per revision id. Never throws: a failed read resolves null and the caller
 * degrades (the cache stays empty, so a retry can succeed).
 */
export function ensureRevisionComposite(
  revisionId: string,
): Promise<RevisionPageDoc | null> {
  const eventSetHash = getState().history?.eventSetHash;
  const cached = compositeCache.get(revisionId);
  if (cached && cached.eventSetHash === eventSetHash)
    return Promise.resolve(cached.doc);
  const pending = compositeInFlight.get(revisionId);
  if (pending) return pending;
  const read = fetchJSON(`/api/revisions/${encodeURIComponent(revisionId)}`)
    .then((d) => {
      const doc = d as RevisionPageDoc;
      compositeCache.set(revisionId, { doc, eventSetHash });
      return doc;
    })
    .catch(() => null)
    .finally(() => {
      compositeInFlight.delete(revisionId);
    });
  compositeInFlight.set(revisionId, read);
  return read;
}

/**
 * The composite document's review facts as diff annotations — the cold-path
 * twin of `model.annotationsForRevision`, which reads the loaded history window
 * and therefore misses cold and grouped-away revisions. The diff page derives
 * its annotations here so a deep link paints annotated with nothing loaded.
 */
export function compositeAnnotations(doc: RevisionPageDoc): Annotation[] {
  const out: Annotation[] = [];
  for (const o of doc.observations ?? []) {
    out.push({
      kind: "observation",
      id: o.id ?? "",
      title: o.title ?? "(observation)",
      body: o.body ?? "",
      bodyContentType: o.bodyContentType,
      track: o.trackId ?? "",
      tags: Array.isArray(o.tags) ? o.tags : [],
      target: o.target ?? {},
    });
  }
  for (const r of doc.inputRequests ?? []) {
    const meta = [r.mode, r.reasonCode].filter(Boolean).join(" · ");
    out.push({
      kind: "input-request",
      id: r.id ?? "",
      title: r.title ?? "(input request)",
      body: r.body ?? "",
      bodyContentType: r.bodyContentType,
      track: r.trackId ?? "",
      tags: meta ? [meta] : [],
      target: r.target ?? {},
    });
  }
  for (const a of doc.assessments ?? []) {
    const label = assessmentDisplayLabel(a.assessment ?? "");
    out.push({
      kind: "assessment",
      id: a.id ?? "",
      title: `assessment: ${label || "?"}`,
      body: a.summary ?? "",
      bodyContentType: a.summaryContentType,
      track: a.trackId ?? "",
      tags: [],
      target: a.target ?? {},
    });
  }
  return out;
}

// Reading-position memory for the detail pane, keyed by the painted entity
// (event or revision id). Session-only transient view-cache, like
// `shownCompositeId`: a NEW entity starts at the top (the offset is applied in
// the same synchronous task as the content swap — one paint, no flash), a
// REVISITED entity restores the reader's place, and a same-entity repaint
// (freshness poll, filter toggle) never touches the scroll at all. Capped so a
// long session cannot grow it unbounded.
const SCROLL_MEMORY_CAP = 50;
const scrollMemory = new Map<string, number>();
let shownDetailKey: string | null = null;

/** Save the outgoing entity's offset — call BEFORE the content swap (clamping). */
function rememberScroll(): void {
  const pane = $("#detail");
  if (!pane || shownDetailKey === null) return;
  scrollMemory.set(shownDetailKey, pane.scrollTop);
  if (scrollMemory.size > SCROLL_MEMORY_CAP) {
    const oldest = scrollMemory.keys().next().value;
    if (oldest !== undefined) scrollMemory.delete(oldest);
  }
}

/** Apply the incoming entity's offset — call AFTER the content swap. */
function projectScroll(newKey: string | null): void {
  const pane = $("#detail");
  if (!pane) {
    shownDetailKey = newKey;
    return;
  }
  if (shownDetailKey === newKey) return; // same entity: leave the reader alone
  pane.scrollTop = (newKey ? scrollMemory.get(newKey) : undefined) ?? 0;
  shownDetailKey = newKey;
}

// ---------------------------------------------------------------------------
// Event detail
// ---------------------------------------------------------------------------

/**
 * A plain entity anchor to an entity-primary route. Deliberately NOT a
 * `data-ref-kind` chip: the navigation delegate preventDefaults chips, and a
 * real anchor must keep native behavior — one hashchange navigation on click,
 * and cmd/middle-click opening a new tab. Enter on a focused anchor stays
 * native via onKey's interactive-target yield.
 */
function entityAnchor(
  kind: "event" | "revision",
  id: string,
  label?: string,
): string {
  // Match the ref-chip display contract: the short form as text, the full id
  // in the title tooltip (and in the href).
  return `<a href="#/${kind}/${encodeURIComponent(id)}" title="${escapeHtml(id)}">${escapeHtml(label ?? shortRef(id))}</a>`;
}

/** The first non-empty body fallback for an event: body, then summary, then reason. */
export function eventBodyBlock(e: HistoryEntry): string {
  const s = e.summary ?? {};
  if (s.body) return renderBodyContent(s.body, s.bodyContentType);
  if (s.summary) return renderBodyContent(s.summary, s.summaryContentType);
  if (s.reason) return renderBodyContent(s.reason, s.reasonContentType);
  return "";
}

type DetailRow = [string, string];

function addRow(rows: DetailRow[], label: string, value: unknown): void {
  if (value === undefined || value === null || value === "") return;
  rows.push([label, String(value)]);
}

function addListRow(
  rows: DetailRow[],
  label: string,
  values: string[] | undefined,
): void {
  if (!Array.isArray(values) || values.length === 0) return;
  rows.push([label, values.join(", ")]);
}

function addContentRows(
  rows: DetailRow[],
  label: "body" | "summary" | "reason",
  byteSize?: number,
  hash?: string,
  state?: string,
): void {
  addRow(rows, `${label}Bytes`, byteSize);
  addRow(rows, `${label}Hash`, hash);
  addRow(rows, `${label}State`, state);
}

function endpointSummary(endpoint: EntryTarget | undefined): string {
  if (!endpoint) return "";
  switch (endpoint.kind) {
    case "git_commit":
      return [
        "git_commit",
        endpoint.commitOid,
        endpoint.treeOid ? `tree ${endpoint.treeOid}` : "",
      ]
        .filter(Boolean)
        .join(" · ");
    case "git_tree":
      return ["git_tree", endpoint.treeOid].filter(Boolean).join(" · ");
    case "git_index":
      return ["git_index", endpoint.treeOid].filter(Boolean).join(" · ");
    case "git_working_tree":
      // The raw absolute worktree path stays in the collapsed debug JSON.
      return "git_working_tree";
    default:
      return endpoint.kind ?? "";
  }
}

function sourceSummary(source: EntrySource | undefined): string {
  if (!source) return "";
  const parts = [source.kind, source.mode];
  if (source.includeUntracked !== undefined) {
    parts.push(source.includeUntracked ? "includes untracked" : "tracked only");
  }
  if (source.pathspecs?.length) {
    parts.push(`pathspecs ${source.pathspecs.join(", ")}`);
  }
  return parts.filter(Boolean).join(" · ");
}

function targetSummary(target: EntryTarget | undefined): string {
  if (!target) return "";
  const kind = target.kind || "target";
  const line =
    target.filePath && target.startLine
      ? `${target.filePath}:${target.startLine}-${target.endLine || target.startLine}`
      : target.filePath;
  switch (kind) {
    case "revision":
      return ["revision", target.revisionId].filter(Boolean).join(" · ");
    case "file":
      return ["file", target.revisionId, line].filter(Boolean).join(" · ");
    case "range":
      return ["range", target.revisionId, line, target.side]
        .filter(Boolean)
        .join(" · ");
    case "observation":
      return ["observation", target.observationId, target.revisionId]
        .filter(Boolean)
        .join(" · ");
    case "input_request":
      return ["input request", target.inputRequestId, target.revisionId]
        .filter(Boolean)
        .join(" · ");
    case "assessment":
      return ["assessment", target.assessmentId, target.revisionId]
        .filter(Boolean)
        .join(" · ");
    case "event":
      return ["event", target.eventId, target.revisionId]
        .filter(Boolean)
        .join(" · ");
    default:
      return [kind, target.revisionId, line].filter(Boolean).join(" · ");
  }
}

function pushEventTypeRows(e: HistoryEntry, rows: DetailRow[]): void {
  const s = e.summary ?? {};
  switch (e.eventType) {
    case "review_initialized":
      addRow(rows, "summary", "review initialized");
      break;
    case "work_object_proposed":
      addRow(rows, "snapshot", s.objectId);
      addRow(rows, "engagement", s.engagementId);
      addRow(rows, "artifactHash", s.objectArtifactContentHash);
      addRow(rows, "source", sourceSummary(s.source));
      addRow(rows, "base", endpointSummary(s.base));
      addRow(rows, "targetEndpoint", endpointSummary(s.target));
      break;
    case "review_observation_recorded":
      addRow(rows, "observationId", s.observationId);
      addRow(rows, "target", targetSummary(s.target));
      addRow(rows, "confidence", s.confidence);
      addListRow(rows, "tags", s.tags);
      addListRow(rows, "supersedes", s.supersedes);
      addListRow(rows, "respondsTo", s.respondsTo);
      addContentRows(
        rows,
        "body",
        s.bodyByteSize,
        s.bodyContentHash,
        s.bodyContentState,
      );
      break;
    case "review_assessment_recorded":
      addRow(rows, "assessmentId", s.assessmentId);
      addRow(rows, "assessment", s.assessment);
      addRow(rows, "target", targetSummary(s.target));
      addListRow(rows, "replaces", s.replaces);
      addListRow(rows, "relatedObservations", s.relatedObservations);
      addListRow(rows, "relatedInputRequests", s.relatedInputRequests);
      addContentRows(
        rows,
        "summary",
        s.summaryByteSize,
        s.summaryContentHash,
        s.summaryContentState,
      );
      break;
    case "input_request_opened":
      addRow(rows, "inputRequestId", s.inputRequestId);
      addRow(rows, "mode", s.mode);
      addRow(rows, "reasonCode", s.reasonCode);
      addRow(rows, "target", targetSummary(s.target));
      addContentRows(
        rows,
        "body",
        s.bodyByteSize,
        s.bodyContentHash,
        s.bodyContentState,
      );
      break;
    case "input_request_responded":
      addRow(rows, "inputRequestResponseId", s.inputRequestResponseId);
      addRow(rows, "inputRequestId", s.inputRequestId);
      addRow(rows, "outcome", s.outcome);
      addContentRows(
        rows,
        "reason",
        s.reasonByteSize,
        s.reasonContentHash,
        s.reasonContentState,
      );
      break;
    case "review_note_imported":
      addRow(rows, "summary", "retired note import");
      break;
    case "validation_check_recorded":
      addRow(rows, "validationCheckId", s.validationCheckId);
      addRow(rows, "target", targetSummary(s.target));
      addRow(rows, "check", s.checkName);
      addRow(rows, "status", s.status);
      addRow(rows, "trigger", s.trigger);
      addRow(rows, "exitCode", s.exitCode);
      addRow(rows, "command", s.command);
      addRow(rows, "sourceFingerprint", s.sourceFingerprint);
      addRow(rows, "startedAt", s.startedAt);
      addRow(rows, "completedAt", s.completedAt);
      addListRow(rows, "logArtifacts", s.logArtifactContentHashes);
      addContentRows(
        rows,
        "summary",
        undefined,
        s.summaryContentHash,
        s.summaryContentState,
      );
      break;
    case "revision_ref_associated":
      addRow(rows, "refAssociationId", s.refAssociationId);
      addRow(rows, "refName", s.refName);
      addRow(rows, "headOid", s.headOid);
      break;
    case "revision_ref_withdrawn":
      addRow(rows, "refWithdrawalId", s.refWithdrawalId);
      addRow(rows, "refAssociationId", s.refAssociationId);
      break;
    case "revision_commit_associated":
      addRow(rows, "commitAssociationId", s.commitAssociationId);
      addRow(rows, "commitOid", s.commitOid);
      addRow(rows, "treeOid", s.treeOid);
      break;
    case "revision_commit_withdrawn":
      addRow(rows, "commitWithdrawalId", s.commitWithdrawalId);
      addRow(rows, "commitAssociationId", s.commitAssociationId);
      break;
    default:
      addRow(rows, "summaryKind", s.kind);
      break;
  }
}

function rawEventBlock(e: HistoryEntry): string {
  const raw = escapeHtml(JSON.stringify(e, null, 2));
  return `<details class="${CLASS.rawEvent}">
    <summary>Raw event</summary>
    <div class="${CLASS.rawEventActions}"><button class="${CLASS.ghost}" type="button" data-copy-raw-event>copy</button></div>
    <pre data-raw-event>${raw}</pre>
  </details>`;
}

/** Paint `#detail-body` from the selected event, or the empty prompt when none is selected. */
export function renderDetail(): void {
  // Showing the event/empty pane means no composite is shown — so a later
  // re-selection of a revision re-fetches its composite.
  shownCompositeId = null;
  const el = $("#detail-body");
  if (!el) return;
  rememberScroll();
  const selected = selectedEventId();
  const e = selected ? eventForId(selected) : undefined;
  if (!e) {
    el.innerHTML = `<p class="${CLASS.empty}">Select an event or revision to inspect.</p>`;
    projectScroll(null);
    return;
  }
  const revisionId = entryRevisionId(e);
  const kv: [string, string][] = [
    ["type", `${typeLabel(e.eventType)} (${e.eventType})`],
    ["occurredAt", fmtDateTime(e.occurredAt ?? "")],
    ["eventId", e.eventId ?? ""],
    ["payloadHash", e.payloadHash ?? ""],
    ["revision", revisionId || "—"],
    ["track", entryTrack(e) || "—"],
    ["actor", entryActor(e) || "—"],
  ];
  const principal = principalLabel(e);
  if (principal) kv.push(["principal", principal]);
  const snapshotId = revisionId ? snapshotIdForRevision(revisionId) : "";
  const s = e.summary ?? {};
  if (e.eventType === "work_object_proposed") {
    const predecessors = supersedesRevision(revisionId);
    if (predecessors.length) kv.push(["supersedes", predecessors.join(", ")]);
  }
  pushEventTypeRows(e, kv);
  let focusId: string | null = null;
  let focusNoun = "";
  if (e.eventType === "review_observation_recorded") {
    focusId = s.observationId ?? null;
    focusNoun = "observation";
  } else if (e.eventType === "review_assessment_recorded") {
    focusId = s.assessmentId ?? null;
    focusNoun = "assessment";
  } else if (e.eventType === "input_request_opened") {
    focusId = s.inputRequestId ?? null;
    focusNoun = "input request";
  }
  const bodyBlock = eventBodyBlock(e);
  const btnLabel = focusId
    ? `show this ${focusNoun} in the diff`
    : "view snapshot diff";
  const verifyChip = verificationChip(e.verificationStatus ?? "");
  const endorse = endorsementsBlock(e.endorsements);
  // Persistent, visible reader-scope cue at the head of the readback region (the
  // quietest tier), so the reader-relative framing is never tooltip-only.
  const readback =
    verifyChip || endorse
      ? `<div class="${CLASS.readback}"><p class="${CLASS.readerScopeNote}">reader-relative — computed against your enrolled keys</p>${verifyChip ? `<div class="${CLASS.readbackRow}">${verifyChip}</div>` : ""}${endorse}</div>`
      : "";
  // The diff affordance carries its open-diff datasets; the once-installed
  // #detail delegate opens the overlay through diff/controller (no per-render
  // listener). data-diff-hash pairs the snapshot with its captured artifact hash.
  const diffButton = snapshotId
    ? `<button class="${CLASS.ghost} ${CLASS.diffBtn}" id="detail-diff-btn" data-open-diff="${escapeHtml(snapshotId)}" data-diff-hash="${escapeHtml(snapshotContentHashForRevision(revisionId))}" data-diff-focus="${escapeHtml(focusId ?? "")}">${escapeHtml(btnLabel)}</button>`
    : "";
  // The title and the eventId/revision rows are real anchors to their
  // entity-primary routes (native open-in-new-tab); every other value keeps the
  // linkify chip treatment. The title is ONE anchor with escaped text — nesting
  // chips inside an anchor is an a11y fault, and its embedded ids repeat in the
  // kv rows below.
  const kvValue = (k: string, v: string): string => {
    if (k === "eventId" && e.eventId) return entityAnchor("event", e.eventId);
    if (k === "revision" && revisionId)
      return entityAnchor("revision", revisionId);
    if (k === "actor" && v !== "—") return actorChip(v);
    return linkify(v);
  };
  el.innerHTML = `
    <h2>${e.eventId ? entityAnchor("event", e.eventId, entryTitle(e)) : linkify(entryTitle(e))}</h2>
    <dl class="${CLASS.kv}">${kv.map(([k, v]) => `<dt>${escapeHtml(k)}</dt><dd>${kvValue(k, v)}</dd>`).join("")}</dl>
    ${readback}
    ${diffButton}
    ${bodyBlock}
    ${rawEventBlock(e)}`;
  projectScroll(e.eventId ?? null);
}

// ---------------------------------------------------------------------------
// Revision composite page
// ---------------------------------------------------------------------------

// The revision-level supersession block (fork-gated: the server omits the field
// for a singleton component and degrades to omission on read/layout errors, so
// an absent or unlaid block simply means non-forked — no error UI). The block
// renders from component data identically under every member's page: heads are
// an unranked, id-ordered peer set (the server's wire order, never re-sorted)
// with no primary chrome; the reader's own head carries only a quiet marker.
function renderRevisionSupersessionBlock(
  thread: Thread | undefined,
  selfId: string,
): string {
  const laid = thread?.laidOut;
  if (!thread || !laid || !(laid.nodes ?? []).length) return "";
  const svg = renderSupersessionSvg(laid, {
    idAttr: "data-revision-id",
    ariaNoun: "revision",
    interactive: true,
    isSelected: (id) => id === selfId,
  });
  if (!svg) return "";
  const heads = thread.heads ?? [];
  // A fork surfaces every competing head as a navigable chip — never a null head.
  const chips = thread.competing
    ? `<div class="${CLASS.revisionHeads}"><span class="${CLASS.factStatus} ${CLASS.competing}">competing revisions (${heads.length})</span> ${heads
        .map(
          (h) =>
            linkify(h) +
            (h === selfId
              ? `<span class="${CLASS.revisionSelf}">you are here</span>`
              : ""),
        )
        .join(" ")}</div>`
    : "";
  const caption = `revision supersession${thread.competing ? ` — ${heads.length} competing` : ""}`;
  return `<figure class="${CLASS.revisionSupersession}"><figcaption>${escapeHtml(caption)}</figcaption>${chips}${svg}</figure>`;
}

// Wire the supersession DAG nodes into the IA: click / Enter / Space navigate to
// the revision via the router; hover/focus traces the node and its incident
// edges by class toggle (no re-render). The tracing cannot delegate, so this
// stays imperative, re-run per composite paint; it scopes to the revision block
// (the fact DAGs are non-interactive).
function wireDagInteractions(scope: HTMLElement): void {
  const nav = (node: Element): void => {
    const id = node.getAttribute("data-revision-id");
    if (id)
      navigate({
        selected: { kind: "revision", id },
        ...DIFF_ROUTE_CLEARED,
      });
  };
  for (const node of Array.from(
    scope.querySelectorAll<SVGGElement>(".dag-node"),
  )) {
    node.addEventListener("click", () => nav(node));
    node.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        nav(node);
      }
    });
    const trace = (on: boolean): void => {
      const id = node.getAttribute("data-revision-id");
      node.classList.toggle("traced", on);
      for (const edge of Array.from(
        scope.querySelectorAll<SVGPolylineElement>(
          `.dag-edge[data-from="${id}"], .dag-edge[data-to="${id}"]`,
        ),
      )) {
        edge.classList.toggle("traced", on);
        // Swap the arrowhead to the accent marker via the marker-end attribute
        // (cross-browser; not CSS context paint) so it tracks the edge highlight.
        edge.setAttribute(
          "marker-end",
          on ? "url(#dag-arrow-traced)" : "url(#dag-arrow)",
        );
      }
    };
    node.addEventListener("mouseenter", () => trace(true));
    node.addEventListener("mouseleave", () => trace(false));
    node.addEventListener("focus", () => trace(true));
    node.addEventListener("blur", () => trace(false));
  }
}

// ---------------------------------------------------------------------------
// The per-revision outstanding block (the scoped /api/attention read)
// ---------------------------------------------------------------------------

// The scoped attention set the open revision's page renders, cached under the
// revision id AND the global attention doc's eventSetHash it was fetched under:
// when the freshness poll moves the global doc, the composite revision-id dedupe
// must not pin this block, so a repaint under a changed hash re-fetches. `items`
// is null when the scoped read failed — the block degrades to omission and the
// page stays functional. Transient view-cache — never on the store.
interface ScopedAttention {
  revisionId: string;
  eventSetHash: string | undefined;
  items: AttentionItem[] | null;
}
let scopedAttention: ScopedAttention | null = null;
// The key of the newest scoped read still in flight, so repaints during the
// read (every store commit repaints) do not stack duplicate fetches. A
// monotonic generation marks each read; only the newest may commit its
// response or clear this marker, so a superseded read settling out of order
// can neither overwrite a fresher cache nor leave the marker pinned.
let scopedAttentionPending: Omit<ScopedAttention, "items"> | null = null;
let scopedAttentionGeneration = 0;

/** Whether the cached (or in-flight) scoped set matches the revision and the
 * CURRENT global attention hash — false means a re-fetch is due. */
function scopedAttentionFresh(revisionId: string): boolean {
  const eventSetHash = getState().attention?.eventSetHash;
  const hit = (s: { revisionId: string; eventSetHash?: string } | null) =>
    s?.revisionId === revisionId && s.eventSetHash === eventSetHash;
  return hit(scopedAttention) || hit(scopedAttentionPending);
}

/** Fetch the revision-scoped attention set and cache it. Never throws: a failed
 * read caches null items (omission), so it can ride `Promise.all` with the
 * composite fetch without turning the page into an error paint. A read that a
 * newer one superseded drops its response instead of committing it. */
async function fetchScopedAttention(revisionId: string): Promise<void> {
  const eventSetHash = getState().attention?.eventSetHash;
  const generation = ++scopedAttentionGeneration;
  scopedAttentionPending = { revisionId, eventSetHash };
  let items: AttentionItem[] | null;
  try {
    const doc = (await fetchJSON(
      `/api/attention?revision=${encodeURIComponent(revisionId)}`,
    )) as AttentionDoc;
    items = doc.items ?? [];
  } catch {
    items = null;
  }
  if (generation !== scopedAttentionGeneration) return;
  scopedAttentionPending = null;
  scopedAttention = { revisionId, eventSetHash, items };
}

// The outstanding block: the scoped attention item SET, one row per item (kind
// chip + the lens's ask wording + a navigation chip to the anchor) — never
// collapsed to one chip or a stage scalar. Read-only: navigation is the only
// interaction; there is no dismissal affordance of any kind. Empty (or failed)
// scoped reads render nothing.
function renderOutstandingBlock(revisionId: string): string {
  const items =
    scopedAttention?.revisionId === revisionId ? scopedAttention.items : null;
  if (!items?.length) return "";
  const rows = items
    .map((item) => {
      const anchor = anchorRevision(item);
      const kind = escapeHtml(item.kind.replace(/_/g, "-"));
      return `<li><span class="${CLASS.attentionKind}">${kind}</span> ${escapeHtml(askLabel(item))}${anchor ? ` ${linkify(anchor)}` : ""}</li>`;
    })
    .join("");
  return `<section class="${CLASS.outstandingSet}"><h2>Outstanding (${items.length})</h2><ul>${rows}</ul></section>`;
}

/** Re-fetch the scoped set when the global attention doc moved under the open
 * revision, repainting only the block's host (not the composite). */
async function refreshOutstandingIfStale(revisionId: string): Promise<void> {
  if (scopedAttentionFresh(revisionId)) return;
  await fetchScopedAttention(revisionId);
  // The shown revision may have changed under the await; its paint owns the host.
  if (revisionId !== shownCompositeId) return;
  const host = $("#detail-body")?.querySelector<HTMLElement>(
    "[data-outstanding-host]",
  );
  if (host) host.innerHTML = renderOutstandingBlock(revisionId);
}

/** The "superseded by <successors>" context repeated near each fact section, or "". */
export function staleFactSectionContext(revisionId: string): string {
  const successors = supersededByRevision(revisionId);
  if (!successors.length) return "";
  return `<p class="${CLASS.factStaleContext}">superseded by ${successors.map(linkify).join(" ")}</p>`;
}

/** Paint `#detail` with a revision's composite page from the `/api/revisions/{id}` document. */
export function renderRevisionPage(d: RevisionPageDoc): void {
  const ru = d.revision ?? {};
  const base = ru.base ?? {};
  const s = d.summary ?? {};
  const revisionId = ru.id ?? "";
  const badge = supersessionBadge(revisionId);
  const title =
    ru.summary ||
    ru.targetDisplay?.workLabel?.text ||
    `${shortId(ru.id)}${base.commitOid ? ` · base ${shortId(base.commitOid)}` : ""}`;
  const staleContext = staleFactSectionContext(revisionId);
  // Prepend the fork-gated fact DAG (server-gated, absent otherwise) above the
  // stale context in the Observations / Assessments sections; both are "" when the
  // fact type does not fork, so the sections are unchanged for the common case.
  const observationContext =
    renderFactSupersessionBlock(
      d.factSupersession?.observations,
      "observation",
    ) + staleContext;
  const assessmentContext =
    renderFactSupersessionBlock(d.factSupersession?.assessments, "assessment") +
    staleContext;
  const snapshotUnavailable = revisionSnapshotUnavailable(d);

  const stat = (label: string, n?: number): string =>
    `<span class="${CLASS.upStat}"><b>${n ?? 0}</b> ${label}</span>`;
  const sections: string[] = [];

  sections.push(`<section><h2>Revision</h2><dl class="${CLASS.upIdentity}">
    <dt>id</dt><dd>${linkify(ru.id)}</dd>
    <dt>summary</dt><dd>${escapeHtml(ru.summary ?? "—")}</dd>
    <dt>work</dt><dd>${workLabelText(ru.targetDisplay)}</dd>
    <dt>base</dt><dd>${base.commitOid ? linkify(base.commitOid) : "—"} ${base.kind ? `<span class="${CLASS.factStatus}">${escapeHtml(base.kind)}</span>` : ""}</dd>
    <dt>target</dt><dd>${targetDisplayLabel(ru.targetDisplay)}${targetHeadBadge(ru.targetDisplay)}</dd>
    <dt>worktree</dt><dd>${escapeHtml(ru.targetDisplay?.label ?? "working tree")}</dd>
    <dt>head</dt><dd>${escapeHtml(ru.targetDisplay?.head?.label ?? "—")}</dd>
    <dt>supersession</dt><dd>${badge || "—"}</dd>
    <dt>snapshot</dt><dd>${linkify(ru.objectId)}</dd>
  </dl>${revisionDiagnostics(d)}${renderRevisionSupersessionBlock(d.revisionSupersession, revisionId)}</section>`);

  sections.push(renderAssociationAndLanding(d.commitRange, d.diagnostics));

  sections.push(
    `<section><h2>Current assessment</h2>${verdictBadge(d.currentAssessment)}${currentAssessmentSummary(d)}<p class="${CLASS.advisoryNote}">advisory — a recorded judgement, not a merge gate</p></section>`,
  );

  // The per-revision outstanding set sits by the judgment context. The host div
  // is stable across scoped re-fetches, so a moved global attention doc repaints
  // only the block, never the composite.
  sections.push(
    `<div data-outstanding-host>${renderOutstandingBlock(revisionId)}</div>`,
  );

  // The annotated-diff affordance carries its open-diff datasets (the #detail
  // delegate opens it); the "show in timeline" affordance carries a
  // data-reveal-revision dataset the navigation delegate resolves.
  sections.push(`<section><h2>Summary</h2><div class="${CLASS.upStats}">
    ${stat("files", s.fileCount)}${stat("rows", s.rowCount)}${stat("observations", s.observationCount)}${stat("input requests", s.inputRequestCount)}${stat("assessments", s.assessmentCount)}${stat("validation checks", s.validationCheckCount)}
  </div>
  <div style="margin-top:10px">
    ${
      snapshotUnavailable
        ? `<button class="${CLASS.ghost} ${CLASS.diffBtn}" id="up-diff-btn" type="button" disabled title="captured snapshot content is unavailable">snapshot unavailable</button>`
        : `<button class="${CLASS.ghost} ${CLASS.diffBtn}" id="up-diff-btn" type="button" data-open-diff="${escapeHtml(ru.objectId ?? "")}" data-diff-hash="${escapeHtml(ru.objectArtifactContentHash ?? "")}">view annotated diff</button>`
    }
    <button class="${CLASS.ghost}" id="up-timeline-btn" data-reveal-revision="${escapeHtml(revisionId)}" style="margin-left:6px">show in timeline</button>
  </div></section>`);

  sections.push(
    factSection(
      "Observations",
      d.observations,
      renderObservationCard,
      observationContext,
    ),
  );
  sections.push(
    factSection(
      "Input requests",
      d.inputRequests,
      renderInputRequestCard,
      staleContext,
    ),
  );
  sections.push(
    factSection(
      "Assessments",
      d.assessments,
      renderAssessmentCard,
      assessmentContext,
    ),
  );

  // Validation checks: a first-class section after Assessments, rendered from the
  // document array (not raw events). Advisory-only — a context-only caption,
  // structurally separate from Current assessment, never a verdict aggregate.
  const validationChecks = d.validationChecks ?? [];
  const validationBody = validationChecks.length
    ? `${validationChecks.map(renderValidationCheckCard).join("")}<p class="${CLASS.validationNote}">context only — does not affect the current assessment</p>`
    : `<p class="${CLASS.upEmpty}">none</p>`;
  sections.push(
    `<section><h2>Validation checks (${validationChecks.length})</h2>${staleContext}${validationBody}</section>`,
  );

  const el = $("#detail-body");
  if (el) {
    el.innerHTML = `<div class="${CLASS.unitPage}"><p class="${CLASS.unitPageTitle}">${escapeHtml(title)}</p>${sections.join("")}</div>`;
    const block = el.querySelector<HTMLElement>(
      `.${CLASS.revisionSupersession}`,
    );
    if (block) wireDagInteractions(block);
  }
  projectScroll(revisionId || null);
}

/** Fetch a revision's composite document and paint it, guarding a superseding selection. */
export async function openRevision(revisionId: string): Promise<void> {
  const el = $("#detail-body");
  rememberScroll();
  if (el) el.innerHTML = `<p class="${CLASS.upEmpty}">loading…</p>`;
  // The scoped attention set rides the same paint; neither read throws
  // (fetchScopedAttention degrades to omission, ensureRevisionComposite
  // resolves null), so a failed composite reaches the error paint below.
  const [d] = await Promise.all([
    ensureRevisionComposite(revisionId),
    fetchScopedAttention(revisionId),
  ]);
  // A later selection change may have superseded this fetch.
  const sel = getState().selected;
  if (sel.kind !== "revision" || sel.id !== revisionId) return;
  if (!d) {
    const live = $("#detail-body");
    if (live)
      live.innerHTML = `<p class="${CLASS.upEmpty}">error: revision ${escapeHtml(
        shortRef(revisionId),
      )} could not be loaded</p>`;
    return;
  }
  renderRevisionPage(d);
}

/**
 * Show a revision's composite, skipping the fetch when it is already shown. Returns
 * the in-flight fetch so a caller can await the paint; render ignores the return.
 */
export function showComposite(revisionId: string): Promise<void> {
  // The revision-id dedupe guards the composite fetch only — it must not pin
  // the outstanding block, which follows the global attention doc's freshness.
  if (revisionId === shownCompositeId)
    return refreshOutstandingIfStale(revisionId);
  shownCompositeId = revisionId;
  return openRevision(revisionId);
}

async function copyRawEvent(button: HTMLElement): Promise<void> {
  const raw = button
    .closest(`.${CLASS.rawEvent}`)
    ?.querySelector<HTMLElement>("[data-raw-event]")?.textContent;
  if (!raw) return;
  const previous = button.textContent ?? "copy";
  try {
    if (!navigator.clipboard?.writeText) {
      throw new Error("clipboard unavailable");
    }
    await navigator.clipboard.writeText(raw);
    button.textContent = "copied";
  } catch {
    button.textContent = "copy failed";
  } finally {
    window.setTimeout(() => {
      button.textContent = previous;
    }, 1200);
  }
}

// ---------------------------------------------------------------------------
// Fixed-id controls (wired once by the composition root)
// ---------------------------------------------------------------------------

/**
 * Wire the `#detail` open-diff delegate. The rendered diff buttons carry
 * `data-open-diff` / `data-diff-hash` / `data-diff-focus`; this single delegated
 * handler opens the overlay through `diff/controller`. Installed once (called by
 * the composition root), never at a render call site.
 */
export function initControls(): void {
  const el = $<HTMLElement>("#detail");
  el?.addEventListener("click", (ev) => {
    const t = ev.target;
    if (!(t instanceof Element)) return;
    const rawCopyBtn = t.closest<HTMLElement>("[data-copy-raw-event]");
    if (rawCopyBtn) {
      void copyRawEvent(rawCopyBtn);
      return;
    }
    const diffBtn = t.closest<HTMLElement>("[data-open-diff]");
    if (diffBtn) {
      const snapshotId = diffBtn.dataset.openDiff;
      if (snapshotId)
        openDiff(
          snapshotId,
          diffBtn.dataset.diffFocus || null,
          diffBtn.dataset.diffHash || null,
        );
    }
  });
}
