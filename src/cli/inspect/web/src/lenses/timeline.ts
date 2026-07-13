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
import { fetchHistoryPage, HISTORY_PAGE } from "../data";
import { $ } from "../dom";
import { escapeHtml } from "../escape";
import { fmtDate, fmtTime } from "../format";
import {
  captureSupersedesBadge,
  factSupersessionBadge,
  selectedEventId,
  supersessionStaleBadge,
} from "../model";
import {
  entryActor,
  entryAnchor,
  entryRevisionId,
  entryTags,
  entryTitle,
  entryTrack,
  verificationChip,
} from "../projection";
import { actorChip, linkify, shortId } from "../refs";
import { getState, type State } from "../store";
import type { HistoryEntry } from "../types";
import { typeColor, typeLabel } from "../types";

/**
 * Fallback row height in pixels for the virtual window math, used until (and
 * unless) the painted rows can be measured — no layout, nothing painted yet.
 * The live estimate re-derives from real layout (`remeasureTimelineRows`)
 * because the real height moves systematically with the pane: a narrow master
 * wraps the meta line taller, compact density pads shorter. Residual per-row
 * variance around the measured mean is absorbed by `OVERSCAN`, and reveal does
 * a final `scrollIntoView` to correct the exact offset.
 */
export const ROW_H = 52;

// The live per-row estimate every consumer (window math, spacers, reveal)
// reads. A module singleton like the store: one timeline, one geometry.
let rowH: number = ROW_H;

/** The live row-height estimate driving the virtual window math. */
export function timelineRowHeight(): number {
  return rowH;
}

/** Extra rows rendered above and below the viewport so a fast scroll never flashes blank. */
const OVERSCAN = 8;

/** Trailing settle before a re-measure, so a divider drag or a window-resize burst coalesces. */
const REMEASURE_SETTLE_MS = 150;

let remeasureTimer: ReturnType<typeof setTimeout> | undefined;

// Whether a measurement has ever succeeded. Until one has, every paint retries
// (a paint without layout measures nothing), so the estimate is seeded by the
// FIRST laid-out paint — before the load-time reveal computes with it — rather
// than waiting on the observer's initial (async) delivery.
let everMeasured = false;

/**
 * Re-derive the row estimate from the painted rows' live layout: the mean
 * `getBoundingClientRect().height` (unrounded — a half-pixel rounding error
 * times hundreds of spacer rows is real drift). Guarded to keep the current
 * estimate when there is nothing to measure: no painted rows (the empty state)
 * or a zero/non-finite mean (no layout — tests, a hidden pane). A changed
 * estimate repaints so the spacers and the visible window re-derive.
 */
export function remeasureTimelineRows(): void {
  const list = $<HTMLElement>("#timeline");
  if (!list) return;
  const rows = list.querySelectorAll<HTMLElement>("li.event[data-event-id]");
  if (rows.length === 0) return;
  let total = 0;
  for (const row of rows) total += row.getBoundingClientRect().height;
  const mean = total / rows.length;
  if (!Number.isFinite(mean) || mean <= 0) return;
  everMeasured = true;
  if (Math.abs(mean - rowH) < 0.5) return;
  // Anchor the reading position before the repaint: the new estimate
  // re-derives the whole scroll track, which would otherwise teleport the
  // content under the reader (a density flip) or displace a just-revealed
  // selection (the first measurement after a load's deep-link reveal).
  const anchored = anchoredScrollTop(list, rowH, mean);
  rowH = mean;
  list.scrollTop = anchored;
  renderTimeline();
}

/**
 * Where the viewport top lands on the re-derived track. Prefer the painted
 * row whose real rect straddles the viewport top and keep its exact offset —
 * scrollTop itself cannot be trusted as `index * estimate`, because reveal's
 * final `scrollIntoView` corrects it against REAL row heights. When no
 * painted row intersects the top (a spacer region, or no layout), fall back
 * to scaling scrollTop so the same global index stays at the top.
 */
function anchoredScrollTop(
  list: HTMLElement,
  prevRowH: number,
  nextRowH: number,
): number {
  const listTop = list.getBoundingClientRect().top;
  const first = list.firstElementChild as HTMLElement | null;
  const leadingPx =
    first?.dataset.spacer === "1"
      ? Number.parseFloat(first.style.height) || 0
      : 0;
  const paintStart = Math.round(leadingPx / prevRowH);
  const rows = list.querySelectorAll<HTMLElement>("li.event[data-event-id]");
  let idx = 0;
  for (const row of rows) {
    const r = row.getBoundingClientRect();
    if (r.height > 0 && r.bottom > listTop)
      return Math.max(0, (paintStart + idx) * nextRowH - (r.top - listTop));
    idx++;
  }
  return (list.scrollTop / prevRowH) * nextRowH;
}

/** Coalesce a burst of layout changes into one trailing `remeasureTimelineRows`. */
export function scheduleTimelineRemeasure(): void {
  clearTimeout(remeasureTimer);
  remeasureTimer = setTimeout(remeasureTimelineRows, REMEASURE_SETTLE_MS);
}

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
  const maxScroll = Math.max(0, rowCount * rowH - viewportH);
  const clamped = Math.min(Math.max(0, scrollTop), maxScroll);
  const start = Math.max(0, Math.floor(clamped / rowH) - OVERSCAN);
  const end = Math.min(
    rowCount,
    Math.ceil((clamped + viewportH) / rowH) + OVERSCAN,
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
  const staleTag = supersessionStaleBadge(e, { tabIndex: -1 });
  const supersedesTag = captureSupersedesBadge(e, { tabIndex: -1 });
  const factTag = factSupersessionBadge(e);
  li.innerHTML = `
      <span class="${CLASS.time}"><span class="${CLASS.eventDate}">${escapeHtml(fmtDate(e.occurredAt ?? ""))}</span><span>${escapeHtml(fmtTime(e.occurredAt ?? ""))}</span></span>
      <span class="${CLASS.rail}" style="background:${typeColor(e.eventType)}"></span>
      <span class="${CLASS.body}">
        <span class="${CLASS.title}">${linkify(entryTitle(e), { tabIndex: -1 })} ${tags} ${supersedesTag} ${staleTag} ${factTag}</span>
        <span class="${CLASS.meta}">
          <span class="${CLASS.type}" style="color:${typeColor(e.eventType)}">${escapeHtml(typeLabel(e.eventType))}</span>
          ${entryTrack(e) ? `<span>${escapeHtml(entryTrack(e))}</span>` : ""}
          ${entryActor(e) ? actorChip(entryActor(e), { tabIndex: -1 }) : ""}
          ${revisionId ? `<span>revision ${escapeHtml(shortId(revisionId))}</span>` : ""}
          ${entryAnchor(e) ? `<span>${escapeHtml(entryAnchor(e))}</span>` : ""}
          ${verificationChip(e.verificationStatus ?? "")}
        </span>
      </span>`;
  return li;
}

// Repaint the visible window when the viewport scrolls, and re-measure the row
// estimate when the element's size changes. Registered once per `#timeline`
// element (guarded by the `virtualized` dataset flag); the element is stable
// across timeline renders (renderMaster rebuilds it only on a lens change).
// One size observation covers every width-changing trigger — divider release
// (the drag burst settles through the trailing debounce), window resize, the
// narrow media query, detail-pane open/close, and reading mode — plus the
// initial delivery on observe, which seeds the first real measurement. Density
// is the one height-changing trigger with no width signal; the composition
// root routes that toggle here explicitly.
function ensureScrollListener(list: HTMLElement): void {
  if (list.dataset.virtualized) return;
  list.dataset.virtualized = "1";
  list.addEventListener("scroll", () => renderTimeline());
  if (typeof ResizeObserver !== "undefined")
    new ResizeObserver(scheduleTimelineRemeasure).observe(list);
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
  if (paintStart > 0) list.appendChild(spacer(paintStart * rowH));
  for (let i = paintStart; i < paintEnd; i++)
    list.appendChild(eventRow(rows[i - offset], selected));
  if (paintEnd < matchCount)
    list.appendChild(spacer((matchCount - paintEnd) * rowH));
  maybeExtendWindow(viewportH, start, end, offset, loadEnd, matchCount);
  // Seed the estimate from the first paint that has anything to measure (no
  // recursion: a successful measurement flips `everMeasured` before its
  // repaint). One synchronous layout read, once per load.
  if (!everMeasured && paintEnd > paintStart) remeasureTimelineRows();
}

// Fetch the next page when the viewport nears the trailing loaded edge and more
// of the matched set remains. Only with a real viewport (no layout ⇒ a full
// fallback paint, not a scroll), and the page-fetch helper dedupes concurrent
// requests so a burst of scroll events issues one fetch.
function maybeExtendWindow(
  viewportH: number,
  visibleStart: number,
  visibleEnd: number,
  loadStart: number,
  loadEnd: number,
  matchCount: number,
): void {
  if (viewportH <= 0) return;
  // Forward: the viewport nears the trailing loaded edge and more remains.
  if (loadEnd < matchCount && visibleEnd >= loadEnd - OVERSCAN) {
    void fetchHistoryPage({ offset: loadEnd });
  }
  // Backward (symmetric): the viewport nears the leading loaded edge and the
  // window starts above the set's start (e.g. a reveal landed it mid-set). The
  // fixed-height spacer model keeps the scroll track a stable `matchCount` rows,
  // so a prepend back-fills leading-spacer rows in place with no scroll jump.
  if (loadStart > 0 && visibleStart <= loadStart + OVERSCAN) {
    void fetchHistoryPage({ offset: Math.max(0, loadStart - HISTORY_PAGE) });
  }
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
  // Sync the estimate with the live layout first: it can be stale at reveal
  // time (the load path measures at full width, then the same render pass
  // opens the split and reflows the rows before the deep-link reveal runs).
  // Seeding from a stale estimate makes the scrollIntoView correction's
  // repaint recompute a window that drops the target — the lost cursor.
  remeasureTimelineRows();
  const global = loadedWindow(getState()).offset + local;
  const centered = global * rowH - Math.max(0, (list.clientHeight - rowH) / 2);
  list.scrollTop = Math.max(0, centered);
  renderTimeline();
  const el = list.querySelector(`li[data-event-id="${eventId}"]`);
  if (el) el.scrollIntoView({ block: "center" });
}
