// The detail pane: the `#detail` projection of the single selection. It paints
// the event detail (identity table + readback + body + raw payload), the revision
// composite page (the on-demand `/api/revision` fetch mounting the pure `cards`
// renderers), and the state-bound `staleFactSectionContext` fed into the pure
// `cards.factSection`. Ported from the served app.js detail cluster (`renderDetail`
// / `renderUnitPage` → `renderRevisionPage` / `openUnit` → `openRevision` /
// `showComposite` / `eventBodyBlock` / `staleFactSectionContext`), in the revision
// vocabulary.
//
// Two structural moves preserve the served behaviour behind the new boundaries:
//   - Detail mutates `#detail` and reads state / `http`; it never calls render
//     (the store subscriber repaints on commit) and never imports a navigation
//     module. The "show in timeline" affordance is therefore emitted as a
//     `data-reveal-revision` dataset for the navigation delegate to resolve.
//   - The per-render diff-button listeners become one delegated `#detail` click
//     handler (installed once by the composition root): the buttons carry
//     `data-open-diff` / `data-diff-hash` / `data-diff-focus`, and the delegate
//     opens the diff through `diff/controller.openDiff` (import direction
//     detail → diff/controller, never the reverse).

import {
  type AdapterNote,
  currentAssessmentSummary,
  factSection,
  type InputRequest,
  type Observation,
  type RevisionDetail,
  renderAdapterNoteCard,
  renderAssessmentCard,
  renderInputRequestCard,
  renderObservationCard,
  renderValidationCheckCard,
  type ValidationCheck,
  verdictBadge,
} from "./cards";
import { CLASS } from "./classNames";
import { openDiff } from "./diff/controller";
import { $ } from "./dom";
import { escapeHtml } from "./escape";
import { fmtDateTime } from "./format";
import { fetchJSON } from "./http";
import { renderBodyContent } from "./markdown";
import {
  objectArtifactHashForRevision,
  selectedEventId,
  snapshotIdForRevision,
  supersededByRevision,
  supersedesRevision,
  supersessionBadge,
} from "./model";
import {
  endorsementsBlock,
  entryRevisionId,
  entryTitle,
  entryTrack,
  principalLabel,
  verificationChip,
} from "./projection";
import {
  linkify,
  shortId,
  type TargetDisplay,
  targetDisplayLabel,
  targetHeadBadge,
} from "./refs";
import { getState } from "./store";
import { type EntryBase, type HistoryEntry, typeLabel } from "./types";

// ---------------------------------------------------------------------------
// The /api/revision composite-page document (the fields the page reads)
// ---------------------------------------------------------------------------

/** The revision record on the `/api/revision` document (keyed on `id`, not `revisionId`). */
interface RevisionPageRevision {
  id?: string;
  base?: EntryBase;
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
  adapterNoteCount?: number;
}

/** The `/api/revision` composite document the revision page projects. */
interface RevisionPageDoc extends RevisionDetail {
  revision?: RevisionPageRevision;
  summary?: RevisionPageSummary;
  observations?: Observation[];
  inputRequests?: InputRequest[];
  validationChecks?: ValidationCheck[];
  adapterNotes?: AdapterNote[];
}

// The revision whose composite is currently shown, so a re-render with an
// unchanged revision selection does not re-fetch. Transient view-cache — never on
// the store.
let shownCompositeId: string | null = null;

// ---------------------------------------------------------------------------
// Event detail
// ---------------------------------------------------------------------------

/** The first non-empty body fallback for an event: body, then summary, then reason. */
export function eventBodyBlock(e: HistoryEntry): string {
  const s = e.summary ?? {};
  if (s.body) return renderBodyContent(s.body, s.bodyContentType);
  if (s.summary) return renderBodyContent(s.summary, s.summaryContentType);
  if (s.reason) return renderBodyContent(s.reason, s.reasonContentType);
  return "";
}

/** Paint `#detail` from the selected event, or the empty prompt when none is selected. */
export function renderDetail(): void {
  // Showing the event/empty pane means no composite is shown — so a later
  // re-selection of a revision re-fetches its composite.
  shownCompositeId = null;
  const el = $("#detail");
  if (!el) return;
  const entries = getState().history?.entries ?? [];
  const e = entries.find((x) => x.eventId === selectedEventId());
  if (!e) {
    el.innerHTML = `<p class="${CLASS.empty}">Select an event or revision to inspect.</p>`;
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
    ["writer", principalLabel(e) || (e.writer ? e.writer.actorId || "—" : "—")],
  ];
  const snapshotId = revisionId ? snapshotIdForRevision(revisionId) : null;
  const s = e.summary ?? {};
  if (e.eventType === "work_object_proposed") {
    const predecessors = supersedesRevision(revisionId);
    if (predecessors.length) kv.push(["supersedes", predecessors.join(", ")]);
  }
  if (e.eventType === "validation_check_recorded") {
    kv.push(["check", s.checkName || "—"]);
    kv.push(["status", s.status || "—"]);
    kv.push(["trigger", s.trigger || "—"]);
    if (s.exitCode != null) kv.push(["exit code", String(s.exitCode)]);
    if (s.command) kv.push(["command", s.command]);
    kv.push(["validationCheckId", s.validationCheckId || "—"]);
  }
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
    ? `<button class="${CLASS.ghost} ${CLASS.diffBtn}" id="detail-diff-btn" data-open-diff="${escapeHtml(snapshotId)}" data-diff-hash="${escapeHtml(objectArtifactHashForRevision(revisionId))}" data-diff-focus="${escapeHtml(focusId ?? "")}">${escapeHtml(btnLabel)}</button>`
    : "";
  el.innerHTML = `
    <h2>${linkify(entryTitle(e))}</h2>
    <dl class="${CLASS.kv}">${kv.map(([k, v]) => `<dt>${escapeHtml(k)}</dt><dd>${linkify(v)}</dd>`).join("")}</dl>
    ${readback}
    ${diffButton}
    ${bodyBlock}
    <pre>${escapeHtml(JSON.stringify(e, null, 2))}</pre>`;
}

// ---------------------------------------------------------------------------
// Revision composite page
// ---------------------------------------------------------------------------

/** The "superseded by <successors>" context repeated near each fact section, or "". */
export function staleFactSectionContext(revisionId: string): string {
  const successors = supersededByRevision(revisionId);
  if (!successors.length) return "";
  return `<p class="${CLASS.factStaleContext}">superseded by ${successors.map(linkify).join(" ")}</p>`;
}

/** Paint `#detail` with a revision's composite page from the `/api/revision` document. */
export function renderRevisionPage(d: RevisionPageDoc): void {
  const ru = d.revision ?? {};
  const base = ru.base ?? {};
  const s = d.summary ?? {};
  const revisionId = ru.id ?? "";
  const badge = supersessionBadge(revisionId);
  const title = `${shortId(ru.id)}${base.commitOid ? ` · base ${shortId(base.commitOid)}` : ""}`;
  const staleContext = staleFactSectionContext(revisionId);

  const stat = (label: string, n?: number): string =>
    `<span class="${CLASS.upStat}"><b>${n ?? 0}</b> ${label}</span>`;
  const sections: string[] = [];

  sections.push(`<section><h2>Revision</h2><dl class="${CLASS.upIdentity}">
    <dt>id</dt><dd>${linkify(ru.id)}</dd>
    <dt>base</dt><dd>${base.commitOid ? linkify(base.commitOid) : "—"} ${base.kind ? `<span class="${CLASS.factStatus}">${escapeHtml(base.kind)}</span>` : ""}</dd>
    <dt>target</dt><dd>${targetDisplayLabel(ru.targetDisplay)}${targetHeadBadge(ru.targetDisplay)}</dd>
    <dt>worktree</dt><dd>${escapeHtml(ru.targetDisplay?.label ?? "working tree")}</dd>
    <dt>head</dt><dd>${escapeHtml(ru.targetDisplay?.head?.label ?? "—")}</dd>
    <dt>supersession</dt><dd>${badge || "—"}</dd>
    <dt>snapshot</dt><dd>${linkify(ru.objectId)}</dd>
  </dl></section>`);

  sections.push(
    `<section><h2>Current assessment</h2>${verdictBadge(d.currentAssessment)}${currentAssessmentSummary(d)}<p class="${CLASS.advisoryNote}">advisory — a recorded judgement, not a merge gate</p></section>`,
  );

  // The annotated-diff affordance carries its open-diff datasets (the #detail
  // delegate opens it); the "show in timeline" affordance carries a
  // data-reveal-revision dataset the navigation delegate resolves.
  sections.push(`<section><h2>Summary</h2><div class="${CLASS.upStats}">
    ${stat("files", s.fileCount)}${stat("rows", s.rowCount)}${stat("observations", s.observationCount)}${stat("input requests", s.inputRequestCount)}${stat("assessments", s.assessmentCount)}${stat("validation checks", s.validationCheckCount)}${stat("adapter notes", s.adapterNoteCount)}
  </div>
  <div style="margin-top:10px">
    <button class="${CLASS.ghost} ${CLASS.diffBtn}" id="up-diff-btn" data-open-diff="${escapeHtml(ru.objectId ?? "")}" data-diff-hash="${escapeHtml(ru.objectArtifactContentHash ?? "")}">view annotated diff</button>
    <button class="${CLASS.ghost}" id="up-timeline-btn" data-reveal-revision="${escapeHtml(revisionId)}" style="margin-left:6px">show in timeline</button>
  </div></section>`);

  sections.push(
    factSection(
      "Observations",
      d.observations,
      renderObservationCard,
      staleContext,
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
      staleContext,
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

  if ((d.adapterNotes ?? []).length) {
    sections.push(
      factSection("Adapter notes", d.adapterNotes, renderAdapterNoteCard),
    );
  }

  const el = $("#detail");
  if (el)
    el.innerHTML = `<div class="${CLASS.unitPage}"><p class="${CLASS.unitPageTitle}">${escapeHtml(title)}</p>${sections.join("")}</div>`;
}

/** Fetch a revision's composite document and paint it, guarding a superseding selection. */
export async function openRevision(revisionId: string): Promise<void> {
  const el = $("#detail");
  if (el) el.innerHTML = `<p class="${CLASS.upEmpty}">loading…</p>`;
  try {
    const d = await fetchJSON(
      `/api/revision?id=${encodeURIComponent(revisionId)}`,
    );
    // A later selection change may have superseded this fetch.
    const sel = getState().selected;
    if (sel.kind !== "revision" || sel.id !== revisionId) return;
    renderRevisionPage(d as RevisionPageDoc);
  } catch (err: unknown) {
    const sel = getState().selected;
    if (sel.kind === "revision" && sel.id === revisionId) {
      const live = $("#detail");
      if (live)
        live.innerHTML = `<p class="${CLASS.upEmpty}">error: ${escapeHtml(
          err instanceof Error ? err.message : String(err),
        )}</p>`;
    }
  }
}

/**
 * Show a revision's composite, skipping the fetch when it is already shown. Returns
 * the in-flight fetch so a caller can await the paint; render ignores the return.
 */
export function showComposite(revisionId: string): Promise<void> {
  if (revisionId === shownCompositeId) return Promise.resolve();
  shownCompositeId = revisionId;
  return openRevision(revisionId);
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
    const diffBtn = t.closest<HTMLElement>("[data-open-diff]");
    if (diffBtn) {
      const objectId = diffBtn.dataset.openDiff;
      if (objectId)
        openDiff(
          objectId,
          diffBtn.dataset.diffFocus || null,
          diffBtn.dataset.diffHash || null,
        );
    }
  });
}
