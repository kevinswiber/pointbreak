// The timeline master lens: paint the event timeline into the `#timeline` body
// (injected by the render orchestrator). Ported from the served app.js
// `renderTimeline`. State-reading (filters/order/selection off the store) and
// DOM-writing, with one fidelity-preserving change from app.js: the per-row click
// listener is dropped. Each row carries the `data-event-id` delegation dataset and
// the `#master` delegate (wired once by the composition root) handles selection,
// skipping ref chips via its `closest("[data-ref-kind]")` guard.
//
// The server filters, searches, and orders the history now, and returns only a
// window of the matched set. The DOM is virtualized over the GLOBAL matched set:
// the scroll height is sized by the server `matchCount`, the loaded window sits at
// its `offset`, only the rows visible in the viewport are painted (bracketed by
// top and bottom spacers that preserve the full scroll height), and a scroll that
// nears the loaded edge fetches the next page. When the viewport height is unknown
// (no layout — e.g. detached/unmounted), the render falls back to painting the
// whole loaded window without paging.

import { CLASS } from "../classNames";
import { fetchHistoryPage } from "../data";
import { $ } from "../dom";
import { escapeHtml } from "../escape";
import { fmtTime } from "../format";
import {
  captureSupersedesBadge,
  factSupersessionBadge,
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
import { getState, type State } from "../store";
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

/** The loaded page entries — the server already filtered and ordered them. */
export function timelineRows(): HistoryEntry[] {
  return getState().history?.entries ?? [];
}

/**
 * The loaded window's geometry in the GLOBAL matched-set index space: where the
 * window starts (`offset`), how many rows are loaded (`count`), and the total
 * matched size (`matchCount`, which sizes the virtual scrollbar). Shared with the
 * keyboard stepper so the scroller and the stepper never disagree on geometry.
 */
export function loadedWindow(state: State): {
  offset: number;
  count: number;
  matchCount: number;
} {
  const h = state.history;
  const entries = h?.entries ?? [];
  const offset = h?.offset ?? 0;
  const matchCount = h?.matchCount ?? entries.length;
  return { offset, count: entries.length, matchCount };
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
  // Clamp to the content's real max scroll. After the list shrinks (a filter or
  // search narrows the result set) the viewport's scrollTop can be far past the
  // new content height; without this clamp `start` would exceed `rowCount` and
  // the window would be empty, painting a blank list under a giant spacer.
  const maxScroll = Math.max(0, rowCount * ROW_H - viewportH);
  const clamped = Math.min(Math.max(0, scrollTop), maxScroll);
  const start = Math.max(0, Math.floor(clamped / ROW_H) - OVERSCAN);
  const end = Math.min(
    rowCount,
    Math.ceil((clamped + viewportH) / ROW_H) + OVERSCAN,
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
  const factTag = factSupersessionBadge(e);
  li.innerHTML = `
      <span class="${CLASS.time}">${escapeHtml(fmtTime(e.occurredAt ?? ""))}</span>
      <span class="${CLASS.rail}" style="background:${typeColor(e.eventType)}"></span>
      <span class="${CLASS.body}">
        <span class="${CLASS.title}">${linkify(entryTitle(e))} ${tags} ${supersedesTag} ${staleTag} ${factTag}</span>
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

/** Paint the visible window of the server-filtered timeline page into `#timeline`. */
export function renderTimeline(): void {
  const list = $<HTMLElement>("#timeline");
  if (!list) return;
  const state = getState();
  const rows = timelineRows();
  const { offset, matchCount } = loadedWindow(state);
  if (matchCount === 0) {
    list.innerHTML = "";
    const li = document.createElement("li");
    li.className = "event";
    li.innerHTML = `<span></span><span></span><span class="${CLASS.body}"><span class="${CLASS.title}" style="color:var(--fg-dim)">no events match the current filters</span></span>`;
    list.appendChild(li);
    return;
  }
  ensureScrollListener(list);
  const loadEnd = offset + rows.length;
  const viewportH = list.clientHeight;
  // The visible range is computed in the GLOBAL matched-set index space, then
  // clamped to the loaded window so only loaded rows paint (unloaded global rows
  // are covered by the spacers until a page fetch fills them in).
  const { start, end } = visibleRange(list.scrollTop, viewportH, matchCount);
  const paintStart = Math.min(Math.max(start, offset), loadEnd);
  const paintEnd = Math.min(Math.max(end, offset), loadEnd);
  const selected = selectedEventId();
  list.innerHTML = "";
  if (paintStart > 0) list.appendChild(spacer(paintStart * ROW_H));
  for (let i = paintStart; i < paintEnd; i++)
    list.appendChild(eventRow(rows[i - offset], selected));
  if (paintEnd < matchCount)
    list.appendChild(spacer((matchCount - paintEnd) * ROW_H));
  maybeExtendWindow(viewportH, end, loadEnd, matchCount);
}

// Fetch the next page when the viewport nears the trailing loaded edge and more
// of the matched set remains. Only with a real viewport (no layout ⇒ a full
// fallback paint, not a scroll), and the page-fetch helper dedupes concurrent
// requests so a burst of scroll events issues one fetch.
function maybeExtendWindow(
  viewportH: number,
  visibleEnd: number,
  loadEnd: number,
  matchCount: number,
): void {
  if (viewportH <= 0) return;
  if (loadEnd >= matchCount) return;
  if (visibleEnd < loadEnd - OVERSCAN) return;
  void fetchHistoryPage({ offset: loadEnd });
}

/**
 * Scroll the selected event's row into the virtual window and into view. Under
 * virtualization an off-screen row is not in the DOM, so set the scroll position
 * to center the row's GLOBAL index, repaint that window, then `scrollIntoView` the
 * now-rendered row. A target outside the loaded window is left to the fetch-to-
 * reveal path (it must fetch the containing page first). The centralized selection
 * scroller (render.ts) routes event selection here.
 */
export function scrollTimelineSelectionIntoView(eventId: string): void {
  const list = $<HTMLElement>("#timeline");
  if (!list) return;
  const local = timelineRows().findIndex((e) => e.eventId === eventId);
  if (local < 0) return;
  const global = loadedWindow(getState()).offset + local;
  const centered =
    global * ROW_H - Math.max(0, (list.clientHeight - ROW_H) / 2);
  list.scrollTop = Math.max(0, centered);
  renderTimeline();
  const el = list.querySelector(`li[data-event-id="${eventId}"]`);
  if (el) el.scrollIntoView({ block: "center" });
}
