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
  // The master pane projection: one of timeline | list | threads. Serialized
  // into the URL fragment by the router.
  lens: "timeline",
  // The single selection through-line: { kind: "event" | "revision" | null, id }.
  // The detail pane is a pure projection of this; it replaces the three former
  // scattered selection sites.
  selected: { kind: null, id: null },
  enabledTypes: new Set(TYPES.map((t) => t.id)),
  seenTypes: new Set(TYPES.map((t) => t.id)),
  // The structured query string (serialized as q=). It carries free-text terms
  // plus field:value clauses (type/track/revision/object/status); the revision
  // filter lives here as `revision:<id>`, so it is shareable like the rest.
  filterText: "",
  filterTrack: "",
  filterObject: "",
  order: "desc", // "desc" = newest first (default), "asc" = chronological
  // The route-preserving diff overlay: the object id being shown, plus the
  // set-valued in-diff fact highlight.
  diff: null,
  focus: null,
  lastHash: null,
  lastDiagnosticCount: null,
};

const $ = (sel) => document.querySelector(sel);

// Local display preferences (theme/density). These are reader-local choices,
// persisted in localStorage and never encoded in the URL/hash — they are not
// shareable view state. Applied as the script runs (before first paint) so the
// chosen theme is in place immediately.
const THEME_KEY = "shore-inspect-theme";
function preferredTheme() {
  const stored = localStorage.getItem(THEME_KEY);
  if (stored === "light" || stored === "dark") return stored;
  return window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
}
function applyTheme(theme) {
  document.documentElement.setAttribute("data-theme", theme);
}
function toggleTheme() {
  const next =
    document.documentElement.getAttribute("data-theme") === "light" ? "dark" : "light";
  localStorage.setItem(THEME_KEY, next);
  applyTheme(next);
}
applyTheme(preferredTheme());

const DENSITY_KEY = "shore-inspect-density";
function applyDensity(mode) {
  document.documentElement.classList.toggle("compact", mode === "compact");
}
function toggleDensity() {
  const next =
    document.documentElement.classList.contains("compact") ? "comfortable" : "compact";
  localStorage.setItem(DENSITY_KEY, next);
  applyDensity(next);
}
applyDensity(localStorage.getItem(DENSITY_KEY) || "comfortable");

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

// A reference chip resolves to a navigation through the router (set the
// selection / scope and push a hash), never an in-place filter mutation.
// Navigating to a named reference also dismisses any open diff overlay.
function resolveRef(kind, id) {
  switch (kind) {
    // The revision and the (retired) review-unit prefix both address a revision's
    // composite — its identity is unified onto RevisionId.
    case "rev":
    case "review-unit":
      navigate({ selected: { kind: "revision", id }, diff: null, focus: null });
      break;
    case "track":
      navigateToTrack(id);
      break;
    case "snap":
      openDiff(id);
      break;
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

// Show a single revision's events on the timeline. The revision filter is the
// structured query `revision:<id>`, so it is shareable like the rest of the
// query; clear the cross-lens scope that could otherwise leave the timeline
// empty and switch to the timeline lens through the router.
function navigateToUnit(id) {
  navigate({
    lens: "timeline",
    filterText: "revision:" + id,
    filterTrack: "",
    filterObject: "",
  });
}

function navigateToTrack(id) {
  navigate({ lens: "timeline", filterTrack: id, diff: null, focus: null });
}

function revealBy(predicate) {
  const e = (state.history?.entries || []).find(predicate);
  if (e) revealEvent(e.eventId);
}

// Make an event visible (clearing filters that would hide it) and select it, all
// through the router so the URL is the single source of truth. The
// selection-scroll happens on the render that follows.
function revealEvent(eventId) {
  const e = (state.history?.entries || []).find((x) => x.eventId === eventId);
  if (!e) return;
  // Enable the target's type and clear every filter that could hide the target
  // row, including the track filter (a cross-track chip, e.g. an assessment
  // linking to another track's observation, would otherwise select a hidden row).
  const types = new Set(state.enabledTypes);
  types.add(e.eventType);
  navigate({
    lens: "timeline",
    selected: { kind: "event", id: eventId },
    filterText: "",
    filterTrack: "",
    filterObject: "",
    enabledTypes: types,
    diff: null,
    focus: null,
  });
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
      fetchJSON("/api/revisions"),
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
    // Build the per-entry search index once (not per keystroke): a lowercased
    // haystack plus a small structured projection the query grammar reads.
    for (const e of history.entries || []) {
      const revision = entryRevisionId(e);
      e.__search = {
        text: buildHaystack(e),
        type: e.eventType,
        track: entryTrack(e),
        revision,
        object: objectIdForRevision(revision),
        status: (e.summary || {}).status || "",
      };
    }
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

// Populate the data-driven surfaces (stats, option lists, the lens bodies that do
// not depend on the transient filters), then project the current view. Called on
// load and on every freshness refresh; the per-navigation re-projection is
// render().
function renderAll() {
  renderStats();
  renderDiagnostics();
  render();
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
  // Live facet counts: how many events each type would contribute under the rest
  // of the current query (distribution first, then narrow).
  const counts = facetCounts();
  for (const id of presentTypes()) {
    // Default a newly-seen type (e.g. an unknown event type) to enabled once;
    // after that the user's toggle sticks instead of being re-enabled here.
    if (!state.seenTypes.has(id)) {
      state.seenTypes.add(id);
      state.enabledTypes.add(id);
    }
    const btn = document.createElement("button");
    btn.className = "type-toggle" + (state.enabledTypes.has(id) ? "" : " off");
    btn.innerHTML = `<span class="dot" style="background:${typeColor(id)}"></span>${escapeHtml(typeLabel(id))}<span class="type-count">${counts[id] || 0}</span>`;
    btn.title = id;
    btn.addEventListener("click", () => {
      const types = new Set(state.enabledTypes);
      if (types.has(id)) types.delete(id);
      else types.add(id);
      navigate({ enabledTypes: types }, { replace: true });
    });
    container.appendChild(btn);
  }
}

// The supersession threads (connected components of the supersession DAG, each
// labeled domain-side), all from /api/objects.
function objectThreads() {
  return state.objects?.threads || [];
}
// The server-computed per-revision supersession classification (state +
// direct superseders/predecessors). The client reads this field instead of
// re-deriving head/superseded status from the edge maps every render.
function revisionClassification(revisionId) {
  return (state.objects?.revisionClassification && state.objects.revisionClassification[revisionId]) || null;
}
function supersededByRevision(revisionId) {
  return revisionClassification(revisionId)?.supersededBy || [];
}
function supersedesRevision(revisionId) {
  return revisionClassification(revisionId)?.supersedes || [];
}
function revisionIsHead(revisionId) {
  const klass = revisionClassification(revisionId)?.state;
  // A lone root (isolated) is a current head with no incident edges.
  return klass === "head" || klass === "isolated";
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

// The selected event id, when the single selection is an event (else null).
function selectedEventId() {
  return state.selected && state.selected.kind === "event" ? state.selected.id : null;
}

// ---------------------------------------------------------------------------
// Search index + structured query grammar
//
// Each entry carries a once-per-load `__search` record (a lowercased haystack of
// the human-relevant fields plus a small structured projection), so the filter
// never re-serializes the whole event per keystroke. The query box parses a
// small grammar: bare terms (free-text AND), field:value equality over
// type/track/revision/object/status, `-` negation, and "quoted phrases".
// ---------------------------------------------------------------------------
const QUERY_FIELDS = ["type", "track", "revision", "object", "status"];

// The lowercased haystack of an entry's human-relevant fields (not the whole
// serialized object).
function buildHaystack(e) {
  const s = e.summary || {};
  const parts = [
    entryTitle(e),
    s.body,
    s.summary,
    s.assessment,
    s.outcome,
    s.reasonCode,
    e.eventId,
    entryRevisionId(e),
    s.observationId,
    s.assessmentId,
    s.inputRequestId,
    s.validationCheckId,
    entryTrack(e),
    entryAnchor(e),
    s.checkName,
    s.command,
    ...(entryTags(e) || []),
  ];
  return parts.filter(Boolean).join(" ").toLowerCase();
}

// Split a query into tokens, honoring "quoted phrases" (optionally negated /
// field-prefixed) and bare runs.
function tokenizeQuery(q) {
  const out = [];
  const re = /-?(?:[a-z]+:)?"[^"]*"|\S+/gi;
  let m;
  while ((m = re.exec(q)) !== null) out.push(m[0]);
  return out;
}

// Parse a query string into a list of clauses. A `field:value` whose field is a
// recognized id-shaped field reuses refInfo's classification; everything else is
// a free-text clause.
function parseSearchQuery(q) {
  const clauses = [];
  for (let tok of tokenizeQuery(q || "")) {
    let negate = false;
    if (tok.length > 1 && tok[0] === "-") {
      negate = true;
      tok = tok.slice(1);
    }
    const colon = tok.indexOf(":");
    const field = colon > 0 ? tok.slice(0, colon).toLowerCase() : "";
    if (field && QUERY_FIELDS.includes(field)) {
      const raw = tok.slice(colon + 1).replace(/^"|"$/g, "");
      // refInfo classifies the operand (id-shaped values stay id-shaped); the
      // value is matched as a substring of the stored field so short ids work.
      refInfo(raw);
      clauses.push({ kind: "field", field, value: raw.toLowerCase(), negate });
    } else {
      const term = tok.replace(/^"|"$/g, "").toLowerCase();
      if (term) clauses.push({ kind: "text", value: term, negate });
    }
  }
  return clauses;
}

function fieldMatches(idx, field, value) {
  if (field === "type") {
    // Accept the human label (e.g. "observation") or the raw event-type id.
    const known = TYPES.find((t) => t.label === value || t.id === value);
    return idx.type === (known ? known.id : value);
  }
  return (idx[field] || "").toLowerCase().includes(value);
}

function matchesQuery(idx, clauses) {
  for (const c of clauses) {
    const hit = c.kind === "field" ? fieldMatches(idx, c.field, c.value) : idx.text.includes(c.value);
    if (c.negate ? hit : !hit) return false;
  }
  return true;
}

// Parse the query once per render and memoize on the raw string (matchesFilters
// is called per entry).
let queryCache = { raw: null, clauses: [] };
function currentClauses() {
  if (queryCache.raw !== state.filterText) {
    queryCache = { raw: state.filterText, clauses: parseSearchQuery(state.filterText) };
  }
  return queryCache.clauses;
}

function matchesFilters(e) {
  if (!state.enabledTypes.has(e.eventType)) return false;
  if (state.filterTrack && entryTrack(e) !== state.filterTrack) return false;
  if (state.filterObject && !eventMatchesObject(e, state.filterObject)) return false;
  return matchesQuery(e.__search || {}, currentClauses());
}

// Per-type counts over the events matching everything except the type toggles,
// so each toggle shows the distribution it would contribute.
function facetCounts() {
  const counts = {};
  const clauses = currentClauses();
  for (const e of state.history?.entries || []) {
    if (state.filterTrack && entryTrack(e) !== state.filterTrack) continue;
    if (state.filterObject && !eventMatchesObject(e, state.filterObject)) continue;
    if (!matchesQuery(e.__search || {}, clauses)) continue;
    counts[e.eventType] = (counts[e.eventType] || 0) + 1;
  }
  return counts;
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
    if (e.eventId === selectedEventId()) li.setAttribute("aria-selected", "true");
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
      navigate({ selected: { kind: "event", id: e.eventId } });
    });
    list.appendChild(li);
  }
}

function renderDetail() {
  const el = $("#detail");
  const entries = state.history?.entries || [];
  const e = entries.find((x) => x.eventId === selectedEventId());
  if (!e) {
    el.innerHTML = `<p class="empty">Select an event or revision to inspect.</p>`;
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
  const snapshotId = revisionId ? snapshotIdForRevision(revisionId) : null;
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
  // Persistent, visible reader-scope cue at the head of the readback region (the
  // quietest tier), so the reader-relative framing is never tooltip-only.
  const readback =
    verifyChip || endorse
      ? `<div class="readback"><p class="reader-scope-note">reader-relative — computed against your enrolled keys</p>${verifyChip ? `<div class="readback-row">${verifyChip}</div>` : ""}${endorse}</div>`
      : "";
  el.innerHTML = `
    <h2>${linkify(entryTitle(e))}</h2>
    <dl class="kv">${kv.map(([k, v]) => `<dt>${escapeHtml(k)}</dt><dd>${linkify(String(v))}</dd>`).join("")}</dl>
    ${readback}
    ${snapshotId ? `<button class="ghost diff-btn" id="detail-diff-btn">${escapeHtml(btnLabel)}</button>` : ""}
    <pre>${escapeHtml(JSON.stringify(e, null, 2))}</pre>`;
  if (snapshotId) {
    const btn = el.querySelector("#detail-diff-btn");
    if (btn) btn.addEventListener("click", () => openDiff(snapshotId, focusId));
  }
}

function snapshotIdForRevision(revisionId) {
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

// Open the route-preserving diff overlay for a captured object. The overlay is a
// fragment param (`diff=`/`focus=`); the modal body is reconciled from state on
// the render that follows, so a deep link or Back/Forward reopens it identically.
function openDiff(objectId, focusId = null) {
  navigate({ diff: objectId, focus: focusId ? [focusId] : null });
}

function closeDiff() {
  if (!state.diff && $("#diff-modal").classList.contains("hidden")) return;
  navigate({ diff: null, focus: null });
}

// The revision that captured an object, via the units list (its snapshot id is
// the content-addressed object).
function revisionIdForObject(objectId) {
  const unit = (state.units?.entries || []).find((u) => u.objectId === objectId);
  return unit ? unit.revisionId : null;
}

// The object id currently painted in the modal, so a re-render with an unchanged
// overlay does not re-fetch.
let shownDiffObject = null;

// Reconcile the diff modal DOM with `state.diff`/`state.focus`. Part of the
// render path: it both opens (user action, deep link, Back/Forward) and closes.
function renderDiffOverlay() {
  const modal = $("#diff-modal");
  if (!state.diff) {
    modal.classList.add("hidden");
    shownDiffObject = null;
    return;
  }
  if (state.diff === shownDiffObject) {
    applyDiffFocus();
    return;
  }
  shownDiffObject = state.diff;
  const objectId = state.diff;
  const revisionId = revisionIdForObject(objectId);
  const label = revisionId ? shortId(revisionId) : "";
  $("#diff-title").textContent = label
    ? `${label} · snapshot ${shortId(objectId)}`
    : shortId(objectId);
  $("#diff-body").innerHTML = `<p class="empty">loading snapshot…</p>`;
  modal.classList.remove("hidden");
  // The object endpoint is object-scoped (no revision id on the wire); the
  // revision id is recovered from the units list for annotation lookup.
  fetchJSON("/api/object?id=" + encodeURIComponent(objectId))
    .then((artifact) => {
      // A later overlay change may have superseded this fetch.
      if (state.diff !== objectId) return;
      const annotations = annotationsForUnit(revisionId);
      $("#diff-body").innerHTML = renderDiff(artifact, annotations);
      applyDiffFocus();
    })
    .catch((err) => {
      if (state.diff !== objectId) return;
      $("#diff-body").innerHTML = `<p class="empty">error: ${escapeHtml(err.message)}</p>`;
    });
}

function applyDiffFocus() {
  const focusId = state.focus && state.focus[0];
  if (!focusId) return;
  const target = $("#diff-body").querySelector(`[data-anno="${focusId}"]`);
  if (target) {
    target.scrollIntoView({ block: "center" });
    target.classList.add("anno-flash");
  }
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
    card.dataset.revisionId = u.revisionId;
    if (state.selected.kind === "revision" && state.selected.id === u.revisionId) {
      card.setAttribute("aria-selected", "true");
    }
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
      navigate({ selected: { kind: "revision", id: u.revisionId } });
    });
    const actions = document.createElement("div");
    actions.className = "actions";
    const diffBtn = document.createElement("button");
    diffBtn.className = "ghost diff-btn";
    diffBtn.textContent = "view snapshot diff";
    diffBtn.addEventListener("click", (ev) => {
      ev.stopPropagation();
      openDiff(u.objectId);
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
  const superseded = thread.superseded || [];
  const card = document.createElement("div");
  card.className = "unit-card thread-card" + (thread.competing ? " competing" : "");
  // A fork surfaces every competing head as a navigable chip — never a null head.
  const competingBadge = thread.competing
    ? `<div class="thread-competing"><span class="fact-status competing">competing revisions (${heads.length})</span> ${heads.map((h) => linkify(h)).join(" ")}</div>`
    : "";
  card.innerHTML = `
    <h3>${escapeHtml(threadLabel(thread))}</h3>
    ${competingBadge}
    <div class="kv">
      <span>revisions</span><b>${escapeHtml(String(revisions.length))}</b>
      <span>heads</span><b>${escapeHtml(String(heads.length))}</b>
      <span>superseded</span><b>${escapeHtml(String(superseded.length))}</b>
    </div>
    ${renderThreadSvg(thread.laidOut)}`;
  wireDagInteractions(card);
  return card;
}

// Pure painter of the server-laid geometry: nodes are <rect>+<text> groups
// keyed by revision id, edges are routed polylines. No client-side layout —
// every coordinate comes from `laidOut`, which is already normalized to a
// (0,0) origin, so the viewBox contains the whole graph with no clipping. Heads
// carry no centering/bold/sort-first (peer-equal); the head-vs-superseded shape
// cue lives in the CSS, not in color alone.
function renderThreadSvg(laid) {
  if (!laid || !(laid.nodes && laid.nodes.length)) return "";
  const w = laid.bounds.w;
  const h = laid.bounds.h;
  const edges = (laid.edges || [])
    .map((e) => {
      const pts = e.path.map(([x, y]) => `${x},${y}`).join(" ");
      return `<polyline class="dag-edge" data-from="${escapeHtml(e.from)}" data-to="${escapeHtml(e.to)}" points="${pts}" />`;
    })
    .join("");
  const nodes = laid.nodes
    .map((n) => {
      const sel = state.selected.kind === "revision" && state.selected.id === n.id;
      const cls = `dag-node${n.isHead ? " head" : ""}${n.isSuperseded ? " superseded" : ""}`;
      return `<g class="${cls}" data-revision-id="${escapeHtml(n.id)}" tabindex="0" role="link"${sel ? ' aria-selected="true"' : ""} aria-label="revision ${escapeHtml(shortId(n.id))}">
        <rect x="${n.x - n.w / 2}" y="${n.y - n.h / 2}" width="${n.w}" height="${n.h}" rx="6" />
        <text x="${n.x}" y="${n.y}" text-anchor="middle" dominant-baseline="middle">${escapeHtml(shortId(n.id))}</text>
      </g>`;
    })
    .join("");
  // Render at natural pixel size (1 user unit = 1px) so the node text is not
  // scaled down to illegibility; CSS `max-width:100%` shrinks an oversized graph
  // proportionally. Boxes are sized server-side to the short label.
  return `<svg class="revision-dag" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}" preserveAspectRatio="xMinYMin meet" role="group" aria-label="supersession graph">${edges}${nodes}</svg>`;
}

// Wire the DAG nodes into the IA: click / Enter / Space navigate to the
// revision via the router; hover/focus traces the node and its incident edges
// by class toggle (no re-render).
function wireDagInteractions(card) {
  const nav = (node) => {
    const id = node.getAttribute("data-revision-id");
    if (id) navigate({ selected: { kind: "revision", id }, diff: null, focus: null });
  };
  card.querySelectorAll(".dag-node").forEach((node) => {
    node.addEventListener("click", () => nav(node));
    node.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        nav(node);
      }
    });
    const trace = (on) => {
      const id = node.getAttribute("data-revision-id");
      node.classList.toggle("traced", on);
      card
        .querySelectorAll(`.dag-edge[data-from="${id}"], .dag-edge[data-to="${id}"]`)
        .forEach((edge) => edge.classList.toggle("traced", on));
    };
    node.addEventListener("mouseenter", () => trace(true));
    node.addEventListener("mouseleave", () => trace(false));
    node.addEventListener("focus", () => trace(true));
    node.addEventListener("blur", () => trace(false));
  });
}

// Mark the active lens on the switcher.
function renderLensSwitcher() {
  document.querySelectorAll(".lens-tab").forEach((t) =>
    t.setAttribute("aria-selected", String(t.dataset.lens === state.lens)),
  );
}

// Reflect the current filter/order state into the toolbar controls (the option
// lists are rebuilt only on load, so a navigation that changes a filter syncs the
// displayed value here rather than rebuilding the controls). The toolbar filters
// the event timeline, so it is shown only for that lens.
function syncControls() {
  const text = $("#filter-text");
  if (text && text.value !== (state.filterText || "")) text.value = state.filterText || "";
  const order = $("#order-toggle");
  if (order) order.textContent = state.order === "desc" ? "newest first" : "oldest first";
  const toolbar = $("#toolbar");
  if (toolbar) toolbar.classList.toggle("hidden", state.lens !== "timeline");
}

// Master pane: swap in the active lens body and populate it. The scaffold is
// rebuilt only on a lens change; the populate runs every render so the lens
// reflects the current filters/selection. The threads-lens body is a clean,
// replaceable seam (its flat node list becomes a laid-out graph later).
let lastMasterLens = null;
function renderMaster() {
  const master = $("#master");
  if (state.lens !== lastMasterLens) {
    lastMasterLens = state.lens;
    if (state.lens === "list") {
      master.innerHTML = `<div id="units" class="units"></div>`;
    } else if (state.lens === "threads") {
      master.innerHTML = `<div id="revisions" class="units" aria-label="supersession threads"></div>`;
    } else {
      master.innerHTML = `<ol id="timeline" class="timeline" aria-label="event timeline"></ol>`;
    }
  }
  if (state.lens === "list") renderUnits();
  else if (state.lens === "threads") renderRevisions();
  else renderTimeline();
}

// Detail pane: a pure projection of the single selection — the event detail, the
// revision composite, or the empty prompt.
function renderSelected() {
  if (state.selected.kind === "revision") {
    showComposite(state.selected.id);
  } else {
    shownCompositeId = null;
    renderDetail();
  }
}

// The single render entry. Projects the whole view from state: the lens switcher,
// the toolbar controls, the master lens body, the detail selection, and the
// route-preserving diff overlay. Boot, popstate, hashchange, and every
// navigate() funnel through here.
function render() {
  renderLensSwitcher();
  syncControls();
  renderTypeToggles();
  renderMaster();
  renderSelected();
  scrollSelectionIntoView();
  renderDiffOverlay();
}

// Scroll the selected entry into view within the master pane (the timeline row,
// the list card, or the DAG node), so cursor stepping keeps the selection
// visible.
function scrollSelectionIntoView() {
  const sel = state.selected;
  if (!sel.id) return;
  const master = $("#master");
  if (!master) return;
  const el =
    sel.kind === "event"
      ? master.querySelector('.event[aria-selected="true"]')
      : master.querySelector('[data-revision-id="' + sel.id + '"]');
  if (el) el.scrollIntoView({ block: "center" });
}

// The revision whose composite is currently shown, so a re-render with an
// unchanged revision selection does not re-fetch.
let shownCompositeId = null;
function showComposite(revisionId) {
  if (revisionId === shownCompositeId) return;
  shownCompositeId = revisionId;
  openUnit(revisionId);
}

async function openUnit(revisionId) {
  const detail = $("#detail");
  detail.innerHTML = `<p class="up-empty">loading…</p>`;
  try {
    const d = await fetchJSON("/api/revision?id=" + encodeURIComponent(revisionId));
    // A later selection change may have superseded this fetch.
    if (state.selected.kind !== "revision" || state.selected.id !== revisionId) return;
    renderUnitPage(d);
  } catch (err) {
    if (state.selected.kind === "revision" && state.selected.id === revisionId) {
      detail.innerHTML = `<p class="up-empty">error: ${escapeHtml(err.message)}</p>`;
    }
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
  const title = `${shortId(ru.id)}${base.commitOid ? " · base " + shortId(base.commitOid) : ""}`;

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

  sections.push(`<section><h2>Current assessment</h2>${verdictBadge(d.currentAssessment)}${currentAssessmentSummary(d)}<p class="advisory-note">advisory — a recorded judgement, not a merge gate</p></section>`);

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

  $("#detail").innerHTML = `<div class="unit-page"><p class="unit-page-title">${escapeHtml(title)}</p>${sections.join("")}</div>`;

  const diffBtn = $("#up-diff-btn");
  if (diffBtn && ru.objectId) diffBtn.addEventListener("click", () => openDiff(ru.objectId));
  const tlBtn = $("#up-timeline-btn");
  if (tlBtn) tlBtn.addEventListener("click", () => navigateToUnit(ru.id));
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

// ---------------------------------------------------------------------------
// URL fragment route grammar
//
// location.hash is the single serialization of {lens|entity, selection,
// filters, diff overlay}. The fragment never reaches the GET-only server, so
// routing is entirely client-side and the server is untouched. Theme and
// density are reader-local localStorage preferences and are deliberately NOT
// part of the fragment (they are not shareable view state).
//
//   #/<lens>                      lens-primary (lens ∈ timeline | list | threads)
//   #/<lens>?sel=<id>             a selection within the lens
//   #/revision/<revisionId>       entity-primary: the named revision is selected
//   #/event/<eventId>             entity-primary: the named event is selected
//   ?lens=<lens>                  the master lens behind an entity-primary path
//   ?track= ?object=             cross-lens scope (survive a lens switch)
//   ?order= ?types= ?q=           per-lens timeline controls
//   ?diff=<objectId> ?focus=<set> the route-preserving diff overlay
//   ?v=1                          grammar version (reserved)
//   ?journal= ?asof=             reserved: parsed and ignored, never an error
// ---------------------------------------------------------------------------
const LENSES = ["timeline", "list", "threads"];
const DEFAULT_LENS = "timeline";

// Guards re-deriving the view from a fragment this router just wrote.
let routerLastHash = null;

// Classify a selection id as a revision or an event (a `rev:`/`review-unit:` id
// is a revision; anything else is treated as an event).
function selectionKind(id) {
  const info = refInfo(id);
  if (info && (info.kind === "rev" || info.kind === "review-unit")) return "revision";
  return "event";
}

function parseQuery(queryString) {
  const params = {};
  for (const pair of String(queryString || "").split("&")) {
    if (!pair) continue;
    const eq = pair.indexOf("=");
    const key = decodeURIComponent(eq < 0 ? pair : pair.slice(0, eq));
    const value = eq < 0 ? "" : decodeURIComponent(pair.slice(eq + 1));
    params[key] = value;
  }
  return params;
}

// Parse a fragment into a complete state patch. Absent params resolve to their
// defaults so the fragment fully determines the filter/selection state (so
// Back/Forward to a barer fragment clears what it omits).
function parseHash(hash) {
  const raw = String(hash || "").replace(/^#/, "");
  const q = raw.indexOf("?");
  const path = q < 0 ? raw : raw.slice(0, q);
  const p = parseQuery(q < 0 ? "" : raw.slice(q + 1));

  const patch = {
    selected: { kind: null, id: null },
    filterTrack: p.track != null ? p.track : "",
    filterObject: p.object != null ? p.object : "",
    order: p.order === "asc" || p.order === "desc" ? p.order : "desc",
    filterText: p.q != null ? p.q : "",
    enabledTypes:
      p.types != null ? new Set(p.types.split(",").filter(Boolean)) : new Set(presentTypes()),
    diff: p.diff || null,
    focus: p.focus ? p.focus.split(" ").filter(Boolean) : null,
  };

  const segs = path.split("/").filter(Boolean); // "/timeline" -> ["timeline"]
  if (segs.length === 0) {
    patch.lens = DEFAULT_LENS;
  } else if (segs[0] === "revision" && segs[1]) {
    patch.selected = { kind: "revision", id: decodeURIComponent(segs[1]) };
    patch.lens = LENSES.includes(p.lens) ? p.lens : DEFAULT_LENS;
  } else if (segs[0] === "event" && segs[1]) {
    patch.selected = { kind: "event", id: decodeURIComponent(segs[1]) };
    patch.lens = LENSES.includes(p.lens) ? p.lens : DEFAULT_LENS;
  } else if (LENSES.includes(segs[0])) {
    patch.lens = segs[0];
    if (p.sel) patch.selected = { kind: selectionKind(p.sel), id: p.sel };
  } else {
    patch.lens = DEFAULT_LENS;
    patch.unknownPath = path; // resolve() surfaces a visible fallback diagnostic
  }
  return patch;
}

// Serialize the current state into a fragment, omitting defaults to keep the URL
// short. A selection is entity-primary (durable identity); the lens-primary
// `sel=` form is the inverse of the parser's `?sel=` handling.
function serializeState() {
  const params = [];
  const sel = state.selected || { kind: null, id: null };
  let path = state.lens === DEFAULT_LENS ? "#/timeline" : "#/" + state.lens;
  if (sel.id && (sel.kind === "revision" || sel.kind === "event")) {
    path =
      sel.kind === "revision"
        ? "#/revision/" + encodeURIComponent(sel.id)
        : "#/event/" + encodeURIComponent(sel.id);
    if (state.lens && state.lens !== DEFAULT_LENS) params.push("lens=" + encodeURIComponent(state.lens));
  } else if (sel.id) {
    params.push("sel=" + encodeURIComponent(sel.id));
  }
  if (state.filterTrack) params.push("track=" + encodeURIComponent(state.filterTrack));
  if (state.filterObject) params.push("object=" + encodeURIComponent(state.filterObject));
  if (state.order && state.order !== "desc") params.push("order=" + encodeURIComponent(state.order));
  const all = presentTypes();
  if (state.enabledTypes && all.some((id) => !state.enabledTypes.has(id))) {
    params.push("types=" + encodeURIComponent(all.filter((id) => state.enabledTypes.has(id)).join(",")));
  }
  if (state.filterText) params.push("q=" + encodeURIComponent(state.filterText));
  if (state.diff) params.push("diff=" + encodeURIComponent(state.diff));
  if (state.focus && state.focus.length) params.push("focus=" + encodeURIComponent(state.focus.join(" ")));
  return params.length ? path + "?" + params.join("&") : path;
}

// The single mutation + history + render choke point. Distinct navigations push
// history; refinements (search-as-you-type, cursor steps) replace it.
function navigate(patch, opts) {
  opts = opts || {};
  Object.assign(state, patch);
  if (!state.selected) state.selected = { kind: null, id: null };
  const hash = serializeState();
  routerLastHash = hash;
  history[opts.replace ? "replaceState" : "pushState"]({}, "", hash);
  render();
}

// Derive the whole view from the current fragment. Called on boot and from the
// popstate / hashchange listeners (Back/Forward + manual edits).
function applyHash() {
  const hash = location.hash;
  routerLastHash = hash;
  Object.assign(state, resolve(parseHash(hash)));
  render();
}

// Resolve a parsed patch against the loaded data, falling back up the hierarchy
// (revision → its thread → the lens → timeline) with a visible diagnostic when a
// deep link names an absent entity — never a 404, never a blank view.
function resolve(patch) {
  if (patch.unknownPath != null) {
    showRouteDiagnostic("fell back to the timeline — unknown route " + patch.unknownPath);
    patch.lens = DEFAULT_LENS;
    patch.selected = { kind: null, id: null };
    delete patch.unknownPath;
    return patch;
  }
  const sel = patch.selected || { kind: null, id: null };
  if (sel.kind === "revision" && sel.id && !revisionExists(sel.id)) {
    if (revisionInAnyThread(sel.id)) {
      showRouteDiagnostic(
        "fell back to the threads lens — revision " + shortRef(sel.id) + " is not directly selectable",
      );
      patch.lens = "threads";
    } else {
      // Keep the requested lens (only the selection was absent); name it in the
      // diagnostic so the message matches the lens actually shown.
      const lens = patch.lens || DEFAULT_LENS;
      showRouteDiagnostic(
        "fell back to the " + lens + " lens — revision " + shortRef(sel.id) + " is not in this store",
      );
      patch.lens = lens;
    }
    patch.selected = { kind: null, id: null };
    return patch;
  }
  if (sel.kind === "event" && sel.id && !eventExists(sel.id)) {
    showRouteDiagnostic(
      "fell back to the " + (patch.lens || DEFAULT_LENS) + " lens — event " + shortRef(sel.id) + " is not in this store",
    );
    patch.selected = { kind: null, id: null };
    return patch;
  }
  clearRouteDiagnostic();
  return patch;
}

function revisionExists(id) {
  return (state.units?.entries || []).some((u) => u.revisionId === id);
}
function revisionInAnyThread(id) {
  return objectThreads().some((t) => (t.revisions || []).includes(id));
}
function eventExists(id) {
  return (state.history?.entries || []).some((e) => e.eventId === id);
}

function showRouteDiagnostic(message) {
  const el = $("#route-diagnostic");
  if (!el) return;
  el.textContent = message;
  el.classList.remove("hidden");
}
function clearRouteDiagnostic() {
  const el = $("#route-diagnostic");
  if (!el) return;
  el.textContent = "";
  el.classList.add("hidden");
}

// ---------------------------------------------------------------------------
// Keyboard layer
//
// One delegated keydown handler: step the selection (j/k, ↓/↑), focus search
// (/), jump lenses (g-then-t/l/r), activate the selection (Enter), a layered
// Escape, and a cheat sheet (?). Keystrokes are ignored while an input/textarea
// is focused, except the global Escape. No key activates an advisory fact as an
// operative action — Enter only opens read affordances (the snapshot diff).
// ---------------------------------------------------------------------------

// Whether the focused element is a text field, so the layer yields to typing.
function isTypingTarget(el) {
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || el.isContentEditable;
}

// The ordered selectable entries of the active lens, for cursor stepping. Pure:
// reads state, returns { kind, id } in the lens's display order.
function lensEntryIds() {
  if (state.lens === "list") {
    return (state.units?.entries || []).map((u) => ({ kind: "revision", id: u.revisionId }));
  }
  if (state.lens === "threads") {
    const ids = [];
    for (const t of objectThreads()) for (const r of t.revisions || []) ids.push({ kind: "revision", id: r });
    return ids;
  }
  let entries = (state.history?.entries || []).filter(matchesFilters);
  if (state.order === "desc") entries = entries.slice().reverse();
  return entries.map((e) => ({ kind: "event", id: e.eventId }));
}

// Move the selection by delta within the active lens (replaceState — stepping a
// cursor is a refinement, not a distinct navigation).
function stepSelection(delta) {
  const ids = lensEntryIds();
  if (!ids.length) return;
  let idx = ids.findIndex((x) => x.id === state.selected.id);
  if (idx < 0) idx = delta > 0 ? -1 : 0;
  const next = Math.max(0, Math.min(ids.length - 1, idx + delta));
  navigate({ selected: ids[next] }, { replace: true });
}

// Open the selection's snapshot diff — a read affordance, never a gate.
function activateSelection() {
  const sel = state.selected;
  if (sel.kind === "revision") {
    const obj = objectIdForRevision(sel.id);
    if (obj) openDiff(obj);
  } else if (sel.kind === "event") {
    const rev = entryRevisionId((state.history?.entries || []).find((e) => e.eventId === sel.id) || {});
    const obj = rev ? snapshotIdForRevision(rev) : null;
    if (obj) openDiff(obj);
  }
}

function focusSearch() {
  if (state.lens !== "timeline") navigate({ lens: "timeline" });
  const box = $("#filter-text");
  if (box) box.focus();
}

function toggleKeyHelp() {
  const help = $("#key-help");
  if (help) help.classList.toggle("hidden");
}

// Layered Escape: close the diff, then the cheat sheet, then blur a field, then
// clear the query — one precedence chain, top-down. (A higher-priority overlay
// inserts itself at the head of this chain.)
function handleEscape() {
  if (cmdOpen) {
    closePalette();
    return;
  }
  if (state.diff) {
    navigate({ diff: null, focus: null });
    return;
  }
  const help = $("#key-help");
  if (help && !help.classList.contains("hidden")) {
    help.classList.add("hidden");
    return;
  }
  const active = document.activeElement;
  if (isTypingTarget(active)) {
    active.blur();
    return;
  }
  if (state.filterText) navigate({ filterText: "" }, { replace: true });
}

// A short-lived two-key chord (g-then-…). Cleared after ~1s.
let pendingChord = null;
let chordTimer = null;
function setChord(key) {
  pendingChord = key;
  if (chordTimer) clearTimeout(chordTimer);
  chordTimer = setTimeout(() => {
    pendingChord = null;
  }, 1000);
}

function onKey(ev) {
  // A focused reference chip activates on Enter/Space (it carries role=link +
  // tabindex=0 but had no key handler), resolving the reference like a click.
  const chip = ev.target.closest && ev.target.closest("[data-ref-kind]");
  if (chip && (ev.key === "Enter" || ev.key === " ")) {
    ev.preventDefault();
    resolveRef(chip.dataset.refKind, chip.dataset.refId);
    return;
  }
  // The command palette opens from anywhere, including a focused field. Cmd-K /
  // Ctrl-K, plus a Ctrl-Shift-P fallback (Ctrl-K collides with some browsers'
  // address-bar binding), all preventDefault so the binding does not fight.
  if ((ev.metaKey || ev.ctrlKey) && ev.key.toLowerCase() === "k") {
    ev.preventDefault();
    togglePalette();
    return;
  }
  if (ev.ctrlKey && ev.shiftKey && ev.key.toLowerCase() === "p") {
    ev.preventDefault();
    togglePalette();
    return;
  }
  // Escape is global (it fires even while typing); everything else yields to a
  // focused text field.
  if (ev.key === "Escape") {
    handleEscape();
    return;
  }
  if (isTypingTarget(document.activeElement)) return;

  if (pendingChord === "g") {
    pendingChord = null;
    if (ev.key === "t") return navigate({ lens: "timeline" });
    if (ev.key === "l") return navigate({ lens: "list" });
    if (ev.key === "r") return navigate({ lens: "threads" });
  }

  switch (ev.key) {
    case "g":
      setChord("g");
      return;
    case "/":
      ev.preventDefault();
      focusSearch();
      return;
    case "j":
    case "ArrowDown":
      ev.preventDefault();
      stepSelection(1);
      return;
    case "k":
    case "ArrowUp":
      ev.preventDefault();
      stepSelection(-1);
      return;
    case "Enter":
      activateSelection();
      return;
    case "?":
      ev.preventDefault();
      toggleKeyHelp();
      return;
    default:
      return;
  }
}

// ---------------------------------------------------------------------------
// Command palette (Cmd/Ctrl-K)
//
// One searchable overlay that unifies jump-to-entity + actions and is the
// scalable replacement for the dropdowns' jump role. Every command navigates via
// the router or runs a read/copy action — none is operative or gating.
// ---------------------------------------------------------------------------
let cmdOpen = false;
let cmdItems = [];
let cmdFiltered = [];
let cmdActive = 0;
let cmdPriorFocus = null;

function copyText(text) {
  if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(text);
}

// The candidate commands, built over the loaded state. (When a search index
// exists, jumps query it instead of re-deriving — the source is a single fn.)
function buildCommands() {
  const cmds = [];
  for (const u of state.units?.entries || []) {
    cmds.push({
      kind: "Revisions",
      label: shortRef(u.revisionId),
      hint: shortId(u.objectId),
      run: () => navigate({ selected: { kind: "revision", id: u.revisionId }, diff: null, focus: null }),
    });
  }
  for (const o of [...new Set((state.units?.entries || []).map((u) => u.objectId).filter(Boolean))]) {
    cmds.push({ kind: "Objects", label: shortRef(o), hint: "open diff", run: () => openDiff(o) });
  }
  for (const t of [...new Set((state.history?.entries || []).map(entryTrack).filter(Boolean))].sort()) {
    cmds.push({ kind: "Tracks", label: t, hint: "filter timeline", run: () => navigate({ lens: "timeline", filterTrack: t }) });
  }
  for (const e of state.history?.entries || []) {
    cmds.push({
      kind: "Events",
      label: entryTitle(e),
      hint: typeLabel(e.eventType),
      run: () => navigate({ selected: { kind: "event", id: e.eventId }, diff: null, focus: null }),
    });
  }
  cmds.push({ kind: "Actions", label: "Switch to timeline lens", hint: "lens", run: () => navigate({ lens: "timeline" }) });
  cmds.push({ kind: "Actions", label: "Switch to list lens", hint: "lens", run: () => navigate({ lens: "list" }) });
  cmds.push({ kind: "Actions", label: "Switch to threads lens", hint: "lens", run: () => navigate({ lens: "threads" }) });
  cmds.push({
    kind: "Actions",
    label: "Toggle timeline order",
    hint: "order",
    run: () => navigate({ order: state.order === "desc" ? "asc" : "desc" }, { replace: true }),
  });
  cmds.push({
    kind: "Actions",
    label: "Clear filters",
    hint: "filters",
    run: () =>
      navigate(
        { filterText: "", filterTrack: "", filterObject: "", enabledTypes: new Set(presentTypes()) },
        { replace: true },
      ),
  });
  cmds.push({ kind: "Actions", label: "Copy current view link", hint: "share", run: () => copyText(location.href) });
  cmds.push({
    kind: "Actions",
    label: "Copy selected id",
    hint: "clipboard",
    run: () => {
      if (state.selected.id) copyText(state.selected.id);
    },
  });
  return cmds;
}

function togglePalette() {
  if (cmdOpen) closePalette();
  else openPalette();
}

function openPalette() {
  cmdOpen = true;
  cmdPriorFocus = document.activeElement;
  cmdItems = buildCommands();
  const input = $("#cmd-input");
  input.value = "";
  filterPalette("");
  $("#cmd-palette").classList.remove("hidden");
  input.focus();
}

function closePalette() {
  cmdOpen = false;
  $("#cmd-palette").classList.add("hidden");
  if (cmdPriorFocus && cmdPriorFocus.focus) cmdPriorFocus.focus();
  cmdPriorFocus = null;
}

function filterPalette(query) {
  const needle = query.trim().toLowerCase();
  cmdFiltered = needle
    ? cmdItems.filter((c) => (c.label + " " + (c.hint || "")).toLowerCase().includes(needle))
    : cmdItems.slice();
  cmdActive = 0;
  renderPalette();
}

function renderPalette() {
  const list = $("#cmd-results");
  if (!cmdFiltered.length) {
    list.innerHTML = `<li class="cmd-empty" role="option" aria-disabled="true">No matches</li>`;
    return;
  }
  let html = "";
  let lastKind = null;
  cmdFiltered.forEach((c, i) => {
    if (c.kind !== lastKind) {
      lastKind = c.kind;
      html += `<li class="cmd-group" role="presentation">${escapeHtml(c.kind)}</li>`;
    }
    html += `<li class="cmd-item${i === cmdActive ? " active" : ""}" role="option" data-idx="${i}" aria-selected="${i === cmdActive}"><span class="cmd-label">${escapeHtml(c.label)}</span>${c.hint ? `<span class="cmd-hint">${escapeHtml(c.hint)}</span>` : ""}</li>`;
  });
  list.innerHTML = html;
  const active = list.querySelector(".cmd-item.active");
  if (active) active.scrollIntoView({ block: "nearest" });
}

function movePaletteActive(delta) {
  if (!cmdFiltered.length) return;
  cmdActive = (cmdActive + delta + cmdFiltered.length) % cmdFiltered.length;
  renderPalette();
}

function runPaletteActive() {
  const cmd = cmdFiltered[cmdActive];
  closePalette();
  if (cmd) cmd.run();
}

function wireControls() {
  document.querySelectorAll(".lens-tab").forEach((tab) =>
    tab.addEventListener("click", () =>
      navigate({ lens: LENSES.includes(tab.dataset.lens) ? tab.dataset.lens : DEFAULT_LENS, selected: { kind: null, id: null } }),
    ),
  );
  $("#filter-text").addEventListener("input", (ev) => {
    navigate({ filterText: ev.target.value }, { replace: true });
  });
  $("#filter-clear").addEventListener("click", () => {
    navigate(
      {
        filterText: "",
        filterTrack: "",
        filterObject: "",
        enabledTypes: new Set(presentTypes()),
      },
      { replace: true },
    );
  });
  $("#theme-toggle").addEventListener("click", toggleTheme);
  $("#density-toggle").addEventListener("click", toggleDensity);
  $("#order-toggle").addEventListener("click", () => {
    navigate({ order: state.order === "desc" ? "asc" : "desc" }, { replace: true });
  });
  $("#diff-close").addEventListener("click", closeDiff);
  $("#diff-modal").addEventListener("click", (ev) => {
    if (ev.target === $("#diff-modal")) closeDiff();
  });
  $("#key-help-close").addEventListener("click", () => $("#key-help").classList.add("hidden"));
  $("#key-help").addEventListener("click", (ev) => {
    if (ev.target === $("#key-help")) $("#key-help").classList.add("hidden");
  });
  $("#cmd-input").addEventListener("input", (ev) => filterPalette(ev.target.value));
  $("#cmd-input").addEventListener("keydown", (ev) => {
    if (ev.key === "ArrowDown") {
      ev.preventDefault();
      movePaletteActive(1);
    } else if (ev.key === "ArrowUp") {
      ev.preventDefault();
      movePaletteActive(-1);
    } else if (ev.key === "Enter") {
      ev.preventDefault();
      runPaletteActive();
    }
  });
  $("#cmd-palette").addEventListener("click", (ev) => {
    if (ev.target === $("#cmd-palette")) {
      closePalette();
      return;
    }
    const item = ev.target.closest(".cmd-item");
    if (item) {
      cmdActive = Number(item.dataset.idx);
      runPaletteActive();
    }
  });
  document.addEventListener("keydown", onKey);
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
window.addEventListener("popstate", applyHash);
window.addEventListener("hashchange", applyHash);
load().then(() => {
  applyHash();
  $("#refresh").textContent = "watching";
  setInterval(pollFreshness, 3000);
});
