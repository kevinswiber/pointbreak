// The timeline master lens: paint the event timeline into the `#timeline` body
// (injected by the render orchestrator). Ported from the served app.js
// `renderTimeline`. State-reading (filters/order/selection off the store) and
// DOM-writing, with one fidelity-preserving change from app.js: the per-row click
// listener is dropped. Each row carries the `data-event-id` delegation dataset and
// the `#master` delegate (wired once by the composition root) handles selection,
// skipping ref chips via its `closest("[data-ref-kind]")` guard.

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
import { typeColor, typeLabel } from "../types";

/** Paint the filtered, ordered event timeline into the `#timeline` body. */
export function renderTimeline(): void {
  const list = $("#timeline");
  if (!list) return;
  list.innerHTML = "";
  // Server returns entries oldest->newest (occurredAt asc); default display is
  // newest-first, with a toolbar toggle back to chronological.
  const state = getState();
  let entries = (state.history?.entries ?? []).filter(matchesFilters);
  if (state.order === "desc") entries = entries.slice().reverse();
  if (!entries.length) {
    const li = document.createElement("li");
    li.className = "event";
    li.innerHTML = `<span></span><span></span><span class="${CLASS.body}"><span class="${CLASS.title}" style="color:var(--fg-dim)">no events match the current filters</span></span>`;
    list.appendChild(li);
    return;
  }
  const selected = selectedEventId();
  for (const e of entries) {
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
    list.appendChild(li);
  }
}
