// The revision-centric master lens: the flat revision list (`renderRevisionList`,
// the `#units` body). Ported from the served app.js `renderUnits`, in the
// revision vocabulary.
//
// State-reading + DOM-writing. One fidelity-preserving shape change from app.js:
// the per-card click listener and the per-card overview-cue listener are dropped —
// cards carry the `data-revision-id` / `[data-open-diff]` / `[data-attention-query]`
// delegation datasets and the `#master` delegate (wired once by the composition
// root) handles selection, open-diff, and cue filtering.

import { CLASS } from "../classNames";
import { $ } from "../dom";
import { escapeHtml } from "../escape";
import { fmtDateTime } from "../format";
import {
  matchesRevisionFilters,
  orderedRevisionEntries,
  overviewForRevision,
  supersessionBadge,
} from "../model";
import {
  renderRevisionOverview,
  revisionSnapshotUnavailable,
} from "../projection";
import {
  shortId,
  targetDisplayLabel,
  targetHeadBadge,
  workLabelText,
} from "../refs";
import { getState } from "../store";

// ---------------------------------------------------------------------------
// The flat revision list lens (#units)
// ---------------------------------------------------------------------------

/** Paint the filtered revision list into the `#units` body, one card per revision. */
export function renderRevisionList(): void {
  const el = $("#units");
  if (!el) return;
  const state = getState();
  const entries = orderedRevisionEntries(
    (state.revisions?.entries ?? []).filter(matchesRevisionFilters),
    state.order,
    state.sortKey,
  );
  if (!entries.length) {
    el.innerHTML = `<p class="${CLASS.empty}" style="color:var(--fg-dim)">${
      state.filterText || state.filterSnapshot
        ? "No revisions match the current filters."
        : "No captured revisions in this store."
    }</p>`;
    return;
  }
  const selected = state.selected;
  const kv = ([k, v]: [string, string]): string =>
    `<span>${escapeHtml(k)}</span><b>${escapeHtml(v)}</b>`;
  el.innerHTML = entries
    .map((u) => {
      const base = u.base ?? {};
      const overview = u.overview ?? overviewForRevision(u.revisionId ?? "");
      const revisionId = u.revisionId ?? "";
      const isSelected =
        selected.kind === "revision" && selected.id === revisionId;
      const badge = supersessionBadge(revisionId);
      const snapshotUnavailable = revisionSnapshotUnavailable(u);
      const rows: [string, string][] = [
        ["captured", fmtDateTime(u.capturedAt ?? "")],
        [
          "base",
          base.commitOid
            ? `${shortId(base.commitOid)} (${base.kind ?? ""})`
            : (base.kind ?? "—"),
        ],
      ];
      // The landing cell mirrors the CLI merge-status vocabulary
      // (merged/open/unreachable/unknown) so the surfaces agree.
      const tail: [string, string][] = [
        ["revision", shortId(revisionId)],
        ["landing", u.mergeStatus || "unknown"],
        ["snapshot", shortId(u.snapshotId)],
      ];
      // The target cell carries pre-escaped derived HTML (label + head badge), so
      // it bypasses the generic escaping cell renderer rather than double-escaping.
      const targetCell = `<span>target</span><b>${targetDisplayLabel(u.targetDisplay)}${targetHeadBadge(u.targetDisplay)}</b>`;
      // The diff button and the card both carry delegation datasets; the #master
      // delegate (wired by the composition root) opens the diff and selects the
      // card. The data-open-diff value is the captured snapshot id, paired with
      // its content hash for rebased-recapture disambiguation.
      return `<div class="${CLASS.unitCard}" data-revision-id="${escapeHtml(revisionId)}"${
        isSelected ? ' aria-selected="true"' : ""
      } title="${escapeHtml(revisionId)}\nclick to open the revision page">
      <h3>${typeof u.summary === "string" && u.summary ? escapeHtml(u.summary) : workLabelText(u.targetDisplay)}</h3>
      ${badge ? `<div class="${CLASS.supersessionBadges}">${badge}</div>` : ""}
      ${renderRevisionOverview(u, overview)}
      <div class="${CLASS.kv} ${CLASS.tierMedium}">${rows.map(kv).join("")}${targetCell}${tail.map(kv).join("")}</div>
      <div class="${CLASS.actions}">${
        snapshotUnavailable
          ? `<button class="${CLASS.ghost} ${CLASS.diffBtn}" type="button" disabled title="captured snapshot content is unavailable">snapshot unavailable</button>`
          : `<button class="${CLASS.ghost} ${CLASS.diffBtn}" type="button" data-open-diff="${escapeHtml(u.snapshotId ?? "")}" data-diff-hash="${escapeHtml(u.snapshotContentHash ?? "")}">view snapshot diff</button>`
      }</div>
    </div>`;
    })
    .join("");
}
