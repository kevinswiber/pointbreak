"use strict";

// Known durable event types, with display labels and timeline colors.
const TYPES = [
  { id: "review_initialized", label: "init", color: "#7f8c9b" },
  { id: "review_unit_captured", label: "capture", color: "#5aa9e6" },
  { id: "review_observation_recorded", label: "observation", color: "#6dd28a" },
  { id: "review_assessment_recorded", label: "assessment", color: "#b388ff" },
  { id: "input_request_opened", label: "request", color: "#f0b75a" },
  { id: "input_request_responded", label: "response", color: "#4fd0c0" },
  { id: "review_note_imported", label: "note", color: "#9aa7b5" },
];
const TYPE_MAP = Object.fromEntries(TYPES.map((t) => [t.id, t]));

const state = {
  history: null,
  units: null,
  view: "timeline",
  enabledTypes: new Set(TYPES.map((t) => t.id)),
  seenTypes: new Set(TYPES.map((t) => t.id)),
  filterText: "",
  filterTrack: "",
  filterUnit: "",
  order: "desc", // "desc" = newest first (default), "asc" = chronological
  selectedEventId: null,
  lastHash: null,
};

const $ = (sel) => document.querySelector(sel);

function typeColor(id) {
  return (TYPE_MAP[id] || {}).color || "#9aa7b5";
}
function typeLabel(id) {
  return (TYPE_MAP[id] || {}).label || id;
}

function shortId(id) {
  if (!id) return "";
  const tail = String(id).split(":").pop() || "";
  return tail.length > 12 ? tail.slice(0, 12) : tail;
}

// Git-style short form for Shoreline IDs and hashes, keeping the meaningful
// kind prefix: `review-unit:sha256:1ace…` -> `review-unit:1ace028b`.
function shortRef(id) {
  const s = String(id);
  let m = s.match(/^([a-z][a-z-]*):(?:git:)?sha256:([0-9a-f]{6,})$/i);
  if (m) return `${m[1]}:${m[2].slice(0, 8)}`;
  m = s.match(/^sha256:([0-9a-f]{8,})$/i);
  if (m) return `sha256:${m[1].slice(0, 8)}`;
  if (/^[0-9a-f]{40}$/i.test(s)) return s.slice(0, 10);
  return s;
}

// Classify a token as a navigable Shoreline reference, a non-navigable hash,
// or a track lane. Returns null if it is not a recognized id.
function refInfo(token) {
  let m = token.match(/^([a-z][a-z-]*):(?:git:)?sha256:[0-9a-f]+$/i);
  if (m) return { kind: m[1].toLowerCase(), clickable: true };
  if (/^sha256:[0-9a-f]+$/i.test(token)) return { kind: "hash", clickable: false };
  if (/^[0-9a-f]{40}$/i.test(token)) return { kind: "commit", clickable: false };
  if (/^(agent|human):[a-z0-9][a-z0-9_-]*$/i.test(token)) return { kind: "track", clickable: true };
  return null;
}

const REF_RE =
  /\b(?:review-unit|input-request-response|input-request|obs|assess|snap|rev|evt|note):(?:git:)?sha256:[0-9a-f]{6,}\b|\bsha256:[0-9a-f]{16,}\b|\b[0-9a-f]{40}\b|\b(?:agent|human):[a-z0-9][a-z0-9_-]*\b/gi;

// Escape text, then replace embedded IDs with truncated reference chips.
// Navigable kinds carry data attributes that the delegated click handler
// resolves; hashes/commits render as truncated text with the full value on
// hover.
function linkify(text) {
  const escaped = escapeHtml(String(text ?? ""));
  return escaped.replace(REF_RE, (token) => {
    const info = refInfo(token);
    if (!info) return token;
    const display = escapeHtml(shortRef(token));
    if (!info.clickable) {
      return `<span class="ref ref-${info.kind}" title="${escapeHtml(token)}">${display}</span>`;
    }
    return `<span class="ref ref-${info.kind}" role="link" tabindex="0" data-ref-kind="${info.kind}" data-ref-id="${escapeHtml(token)}" title="${escapeHtml(token)}">${display}</span>`;
  });
}

// Render a single id as a reference chip (for fields that are exactly one id).
function refChip(id) {
  return id ? linkify(id) : "—";
}

function resolveRef(kind, id) {
  closeDiff();
  switch (kind) {
    case "review-unit":
      openUnit(id);
      break;
    case "track":
      navigateToTrack(id);
      break;
    case "snap": {
      const unit = (state.units?.entries || []).find((u) => u.snapshotId === id);
      openDiff(id, unit ? shortId(unit.reviewUnitId) : "");
      break;
    }
    case "obs":
      revealBy((e) => (e.summary || {}).observationId === id);
      break;
    case "assess":
      revealBy((e) => (e.summary || {}).assessmentId === id);
      break;
    case "input-request":
      revealBy((e) => e.eventType === "input_request_opened" && (e.summary || {}).inputRequestId === id);
      break;
    case "rev":
      revealBy((e) => e.revisionId === id);
      break;
    case "evt":
      revealEvent(id);
      break;
    default:
      break;
  }
}

function navigateToUnit(id) {
  state.filterUnit = id;
  $("#filter-unit").value = id;
  switchView("timeline");
  renderTimeline();
}

function navigateToTrack(id) {
  state.filterTrack = id;
  $("#filter-track").value = id;
  switchView("timeline");
  renderTimeline();
}

function revealBy(predicate) {
  const e = (state.history?.entries || []).find(predicate);
  if (e) revealEvent(e.eventId);
}

// Make an event visible (clearing filters that would hide it) and select it.
function revealEvent(eventId) {
  const e = (state.history?.entries || []).find((x) => x.eventId === eventId);
  if (!e) return;
  // Clear every filter that could hide the target row, including the track
  // filter (a cross-track chip, e.g. an assessment linking to another track's
  // observation, would otherwise select a row that stays hidden).
  state.filterText = "";
  state.filterUnit = "";
  state.filterTrack = "";
  $("#filter-text").value = "";
  $("#filter-unit").value = "";
  $("#filter-track").value = "";
  state.enabledTypes.add(e.eventType);
  state.selectedEventId = eventId;
  switchView("timeline");
  renderTypeToggles();
  renderTimeline();
  renderDetail();
  const row = $("#timeline").querySelector('.event[aria-selected="true"]');
  if (row) row.scrollIntoView({ block: "center" });
}

function parseMs(occurredAt) {
  if (typeof occurredAt !== "string") return null;
  const m = occurredAt.match(/(\d+)\s*$/);
  return m ? Number(m[1]) : null;
}
function fmtTime(occurredAt) {
  const ms = parseMs(occurredAt);
  if (ms == null) return occurredAt || "";
  const d = new Date(ms);
  return d.toLocaleTimeString([], { hour12: false }) + "." + String(ms % 1000).padStart(3, "0");
}
function fmtDateTime(occurredAt) {
  const ms = parseMs(occurredAt);
  if (ms == null) return occurredAt || "";
  return new Date(ms).toLocaleString([], { hour12: false });
}

// The typed, type-specific detail of an entry lives in the top-level `summary`
// object (title, body, assessment value, target, tags); `trackId` is also
// top-level. `subject` only carries the target ref, so we read from `summary`.
function entryTrack(e) {
  return e.trackId || (e.writer && e.writer.actorId) || "";
}
function entryTitle(e) {
  const s = e.summary || {};
  if (s.title) return s.title;
  if (s.assessment) return s.assessment;
  if (s.outcome) return s.outcome;
  if (s.reasonCode) return s.reasonCode;
  if (e.eventType === "review_unit_captured") {
    const base = (s.base && s.base.commitOid) || "";
    return base ? `capture · base ${shortId(base)}` : "capture";
  }
  return typeLabel(e.eventType);
}
function entryTags(e) {
  const s = e.summary || {};
  return Array.isArray(s.tags) ? s.tags : [];
}
function entryAnchor(e) {
  const t = (e.summary || {}).target || {};
  if (!t.filePath) return "";
  if (t.startLine) return `${t.filePath}:${t.startLine}-${t.endLine || t.startLine}`;
  return t.filePath;
}

async function fetchJSON(path) {
  const res = await fetch(path, { cache: "no-store" });
  const text = await res.text();
  let data;
  try {
    data = JSON.parse(text);
  } catch (_) {
    throw new Error(`${path}: non-JSON response (${res.status})`);
  }
  if (!res.ok || (data && data.error)) {
    throw new Error((data && data.error) || `${path}: HTTP ${res.status}`);
  }
  return data;
}

function showError(message) {
  const el = $("#error");
  if (!message) {
    el.classList.add("hidden");
    el.textContent = "";
    return;
  }
  el.textContent = "error: " + message;
  el.classList.remove("hidden");
}

async function load() {
  try {
    const [history, units] = await Promise.all([fetchJSON("/api/history"), fetchJSON("/api/units")]);
    state.history = history;
    state.units = units;
    state.lastHash = history.eventSetHash;
    showError(null);
    renderAll();
  } catch (err) {
    showError(err.message);
  }
}

async function pollFreshness() {
  try {
    const f = await fetchJSON("/api/freshness");
    const refresh = $("#refresh");
    if (f.eventSetHash !== state.lastHash) {
      refresh.textContent = "updated";
      refresh.classList.add("live");
      await load();
      setTimeout(() => {
        refresh.textContent = "watching";
        refresh.classList.remove("live");
      }, 1200);
    } else {
      refresh.textContent = "watching";
    }
  } catch (_) {
    $("#refresh").textContent = "stalled";
  }
}

function renderAll() {
  renderStats();
  renderDiagnostics();
  renderTypeToggles();
  renderTrackOptions();
  renderUnitOptions();
  renderTimeline();
  renderUnits();
  renderDetail();
}

function renderStats() {
  const h = state.history || {};
  const u = state.units || {};
  $("#stat-events").textContent = `${h.eventCount ?? "—"} events`;
  $("#stat-units").textContent = `${u.reviewUnitCount ?? "—"} units`;
  $("#stat-hash").textContent = shortId(h.eventSetHash);
}

function renderDiagnostics() {
  const el = $("#diagnostics");
  const diags = (state.history && state.history.diagnostics) || [];
  if (!diags.length) {
    el.classList.add("hidden");
    el.innerHTML = "";
    return;
  }
  el.classList.remove("hidden");
  el.innerHTML = diags
    .map((d) => `<div><span class="code">${escapeHtml(d.code || "diagnostic")}</span>${escapeHtml(d.message || "")}</div>`)
    .join("");
}

function presentTypes() {
  const present = new Set((state.history?.entries || []).map((e) => e.eventType));
  const ordered = TYPES.map((t) => t.id).filter((id) => present.has(id));
  for (const id of present) if (!TYPE_MAP[id]) ordered.push(id);
  return ordered;
}

function renderTypeToggles() {
  const container = $("#filter-types");
  container.innerHTML = "";
  for (const id of presentTypes()) {
    // Default a newly-seen type (e.g. an unknown event type) to enabled once;
    // after that the user's toggle sticks instead of being re-enabled here.
    if (!state.seenTypes.has(id)) {
      state.seenTypes.add(id);
      state.enabledTypes.add(id);
    }
    const btn = document.createElement("button");
    btn.className = "type-toggle" + (state.enabledTypes.has(id) ? "" : " off");
    btn.innerHTML = `<span class="dot" style="background:${typeColor(id)}"></span>${escapeHtml(typeLabel(id))}`;
    btn.title = id;
    btn.addEventListener("click", () => {
      if (state.enabledTypes.has(id)) state.enabledTypes.delete(id);
      else state.enabledTypes.add(id);
      renderTypeToggles();
      renderTimeline();
    });
    container.appendChild(btn);
  }
}

function fillSelect(select, values, current) {
  const keep = current && values.includes(current) ? current : "";
  select.querySelectorAll("option:not(:first-child)").forEach((o) => o.remove());
  for (const v of values) {
    const opt = document.createElement("option");
    opt.value = v;
    opt.textContent = v.length > 40 ? v.slice(0, 18) + "…" + v.slice(-12) : v;
    select.appendChild(opt);
  }
  select.value = keep;
  return keep;
}

function renderTrackOptions() {
  const tracks = [...new Set((state.history?.entries || []).map(entryTrack).filter(Boolean))].sort();
  state.filterTrack = fillSelect($("#filter-track"), tracks, state.filterTrack);
}

function renderUnitOptions() {
  const units = [...new Set((state.history?.entries || []).map((e) => e.reviewUnitId).filter(Boolean))].sort();
  state.filterUnit = fillSelect($("#filter-unit"), units, state.filterUnit);
}

function matchesFilters(e) {
  if (!state.enabledTypes.has(e.eventType)) return false;
  if (state.filterTrack && entryTrack(e) !== state.filterTrack) return false;
  if (state.filterUnit && e.reviewUnitId !== state.filterUnit) return false;
  if (state.filterText) {
    const hay = JSON.stringify(e).toLowerCase();
    if (!hay.includes(state.filterText.toLowerCase())) return false;
  }
  return true;
}

function renderTimeline() {
  const list = $("#timeline");
  list.innerHTML = "";
  // Server returns entries oldest->newest (occurredAt asc); default display is
  // newest-first, with a toolbar toggle back to chronological.
  let entries = (state.history?.entries || []).filter(matchesFilters);
  if (state.order === "desc") entries = entries.slice().reverse();
  if (!entries.length) {
    const li = document.createElement("li");
    li.className = "event";
    li.innerHTML = `<span></span><span></span><span class="body"><span class="title" style="color:var(--fg-dim)">no events match the current filters</span></span>`;
    list.appendChild(li);
    return;
  }
  for (const e of entries) {
    const li = document.createElement("li");
    li.className = "event";
    li.dataset.eventId = e.eventId;
    if (e.eventId === state.selectedEventId) li.setAttribute("aria-selected", "true");
    const tags = entryTags(e)
      .map((t) => `<span class="badge">${escapeHtml(t)}</span>`)
      .join(" ");
    li.innerHTML = `
      <span class="time">${escapeHtml(fmtTime(e.occurredAt))}</span>
      <span class="rail" style="background:${typeColor(e.eventType)}"></span>
      <span class="body">
        <span class="title">${linkify(entryTitle(e))} ${tags}</span>
        <span class="meta">
          <span class="type" style="color:${typeColor(e.eventType)}">${escapeHtml(typeLabel(e.eventType))}</span>
          ${entryTrack(e) ? `<span>${escapeHtml(entryTrack(e))}</span>` : ""}
          ${e.reviewUnitId ? `<span>unit ${escapeHtml(shortId(e.reviewUnitId))}</span>` : ""}
          ${entryAnchor(e) ? `<span>${escapeHtml(entryAnchor(e))}</span>` : ""}
        </span>
      </span>`;
    li.addEventListener("click", (ev) => {
      if (ev.target.closest("[data-ref-kind]")) return; // let the ref handler navigate
      state.selectedEventId = e.eventId;
      list.querySelectorAll(".event[aria-selected]").forEach((n) => n.removeAttribute("aria-selected"));
      li.setAttribute("aria-selected", "true");
      renderDetail();
    });
    list.appendChild(li);
  }
}

function renderDetail() {
  const el = $("#detail");
  const entries = state.history?.entries || [];
  const e = entries.find((x) => x.eventId === state.selectedEventId);
  if (!e) {
    el.innerHTML = `<p class="empty">Select an event to inspect its full payload.</p>`;
    return;
  }
  const kv = [
    ["type", typeLabel(e.eventType) + ` (${e.eventType})`],
    ["occurredAt", fmtDateTime(e.occurredAt)],
    ["eventId", e.eventId],
    ["payloadHash", e.payloadHash],
    ["reviewUnit", e.reviewUnitId || "—"],
    ["track", entryTrack(e) || "—"],
    ["writer", e.writer ? `${e.writer.actorId || ""} ${e.writer.role ? "(" + e.writer.role + ")" : ""}` : "—"],
  ];
  const snapshotId = e.reviewUnitId ? snapshotIdForUnit(e.reviewUnitId) : null;
  const s = e.summary || {};
  let focusId = null;
  let focusNoun = "";
  if (e.eventType === "review_observation_recorded") {
    focusId = s.observationId;
    focusNoun = "observation";
  } else if (e.eventType === "review_assessment_recorded") {
    focusId = s.assessmentId;
    focusNoun = "assessment";
  } else if (e.eventType === "input_request_opened") {
    focusId = s.inputRequestId;
    focusNoun = "input request";
  }
  const btnLabel = focusId ? `show this ${focusNoun} in the diff` : "view snapshot diff";
  el.innerHTML = `
    <h2>${linkify(entryTitle(e))}</h2>
    <dl class="kv">${kv.map(([k, v]) => `<dt>${escapeHtml(k)}</dt><dd>${linkify(String(v))}</dd>`).join("")}</dl>
    ${snapshotId ? `<button class="ghost diff-btn" id="detail-diff-btn">${escapeHtml(btnLabel)}</button>` : ""}
    <pre>${escapeHtml(JSON.stringify(e, null, 2))}</pre>`;
  if (snapshotId) {
    const btn = el.querySelector("#detail-diff-btn");
    if (btn) btn.addEventListener("click", () => openDiff(snapshotId, shortId(e.reviewUnitId), focusId));
  }
}

function snapshotIdForUnit(reviewUnitId) {
  const unit = (state.units?.entries || []).find((u) => u.reviewUnitId === reviewUnitId);
  return unit ? unit.snapshotId : null;
}

// Gather the review facts on a ReviewUnit — observations, input requests, and
// assessments — into one annotation list with a shared shape.
function annotationsForUnit(reviewUnitId) {
  const out = [];
  for (const e of state.history?.entries || []) {
    if (e.reviewUnitId !== reviewUnitId) continue;
    const s = e.summary || {};
    if (e.eventType === "review_observation_recorded") {
      out.push({
        kind: "observation",
        id: s.observationId || e.eventId,
        title: s.title || "(observation)",
        body: s.body || "",
        track: e.trackId || "",
        tags: Array.isArray(s.tags) ? s.tags : [],
        target: s.target || {},
      });
    } else if (e.eventType === "input_request_opened") {
      const meta = [s.mode, s.reasonCode].filter(Boolean).join(" · ");
      out.push({
        kind: "input-request",
        id: s.inputRequestId || e.eventId,
        title: s.title || "(input request)",
        body: s.body || "",
        track: e.trackId || "",
        tags: meta ? [meta] : [],
        target: s.target || {},
      });
    } else if (e.eventType === "review_assessment_recorded") {
      out.push({
        kind: "assessment",
        id: s.assessmentId || e.eventId,
        title: `assessment: ${s.assessment || "?"}`,
        body: s.summary || "",
        track: e.trackId || "",
        tags: [],
        target: s.target || {},
      });
    }
  }
  return out;
}

async function openDiff(snapshotId, label, focusId) {
  const modal = $("#diff-modal");
  $("#diff-title").textContent = label ? `${label} · snapshot ${shortId(snapshotId)}` : shortId(snapshotId);
  $("#diff-body").innerHTML = `<p class="empty">loading snapshot…</p>`;
  modal.classList.remove("hidden");
  try {
    const artifact = await fetchJSON("/api/snapshot?id=" + encodeURIComponent(snapshotId));
    const annotations = annotationsForUnit(artifact.reviewUnitId);
    $("#diff-body").innerHTML = renderDiff(artifact, annotations);
    if (focusId) {
      const target = $("#diff-body").querySelector(`[data-anno="${focusId}"]`);
      if (target) {
        target.scrollIntoView({ block: "center" });
        target.classList.add("anno-flash");
      }
    }
  } catch (err) {
    $("#diff-body").innerHTML = `<p class="empty">error: ${escapeHtml(err.message)}</p>`;
  }
}

function closeDiff() {
  $("#diff-modal").classList.add("hidden");
}

function lineMatch(fact, row) {
  const t = fact.target || {};
  if (t.kind !== "range" || t.startLine == null) return false;
  const line = t.side === "old" ? row.old_line : row.new_line;
  return line != null && line >= t.startLine && line <= (t.endLine ?? t.startLine);
}

function renderAnnotation(a, showLocation) {
  const tags = (a.tags || []).map((t) => `<span class="badge">${escapeHtml(t)}</span>`).join(" ");
  const body = a.body ? `<div class="anno-body">${linkify(a.body)}</div>` : "";
  const t = a.target || {};
  const loc =
    showLocation && t.filePath
      ? `<span class="anno-loc">${escapeHtml(t.filePath)}${t.startLine ? `:${t.startLine}-${t.endLine || t.startLine}` : ""}</span>`
      : "";
  return `<div class="anno anno-${a.kind}" data-anno="${escapeHtml(a.id)}">
    <div class="anno-head"><span class="anno-kind anno-kind-${a.kind}">${a.kind}</span><span class="anno-track">${escapeHtml(a.track)}</span><span class="anno-title">${linkify(a.title)}</span> ${tags} ${loc}</div>${body}</div>`;
}

function renderDiff(artifact, annotations) {
  annotations = annotations || [];
  const files = (artifact.snapshot && artifact.snapshot.files) || [];
  const filePaths = new Set();
  for (const f of files) {
    if (f.new_path) filePaths.add(f.new_path);
    if (f.old_path) filePaths.add(f.old_path);
  }
  const anchored = [];
  const unanchored = [];
  for (const a of annotations) {
    const t = a.target || {};
    if ((t.kind === "range" || t.kind === "file") && t.filePath && filePaths.has(t.filePath)) anchored.push(a);
    else unanchored.push(a);
  }

  const counts = annotations.reduce((acc, a) => ((acc[a.kind] = (acc[a.kind] || 0) + 1), acc), {});
  const breakdown = Object.entries(counts)
    .map(([k, n]) => `${n} ${k}${n === 1 ? "" : "s"}`)
    .join(", ");
  let html = `<div class="anno-summary">${annotations.length} review fact${annotations.length === 1 ? "" : "s"} on this ReviewUnit${
    breakdown ? ` · ${breakdown}` : ""
  }${unanchored.length ? ` · ${unanchored.length} not anchored to a diff line` : ""}</div>`;
  if (unanchored.length) {
    html += `<div class="anno-group">${unanchored.map((a) => renderAnnotation(a, true)).join("")}</div>`;
  }
  if (!files.length) return html + `<p class="empty">No files captured in this snapshot.</p>`;

  const emitted = new Set();
  html += files.map((f) => renderDiffFile(f, anchored, emitted)).join("");
  return html;
}

function renderDiffFile(f, anchored, emitted) {
  const oldp = f.old_path;
  const newp = f.new_path;
  const path = oldp && newp && oldp !== newp ? `${oldp} → ${newp}` : newp || oldp || "(unknown path)";
  const fileFacts = anchored.filter((a) => {
    const p = (a.target || {}).filePath;
    return p === newp || p === oldp;
  });
  const rangeFacts = fileFacts.filter((a) => (a.target || {}).kind === "range");
  const fileLevelFacts = fileFacts.filter((a) => (a.target || {}).kind === "file");

  let html = `<section class="dfile"><header class="dfile-head">
    <span class="dstatus s-${escapeHtml(f.status)}">${escapeHtml(f.status)}</span>
    <span class="dpath">${escapeHtml(path)}</span>
    ${fileFacts.length ? `<span class="dfile-notes">${fileFacts.length} note${fileFacts.length === 1 ? "" : "s"}</span>` : ""}</header>`;

  for (const a of fileLevelFacts) {
    html += renderAnnotation(a, false);
    emitted.add(a.id);
  }
  for (const m of f.metadata_rows || []) {
    html += `<div class="drow drow-meta"><span class="dtext">${escapeHtml(m.text)}</span></div>`;
  }

  const hunks = f.hunks || [];
  for (const h of hunks) {
    html += `<div class="dhunk">${escapeHtml(h.header)}</div>`;
    for (const r of h.rows || []) {
      const matching = rangeFacts.filter((a) => lineMatch(a, r));
      const sign = r.kind === "added" ? "+" : r.kind === "removed" ? "-" : " ";
      html += `<div class="drow drow-${escapeHtml(r.kind)}${matching.length ? " drow-noted" : ""}">
        <span class="ln">${r.old_line ?? ""}</span>
        <span class="ln">${r.new_line ?? ""}</span>
        <span class="sign">${sign}</span>
        <span class="dtext">${escapeHtml(r.text)}</span></div>`;
      for (const a of matching) {
        if (!emitted.has(a.id)) {
          html += renderAnnotation(a, false);
          emitted.add(a.id);
        }
      }
    }
  }

  // Range facts whose anchor line was not a captured row: surface them anyway
  // so no review fact is silently dropped from the view.
  for (const a of rangeFacts) {
    if (!emitted.has(a.id)) {
      html += renderAnnotation(a, true);
      emitted.add(a.id);
    }
  }
  if (!hunks.length && !(f.metadata_rows || []).length) {
    const why = f.is_binary ? "binary" : f.is_mode_only ? "mode change only" : "no captured content";
    html += `<div class="drow drow-meta"><span class="dtext">(${why})</span></div>`;
  }
  return html + `</section>`;
}

function renderUnits() {
  const el = $("#units");
  const entries = state.units?.entries || [];
  if (!entries.length) {
    el.innerHTML = `<p class="empty" style="color:var(--fg-dim)">No captured ReviewUnits in this store.</p>`;
    return;
  }
  el.innerHTML = "";
  for (const u of entries) {
    const base = u.base || {};
    const target = u.target || {};
    const card = document.createElement("div");
    card.className = "unit-card";
    const rows = [
      ["captured", fmtDateTime(u.capturedAt)],
      ["base", base.commitOid ? shortId(base.commitOid) + " (" + (base.kind || "") + ")" : base.kind || "—"],
      ["target", target.kind === "git_working_tree" ? "working tree" : target.kind || "—"],
      ["snapshot", shortId(u.snapshotId)],
      ["session", shortId(u.sessionId)],
    ];
    card.innerHTML = `
      <h3>${escapeHtml(shortId(u.reviewUnitId))}</h3>
      <div class="kv">${rows.map(([k, v]) => `<span>${escapeHtml(k)}</span><b>${escapeHtml(String(v))}</b>`).join("")}</div>`;
    card.title = u.reviewUnitId + "\nclick to open the unit page";
    card.addEventListener("click", () => openUnit(u.reviewUnitId));
    const actions = document.createElement("div");
    actions.className = "actions";
    const diffBtn = document.createElement("button");
    diffBtn.className = "ghost diff-btn";
    diffBtn.textContent = "view snapshot diff";
    diffBtn.addEventListener("click", (ev) => {
      ev.stopPropagation();
      openDiff(u.snapshotId, shortId(u.reviewUnitId));
    });
    actions.appendChild(diffBtn);
    card.appendChild(actions);
    el.appendChild(card);
  }
}

function switchView(view) {
  state.view = view;
  // The single-unit page is a drill-in under Units, so keep the Units tab lit.
  document.querySelectorAll(".tab").forEach((t) =>
    t.setAttribute(
      "aria-selected",
      String(t.dataset.view === view || (view === "unit" && t.dataset.view === "units")),
    ),
  );
  $("#view-timeline").classList.toggle("hidden", view !== "timeline");
  $("#view-units").classList.toggle("hidden", view !== "units");
  $("#view-unit").classList.toggle("hidden", view !== "unit");
}

async function openUnit(reviewUnitId) {
  switchView("unit");
  $("#unit-page-title").textContent = shortId(reviewUnitId);
  $("#unit-page").innerHTML = `<p class="up-empty">loading…</p>`;
  try {
    const d = await fetchJSON("/api/unit?id=" + encodeURIComponent(reviewUnitId));
    renderUnitPage(d);
  } catch (err) {
    $("#unit-page").innerHTML = `<p class="up-empty">error: ${escapeHtml(err.message)}</p>`;
  }
}

function verdictBadge(ca) {
  const status = (ca && ca.status) || "unassessed";
  let value;
  let cls;
  if (status === "resolved") {
    value = ca.assessment;
    cls = `verdict-${ca.assessment}`;
  } else if (status === "ambiguous") {
    value = `ambiguous (${(ca.candidates || []).length} candidates)`;
    cls = "verdict-ambiguous";
  } else {
    value = "unassessed";
    cls = "verdict-unassessed";
  }
  return `<div class="verdict ${cls}"><span class="verdict-status">current assessment</span><span class="verdict-value">${escapeHtml(value)}</span></div>`;
}

function currentAssessmentSummary(d) {
  const ca = d.currentAssessment || {};
  if (ca.status === "resolved" && ca.assessmentId) {
    const a = (d.assessments || []).find((x) => x.id === ca.assessmentId);
    if (a && a.summary) return `<div class="verdict-summary">${linkify(a.summary)}</div>`;
  }
  if (ca.status === "ambiguous") {
    return `<div class="verdict-summary">${(ca.candidates || []).length} unreplaced assessments — see Assessments below.</div>`;
  }
  return "";
}

function targetLabel(t) {
  t = t || {};
  switch (t.kind) {
    case "range":
      return `${escapeHtml(t.filePath)}:${t.startLine}-${t.endLine ?? t.startLine} (${escapeHtml(t.side || "new")})`;
    case "file":
      return escapeHtml(t.filePath || "");
    case "review_unit":
      return "whole unit";
    case "observation":
      return `→ ${linkify(t.observationId)}`;
    case "input_request":
      return `→ ${linkify(t.inputRequestId)}`;
    case "assessment":
      return `→ ${linkify(t.assessmentId)}`;
    case "event":
      return `→ ${linkify(t.eventId)}`;
    default:
      return escapeHtml(t.kind || "");
  }
}

function factCard(kind, opts) {
  const tags = (opts.tags || []).filter(Boolean).map((t) => `<span class="badge">${escapeHtml(t)}</span>`).join(" ");
  const body = opts.body ? `<div class="anno-body">${linkify(opts.body)}</div>` : "";
  return `<div class="anno anno-${kind}">
    <div class="anno-head">
      <span class="anno-kind anno-kind-${kind}">${kind}</span>
      <span class="anno-track">${escapeHtml(opts.track || "")}</span>
      <span class="anno-title">${linkify(opts.title || "")}</span>
      ${opts.status ? `<span class="fact-status ${escapeHtml(opts.status)}">${escapeHtml(opts.status)}</span>` : ""}
      ${opts.target ? `<span class="anno-loc">${opts.target}</span>` : ""}
      ${tags}
    </div>
    ${body}
    ${opts.extra || ""}</div>`;
}

function renderObservationCard(o) {
  const extra = (o.supersedes || []).length
    ? `<div class="fact-rel">supersedes ${o.supersedes.map(linkify).join(", ")}</div>`
    : "";
  return factCard("observation", {
    track: o.trackId,
    title: o.title,
    status: o.status,
    target: targetLabel(o.target),
    tags: o.tags,
    body: o.body,
    extra,
  });
}

function renderInputRequestCard(ir) {
  const responses = (ir.responses || [])
    .map((r) => `<div class="fact-response"><span class="outcome">${escapeHtml(r.outcome)}</span>${r.reason ? `: ${linkify(r.reason)}` : ""}</div>`)
    .join("");
  return factCard("input-request", {
    track: ir.trackId,
    title: ir.title,
    status: ir.status,
    target: targetLabel(ir.target),
    tags: [ir.mode, ir.reasonCode],
    body: ir.body,
    extra: responses ? `<div class="fact-responses">${responses}</div>` : "",
  });
}

function renderAssessmentCard(a) {
  const rel = [];
  if ((a.replaces || []).length) rel.push(`replaces ${a.replaces.map(linkify).join(", ")}`);
  if ((a.relatedObservations || []).length) rel.push(`re ${a.relatedObservations.map(linkify).join(", ")}`);
  if ((a.relatedInputRequests || []).length) rel.push(`re ${a.relatedInputRequests.map(linkify).join(", ")}`);
  return factCard("assessment", {
    track: a.trackId,
    title: a.assessment,
    status: a.status,
    target: targetLabel(a.target),
    body: a.summary,
    extra: rel.length ? `<div class="fact-rel">${rel.join(" · ")}</div>` : "",
  });
}

function renderAdapterNoteCard(n) {
  return factCard("observation", {
    track: n.author || "imported",
    title: n.title,
    status: n.status,
    target: n.filePath ? escapeHtml(n.filePath) : "",
    body: n.body,
  });
}

function factSection(title, items, render) {
  items = items || [];
  const body = items.length ? items.map(render).join("") : `<p class="up-empty">none</p>`;
  return `<section><h2>${escapeHtml(title)} (${items.length})</h2>${body}</section>`;
}

function renderUnitPage(d) {
  const ru = d.reviewUnit || {};
  const base = ru.base || {};
  const target = ru.target || {};
  const s = d.summary || {};
  $("#unit-page-title").textContent = `${shortId(ru.id)}${base.commitOid ? " · base " + shortId(base.commitOid) : ""}`;

  const stat = (label, n) => `<span class="up-stat"><b>${n ?? 0}</b> ${label}</span>`;
  const sections = [];

  sections.push(`<section><h2>ReviewUnit</h2><dl class="up-identity">
    <dt>id</dt><dd>${linkify(ru.id)}</dd>
    <dt>base</dt><dd>${base.commitOid ? linkify(base.commitOid) : "—"} ${base.kind ? `<span class="fact-status">${escapeHtml(base.kind)}</span>` : ""}</dd>
    <dt>target</dt><dd>${target.kind === "git_working_tree" ? "working tree" : escapeHtml(target.kind || "—")}</dd>
    <dt>snapshot</dt><dd>${linkify(ru.snapshotId)}</dd>
  </dl></section>`);

  sections.push(`<section><h2>Current assessment</h2>${verdictBadge(d.currentAssessment)}${currentAssessmentSummary(d)}</section>`);

  sections.push(`<section><h2>Summary</h2><div class="up-stats">
    ${stat("files", s.fileCount)}${stat("rows", s.rowCount)}${stat("observations", s.observationCount)}${stat("input requests", s.inputRequestCount)}${stat("assessments", s.assessmentCount)}${stat("adapter notes", s.adapterNoteCount)}
  </div>
  <div style="margin-top:10px">
    <button class="ghost diff-btn" id="up-diff-btn">view annotated diff</button>
    <button class="ghost" id="up-timeline-btn" style="margin-left:6px">show in timeline</button>
  </div></section>`);

  sections.push(factSection("Observations", d.observations, renderObservationCard));
  sections.push(factSection("Input requests", d.inputRequests, renderInputRequestCard));
  sections.push(factSection("Assessments", d.assessments, renderAssessmentCard));
  if ((d.adapterNotes || []).length) sections.push(factSection("Adapter notes", d.adapterNotes, renderAdapterNoteCard));

  $("#unit-page").innerHTML = sections.join("");

  const diffBtn = $("#up-diff-btn");
  if (diffBtn && ru.snapshotId) diffBtn.addEventListener("click", () => openDiff(ru.snapshotId, shortId(ru.id)));
  const tlBtn = $("#up-timeline-btn");
  if (tlBtn) tlBtn.addEventListener("click", () => navigateToUnit(ru.id));
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

function wireControls() {
  document.querySelectorAll(".tab").forEach((tab) => tab.addEventListener("click", () => switchView(tab.dataset.view)));
  $("#filter-text").addEventListener("input", (ev) => {
    state.filterText = ev.target.value;
    renderTimeline();
  });
  $("#filter-track").addEventListener("change", (ev) => {
    state.filterTrack = ev.target.value;
    renderTimeline();
  });
  $("#filter-unit").addEventListener("change", (ev) => {
    state.filterUnit = ev.target.value;
    renderTimeline();
  });
  $("#filter-clear").addEventListener("click", () => {
    state.filterText = "";
    state.filterTrack = "";
    state.filterUnit = "";
    state.enabledTypes = new Set(presentTypes());
    $("#filter-text").value = "";
    $("#filter-track").value = "";
    $("#filter-unit").value = "";
    renderTypeToggles();
    renderTimeline();
  });
  $("#unit-back").addEventListener("click", () => switchView("units"));
  $("#order-toggle").addEventListener("click", () => {
    state.order = state.order === "desc" ? "asc" : "desc";
    $("#order-toggle").textContent = state.order === "desc" ? "newest first" : "oldest first";
    renderTimeline();
  });
  $("#diff-close").addEventListener("click", closeDiff);
  $("#diff-modal").addEventListener("click", (ev) => {
    if (ev.target === $("#diff-modal")) closeDiff();
  });
  document.addEventListener("keydown", (ev) => {
    if (ev.key === "Escape") closeDiff();
  });
  // Delegated handler: any reference chip anywhere (timeline, detail, diff)
  // navigates to the resource it names.
  document.addEventListener("click", (ev) => {
    const ref = ev.target.closest("[data-ref-kind]");
    if (!ref) return;
    ev.preventDefault();
    resolveRef(ref.dataset.refKind, ref.dataset.refId);
  });
}

wireControls();
load().then(() => {
  $("#refresh").textContent = "watching";
  setInterval(pollFreshness, 3000);
});
