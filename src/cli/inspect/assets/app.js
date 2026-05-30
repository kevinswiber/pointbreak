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
    if (!state.enabledTypes.has(id)) {
      // keep unknown/new types enabled by default
      if (!TYPE_MAP[id]) state.enabledTypes.add(id);
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
        <span class="title">${escapeHtml(entryTitle(e))} ${tags}</span>
        <span class="meta">
          <span class="type" style="color:${typeColor(e.eventType)}">${escapeHtml(typeLabel(e.eventType))}</span>
          ${entryTrack(e) ? `<span>${escapeHtml(entryTrack(e))}</span>` : ""}
          ${e.reviewUnitId ? `<span>unit ${escapeHtml(shortId(e.reviewUnitId))}</span>` : ""}
          ${entryAnchor(e) ? `<span>${escapeHtml(entryAnchor(e))}</span>` : ""}
        </span>
      </span>`;
    li.addEventListener("click", () => {
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
  const focusObs = e.eventType === "review_observation_recorded" ? (e.summary || {}).observationId : null;
  const btnLabel = focusObs ? "show this observation in the diff" : "view snapshot diff";
  el.innerHTML = `
    <h2>${escapeHtml(entryTitle(e))}</h2>
    <dl class="kv">${kv.map(([k, v]) => `<dt>${escapeHtml(k)}</dt><dd>${escapeHtml(String(v))}</dd>`).join("")}</dl>
    ${snapshotId ? `<button class="ghost diff-btn" id="detail-diff-btn">${escapeHtml(btnLabel)}</button>` : ""}
    <pre>${escapeHtml(JSON.stringify(e, null, 2))}</pre>`;
  if (snapshotId) {
    const btn = el.querySelector("#detail-diff-btn");
    if (btn) btn.addEventListener("click", () => openDiff(snapshotId, shortId(e.reviewUnitId), focusObs));
  }
}

function snapshotIdForUnit(reviewUnitId) {
  const unit = (state.units?.entries || []).find((u) => u.reviewUnitId === reviewUnitId);
  return unit ? unit.snapshotId : null;
}

function observationsForUnit(reviewUnitId) {
  return (state.history?.entries || [])
    .filter((e) => e.eventType === "review_observation_recorded" && e.reviewUnitId === reviewUnitId)
    .map((e) => {
      const s = e.summary || {};
      return {
        id: s.observationId || e.eventId,
        title: s.title || "(observation)",
        body: s.body || "",
        track: e.trackId || "",
        tags: Array.isArray(s.tags) ? s.tags : [],
        target: s.target || {},
      };
    });
}

async function openDiff(snapshotId, label, focusObsId) {
  const modal = $("#diff-modal");
  $("#diff-title").textContent = label ? `${label} · snapshot ${shortId(snapshotId)}` : shortId(snapshotId);
  $("#diff-body").innerHTML = `<p class="empty">loading snapshot…</p>`;
  modal.classList.remove("hidden");
  try {
    const artifact = await fetchJSON("/api/snapshot?id=" + encodeURIComponent(snapshotId));
    const observations = observationsForUnit(artifact.reviewUnitId);
    $("#diff-body").innerHTML = renderDiff(artifact, observations);
    if (focusObsId) {
      const target = $("#diff-body").querySelector(`[data-obs="${focusObsId}"]`);
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

function lineMatch(observation, row) {
  const t = observation.target || {};
  if (t.kind !== "range" || t.startLine == null) return false;
  const line = t.side === "old" ? row.old_line : row.new_line;
  return line != null && line >= t.startLine && line <= (t.endLine ?? t.startLine);
}

function renderObsAnnotation(observation, showLocation) {
  const tags = (observation.tags || []).map((t) => `<span class="badge">${escapeHtml(t)}</span>`).join(" ");
  const body = observation.body ? `<div class="anno-body">${escapeHtml(observation.body)}</div>` : "";
  const t = observation.target || {};
  const loc =
    showLocation && t.filePath
      ? `<span class="anno-loc">${escapeHtml(t.filePath)}${t.startLine ? `:${t.startLine}-${t.endLine || t.startLine}` : ""}</span>`
      : "";
  return `<div class="anno" data-obs="${escapeHtml(observation.id)}">
    <div class="anno-head"><span class="anno-track">${escapeHtml(observation.track)}</span><span class="anno-title">${escapeHtml(observation.title)}</span> ${tags} ${loc}</div>${body}</div>`;
}

function renderDiff(artifact, observations) {
  observations = observations || [];
  const files = (artifact.snapshot && artifact.snapshot.files) || [];
  const filePaths = new Set();
  for (const f of files) {
    if (f.new_path) filePaths.add(f.new_path);
    if (f.old_path) filePaths.add(f.old_path);
  }
  const anchored = [];
  const unanchored = [];
  for (const o of observations) {
    const t = o.target || {};
    if ((t.kind === "range" || t.kind === "file") && t.filePath && filePaths.has(t.filePath)) anchored.push(o);
    else unanchored.push(o);
  }

  const count = observations.length;
  let html = `<div class="anno-summary">${count} observation${count === 1 ? "" : "s"} on this ReviewUnit${
    unanchored.length ? ` · ${unanchored.length} not anchored to a diff line` : ""
  }</div>`;
  if (unanchored.length) {
    html += `<div class="anno-group">${unanchored.map((o) => renderObsAnnotation(o, true)).join("")}</div>`;
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
  const fileObs = anchored.filter((o) => {
    const p = (o.target || {}).filePath;
    return p === newp || p === oldp;
  });
  const rangeObs = fileObs.filter((o) => (o.target || {}).kind === "range");
  const fileLevelObs = fileObs.filter((o) => (o.target || {}).kind === "file");

  let html = `<section class="dfile"><header class="dfile-head">
    <span class="dstatus s-${escapeHtml(f.status)}">${escapeHtml(f.status)}</span>
    <span class="dpath">${escapeHtml(path)}</span>
    ${fileObs.length ? `<span class="dfile-notes">${fileObs.length} note${fileObs.length === 1 ? "" : "s"}</span>` : ""}</header>`;

  for (const o of fileLevelObs) {
    html += renderObsAnnotation(o, false);
    emitted.add(o.id);
  }
  for (const m of f.metadata_rows || []) {
    html += `<div class="drow drow-meta"><span class="dtext">${escapeHtml(m.text)}</span></div>`;
  }

  const hunks = f.hunks || [];
  for (const h of hunks) {
    html += `<div class="dhunk">${escapeHtml(h.header)}</div>`;
    for (const r of h.rows || []) {
      const matching = rangeObs.filter((o) => lineMatch(o, r));
      const sign = r.kind === "added" ? "+" : r.kind === "removed" ? "-" : " ";
      html += `<div class="drow drow-${escapeHtml(r.kind)}${matching.length ? " drow-noted" : ""}">
        <span class="ln">${r.old_line ?? ""}</span>
        <span class="ln">${r.new_line ?? ""}</span>
        <span class="sign">${sign}</span>
        <span class="dtext">${escapeHtml(r.text)}</span></div>`;
      for (const o of matching) {
        if (!emitted.has(o.id)) {
          html += renderObsAnnotation(o, false);
          emitted.add(o.id);
        }
      }
    }
  }

  // Range observations whose anchor line was not a captured row: surface them
  // anyway so no review fact is silently dropped from the view.
  for (const o of rangeObs) {
    if (!emitted.has(o.id)) {
      html += renderObsAnnotation(o, true);
      emitted.add(o.id);
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
    card.title = u.reviewUnitId + "\nclick to filter the timeline to this unit";
    card.addEventListener("click", () => {
      state.filterUnit = u.reviewUnitId;
      $("#filter-unit").value = u.reviewUnitId;
      switchView("timeline");
      renderTimeline();
    });
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
  document.querySelectorAll(".tab").forEach((t) => t.setAttribute("aria-selected", String(t.dataset.view === view)));
  $("#view-timeline").classList.toggle("hidden", view !== "timeline");
  $("#view-units").classList.toggle("hidden", view !== "units");
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
}

wireControls();
load().then(() => {
  $("#refresh").textContent = "watching";
  setInterval(pollFreshness, 3000);
});
