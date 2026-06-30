// The timeline master lens: paint the event timeline into the `#timeline` body
// (injected by the render orchestrator). Ported from the served app.js
// `renderTimeline`. State-reading (filters/order/selection off the store) and
// DOM-writing, with one fidelity-preserving change from app.js: the per-row click
// listener is dropped. Each row carries the `data-event-id` delegation dataset and
// the `#master` delegate (wired once by the composition root) handles selection,
// skipping ref chips via its `closest("[data-ref-kind]")` guard.
//
// The DOM is virtualized: the client still fetches the full `/api/history` and
// filters/searches over the whole set in memory (so client search keeps working),
// but only the rows visible in the scroll viewport are painted, bracketed by top
// and bottom spacers that preserve the full scroll height. When the viewport
// height is unknown (no layout — e.g. detached/unmounted), the render falls back
// to painting every filtered row.

import { CLASS } from "../classNames";
import { $ } from "../dom";
import { escapeHtml } from "../escape";
import { fmtTime } from "../format";
import {
  captureSupersedesBadge,
  matchesFilters,
  selectedEventId,
  supersessionStaleBadge,
} from "../model";
import {
  entryAnchor,
  entryRevisionId,
  entryTags,
  entryTitle,
  entryTrack,
  verificationChip,
} from "../projection";
import { linkify, shortId } from "../refs";
import { getState } from "../store";
import type { HistoryEntry } from "../types";
import { typeColor, typeLabel } from "../types";

/**
 * Estimated row height in pixels for the virtual window math. The real row height
 * varies a little with density and meta wrapping; `OVERSCAN` rows above and below
 * the viewport absorb that variance, and reveal does a final `scrollIntoView` to
 * correct the exact offset.
 */
export const ROW_H = 52;

/** Extra rows rendered above and below the viewport so a fast scroll never flashes blank. */
const OVERSCAN = 8;

/** The filtered + ordered timeline entries — the full list the DOM is a window onto. */
export function timelineRows(): HistoryEntry[] {
  // Server returns entries oldest->newest (occurredAt asc); default display is
  // newest-first, with a toolbar toggle back to chronological.
  const state = getState();
  let entries = (state.history?.entries ?? []).filter(matchesFilters);
  if (state.order === "desc") entries = entries.slice().reverse();
  return entries;
}

/**
 * The `[start, end)` index range to paint for a scroll viewport. When the viewport
 * height is unknown (`<= 0`, i.e. no layout), the whole list is returned so the
 * render degrades to a full paint.
 */
export function visibleRange(
  scrollTop: number,
  viewportH: number,
  rowCount: number,
): { start: number; end: number } {
  if (viewportH <= 0 || rowCount === 0) return { start: 0, end: rowCount };
  const start = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN);
  const end = Math.min(
    rowCount,
    Math.ceil((scrollTop + viewportH) / ROW_H) + OVERSCAN,
  );
  return { start, end };
}

/** A zero-content `<li>` of the given height, holding the scroll geometry off-screen rows occupy. */
function spacer(height: number): HTMLLIElement {
  const li = document.createElement("li");
  li.dataset.spacer = "1";
  li.setAttribute("aria-hidden", "true");
  li.style.height = `${height}px`;
  return li;
}

/** Build one timeline row `<li>` with the per-row markup and the delegation dataset. */
function eventRow(e: HistoryEntry, selected: string | null): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "event";
  li.dataset.eventId = e.eventId ?? "";
  if (e.eventId && e.eventId === selected)
    li.setAttribute("aria-selected", "true");
  const tags = entryTags(e)
    .map((t) => `<span class="${CLASS.badge}">${escapeHtml(t)}</span>`)
    .join(" ");
  const revisionId = entryRevisionId(e);
  const staleTag = supersessionStaleBadge(e);
  const supersedesTag = captureSupersedesBadge(e);
  li.innerHTML = `
      <span class="${CLASS.time}">${escapeHtml(fmtTime(e.occurredAt ?? ""))}</span>
      <span class="${CLASS.rail}" style="background:${typeColor(e.eventType)}"></span>
      <span class="${CLASS.body}">
        <span class="${CLASS.title}">${linkify(entryTitle(e))} ${tags} ${supersedesTag} ${staleTag}</span>
        <span class="${CLASS.meta}">
          <span class="${CLASS.type}" style="color:${typeColor(e.eventType)}">${escapeHtml(typeLabel(e.eventType))}</span>
          ${entryTrack(e) ? `<span>${escapeHtml(entryTrack(e))}</span>` : ""}
          ${revisionId ? `<span>revision ${escapeHtml(shortId(revisionId))}</span>` : ""}
          ${entryAnchor(e) ? `<span>${escapeHtml(entryAnchor(e))}</span>` : ""}
          ${verificationChip(e.verificationStatus ?? "")}
        </span>
      </span>`;
  return li;
}

// Repaint the visible window when the viewport scrolls. Registered once per
// `#timeline` element (guarded by the `virtualized` dataset flag); the element is
// stable across timeline renders (renderMaster rebuilds it only on a lens change).
function ensureScrollListener(list: HTMLElement): void {
  if (list.dataset.virtualized) return;
  list.dataset.virtualized = "1";
  list.addEventListener("scroll", () => renderTimeline());
}

/** Paint the visible window of the filtered, ordered event timeline into `#timeline`. */
export function renderTimeline(): void {
  const list = $<HTMLElement>("#timeline");
  if (!list) return;
  const rows = timelineRows();
  if (!rows.length) {
    list.innerHTML = "";
    const li = document.createElement("li");
    li.className = "event";
    li.innerHTML = `<span></span><span></span><span class="${CLASS.body}"><span class="${CLASS.title}" style="color:var(--fg-dim)">no events match the current filters</span></span>`;
    list.appendChild(li);
    return;
  }
  ensureScrollListener(list);
  const { start, end } = visibleRange(
    list.scrollTop,
    list.clientHeight,
    rows.length,
  );
  const selected = selectedEventId();
  list.innerHTML = "";
  if (start > 0) list.appendChild(spacer(start * ROW_H));
  for (let i = start; i < end; i++)
    list.appendChild(eventRow(rows[i], selected));
  if (end < rows.length) list.appendChild(spacer((rows.length - end) * ROW_H));
}

/**
 * Scroll the selected event's row into the virtual window and into view. Under
 * virtualization an off-screen row is not in the DOM, so set the scroll position
 * to center the row's index, repaint that window, then `scrollIntoView` the
 * now-rendered row. The centralized selection scroller (render.ts) routes event
 * selection here so route-state, deep-link, and keyboard nav all reach off-screen
 * rows.
 */
export function scrollTimelineSelectionIntoView(eventId: string): void {
  const list = $<HTMLElement>("#timeline");
  if (!list) return;
  const index = timelineRows().findIndex((e) => e.eventId === eventId);
  if (index < 0) return;
  const centered = index * ROW_H - Math.max(0, (list.clientHeight - ROW_H) / 2);
  list.scrollTop = Math.max(0, centered);
  renderTimeline();
  const el = list.querySelector(`li[data-event-id="${eventId}"]`);
  if (el) el.scrollIntoView({ block: "center" });
}
