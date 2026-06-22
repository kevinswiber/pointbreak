"use strict";

// Known durable event types, with display labels and timeline colors.
const TYPES = [
  { id: "review_initialized", label: "init", color: "#7f8c9b" },
  { id: "work_object_proposed", label: "capture", color: "#5aa9e6" },
  { id: "review_observation_recorded", label: "observation", color: "#6dd28a" },
  { id: "review_assessment_recorded", label: "assessment", color: "#b388ff" },
  { id: "input_request_opened", label: "request", color: "#f0b75a" },
  { id: "input_request_responded", label: "response", color: "#4fd0c0" },
  { id: "review_note_imported", label: "note", color: "#9aa7b5" },
  { id: "validation_check_recorded", label: "validation", color: "#e88fb0" },
];
const TYPE_MAP = Object.fromEntries(TYPES.map((t) => [t.id, t]));

const state = {
  history: null,
  units: null,
  objects: null,
  view: "timeline",
  enabledTypes: new Set(TYPES.map((t) => t.id)),
  seenTypes: new Set(TYPES.map((t) => t.id)),
  filterText: "",
  filterTrack: "",
  filterUnit: "",
  filterObject: "",
  order: "desc", // "desc" = newest first (default), "asc" = chronological
  selectedEventId: null,
  lastHash: null,
  lastDiagnosticCount: null,
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

// Path-private target label from the server-derived `targetDisplay`. Floors to
// "working tree" for pre-upgrade payloads that lack the block. Returns escaped
// HTML, so callers must not re-escape it. (Distinct from `targetLabel` below,
// which renders ReviewTargetRef kinds for note/observation anchors.)
function targetDisplayLabel(td) {
  if (!td) return "working tree"; // floor fallback (pre-upgrade payloads)
  return escapeHtml(td.label || "working tree");
}

// Ready-to-insert head badge (escaped, safe HTML) for the captured base commit,
// or "" when no head is available. A live branch, if ever present, is shown as
// a current/live qualifier — never as capture-time provenance.
function targetHeadBadge(td) {
  const head = td && td.head;
  if (!head || !head.label) return "";
  let inner = "@ " + escapeHtml(head.label);
  if (head.liveBranch) inner += " · " + escapeHtml(head.liveBranch) + " (current)";
  return ` <span class="badge">${inner}</span>`;
}

// Classify a token as a navigable Shoreline reference, a non-navigable hash,
// or a track lane. Returns null if it is not a recognized id.
function refInfo(token) {
  // Validation check ids have no resolver, so they render as a non-clickable
  // chip rather than dead navigation. Classify before the generic match.
  if (/^validation:(?:git:)?sha256:[0-9a-f]+$/i.test(token)) {
    return { kind: "validation", clickable: false };
  }
  let m = token.match(/^([a-z][a-z-]*):(?:git:)?sha256:[0-9a-f]+$/i);
  if (m) return { kind: m[1].toLowerCase(), clickable: true };
  if (/^sha256:[0-9a-f]+$/i.test(token)) return { kind: "hash", clickable: false };
  if (/^[0-9a-f]{40}$/i.test(token)) return { kind: "commit", clickable: false };
  if (/^(agent|human):[a-z0-9][a-z0-9_-]*$/i.test(token)) return { kind: "track", clickable: true };
  return null;
}

const REF_RE =
  /\b(?:review-unit|input-request-response|input-request|obs|assess|snap|rev|evt|note|validation):(?:git:)?sha256:[0-9a-f]{6,}\b|\bsha256:[0-9a-f]{16,}\b|\b[0-9a-f]{40}\b|\b(?:agent|human):[a-z0-9][a-z0-9_-]*\b/gi;

// Review facts whose currency depends on the revision they target: a fact on a
// superseded revision is stale (named by all superseding successors).
const SUPERSEDABLE_FACT_TYPES = new Set(["review_observation_recorded", "review_assessment_recorded", "input_request_opened", "validation_check_recorded"]);

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

function resolveRef(kind, id) {
  closeDiff();
  switch (kind) {
    // The revision and the (retired) review-unit prefix both address a revision's
    // composite — its identity is unified onto RevisionId.
    case "rev":
    case "review-unit":
      openUnit(id);
      break;
    case "track":
      navigateToTrack(id);
      break;
    case "snap": {
      const unit = (state.units?.entries || []).find((u) => u.objectId === id);
      openDiff(id, unit ? shortId(unit.revisionId) : "", unit?.revisionId);
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
    case "evt":
      revealEvent(id);
      break;
    default:
      break;
  }
}

function navigateToUnit(id) {
  // Clear text/track filters so the unit's events actually show — a stale
  // no-match text or track filter would otherwise leave an empty timeline.
  state.filterText = "";
  state.filterTrack = "";
  state.filterObject = "";
  state.filterUnit = id;
  $("#filter-text").value = "";
  $("#filter-track").value = "";
  $("#filter-object").value = "";
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
  state.filterObject = "";
  $("#filter-text").value = "";
  $("#filter-unit").value = "";
  $("#filter-track").value = "";
  $("#filter-object").value = "";
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
// The revision a history entry addresses. The entry carries it through its
// subject (the ReviewTargetRef) — every review subject variant keys on
// revisionId — so there is no top-level id to read.
function entryRevisionId(e) {
  return (e.subject && e.subject.revisionId) || "";
}
// The human label derived client-side from the structured principal object
// (ADR-0010 structured-first rule). Null unless the agent's principal resolved;
// ambiguous/none entries fall back to the raw actor id at the call site. The
// lane fallback in entryTrack deliberately never reads e.principal — lanes need
// stable strings.
function principalLabel(e) {
  if (!e.principal || e.principal.status !== "resolved" || !e.principal.actorId) return null;
  const agent = (e.writer && e.writer.actorId ? e.writer.actorId : "").replace(/^actor:agent:/, "");
  const principal = e.principal.actorId.replace(/^actor:git-(email|name):/, "");
  return `${agent} (for ${principal})`;
}

// Reader-relative, advisory signature/endorsement readback (#171). Render-only:
// these never gate a write or change a verdict, and the same carrier may read
// differently for another reader (whoever the inspector's host enrolled). Labels
// mirror docs/cli-reference.md "Verification status and endorsement readback".
const VERIFICATION_LABELS = {
  valid: "signature valid",
  invalid: "signature invalid",
  untrusted_key: "untrusted key",
  unsigned: "unsigned",
};

function verificationChip(status) {
  if (!status) return "";
  const label = VERIFICATION_LABELS[status] || status;
  return `<span class="verify verify-${escapeHtml(status)}" title="advisory signature readback — reader-relative, never gates a write">${escapeHtml(label)}</span>`;
}

const ENDORSEMENT_LABELS = {
  "endorsement-trusted": "trusted endorsement",
  unknown_endorser: "unknown endorser",
  ambiguous_endorser: "ambiguous endorser",
};

// Strip the actor namespace for display, matching principalLabel's posture.
function endorserDisplay(actorId) {
  return actorId.replace(/^actor:git-(email|name):/, "");
}

function endorsementRow(en) {
  const cls = en.classification || "";
  const label = ENDORSEMENT_LABELS[cls] || cls;
  const parts = [`<span class="endorse-label">${escapeHtml(label)}</span>`];
  if (en.endorser) parts.push(`<span class="endorse-who">${escapeHtml(endorserDisplay(en.endorser))}</span>`);
  const attrs = en.endorserAttributes || {};
  const attrBits = [];
  if (attrs.kind) attrBits.push(attrs.kind);
  if ((attrs.roles || []).length) attrBits.push(attrs.roles.join(", "));
  if (attrBits.length) parts.push(`<span class="endorse-attrs">${escapeHtml(attrBits.join(" · "))}</span>`);
  return `<li class="endorse endorse-${escapeHtml(cls)}">${parts.join(" ")}</li>`;
}

// Advisory, reader-relative endorsement readback (#171). One row per attestation
// (one per endorsing signer/key) — never collapsed, mirroring the API.
function endorsementsBlock(endorsements) {
  endorsements = endorsements || [];
  if (!endorsements.length) return "";
  const rows = endorsements.map(endorsementRow).join("");
  return `<div class="endorsements" title="advisory endorsement readback — reader-relative, never gates a write">
    <span class="endorsements-label">endorsements</span>
    <ul class="endorse-list">${rows}</ul>
  </div>`;
}

function entryTitle(e) {
  const s = e.summary || {};
  if (s.title) return s.title;
  if (s.assessment) return s.assessment;
  if (s.outcome) return s.outcome;
  if (s.reasonCode) return s.reasonCode;
  if (e.eventType === "work_object_proposed") {
    const base = (s.base && s.base.commitOid) || "";
    return base ? `capture · base ${shortId(base)}` : "capture";
  }
  if (e.eventType === "validation_check_recorded") {
    const name = s.checkName || "validation";
    return s.status ? `${name} · ${s.status}` : name;
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
    const [history, units, objects] = await Promise.all([
      fetchJSON("/api/history"),
      fetchJSON("/api/units"),
      fetchJSON("/api/objects"),
    ]);
    state.history = history;
    state.units = units;
    state.objects = objects;
    state.lastHash = history.eventSetHash;
    // Seed the diagnostic count alongside the hash so the poller can detect a
    // divergence appearing/clearing without a new event (#142). The history
    // payload carries the same diagnostics set the freshness probe counts.
    state.lastDiagnosticCount = (history.diagnostics || []).length;
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
    // Reload when the event set OR the diagnostic count changed. A diagnostic
    // can appear or clear without a new event, so compare the count too, since
    // either source can drive a refresh.
    const hashChanged = f.eventSetHash !== state.lastHash;
    const diagChanged = (f.diagnosticCount ?? 0) !== (state.lastDiagnosticCount ?? 0);
    if (hashChanged || diagChanged) {
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
  renderObjectOptions();
  renderTimeline();
  renderUnits();
  renderRevisions();
  renderDetail();
}

function renderStats() {
  const h = state.history || {};
  const u = state.units || {};
  const o = state.objects || {};
  $("#stat-events").textContent = `${h.eventCount ?? "—"} events`;
  $("#stat-units").textContent = `${u.revisionCount ?? "—"} units`;
  $("#stat-threads").textContent = `${o.threadCount ?? "—"} threads`;
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
  const units = [...new Set((state.history?.entries || []).map(entryRevisionId).filter(Boolean))].sort();
  state.filterUnit = fillSelect($("#filter-unit"), units, state.filterUnit);
}

function renderObjectOptions() {
  const objects = [...new Set((state.units?.entries || []).map((u) => u.objectId).filter(Boolean))].sort();
  state.filterObject = fillSelect($("#filter-object"), objects, state.filterObject);
}

// The supersession threads (connected components of the supersession DAG, each
// labeled domain-side) and the forward/reverse edges, all from /api/objects.
function objectThreads() {
  return state.objects?.threads || [];
}
function supersededByRevision(revisionId) {
  return (state.objects?.supersededBy && state.objects.supersededBy[revisionId]) || [];
}
function supersedesRevision(revisionId) {
  return (state.objects?.supersedes && state.objects.supersedes[revisionId]) || [];
}
function revisionIsHead(revisionId) {
  return objectThreads().some((thread) => (thread.heads || []).includes(revisionId));
}

// The content object id captured for a revision, via the units list (its
// snapshot id is the content-addressed object).
function objectIdForRevision(revisionId) {
  const unit = (state.units?.entries || []).find((u) => u.revisionId === revisionId);
  return unit ? unit.objectId : "";
}

function eventMatchesObject(e, objectId) {
  if (!objectId) return true;
  return objectIdForRevision(entryRevisionId(e)) === objectId;
}

function isSupersedableFact(e) {
  return SUPERSEDABLE_FACT_TYPES.has(e.eventType);
}

// A fact targeting a superseded revision is stale; the badge names every
// superseding successor (fork-tolerant), never a single head.
function supersessionStaleBadge(e) {
  if (!isSupersedableFact(e)) return "";
  const successors = supersededByRevision(entryRevisionId(e));
  if (!successors.length) return "";
  return `<span class="badge stale">superseded by ${successors.map(linkify).join(" ")}</span>`;
}

// The capture row shows the supersession edge it declared (the predecessors it
// supersedes), reusing the navigable revision chip.
function captureSupersedesBadge(e) {
  if (e.eventType !== "work_object_proposed") return "";
  const predecessors = supersedesRevision(entryRevisionId(e));
  if (!predecessors.length) return "";
  return `<span class="badge supersedes">supersedes ${predecessors.map(linkify).join(" ")}</span>`;
}

// The per-revision supersession status, for a unit card / unit page: "head" when
// it is a current head, "superseded by <chips>" when superseded.
function supersessionBadge(revisionId) {
  if (!revisionId) return "";
  if (revisionIsHead(revisionId)) return `<span class="badge head">head</span>`;
  const successors = supersededByRevision(revisionId);
  if (successors.length) return `<span class="badge superseded">superseded by ${successors.map(linkify).join(" ")}</span>`;
  return "";
}

function matchesFilters(e) {
  if (!state.enabledTypes.has(e.eventType)) return false;
  if (state.filterTrack && entryTrack(e) !== state.filterTrack) return false;
  if (state.filterUnit && entryRevisionId(e) !== state.filterUnit) return false;
  if (state.filterObject && !eventMatchesObject(e, state.filterObject)) return false;
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
    const revisionId = entryRevisionId(e);
    const staleTag = supersessionStaleBadge(e);
    const supersedesTag = captureSupersedesBadge(e);
    li.innerHTML = `
      <span class="time">${escapeHtml(fmtTime(e.occurredAt))}</span>
      <span class="rail" style="background:${typeColor(e.eventType)}"></span>
      <span class="body">
        <span class="title">${linkify(entryTitle(e))} ${tags} ${supersedesTag} ${staleTag}</span>
        <span class="meta">
          <span class="type" style="color:${typeColor(e.eventType)}">${escapeHtml(typeLabel(e.eventType))}</span>
          ${entryTrack(e) ? `<span>${escapeHtml(entryTrack(e))}</span>` : ""}
          ${revisionId ? `<span>revision ${escapeHtml(shortId(revisionId))}</span>` : ""}
          ${entryAnchor(e) ? `<span>${escapeHtml(entryAnchor(e))}</span>` : ""}
          ${verificationChip(e.verificationStatus)}
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
  const revisionId = entryRevisionId(e);
  const kv = [
    ["type", typeLabel(e.eventType) + ` (${e.eventType})`],
    ["occurredAt", fmtDateTime(e.occurredAt)],
    ["eventId", e.eventId],
    ["payloadHash", e.payloadHash],
    ["revision", revisionId || "—"],
    ["track", entryTrack(e) || "—"],
    ["writer", principalLabel(e) || (e.writer ? (e.writer.actorId || "—") : "—")],
  ];
  const snapshotId = revisionId ? snapshotIdForUnit(revisionId) : null;
  const s = e.summary || {};
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
  const verifyChip = verificationChip(e.verificationStatus);
  const endorse = endorsementsBlock(e.endorsements);
  const readback =
    verifyChip || endorse
      ? `<div class="readback">${verifyChip ? `<div class="readback-row">${verifyChip}</div>` : ""}${endorse}</div>`
      : "";
  el.innerHTML = `
    <h2>${linkify(entryTitle(e))}</h2>
    <dl class="kv">${kv.map(([k, v]) => `<dt>${escapeHtml(k)}</dt><dd>${linkify(String(v))}</dd>`).join("")}</dl>
    ${readback}
    ${snapshotId ? `<button class="ghost diff-btn" id="detail-diff-btn">${escapeHtml(btnLabel)}</button>` : ""}
    <pre>${escapeHtml(JSON.stringify(e, null, 2))}</pre>`;
  if (snapshotId) {
    const btn = el.querySelector("#detail-diff-btn");
    if (btn) btn.addEventListener("click", () => openDiff(snapshotId, shortId(revisionId), revisionId, focusId));
  }
}

function snapshotIdForUnit(revisionId) {
  const unit = (state.units?.entries || []).find((u) => u.revisionId === revisionId);
  return unit ? unit.objectId : null;
}

// Gather the review facts on a revision — observations, input requests, and
// assessments — into one annotation list with a shared shape.
function annotationsForUnit(revisionId) {
  const out = [];
  for (const e of state.history?.entries || []) {
    if (entryRevisionId(e) !== revisionId) continue;
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

async function openDiff(snapshotId, label, revisionId = null, focusId = null) {
  const modal = $("#diff-modal");
  $("#diff-title").textContent = label ? `${label} · snapshot ${shortId(snapshotId)}` : shortId(snapshotId);
  $("#diff-body").innerHTML = `<p class="empty">loading snapshot…</p>`;
  modal.classList.remove("hidden");
  try {
    // The snapshot endpoint is snapshot-scoped (no revision id on the wire);
    // the caller threads the revision id for annotation lookup.
    const artifact = await fetchJSON("/api/snapshot?id=" + encodeURIComponent(snapshotId));
    const annotations = annotationsForUnit(revisionId);
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
  let html = `<div class="anno-summary">${annotations.length} review fact${annotations.length === 1 ? "" : "s"} on this revision${
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
    el.innerHTML = `<p class="empty" style="color:var(--fg-dim)">No captured revisions in this store.</p>`;
    return;
  }
  el.innerHTML = "";
  for (const u of entries) {
    const base = u.base || {};
    const card = document.createElement("div");
    card.className = "unit-card";
    const badge = supersessionBadge(u.revisionId);
    const rows = [
      ["captured", fmtDateTime(u.capturedAt)],
      ["base", base.commitOid ? shortId(base.commitOid) + " (" + (base.kind || "") + ")" : base.kind || "—"],
    ];
    const tail = [["snapshot", shortId(u.objectId)]];
    const kv = ([k, v]) => `<span>${escapeHtml(k)}</span><b>${escapeHtml(String(v))}</b>`;
    // The target cell carries pre-escaped derived HTML (label + head badge), so
    // it bypasses the generic escaping cell renderer rather than double-escaping.
    const targetCell = `<span>target</span><b>${targetDisplayLabel(u.targetDisplay)}${targetHeadBadge(u.targetDisplay)}</b>`;
    card.innerHTML = `
      <h3>${escapeHtml(shortId(u.revisionId))}</h3>
      ${badge ? `<div class="supersession-badges">${badge}</div>` : ""}
      <div class="kv">${rows.map(kv).join("")}${targetCell}${tail.map(kv).join("")}</div>`;
    card.title = u.revisionId + "\nclick to open the revision page";
    card.addEventListener("click", (ev) => {
      if (ev.target.closest("[data-ref-kind]")) return;
      openUnit(u.revisionId);
    });
    const actions = document.createElement("div");
    actions.className = "actions";
    const diffBtn = document.createElement("button");
    diffBtn.className = "ghost diff-btn";
    diffBtn.textContent = "view snapshot diff";
    diffBtn.addEventListener("click", (ev) => {
      ev.stopPropagation();
      openDiff(u.objectId, shortId(u.revisionId), u.revisionId);
    });
    actions.appendChild(diffBtn);
    card.appendChild(actions);
    el.appendChild(card);
  }
}

// One card per supersession thread (a connected component of the supersession
// DAG, labeled domain-side), rendering the revision DAG: every revision is
// marked head/superseded and carries its forward/reverse edges, so a fork shows
// as multiple heads (competing) rather than a single linear stack.
function renderRevisions() {
  const el = $("#revisions");
  const threads = objectThreads();
  if (!threads.length) {
    el.innerHTML = `<p class="empty" style="color:var(--fg-dim)">No captured revisions in this store.</p>`;
    return;
  }
  el.innerHTML = "";
  for (const thread of threads) {
    el.appendChild(renderThreadCard(thread));
  }
}

function threadLabel(thread) {
  const heads = thread.heads || [];
  if (thread.competing) return `revision thread · ${heads.length} competing heads`;
  if (heads.length === 1) return `revision thread · head ${shortId(heads[0])}`;
  return "revision thread";
}

function renderThreadCard(thread) {
  const revisions = thread.revisions || [];
  const heads = thread.heads || [];
  const superseded = new Set(thread.superseded || []);
  const card = document.createElement("div");
  card.className = "unit-card thread-card" + (thread.competing ? " competing" : "");
  // A fork surfaces every competing head as a navigable chip — never a null head.
  const competingBadge = thread.competing
    ? `<div class="thread-competing"><span class="fact-status competing">competing revisions (${heads.length})</span> ${heads.map((h) => linkify(h)).join(" ")}</div>`
    : "";
  const nodes = revisions.map((rev) => renderRevisionNode(rev, heads, superseded)).join("");
  card.innerHTML = `
    <h3>${escapeHtml(threadLabel(thread))}</h3>
    ${competingBadge}
    <div class="kv">
      <span>revisions</span><b>${escapeHtml(String(revisions.length))}</b>
      <span>heads</span><b>${escapeHtml(String(heads.length))}</b>
      <span>superseded</span><b>${escapeHtml(String(superseded.size))}</b>
    </div>
    <div class="revision-dag">${nodes}</div>`;
  return card;
}

function renderRevisionNode(rev, heads, superseded) {
  const isHead = heads.includes(rev);
  const isSuperseded = superseded.has(rev);
  const successors = supersededByRevision(rev);
  const predecessors = supersedesRevision(rev);
  const marks = [];
  if (isHead) marks.push(`<span class="fact-status current">head</span>`);
  if (isSuperseded) marks.push(`<span class="fact-status superseded">superseded by ${successors.map((s) => linkify(s)).join(" ")}</span>`);
  const supersedesRow = predecessors.length
    ? `<div class="rev-edge">supersedes ${predecessors.map((p) => linkify(p)).join(" ")}</div>`
    : "";
  return `<div class="revision-node${isHead ? " head" : ""}${isSuperseded ? " superseded" : ""}">
    <div class="rev-node-head">${linkify(rev)} ${marks.join(" ")}</div>
    ${supersedesRow}
  </div>`;
}

function switchView(view) {
  state.view = view;
  // Drill-in pages stay under their parent tabs.
  document.querySelectorAll(".tab").forEach((t) =>
    t.setAttribute(
      "aria-selected",
      String(t.dataset.view === view || (view === "unit" && t.dataset.view === "units")),
    ),
  );
  $("#view-timeline").classList.toggle("hidden", view !== "timeline");
  $("#view-units").classList.toggle("hidden", view !== "units");
  $("#view-revisions").classList.toggle("hidden", view !== "revisions");
  $("#view-unit").classList.toggle("hidden", view !== "unit");
}

async function openUnit(revisionId) {
  switchView("unit");
  $("#unit-page-title").textContent = shortId(revisionId);
  $("#unit-page").innerHTML = `<p class="up-empty">loading…</p>`;
  try {
    const d = await fetchJSON("/api/unit?id=" + encodeURIComponent(revisionId));
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
    case "revision":
      return "whole revision";
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
      ${opts.verify || ""}
      ${opts.createdAt ? `<span class="anno-time" title="${escapeHtml(opts.createdAt)}">${escapeHtml(fmtDateTime(opts.createdAt))}</span>` : ""}
    </div>
    ${body}
    ${opts.endorsements || ""}
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
    createdAt: o.createdAt,
    verify: verificationChip(o.verificationStatus),
    endorsements: endorsementsBlock(o.endorsements),
    extra,
  });
}

function renderInputRequestCard(ir) {
  const responses = (ir.responses || [])
    .map(
      (r) =>
        `<div class="fact-response"><span class="outcome">${escapeHtml(r.outcome)}</span>${r.reason ? `: ${linkify(r.reason)}` : ""} ${verificationChip(r.verificationStatus)}${endorsementsBlock(r.endorsements)}</div>`,
    )
    .join("");
  return factCard("input-request", {
    track: ir.trackId,
    title: ir.title,
    status: ir.status,
    target: targetLabel(ir.target),
    tags: [ir.mode, ir.reasonCode],
    body: ir.body,
    createdAt: ir.createdAt,
    verify: verificationChip(ir.verificationStatus),
    endorsements: endorsementsBlock(ir.endorsements),
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
    createdAt: a.createdAt,
    verify: verificationChip(a.verificationStatus),
    endorsements: endorsementsBlock(a.endorsements),
    extra: rel.length ? `<div class="fact-rel">${rel.join(" · ")}</div>` : "",
  });
}

// Validation evidence is advisory: it renders with the shared factCard shape
// (status maps to .fact-status.<status>) but never as a verdict aggregate, and
// the unit-page section caption keeps it "context only".
function renderValidationCheckCard(v) {
  const rel = [];
  if (v.command) rel.push(escapeHtml(v.command));
  if ((v.logArtifactContentHashes || []).length) rel.push(`logs ${v.logArtifactContentHashes.map(linkify).join(", ")}`);
  return factCard("validation", {
    track: v.trackId,
    title: v.checkName,
    status: v.status, // passed | failed | errored | skipped → .fact-status.<status>
    target: targetLabel(v.target),
    tags: [v.trigger, v.exitCode != null ? `exit ${v.exitCode}` : null],
    body: v.summary || "",
    createdAt: v.completedAt || v.createdAt,
    verify: verificationChip(v.verificationStatus),
    endorsements: endorsementsBlock(v.endorsements),
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
    createdAt: n.createdAt,
  });
}

function factSection(title, items, render) {
  items = items || [];
  const body = items.length ? items.map(render).join("") : `<p class="up-empty">none</p>`;
  return `<section><h2>${escapeHtml(title)} (${items.length})</h2>${body}</section>`;
}

function renderUnitPage(d) {
  const ru = d.revision || {};
  const base = ru.base || {};
  const s = d.summary || {};
  const badge = supersessionBadge(ru.id);
  $("#unit-page-title").textContent = `${shortId(ru.id)}${base.commitOid ? " · base " + shortId(base.commitOid) : ""}`;

  const stat = (label, n) => `<span class="up-stat"><b>${n ?? 0}</b> ${label}</span>`;
  const sections = [];

  sections.push(`<section><h2>Revision</h2><dl class="up-identity">
    <dt>id</dt><dd>${linkify(ru.id)}</dd>
    <dt>base</dt><dd>${base.commitOid ? linkify(base.commitOid) : "—"} ${base.kind ? `<span class="fact-status">${escapeHtml(base.kind)}</span>` : ""}</dd>
    <dt>target</dt><dd>${targetDisplayLabel(ru.targetDisplay)}${targetHeadBadge(ru.targetDisplay)}</dd>
    <dt>worktree</dt><dd>${escapeHtml(ru.targetDisplay?.label ?? "working tree")}</dd>
    <dt>head</dt><dd>${escapeHtml(ru.targetDisplay?.head?.label ?? "—")}</dd>
    <dt>supersession</dt><dd>${badge || "—"}</dd>
    <dt>snapshot</dt><dd>${linkify(ru.objectId)}</dd>
  </dl></section>`);

  sections.push(`<section><h2>Current assessment</h2>${verdictBadge(d.currentAssessment)}${currentAssessmentSummary(d)}</section>`);

  sections.push(`<section><h2>Summary</h2><div class="up-stats">
    ${stat("files", s.fileCount)}${stat("rows", s.rowCount)}${stat("observations", s.observationCount)}${stat("input requests", s.inputRequestCount)}${stat("assessments", s.assessmentCount)}${stat("validation checks", s.validationCheckCount)}${stat("adapter notes", s.adapterNoteCount)}
  </div>
  <div style="margin-top:10px">
    <button class="ghost diff-btn" id="up-diff-btn">view annotated diff</button>
    <button class="ghost" id="up-timeline-btn" style="margin-left:6px">show in timeline</button>
  </div></section>`);

  sections.push(factSection("Observations", d.observations, renderObservationCard));
  sections.push(factSection("Input requests", d.inputRequests, renderInputRequestCard));
  sections.push(factSection("Assessments", d.assessments, renderAssessmentCard));

  // Validation checks: a first-class section after Assessments, rendered from
  // the document array (not raw events). Advisory-only — a context-only caption,
  // structurally separate from Current assessment, never a verdict aggregate.
  const validationChecks = d.validationChecks || [];
  const validationBody = validationChecks.length
    ? validationChecks.map(renderValidationCheckCard).join("") +
      `<p class="validation-note">context only — does not affect the current assessment</p>`
    : `<p class="up-empty">none</p>`;
  sections.push(`<section><h2>Validation checks (${validationChecks.length})</h2>${validationBody}</section>`);

  if ((d.adapterNotes || []).length) sections.push(factSection("Adapter notes", d.adapterNotes, renderAdapterNoteCard));

  $("#unit-page").innerHTML = sections.join("");

  const diffBtn = $("#up-diff-btn");
  if (diffBtn && ru.objectId) diffBtn.addEventListener("click", () => openDiff(ru.objectId, shortId(ru.id), ru.id));
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
  $("#filter-object").addEventListener("change", (ev) => {
    state.filterObject = ev.target.value;
    renderTimeline();
  });
  $("#filter-clear").addEventListener("click", () => {
    state.filterText = "";
    state.filterTrack = "";
    state.filterUnit = "";
    state.filterObject = "";
    state.enabledTypes = new Set(presentTypes());
    $("#filter-text").value = "";
    $("#filter-track").value = "";
    $("#filter-unit").value = "";
    $("#filter-object").value = "";
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
