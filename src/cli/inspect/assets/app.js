// @ts-check
/** @typedef {{ entries: any[], diagnostics: any[] }} HistoryDoc */
/** @typedef {{ entries: any[] }} UnitsDoc */
/** @typedef {{ entries: any[], threads: any[], revisionClassification: * }} ObjectsDoc */
/** @typedef {{ files: any[], anchored: any[], unanchored: any[] }} DiffContext */
/**
 * @typedef {Object} AppState
 * @property {HistoryDoc|null} history
 * @property {UnitsDoc|null} units
 * @property {ObjectsDoc|null} objects
 * @property {string} lens
 * @property {*} selected
 * @property {Set<string>} enabledTypes
 * @property {Set<string>} seenTypes
 * @property {string} filterText
 * @property {string} filterTrack
 * @property {string} filterObject
 * @property {string} order
 * @property {*} diff
 * @property {*} diffHash
 * @property {*} focus
 * @property {*} lastHash
 * @property {*} lastDiagnosticCount
 */
// biome-ignore lint/suspicious/noRedundantUseStrict: classic <script>, not an ES module
"use strict";

// Known durable event types, with display labels and timeline colors. The colors
// resolve to the --evt-* tokens (single source of truth in tokens.css), used as
// CSS var() references in the inline dot/rail/label styles so the palette themes
// from one place instead of hard-coded hex here.
const TYPES = [
  { id: "review_initialized", label: "init", color: "var(--evt-init)" },
  { id: "work_object_proposed", label: "capture", color: "var(--evt-capture)" },
  { id: "review_observation_recorded", label: "observation", color: "var(--evt-observation)" },
  { id: "review_assessment_recorded", label: "assessment", color: "var(--evt-assessment)" },
  { id: "input_request_opened", label: "request", color: "var(--evt-request)" },
  { id: "input_request_responded", label: "response", color: "var(--evt-response)" },
  { id: "review_note_imported", label: "note", color: "var(--evt-note)" },
  { id: "validation_check_recorded", label: "validation", color: "var(--evt-validation)" },
];
const TYPE_MAP = Object.fromEntries(TYPES.map((t) => [t.id, t]));

/** @type {AppState} */
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
  // The route-preserving diff overlay: the object id being shown, optional
  // event-bound artifact hash, plus the single in-diff fact highlight.
  diff: null,
  diffHash: null,
  focus: null,
  lastHash: null,
  lastDiagnosticCount: null,
};

/**
 * @param {*} [sel]
 */
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
/**
 * @param {string} theme
 */
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
/**
 * @param {string} mode
 */
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

/**
 * @param {string} id
 * @returns {string}
 */
function typeColor(id) {
  return (TYPE_MAP[id] || {}).color || "var(--evt-note)";
}
/**
 * @param {string} id
 * @returns {string}
 */
function typeLabel(id) {
  return (TYPE_MAP[id] || {}).label || id;
}

/**
 * @param {string} id
 * @returns {string}
 */
function shortId(id) {
  if (!id) return "";
  const tail = String(id).split(":").pop() || "";
  return tail.length > 12 ? tail.slice(0, 12) : tail;
}

// Git-style short form for Shoreline IDs and hashes, keeping the meaningful
// kind prefix: `review-unit:sha256:1ace…` -> `review-unit:1ace028b`.
/**
 * @param {string} id
 * @returns {string}
 */
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
/**
 * @param {*} [td]
 */
function targetDisplayLabel(td) {
  if (!td) return "working tree"; // floor fallback (pre-upgrade payloads)
  return escapeHtml(td.label || "working tree");
}

// Ready-to-insert head badge (escaped, safe HTML) for the captured base commit,
// or "" when no head is available. A live branch, if ever present, is shown as
// a current/live qualifier — never as capture-time provenance.
/**
 * @param {*} [td]
 */
function targetHeadBadge(td) {
  const head = td && td.head;
  if (!head || !head.label) return "";
  let inner = "@ " + escapeHtml(head.label);
  if (head.liveBranch) inner += " · " + escapeHtml(head.liveBranch) + " (current)";
  return ` <span class="badge">${inner}</span>`;
}

// Classify a token as a navigable Shoreline reference, a non-navigable hash,
// or a track lane. Returns null if it is not a recognized id.
/**
 * @param {*} [token]
 */
function refInfo(token) {
  // Validation check ids have no resolver, so they render as a non-clickable
  // chip rather than dead navigation. Classify before the generic match.
  if (/^validation:(?:git:)?sha256:[0-9a-f]+$/i.test(token)) {
    return { kind: "validation", clickable: false };
  }
  // biome-ignore lint/style/useConst: legacy let; const-correctness handled when this moves to a .ts module
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
/**
 * @param {string} escaped
 * @returns {string}
 */
function linkifyEscaped(escaped) {
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

/**
 * @param {string} text
 * @returns {string}
 */
function linkify(text) {
  return linkifyEscaped(escapeHtml(String(text ?? "")));
}

/**
 * @param {string} contentType
 * @returns {boolean}
 */
function isMarkdownContentType(contentType) {
  return contentType === "text/markdown";
}

/**
 * @param {string} text
 * @param {string} contentType
 * @returns {string}
 */
function renderBodyContent(text, contentType) {
  if (!text) return "";
  const cls = isMarkdownContentType(contentType) ? " anno-body markdown-body" : "anno-body";
  return `<div class="${cls}">${renderContentHtml(text, contentType)}</div>`;
}

/**
 * @param {string} text
 * @param {string} contentType
 * @returns {string}
 */
function renderContentHtml(text, contentType) {
  return isMarkdownContentType(contentType) ? renderMarkdown(text) : linkify(text);
}

/**
 * @param {string} text
 * @returns {string}
 */
function renderMarkdown(text) {
  const lines = String(text ?? "").replace(/\r\n?/g, "\n").split("\n");
  const out = [];
  /** @type {any[]} */
  let paragraph = [];
  /** @type {*} */
  let listKind = null;
  /** @type {any[]} */
  let listItems = [];

  const flushParagraph = () => {
    if (!paragraph.length) return;
    out.push(`<p>${renderMarkdownInline(paragraph.join(" "))}</p>`);
    paragraph = [];
  };
  const flushList = () => {
    if (!listKind) return;
    out.push(`<${listKind}>${listItems.map((item) => `<li>${renderMarkdownInline(item)}</li>`).join("")}</${listKind}>`);
    listKind = null;
    listItems = [];
  };
  const flushBlocks = () => {
    flushParagraph();
    flushList();
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const fence = line.match(/^\s*```/);
    if (fence) {
      flushBlocks();
      const code = [];
      i++;
      while (i < lines.length && !/^\s*```/.test(lines[i])) {
        code.push(lines[i]);
        i++;
      }
      out.push(`<pre><code>${escapeHtml(code.join("\n"))}</code></pre>`);
      continue;
    }
    if (!line.trim()) {
      flushBlocks();
      continue;
    }
    const heading = line.match(/^(#{1,6})\s+(.+)$/);
    if (heading) {
      flushBlocks();
      const level = heading[1].length;
      out.push(`<h${level}>${renderMarkdownInline(heading[2].trim())}</h${level}>`);
      continue;
    }
    const unordered = line.match(/^\s*[-*]\s+(.+)$/);
    if (unordered) {
      flushParagraph();
      if (listKind && listKind !== "ul") flushList();
      listKind = "ul";
      listItems.push(unordered[1]);
      continue;
    }
    const ordered = line.match(/^\s*\d+[.)]\s+(.+)$/);
    if (ordered) {
      flushParagraph();
      if (listKind && listKind !== "ol") flushList();
      listKind = "ol";
      listItems.push(ordered[1]);
      continue;
    }
    if (listKind) flushList();
    paragraph.push(line.trim());
  }
  flushBlocks();
  return out.join("");
}

/**
 * @param {string} text
 * @returns {string}
 */
function renderMarkdownInline(text) {
  /** @type {any[]} */
  const placeholders = [];
  /**
   * @param {string} html
   */
  const stash = (html) => {
    const token = `\u0000MD${placeholders.length}\u0000`;
    placeholders.push([token, html]);
    return token;
  };
  let html = escapeHtml(String(text ?? ""));
  html = html.replace(/`([^`]+)`/g, (_, code) => stash(`<code>${code}</code>`));
  html = html.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (_, label, href) => {
    const safe = safeMarkdownHref(href);
    const labelHtml = renderMarkdownInline(label);
    return safe ? stash(`<a href="${safe}" target="_blank" rel="noreferrer">${labelHtml}</a>`) : labelHtml;
  });
  html = html
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/\*([^*]+)\*/g, "<em>$1</em>");
  html = linkifyEscaped(html);
  for (const [token, replacement] of placeholders) {
    html = html.split(token).join(replacement);
  }
  return html;
}

const OVERLAY_SELECTORS = {
  diff: "#diff-modal",
  palette: "#cmd-palette",
  help: "#key-help",
};
/** @type {*} */
let activeOverlay = null;

/**
 * @param {*} [name]
 */
function overlayNode(name) {
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  return $(OVERLAY_SELECTORS[name]);
}

/**
 * @param {*} [node]
 */
function overlayFocusable(node) {
  if (!node) return [];
  return Array.from(
    node.querySelectorAll(
      'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
    ),
  ).filter((el) => el.getClientRects().length > 0 || el === document.activeElement);
}

/**
 * @param {*} [name]
 * @param {*} [initialSelector]
 */
function openOverlay(name, initialSelector) {
  const node = overlayNode(name);
  if (!node) return;
  if (activeOverlay && activeOverlay.name !== name) closeActiveOverlay({ restoreFocus: false });
  const priorFocus = activeOverlay?.name === name ? activeOverlay.priorFocus : document.activeElement;
  activeOverlay = { name, node, priorFocus };
  node.classList.remove("hidden");
  const target = initialSelector ? node.querySelector(initialSelector) : overlayFocusable(node)[0];
  if (target && target.focus) target.focus();
}

/**
 * @param {*} [opts]
 */
function closeActiveOverlay(opts = {}) {
  if (!activeOverlay) return;
  const current = activeOverlay;
  current.node.classList.add("hidden");
  activeOverlay = null;
  if (
    opts.restoreFocus !== false
    && current.priorFocus
    && document.contains(current.priorFocus)
    && current.priorFocus.focus
  ) {
    current.priorFocus.focus();
  }
}

/**
 * @param {*} [name]
 * @param {*} [opts]
 */
function closeOverlay(name, opts = {}) {
  const node = overlayNode(name);
  if (activeOverlay?.name === name) {
    closeActiveOverlay(opts);
  } else if (node) {
    node.classList.add("hidden");
  }
}

/**
 * @param {*} [ev]
 */
function trapOverlayFocus(ev) {
  if (ev.key !== "Tab" || !activeOverlay) return false;
  const focusable = overlayFocusable(activeOverlay.node);
  if (!focusable.length) {
    ev.preventDefault();
    return true;
  }
  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  if (!activeOverlay.node.contains(document.activeElement)) {
    ev.preventDefault();
    first.focus();
    return true;
  }
  if (ev.shiftKey && document.activeElement === first) {
    ev.preventDefault();
    last.focus();
    return true;
  }
  if (!ev.shiftKey && document.activeElement === last) {
    ev.preventDefault();
    first.focus();
    return true;
  }
  return false;
}

/**
 * @param {string} href
 * @returns {string}
 */
function safeMarkdownHref(href) {
  const raw = String(href ?? "").trim();
  if (/^(https?:|mailto:)/i.test(raw) || raw.startsWith("#")) return escapeHtml(raw);
  return "";
}

// A reference chip resolves to a navigation through the router (set the
// selection / scope and push a hash), never an in-place filter mutation.
// Navigating to a named reference also dismisses any open diff overlay.
/**
 * @param {*} [kind]
 * @param {*} [id]
 */
function resolveRef(kind, id) {
  switch (kind) {
    // The revision and the (retired) review-unit prefix both address a revision's
    // composite — its identity is unified onto RevisionId.
    case "rev":
    case "review-unit":
      navigate({ selected: { kind: "revision", id }, diff: null, diffHash: null, focus: null });
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
/**
 * @param {*} [id]
 */
function navigateToUnit(id) {
  navigate({
    lens: "timeline",
    filterText: "revision:" + id,
    filterTrack: "",
    filterObject: "",
  });
}

/**
 * @param {*} [id]
 */
function navigateToTrack(id) {
  navigate({ lens: "timeline", filterTrack: id, diff: null, diffHash: null, focus: null });
}

/**
 * @param {(e: any) => any} predicate
 */
function revealBy(predicate) {
  const e = (state.history?.entries || []).find(predicate);
  if (e) revealEvent(e.eventId);
}

// Make an event visible (clearing filters that would hide it) and select it, all
// through the router so the URL is the single source of truth. The
// selection-scroll happens on the render that follows.
/**
 * @param {*} [eventId]
 */
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
    diffHash: null,
    focus: null,
  });
}

/**
 * @param {string} occurredAt
 * @returns {number}
 */
function parseMs(occurredAt) {
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  if (typeof occurredAt !== "string") return null;
  const m = occurredAt.match(/(\d+)\s*$/);
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  return m ? Number(m[1]) : null;
}
/**
 * @param {string} occurredAt
 * @returns {string}
 */
function fmtTime(occurredAt) {
  const ms = parseMs(occurredAt);
  if (ms == null) return occurredAt || "";
  const d = new Date(ms);
  return d.toLocaleTimeString([], { hour12: false }) + "." + String(ms % 1000).padStart(3, "0");
}
/**
 * @param {string} occurredAt
 * @returns {string}
 */
function fmtDateTime(occurredAt) {
  const ms = parseMs(occurredAt);
  if (ms == null) return occurredAt || "";
  return new Date(ms).toLocaleString([], { hour12: false });
}

// The typed, type-specific detail of an entry lives in the top-level `summary`
// object (title, body, assessment value, target, tags); `trackId` is also
// top-level. `subject` only carries the target ref, so we read from `summary`.
/**
 * @param {*} [e]
 */
function entryTrack(e) {
  return e.trackId || (e.writer && e.writer.actorId) || "";
}
// The revision a history entry addresses. The entry carries it through its
// subject (the ReviewTargetRef) — every review subject variant keys on
// revisionId — so there is no top-level id to read.
/**
 * @param {*} [e]
 */
function entryRevisionId(e) {
  return (e.subject && e.subject.revisionId) || "";
}
// The human label derived client-side from the structured principal object
// (ADR-0010 structured-first rule). Null unless the agent's principal resolved;
// ambiguous/none entries fall back to the raw actor id at the call site. The
// lane fallback in entryTrack deliberately never reads e.principal — lanes need
// stable strings.
/**
 * @param {*} [e]
 */
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

/**
 * @param {string} status
 * @returns {string}
 */
function verificationChip(status) {
  if (!status) return "";
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  const label = VERIFICATION_LABELS[status] || status;
  return `<span class="verify verify-${escapeHtml(status)}" title="advisory signature readback — reader-relative, never gates a write">${escapeHtml(label)}</span>`;
}

const ENDORSEMENT_LABELS = {
  "endorsement-trusted": "trusted endorsement",
  unknown_endorser: "unknown endorser",
  ambiguous_endorser: "ambiguous endorser",
};
const ASSESSMENT_LABELS = {
  accepted: "accepted",
  accepted_with_follow_up: "accepted-with-follow-up",
  needs_changes: "needs-changes",
  needs_clarification: "needs-clarification",
};

// Strip the actor namespace for display, matching principalLabel's posture.
/**
 * @param {string} actorId
 * @returns {string}
 */
function endorserDisplay(actorId) {
  return actorId.replace(/^actor:git-(email|name):/, "");
}

/**
 * @param {*} [en]
 */
function endorsementRow(en) {
  const cls = en.classification || "";
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
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
/**
 * @param {any[]} endorsements
 */
function endorsementsBlock(endorsements) {
  endorsements = endorsements || [];
  if (!endorsements.length) return "";
  const rows = endorsements.map(endorsementRow).join("");
  return `<div class="endorsements" title="advisory endorsement readback — reader-relative, never gates a write">
    <span class="endorsements-label">endorsements</span>
    <ul class="endorse-list">${rows}</ul>
  </div>`;
}

/**
 * @param {string} value
 * @returns {string}
 */
function assessmentDisplayLabel(value) {
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  return ASSESSMENT_LABELS[value] || value || "";
}

/**
 * @param {*} [e]
 */
function entryTitle(e) {
  const s = e.summary || {};
  if (s.title) return s.title;
  if (s.assessment) return assessmentDisplayLabel(s.assessment);
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
/**
 * @param {*} [e]
 */
function entryTags(e) {
  const s = e.summary || {};
  return Array.isArray(s.tags) ? s.tags : [];
}
/**
 * @param {*} [e]
 */
function entryAnchor(e) {
  const t = (e.summary || {}).target || {};
  if (!t.filePath) return "";
  if (t.startLine) return `${t.filePath}:${t.startLine}-${t.endLine || t.startLine}`;
  return t.filePath;
}

/**
 * @param {*} [path]
 */
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

/**
 * @param {string} message
 */
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
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    showError(null);
    renderAll();
  } catch (err) {
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
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
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  $("#stat-events").textContent = `${h.eventCount ?? "—"} events`;
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  $("#stat-units").textContent = `${u.revisionCount ?? "—"} units`;
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  $("#stat-threads").textContent = `${o.threadCount ?? "—"} threads`;
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
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
    const enabled = state.enabledTypes.has(id);
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    const count = counts[id] || 0;
    btn.type = "button";
    btn.className = "type-toggle" + (enabled ? "" : " off");
    btn.setAttribute("aria-pressed", String(enabled));
    btn.setAttribute("aria-label", `${enabled ? "Hide" : "Show"} ${typeLabel(id)} events (${count})`);
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
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

/**
 * @param {*} [thread]
 */
function threadRevisionOrder(thread) {
  const revisions = thread.revisions || [];
  const nodes = thread.laidOut?.nodes || [];
  if (!nodes.length) return revisions;
  const known = new Set(revisions);
  const ordered = nodes
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    .filter((n) => n && known.has(n.id))
    .slice()
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    .sort((a, b) => a.y - b.y || a.x - b.x)
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    .map((n) => n.id);
  if (ordered.length === revisions.length) return ordered;
  const seen = new Set(ordered);
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  return ordered.concat(revisions.filter((id) => !seen.has(id)));
}

// The server-computed per-revision supersession classification (state +
// direct superseders/predecessors). The client reads this field instead of
// re-deriving head/superseded status from the edge maps every render.
/**
 * @param {*} [revisionId]
 */
function revisionClassification(revisionId) {
  return (state.objects?.revisionClassification && state.objects.revisionClassification[revisionId]) || null;
}
/**
 * @param {*} [revisionId]
 */
function supersededByRevision(revisionId) {
  return revisionClassification(revisionId)?.supersededBy || [];
}
/**
 * @param {*} [revisionId]
 */
function supersedesRevision(revisionId) {
  return revisionClassification(revisionId)?.supersedes || [];
}
/**
 * @param {*} [revisionId]
 */
function revisionIsHead(revisionId) {
  const klass = revisionClassification(revisionId)?.state;
  // A lone root (isolated) is a current head with no incident edges.
  return klass === "head" || klass === "isolated";
}

// The content object id captured for a revision, via the units list (its
// snapshot id is the content-addressed object).
/**
 * @param {*} [revisionId]
 */
function objectIdForRevision(revisionId) {
  const unit = unitForRevision(revisionId);
  return unit ? unit.objectId : "";
}

/**
 * @param {*} [revisionId]
 */
function objectArtifactHashForRevision(revisionId) {
  return unitForRevision(revisionId)?.objectArtifactContentHash || "";
}

/**
 * @param {*} [e]
 * @param {*} [objectId]
 */
function eventMatchesObject(e, objectId) {
  if (!objectId) return true;
  return objectIdForRevision(entryRevisionId(e)) === objectId;
}

/**
 * @param {*} [e]
 */
function isSupersedableFact(e) {
  return SUPERSEDABLE_FACT_TYPES.has(e.eventType);
}

// A fact targeting a superseded revision is stale; the badge names every
// superseding successor (fork-tolerant), never a single head.
/**
 * @param {*} [e]
 */
function supersessionStaleBadge(e) {
  if (!isSupersedableFact(e)) return "";
  const successors = supersededByRevision(entryRevisionId(e));
  if (!successors.length) return "";
  return `<span class="badge stale">superseded by ${successors.map(linkify).join(" ")}</span>`;
}

// The capture row shows the supersession edge it declared (the predecessors it
// supersedes), reusing the navigable revision chip.
/**
 * @param {*} [e]
 */
function captureSupersedesBadge(e) {
  if (e.eventType !== "work_object_proposed") return "";
  const predecessors = supersedesRevision(entryRevisionId(e));
  if (!predecessors.length) return "";
  return `<span class="badge supersedes">supersedes ${predecessors.map(linkify).join(" ")}</span>`;
}

// The per-revision supersession status, for a unit card / unit page: "current in
// thread" when it is current, "superseded by <chips>" when superseded.
/**
 * @param {*} [revisionId]
 */
function supersessionBadge(revisionId) {
  if (!revisionId) return "";
  if (revisionIsHead(revisionId)) return `<span class="badge head">current in thread</span>`;
  const successors = supersededByRevision(revisionId);
  if (successors.length) return `<span class="badge superseded">superseded by ${successors.map(linkify).join(" ")}</span>`;
  return "";
}

/**
 * @param {*} [revisionId]
 */
function unitForRevision(revisionId) {
  return (state.units?.entries || []).find((u) => u.revisionId === revisionId) || null;
}

/**
 * @param {*} [revisionId]
 */
function overviewForRevision(revisionId) {
  return unitForRevision(revisionId)?.overview || null;
}

/**
 * @param {string} value
 * @returns {string}
 */
function assessmentLabel(value) {
  if (!value) return "";
  return String(value).replaceAll("_", " ");
}

/**
 * @param {*} [overview]
 */
function assessmentCue(overview) {
  const currentAssessment = overview?.currentAssessment || {};
  const status = currentAssessment.status || "unassessed";
  const assessment = currentAssessment.assessment || "";
  const label =
    assessment ||
    (status === "ambiguous" ? "ambiguous current assessment" : status === "resolved" ? "resolved" : "unassessed");
  const cls = assessment || status;
  return `<span class="overview-assessment"><span>current assessment</span><span class="fact-status ${escapeHtml(cls)}">${escapeHtml(assessmentLabel(label))}</span></span>`;
}

/**
 * @param {number} n
 * @param {string} singular
 * @param {string} [pluralLabel]
 * @returns {string}
 */
function plural(n, singular, pluralLabel = singular + "s") {
  return `${n} ${n === 1 ? singular : pluralLabel}`;
}

/**
 * @param {*} [overview]
 */
function attentionTokens(overview) {
  const attention = overview?.attention || {};
  const tokens = [];
  if (attention.openInputRequestCount) {
    tokens.push({
      token: "open-request",
      query: "attention:open-request",
      label: plural(attention.openInputRequestCount, "open request"),
    });
  }
  if (attention.unassessed) {
    tokens.push({ token: "unassessed", query: "attention:unassessed", label: "unassessed" });
  }
  const validationCount = (attention.failedValidationCount || 0) + (attention.erroredValidationCount || 0);
  if (validationCount) {
    tokens.push({
      token: "validation-context",
      query: "attention:validation-context",
      label: plural(validationCount, "validation context", "validation contexts"),
    });
  }
  if (attention.acceptedWithFollowUp) {
    tokens.push({ token: "follow-up", query: "attention:follow-up", label: "follow-up" });
  }
  return tokens;
}

/**
 * @param {*} [overview]
 */
function attentionCues(overview) {
  const tokens = attentionTokens(overview);
  if (!tokens.length) return `<span class="overview-muted">no attention cues</span>`;
  return tokens
    .map(
      (cue) =>
        `<button class="overview-cue" type="button" data-attention-query="${escapeHtml(cue.query)}" title="filter ${escapeHtml(cue.query)}">${escapeHtml(cue.label)}</button>`,
    )
    .join("");
}

/**
 * @param {*} [overview]
 */
function overviewStats(overview) {
  const counts = overview?.counts || {};
  const facts =
    (counts.observations || 0) +
    (counts.inputRequests || 0) +
    (counts.assessments || 0) +
    (counts.validationChecks || 0) +
    (counts.adapterNotes || 0);
  /**
   * @param {string} label
   * @param {*} [value]
   */
  const stat = (label, value) => `<span class="overview-stat"><b>${value ?? 0}</b> ${escapeHtml(label)}</span>`;
  return `<div class="overview-stats">${stat("files", counts.files)}${stat("rows", counts.rows)}${stat("facts", facts)}</div>`;
}

/**
 * @param {*} [overview]
 */
function latestActivityLine(overview) {
  const latest = overview?.latestActivity;
  if (!latest) return "";
  const title = latest.title || latest.kind || "activity";
  return `<div class="overview-latest"><span>latest</span><b>${escapeHtml(title)}</b><span>${escapeHtml(fmtDateTime(latest.at))}</span></div>`;
}

/**
 * @param {*} [u]
 */
function revisionSearchIndex(u) {
  const overview = u.overview || {};
  const currentAssessment = overview.currentAssessment || {};
  const latest = overview.latestActivity || {};
  const target = u.targetDisplay || {};
  const head = target.head || {};
  const cues = attentionTokens(overview);
  const text = [
    u.revisionId,
    u.objectId,
    target.label,
    head.label,
    currentAssessment.status,
    currentAssessment.assessment,
    latest.kind,
    latest.title,
    ...cues.map((cue) => cue.label),
    "review cues",
    "attention",
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
  return {
    text,
    type: "revision",
    revision: u.revisionId,
    object: u.objectId,
    status: currentAssessment.assessment || currentAssessment.status || "",
    attention: cues.map((cue) => cue.token).join(" "),
  };
}

/**
 * @param {*} [u]
 */
function matchesRevisionFilters(u) {
  if (state.filterObject && u.objectId !== state.filterObject) return false;
  return matchesQuery(revisionSearchIndex(u), currentClauses());
}

/**
 * @param {*} [thread]
 */
function threadMatchesRevisionFilters(thread) {
  const revisions = thread.revisions || [];
  if (!state.filterText && !state.filterObject) return true;
  return revisions.map(unitForRevision).filter(Boolean).some(matchesRevisionFilters);
}

/**
 * @param {*} [thread]
 * @param {any[]} [revisions]
 */
function filteredThreadRevisionIds(thread, revisions = thread.revisions || []) {
  if (!state.filterText && !state.filterObject) return revisions;
  return revisions.filter((revisionId) => {
    const unit = unitForRevision(revisionId);
    return unit ? matchesRevisionFilters(unit) : false;
  });
}

/**
 * @param {*} [root]
 */
function wireOverviewCueFilters(root) {
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  root.querySelectorAll("[data-attention-query]").forEach((button) => {
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    button.addEventListener("click", (ev) => {
      ev.stopPropagation();
      const query = button.getAttribute("data-attention-query");
      if (query) navigate({ filterText: query });
    });
  });
}

/**
 * @param {*} [u]
 * @param {*} [overview]
 */
function renderRevisionOverview(u, overview = u.overview) {
  return `<div class="overview-summary">
    <div class="overview-main">${assessmentCue(overview)}${overviewStats(overview)}</div>
    <div class="overview-cues" aria-label="review cues"><span class="overview-label">review cues</span>${attentionCues(overview)}</div>
    ${latestActivityLine(overview)}
  </div>`;
}

/**
 * @param {*} [revisionId]
 */
function renderThreadRevisionOverview(revisionId) {
  const unit = unitForRevision(revisionId);
  const overview = overviewForRevision(revisionId);
  if (!unit || !overview) return "";
  return `<div class="thread-overview">
    <div><b>${targetDisplayLabel(unit.targetDisplay)}</b> <span>${escapeHtml(shortId(revisionId))}</span></div>
    ${assessmentCue(overview)}
    <div class="overview-cues" aria-label="review cues"><span class="overview-label">review cues</span>${attentionCues(overview)}</div>
  </div>`;
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
// type/track/revision/object/status/attention, `-` negation, and "quoted phrases".
// ---------------------------------------------------------------------------
const QUERY_FIELDS = ["type", "track", "revision", "object", "status", "attention"];

// The lowercased haystack of an entry's human-relevant fields (not the whole
// serialized object).
/**
 * @param {*} [e]
 */
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
/**
 * @param {string} q
 */
function tokenizeQuery(q) {
  const out = [];
  const re = /-?(?:[a-z]+:)?"[^"]*"|\S+/gi;
  let m;
  // biome-ignore lint/suspicious/noAssignInExpressions: legacy assign-in-expression; refactored when this moves to a .ts module
  while ((m = re.exec(q)) !== null) out.push(m[0]);
  return out;
}

// Parse a query string into a list of clauses. A `field:value` whose field is a
// recognized id-shaped field reuses refInfo's classification; everything else is
// a free-text clause.
/**
 * @param {*} [q]
 */
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

/**
 * @param {*} [idx]
 * @param {*} [field]
 * @param {*} [value]
 */
function fieldMatches(idx, field, value) {
  if (field === "type") {
    // Accept the human label (e.g. "observation") or the raw event-type id.
    const known = TYPES.find((t) => t.label === value || t.id === value);
    return idx.type === (known ? known.id : value);
  }
  return (idx[field] || "").toLowerCase().includes(value);
}

/**
 * @param {*} [idx]
 * @param {*} [clauses]
 */
function matchesQuery(idx, clauses) {
  for (const c of clauses) {
    const hit = c.kind === "field" ? fieldMatches(idx, c.field, c.value) : idx.text.includes(c.value);
    if (c.negate ? hit : !hit) return false;
  }
  return true;
}

// Parse the query once per render and memoize on the raw string (matchesFilters
// is called per entry).
/** @type {*} */
let queryCache = { raw: null, clauses: [] };
function currentClauses() {
  if (queryCache.raw !== state.filterText) {
    queryCache = { raw: state.filterText, clauses: parseSearchQuery(state.filterText) };
  }
  return queryCache.clauses;
}

/**
 * @param {*} [e]
 */
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
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
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
      // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
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
      // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
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
  const bodyBlock = eventBodyBlock(e);
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
    ${bodyBlock}
    <pre>${escapeHtml(JSON.stringify(e, null, 2))}</pre>`;
  if (snapshotId) {
    const btn = el.querySelector("#detail-diff-btn");
    if (btn) {
      btn.addEventListener("click", () =>
        openDiff(snapshotId, focusId, objectArtifactHashForRevision(revisionId)),
      );
    }
  }
}

/**
 * @param {*} [e]
 */
function eventBodyBlock(e) {
  const s = e.summary || {};
  if (s.body) return renderBodyContent(s.body, s.bodyContentType);
  if (s.summary) return renderBodyContent(s.summary, s.summaryContentType);
  if (s.reason) return renderBodyContent(s.reason, s.reasonContentType);
  return "";
}

/**
 * @param {*} [revisionId]
 */
function snapshotIdForRevision(revisionId) {
  const unit = (state.units?.entries || []).find((u) => u.revisionId === revisionId);
  return unit ? unit.objectId : null;
}

// Gather the review facts on a revision — observations, input requests, and
// assessments — into one annotation list with a shared shape.
/**
 * @param {*} [revisionId]
 */
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
        bodyContentType: s.bodyContentType,
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
        bodyContentType: s.bodyContentType,
        track: e.trackId || "",
        tags: meta ? [meta] : [],
        target: s.target || {},
      });
    } else if (e.eventType === "review_assessment_recorded") {
      const assessmentLabel = assessmentDisplayLabel(s.assessment);
      out.push({
        kind: "assessment",
        id: s.assessmentId || e.eventId,
        title: `assessment: ${assessmentLabel || "?"}`,
        body: s.summary || "",
        bodyContentType: s.summaryContentType,
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
// DIFF_LENS_ROUTE_SEAM: this modal remains quick readback over `diff=` route
// state. A full-page diff lens route/data contract is deferred until it can be
// designed as its own route and payload seam rather than inferred here.
/**
 * @param {*} [objectId]
 * @param {*} [focusId]
 * @param {*} [contentHash]
 */
function openDiff(objectId, focusId = null, contentHash = null) {
  navigate({ diff: objectId, diffHash: contentHash || null, focus: focusId || null });
}

/**
 * @param {*} [revisionId]
 * @param {*} [focusId]
 */
function openRevisionDiff(revisionId, focusId = null) {
  const objectId = objectIdForRevision(revisionId);
  if (objectId) openDiff(objectId, focusId, objectArtifactHashForRevision(revisionId));
}

function closeDiff() {
  if (!state.diff && $("#diff-modal").classList.contains("hidden")) return;
  navigate({ diff: null, diffHash: null, focus: null }, { replace: true });
}

// The revision that captured an object artifact, via the units list. The content
// hash disambiguates rebased recaptures with the same stable object id.
/**
 * @param {*} [objectId]
 * @param {*} [contentHash]
 */
function revisionIdForObject(objectId, contentHash = null) {
  const entries = state.units?.entries || [];
  const unit =
    entries.find(
      (u) =>
        u.objectId === objectId &&
        (!contentHash || u.objectArtifactContentHash === contentHash),
    ) || entries.find((u) => u.objectId === objectId);
  return unit ? unit.revisionId : null;
}

// The object artifact currently painted in the modal, so a re-render with an
// unchanged overlay does not re-fetch.
/** @type {*} */
let shownDiffObject = null;
/** @type {*} */
let shownDiffHash = null;

// Module-local render context for the open diff: the files + anchored facts the
// delegated #diff-body / #diff-nav listeners read to lazily fill a collapsed
// file body or expand-then-scroll to a fact. Set by renderDiff, cleared when the
// overlay closes. NOT route state (state.diff stays the object-id string | null).
/** @type {DiffContext|null} */
let diffCtx = null;
// Cursors for the diff-local jump keys (next/prev fact, next/prev change), reset
// each time a new diff renders.
let diffFactCursor = -1;
let diffChangeCursor = -1;
let diffNavFilter = "all";

// Reconcile the diff modal DOM with `state.diff`/`state.focus`. Part of the
// render path: it both opens (user action, deep link, Back/Forward) and closes.
function renderDiffOverlay() {
  if (!state.diff) {
    closeOverlay("diff");
    shownDiffObject = null;
    shownDiffHash = null;
    diffCtx = null;
    return;
  }
  if (state.diff === shownDiffObject && state.diffHash === shownDiffHash) {
    if (activeOverlay?.name !== "diff") openOverlay("diff", "#diff-close");
    applyDiffFocus();
    return;
  }
  if (cmdOpen) closePalette({ restoreFocus: false });
  closeKeyHelp({ restoreFocus: false });
  shownDiffObject = state.diff;
  shownDiffHash = state.diffHash;
  const objectId = state.diff;
  const contentHash = state.diffHash;
  const revisionId = revisionIdForObject(objectId, contentHash);
  const label = revisionId ? shortId(revisionId) : "";
  $("#diff-title").textContent = label
    ? `${label} · snapshot ${shortId(objectId)}`
    : shortId(objectId);
  $("#diff-body").innerHTML = `<p class="empty">loading snapshot…</p>`;
  $("#diff-nav").innerHTML = "";
  openOverlay("diff", "#diff-close");
  // The object endpoint is object-scoped (no revision id on the wire); the
  // revision id is recovered from the units list for annotation lookup.
  let objectUrl = "/api/object?id=" + encodeURIComponent(objectId);
  if (contentHash) objectUrl += "&contentHash=" + encodeURIComponent(contentHash);
  fetchJSON(objectUrl)
    .then((artifact) => {
      // A later overlay change may have superseded this fetch.
      if (state.diff !== objectId || state.diffHash !== contentHash) return;
      const annotations = annotationsForUnit(revisionId);
      $("#diff-body").innerHTML = renderDiff(artifact, annotations);
      $("#diff-nav").innerHTML = renderDiffNav();
      applyDiffFocus();
    })
    .catch((err) => {
      if (state.diff !== objectId || state.diffHash !== contentHash) return;
      $("#diff-body").innerHTML = `<p class="empty">error: ${escapeHtml(err.message)}</p>`;
    });
}

function applyDiffFocus() {
  const focusId = state.focus;
  if (focusId) scrollToAnno(focusId);
}

/**
 * @param {*} [id]
 */
function focusDiffFactRoute(id) {
  if (!id || state.focus === id) return false;
  navigate({ focus: id }, { replace: true });
  return true;
}

// Scroll a review fact's annotation into view and flash it, expanding its file
// first if it lives in a default-collapsed section. The single path a focus=
// deep-link, a gutter click, a navigator entry, and the n/p keys all route
// through, so they behave identically.
/**
 * @param {*} [id]
 * @param {*} [opts]
 */
function scrollToAnno(id, opts = {}) {
  if (opts.updateRoute && focusDiffFactRoute(id)) return;
  const sel = `.anno[data-anno="${id}"]`;
  let target = $("#diff-body").querySelector(sel);
  if (!target && diffCtx) {
    const fact = diffCtx.anchored.find((a) => a.id === id);
    const filePath = fact && (fact.target || {}).filePath;
    if (filePath) {
      const idx = diffCtx.files.findIndex((f) => f.new_path === filePath || f.old_path === filePath);
      if (idx >= 0) {
        const section = $("#diff-body").querySelector(`.dfile[data-dfile="${idx}"]`);
        if (section) {
          expandDiffFile(section);
          target = $("#diff-body").querySelector(sel);
        }
      }
    }
  }
  if (target) {
    target.scrollIntoView({ block: "center" });
    flashAnno(target);
  }
}

// Restart the flash animation even if the element was flashed before (n/p may
// land on it twice).
/**
 * @param {*} [el]
 */
function flashAnno(el) {
  el.classList.remove("anno-flash");
  void el.offsetWidth;
  el.classList.add("anno-flash");
}

// Fill a collapsed file's lazy body on first expand, cached via a rendered flag.
/**
 * @param {*} [section]
 */
function ensureDiffFileBody(section) {
  if (!diffCtx) return;
  const body = section.querySelector("[data-dfile-body]");
  if (!body || body.dataset.rendered) return;
  const idx = Number(section.dataset.dfile);
  body.innerHTML = renderDiffFileBody(diffCtx.files[idx], diffCtx.anchored);
  body.removeAttribute("data-fact-vicinity");
  body.dataset.rendered = "1";
}

/**
 * @param {*} [section]
 */
function diffFileHeader(section) {
  return section ? section.querySelector(".dfile-head") : null;
}

/**
 * @param {*} [section]
 */
function diffFileExpanded(section) {
  const head = diffFileHeader(section);
  return head ? head.getAttribute("aria-expanded") === "true" : false;
}

/**
 * @param {*} [section]
 * @param {*} [open]
 */
function setDiffFileExpanded(section, open) {
  if (!section) return;
  const value = String(open);
  section.dataset.expanded = value;
  const head = diffFileHeader(section);
  if (head) head.setAttribute("aria-expanded", value);
}

// Expand one accordion file section (render its body on first expand). Used by
// navigation (navigator entry, focus jump) where the target must end up open.
/**
 * @param {*} [section]
 */
function expandDiffFile(section) {
  if (!section) return;
  ensureDiffFileBody(section);
  setDiffFileExpanded(section, true);
}

// Toggle one accordion file section; render its body on first expand. Transient
// DOM state, reconciled on each overlay render — not route state.
/**
 * @param {*} [section]
 */
function toggleDiffFile(section) {
  if (!section) return;
  const open = diffFileExpanded(section);
  if (!open) ensureDiffFileBody(section);
  setDiffFileExpanded(section, !open);
}

// The file/fact navigator sidebar: one entry per file (status + path + fact
// badge) plus the unanchored-facts panel, so every fact — including those not
// anchored to a captured diff line — is reachable on a large changeset.
function renderDiffNav() {
  if (!diffCtx) return "";
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  const { files, anchored, unanchored, filePaths } = diffCtx;
  const visibleFiles = files
    .map((f, i) => ({ f, i, factCount: fileFactCount(f, anchored) }))
    .filter((item) => {
      if (diffNavFilter === "with-facts") return item.factCount > 0;
      if (diffNavFilter === "unanchored") return false;
      return true;
    });
  const fileItems = visibleFiles
    .map(({ f, i, factCount: n }) => {
      const badge = n ? `<span class="dfile-notes">${n}</span>` : "";
      return `<li><button class="diff-nav-file" data-nav-file="${i}">
        <span class="dstatus s-${escapeHtml(f.status)}">${escapeHtml(f.status)}</span>
        <span class="dpath">${escapeHtml(filePathLabel(f))}</span>${badge}</button></li>`;
    })
    .join("");
  let html = renderDiffNavSummary(diffNavSummary()) + renderDiffNavFilters();
  if (diffNavFilter !== "unanchored") {
    html += `<ol class="diff-nav-files">${fileItems}</ol>`;
  }
  if (unanchored && unanchored.length && diffNavFilter !== "with-facts") {
    const entries = unanchored
      .map(
        (a) =>
          `<li><button class="diff-nav-fact" data-anno="${escapeHtml(a.id)}"><span>${escapeHtml(a.title)}</span><span class="diff-nav-reason">${escapeHtml(unanchoredReason(a, filePaths))}</span></button></li>`,
      )
      .join("");
    html += `<section class="diff-unanchored" aria-label="unanchored review facts">
      <h3>${unanchored.length} not anchored to a diff line</h3>
      <ol>${entries}</ol></section>`;
  }
  return html;
}

function diffNavSummary() {
  if (!diffCtx) return { fileCount: 0, factCount: 0, unanchoredCount: 0 };
  return {
    fileCount: diffCtx.files.length,
    factCount: diffCtx.anchored.length + diffCtx.unanchored.length,
    unanchoredCount: diffCtx.unanchored.length,
  };
}

/**
 * @param {*} [summary]
 */
function renderDiffNavSummary(summary) {
  return `<div class="diff-nav-summary" aria-label="diff summary">
    <span><b>${summary.fileCount}</b> files</span>
    <span><b>${summary.factCount}</b> facts</span>
    <span><b>${summary.unanchoredCount}</b> unanchored</span>
  </div>`;
}

function renderDiffNavFilters() {
  return `<div class="diff-nav-filters" role="group" aria-label="diff navigator filters">
    <button type="button" data-diff-nav-filter="all" aria-pressed="${diffNavFilter === "all"}">all</button>
    <button type="button" data-diff-nav-filter="with-facts" aria-pressed="${diffNavFilter === "with-facts"}">with facts</button>
    <button type="button" data-diff-nav-filter="unanchored" aria-pressed="${diffNavFilter === "unanchored"}">unanchored</button>
  </div>`;
}

/**
 * @param {*} [filter]
 */
function setDiffNavFilter(filter) {
  if (!["all", "with-facts", "unanchored"].includes(filter)) return;
  diffNavFilter = filter;
  $("#diff-nav").innerHTML = renderDiffNav();
}

/**
 * @param {*} a
 * @param {any[]} filePaths
 */
function unanchoredReason(a, filePaths) {
  const t = a.target || {};
  if (a.kind === "assessment") return "broad assessment";
  if (t.kind === "revision" || !t.filePath) return "revision-level";
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  if (t.kind === "range" && filePaths && filePaths.has(t.filePath)) return "line outside captured rows";
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  if (t.filePath && filePaths && !filePaths.has(t.filePath)) return "file missing from snapshot";
  return "not anchored to a diff line";
}

// All rendered fact anchors in document order (inline annotations + unanchored
// bodies) — the ordering n/p cycles through.
function diffFactTargets() {
  return Array.from($("#diff-body").querySelectorAll(".anno[data-anno]"));
}

// All change anchors (hunk headers) in rendered file bodies — the ordering ]/[
// cycles through.
function diffChangeTargets() {
  return Array.from($("#diff-body").querySelectorAll(".dhunk"));
}

/**
 * @param {any[]} targets
 * @param {*} [cursor]
 * @param {*} [dir]
 */
function jumpToTarget(targets, cursor, dir) {
  if (!targets.length) return cursor;
  const next = (cursor + dir + targets.length) % targets.length;
  const el = targets[next];
  const section = el.closest(".dfile");
  if (section && !diffFileExpanded(section)) expandDiffFile(section);
  el.scrollIntoView({ block: "center" });
  return next;
}

/**
 * @param {*} [dir]
 */
function jumpFact(dir) {
  const targets = diffFactTargets();
  if (!targets.length) return;
  diffFactCursor = (diffFactCursor + dir + targets.length) % targets.length;
  const el = targets[diffFactCursor];
  if (el) {
    const section = el.closest(".dfile");
    if (section && !diffFileExpanded(section)) expandDiffFile(section);
    if (focusDiffFactRoute(el.dataset.anno)) return;
    el.scrollIntoView({ block: "center" });
    flashAnno(el);
  }
}

/**
 * @param {*} [dir]
 */
function jumpChange(dir) {
  diffChangeCursor = jumpToTarget(diffChangeTargets(), diffChangeCursor, dir);
}

/**
 * @param {*} [a]
 * @param {*} [showLocation]
 */
function renderAnnotation(a, showLocation) {
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  const tags = (a.tags || []).map((t) => `<span class="badge">${escapeHtml(t)}</span>`).join(" ");
  const body = renderBodyContent(a.body, a.bodyContentType);
  const t = a.target || {};
  const loc =
    showLocation && t.filePath
      ? `<span class="anno-loc">${escapeHtml(t.filePath)}${t.startLine ? `:${t.startLine}-${t.endLine || t.startLine}` : ""}</span>`
      : "";
  return `<div class="anno anno-${a.kind}" data-anno="${escapeHtml(a.id)}">
    <div class="anno-head"><span class="anno-kind anno-kind-${a.kind}">${a.kind}</span><span class="anno-track">${escapeHtml(a.track)}</span><span class="anno-title">${linkify(a.title)}</span> ${tags} ${loc}</div>${body}</div>`;
}

// How many file bodies render eagerly; the rest stay collapsed (empty body)
// until expanded, capping live DOM at the files a reader actually opens.
const DEFAULT_OPEN_FILES = 10;

// The display path for a diff file (a rename shows both sides).
/**
 * @param {*} [f]
 */
function filePathLabel(f) {
  const oldp = f.old_path;
  const newp = f.new_path;
  return oldp && newp && oldp !== newp ? `${oldp} → ${newp}` : newp || oldp || "(unknown path)";
}

// A file body over this many rows is treated as large/generated and collapsed
// by default (it carries little line-by-line review value relative to its size).
const LARGE_FILE_ROWS = 500;

/**
 * @param {*} [f]
 */
function fileRowCount(f) {
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  return (f.hunks || []).reduce((n, h) => n + (h.rows ? h.rows.length : 0), 0);
}

// Classify a file that carries no (or low) reviewable content, returning the
// reason string used both as the default-collapse signal and the collapsed
// one-line summary. `null` means a normal content-bearing file. The single
// source of the reason text (the body's no-content note reuses it).
/**
 * @param {*} [f]
 */
function classifyLowSignal(f) {
  if (f.is_binary) return "binary";
  if (f.is_mode_only) return "mode change only";
  const hunks = f.hunks || [];
  const renamed = f.status === "renamed" || (f.old_path && f.new_path && f.old_path !== f.new_path);
  if (renamed && !hunks.length) {
    return f.similarity != null ? `rename ${f.similarity}%` : "rename";
  }
  if (fileRowCount(f) > LARGE_FILE_ROWS) return "large file";
  return null;
}

// The anchored facts (range + file-level) that belong to one file. The single
// source of the per-file count the header badge and navigator both read.
/**
 * @param {*} f
 * @param {any[]} anchored
 */
function fileFactCount(f, anchored) {
  const oldp = f.old_path;
  const newp = f.new_path;
  let n = 0;
  for (const a of anchored) {
    const p = (a.target || {}).filePath;
    if (p === newp || p === oldp) n += 1;
  }
  return n;
}

/**
 * @param {any[]} files
 * @param {*} [filePath]
 */
function fileForFact(files, filePath) {
  return files.find((f) => f.new_path === filePath || f.old_path === filePath) || null;
}

/**
 * @param {*} [a]
 * @param {*} [file]
 */
function rangeTouchesCapturedRows(a, file) {
  if (!file) return false;
  const t = a.target || {};
  if (t.kind !== "range" || t.startLine == null) return true;
  const side = t.side === "old" ? "old" : "new";
  const end = t.endLine ?? t.startLine;
  for (const h of file.hunks || []) {
    for (const r of h.rows || []) {
      const line = side === "old" ? r.old_line : r.new_line;
      if (line != null && line >= t.startLine && line <= end) return true;
    }
  }
  return false;
}

/**
 * @param {*} f
 * @param {any[]} anchored
 */
function renderDiffFactVicinity(f, anchored) {
  const facts = anchored.filter((a) => {
    const p = (a.target || {}).filePath;
    return p === f.new_path || p === f.old_path;
  });
  return `<div class="diff-fact-vicinity" data-fact-vicinity="true">
    <p>Large annotated file: showing review facts first.</p>
    <button type="button" data-render-diff-file="true">Render all rows</button>
    ${facts.map((a) => renderAnnotation(a, true)).join("")}
  </div>`;
}

/**
 * @param {*} artifact
 * @param {any[]} annotations
 */
function renderDiff(artifact, annotations) {
  annotations = annotations || [];
  const files = (artifact.snapshot && artifact.snapshot.files) || [];
  const filePaths = new Set();
  for (const f of files) {
    if (f.new_path) filePaths.add(f.new_path);
    if (f.old_path) filePaths.add(f.old_path);
  }
  /** @type {any[]} */
  const anchored = [];
  const unanchored = [];
  for (const a of annotations) {
    const t = a.target || {};
    if ((t.kind === "range" || t.kind === "file") && t.filePath && filePaths.has(t.filePath)) {
      const file = fileForFact(files, t.filePath);
      if (t.kind === "range" && !rangeTouchesCapturedRows(a, file)) unanchored.push(a);
      else anchored.push(a);
    } else {
      unanchored.push(a);
    }
  }

  // The render inputs the delegated #diff-body / #diff-nav listeners read to
  // fill a lazy file body or scroll to a fact. NOT state.diff (that stays the
  // object-id string the route grammar serializes).
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  diffCtx = { objectId: shownDiffObject, files, anchored, unanchored, filePaths };
  diffFactCursor = -1;
  diffChangeCursor = -1;
  diffNavFilter = "all";

  // biome-ignore lint/complexity/noCommaOperator: legacy comma operator; refactored when this moves to a .ts module
  // biome-ignore lint/suspicious/noAssignInExpressions: legacy assign-in-expression; refactored when this moves to a .ts module
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

  // File-by-file accordion: every header renders eagerly; a file's hunks/rows
  // render lazily on first expand. Annotated files open by default, then a small
  // budget of the rest, so the live DOM stays bounded on a large changeset.
  // Low-signal files (binary / mode-only / pure-rename / large) collapse by
  // default — unless they carry a fact, which always wins so the fact is visible.
  let openBudget = DEFAULT_OPEN_FILES;
  html += files
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    .map((f, i) => {
      const reason = classifyLowSignal(f);
      const annotated = fileFactCount(f, anchored) > 0;
      const annotatedLarge = annotated && fileRowCount(f) > LARGE_FILE_ROWS;
      const open = (annotated && !annotatedLarge) || (reason ? false : openBudget-- > 0);
      const expanded = annotatedLarge || open;
      const body = annotatedLarge ? renderDiffFactVicinity(f, anchored) : open ? renderDiffFileBody(f, anchored) : "";
      const lowCls = reason ? " dfile-lowsignal" : "";
      const lowAttr = reason ? ` data-lowsignal="${escapeHtml(reason)}"` : "";
      return `<section class="dfile${lowCls}" data-dfile="${i}" data-expanded="${expanded}"${lowAttr}>${renderDiffFileHeader(f, anchored, reason, expanded)}<div class="dfile-body" data-dfile-body="${i}"${
        annotatedLarge ? ` data-fact-vicinity="true"` : open ? ` data-rendered="1"` : ""
      }>${body}</div></section>`;
    })
    .join("");
  return html;
}

// The eager file header: status + path + fact-count badge. Operable as a
// disclosure control (the delegated #diff-body listener toggles its section);
// CSS draws the caret from the section's internal state; the header carries the
// authoritative aria-expanded state for the disclosure control.
/**
 * @param {*} f
 * @param {any[]} anchored
 * @param {*} [reason]
 * @param {*} [open]
 */
function renderDiffFileHeader(f, anchored, reason, open) {
  const n = fileFactCount(f, anchored);
  const summary = reason ? `<span class="dfile-summary">${escapeHtml(reason)}</span>` : "";
  return `<header class="dfile-head" role="button" tabindex="0" aria-expanded="${open}">
    <span class="dstatus s-${escapeHtml(f.status)}">${escapeHtml(f.status)}</span>
    <span class="dpath">${escapeHtml(filePathLabel(f))}</span>${summary}
    ${n ? `<span class="dfile-notes">${n} note${n === 1 ? "" : "s"}</span>` : ""}</header>`;
}

// The lazy file body: file-level facts, metadata rows, and hunks/rows with their
// inline annotations. Built on first expand. Each body owns its own `emitted`
// Set — a fact belongs to exactly one file (fileFacts filters by path), so
// cross-file de-dup was never load-bearing.
/**
 * @param {*} f
 * @param {any[]} anchored
 */
function renderDiffFileBody(f, anchored) {
  const oldp = f.old_path;
  const newp = f.new_path;
  const fileFacts = anchored.filter((a) => {
    const p = (a.target || {}).filePath;
    return p === newp || p === oldp;
  });
  const rangeFacts = fileFacts.filter((a) => (a.target || {}).kind === "range");
  const fileLevelFacts = fileFacts.filter((a) => (a.target || {}).kind === "file");

  const emitted = new Set();
  let html = "";
  for (const a of fileLevelFacts) {
    html += renderAnnotation(a, false);
    emitted.add(a.id);
  }
  for (const m of f.metadata_rows || []) {
    html += `<div class="drow drow-meta"><span class="dtext">${escapeHtml(m.text)}</span></div>`;
  }

  // Bucket range facts by the (side, line) they anchor to, once per file —
  // O(facts) instead of an O(rows × facts) re-scan inside the row loop. The
  // anchoring rule: a fact on the "old" side keys against old_line, otherwise
  // against new_line, across its inclusive [startLine, endLine] line span.
  const factsByLine = new Map();
  for (const a of rangeFacts) {
    const t = a.target || {};
    if (t.startLine == null) continue;
    const side = t.side === "old" ? "old" : "new";
    const end = t.endLine ?? t.startLine;
    for (let line = t.startLine; line <= end; line++) {
      const key = `${side}:${line}`;
      const bucket = factsByLine.get(key);
      if (bucket) bucket.push(a);
      else factsByLine.set(key, [a]);
    }
  }

  const hunks = f.hunks || [];
  for (const h of hunks) {
    html += `<div class="dhunk">${escapeHtml(h.header)}</div>`;
    for (const r of h.rows || []) {
      // Look up this row's facts in O(1): a row matches a range fact on the
      // captured side whose line falls in [startLine, endLine].
      /** @type {any[]} */
      const matching = [];
      const seen = new Set();
      /**
       * @param {string} key
       */
      const collect = (key) => {
        const bucket = factsByLine.get(key);
        if (!bucket) return;
        for (const a of bucket)
          if (!seen.has(a.id)) {
            seen.add(a.id);
            matching.push(a);
          }
      };
      if (r.old_line != null) collect(`old:${r.old_line}`);
      if (r.new_line != null) collect(`new:${r.new_line}`);
      const sign = r.kind === "added" ? "+" : r.kind === "removed" ? "-" : " ";
      // An annotated row is a clickable gutter marker linking to its annotation.
      const notedLink = matching.length
        ? ` drow-noted" data-anno="${escapeHtml(matching[0].id)}" tabindex="0" role="button`
        : "";
      html += `<div class="drow drow-${escapeHtml(r.kind)}${notedLink}">
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

  // Safety fallback: if a range fact was classified as anchored but no rendered
  // row emitted it, surface it inside the file instead of dropping it.
  for (const a of rangeFacts) {
    if (!emitted.has(a.id)) {
      html += renderAnnotation(a, true);
      emitted.add(a.id);
    }
  }
  if (!hunks.length && !(f.metadata_rows || []).length) {
    // The collapsed header already surfaces any low-signal reason; in the body
    // only note files with no classifiable reason (e.g. an empty added file), so
    // the reason text is not double-printed.
    if (!classifyLowSignal(f)) {
      html += `<div class="drow drow-meta"><span class="dtext">(no captured content)</span></div>`;
    }
  }
  return html;
}

function renderUnits() {
  const el = $("#units");
  const entries = (state.units?.entries || []).filter(matchesRevisionFilters);
  if (!entries.length) {
    el.innerHTML = `<p class="empty" style="color:var(--fg-dim)">${state.filterText || state.filterObject ? "No revisions match the current filters." : "No captured revisions in this store."}</p>`;
    return;
  }
  el.innerHTML = "";
  for (const u of entries) {
    const base = u.base || {};
    const overview = u.overview || overviewForRevision(u.revisionId);
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
    /** @type {(pair: any[]) => string} */
    const kv = ([k, v]) => `<span>${escapeHtml(k)}</span><b>${escapeHtml(String(v))}</b>`;
    // The target cell carries pre-escaped derived HTML (label + head badge), so
    // it bypasses the generic escaping cell renderer rather than double-escaping.
    const targetCell = `<span>target</span><b>${targetDisplayLabel(u.targetDisplay)}${targetHeadBadge(u.targetDisplay)}</b>`;
    card.innerHTML = `
      <h3>${escapeHtml(shortId(u.revisionId))}</h3>
      ${badge ? `<div class="supersession-badges">${badge}</div>` : ""}
      ${renderRevisionOverview(u, overview)}
      <div class="kv">${rows.map(kv).join("")}${targetCell}${tail.map(kv).join("")}</div>`;
    card.title = u.revisionId + "\nclick to open the revision page";
    card.addEventListener("click", (ev) => {
      // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
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
      openDiff(u.objectId, null, u.objectArtifactContentHash);
    });
    actions.appendChild(diffBtn);
    card.appendChild(actions);
    wireOverviewCueFilters(card);
    el.appendChild(card);
  }
}

// One card per supersession thread (a connected component of the supersession
// DAG, labeled domain-side), rendering the revision DAG: every revision is
// marked head/superseded and carries its forward/reverse edges, so a fork shows
// as multiple heads (competing) rather than a single linear stack.
function renderRevisions() {
  const el = $("#revisions");
  const threads = objectThreads().filter(threadMatchesRevisionFilters);
  if (!threads.length) {
    el.innerHTML = `<p class="empty" style="color:var(--fg-dim)">${state.filterText || state.filterObject ? "No revision threads match the current filters." : "No captured revisions in this store."}</p>`;
    return;
  }
  el.innerHTML = "";
  for (const thread of threads) {
    el.appendChild(renderThreadCard(thread));
  }
}

/**
 * @param {*} [thread]
 */
function threadLabel(thread) {
  const heads = thread.heads || [];
  if (thread.competing) return `revision thread · ${heads.length} competing heads`;
  if (heads.length === 1) return `revision thread · current in thread ${shortId(heads[0])}`;
  return "revision thread";
}

/**
 * @param {*} [thread]
 */
function renderThreadCard(thread) {
  const revisions = thread.revisions || [];
  const heads = thread.heads || [];
  const superseded = thread.superseded || [];
  const card = document.createElement("div");
  card.className = "unit-card thread-card" + (thread.competing ? " competing" : "");
  // A fork surfaces every competing head as a navigable chip — never a null head.
  const competingBadge = thread.competing
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    ? `<div class="thread-competing"><span class="fact-status competing">competing revisions (${heads.length})</span> ${heads.map((h) => linkify(h)).join(" ")}</div>`
    : "";
  const overviewBlocks = heads.map(renderThreadRevisionOverview).filter(Boolean).join("");
  card.innerHTML = `
    <h3>${escapeHtml(threadLabel(thread))}</h3>
    ${competingBadge}
    ${overviewBlocks ? `<div class="thread-overviews">${overviewBlocks}</div>` : ""}
    <div class="kv">
      <span>revisions</span><b>${escapeHtml(String(revisions.length))}</b>
      <span>heads</span><b>${escapeHtml(String(heads.length))}</b>
      <span>superseded</span><b>${escapeHtml(String(superseded.length))}</b>
    </div>
    ${renderThreadSvg(thread.laidOut)}`;
  wireOverviewCueFilters(card);
  wireDagInteractions(card);
  return card;
}

// Pure painter of the server-laid geometry: nodes are <rect>+<text> groups
// keyed by revision id, edges are routed polylines. No client-side layout —
// every coordinate comes from `laidOut`, which is already normalized to a
// (0,0) origin, so the viewBox contains the whole graph with no clipping. Heads
// carry no centering/bold/sort-first (peer-equal); the head-vs-superseded shape
// cue lives in the CSS, not in color alone.
/**
 * @param {*} [laid]
 */
function renderThreadSvg(laid) {
  if (!laid || !(laid.nodes && laid.nodes.length)) return "";
  const w = laid.bounds.w;
  const h = laid.bounds.h;
  // Node centers, so each edge's arrowhead is oriented by node identity: the
  // arrow points at the superseding `from` head, never by raw points order (a
  // reversed/cycle edge still renders correctly).
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  const center = new Map(laid.nodes.map((n) => [n.id, [n.x, n.y]]));
  // Two shared arrowhead markers: a default (border-colored) and a traced
  // (accent-colored) one. CSS swaps a traced edge to the accent marker — a
  // cross-browser alternative to `context-stroke` (which Safari does not paint).
  /**
   * @param {string} id
   * @param {string} cls
   */
  const marker = (id, cls) =>
    `<marker id="${id}" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto" markerUnits="userSpaceOnUse"><path class="${cls}" d="M0,0 L7,4 L0,8 z" /></marker>`;
  const defs = `<defs>${marker("dag-arrow", "dag-arrow-head")}${marker("dag-arrow-traced", "dag-arrow-head-traced")}</defs>`;
  const edges = (laid.edges || [])
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    .map((e) => {
      // Draw so the LAST point is nearest the `from` (superseding head) node;
      // marker-end then points the arrowhead at that head, so succession reads
      // bottom-up (a fork diverges upward into its competing heads).
      let path = e.path;
      const from = center.get(e.from);
      if (from && path.length > 1) {
        /**
         * @param {number[]} p
         */
        const dist2 = (p) => (p[0] - from[0]) ** 2 + (p[1] - from[1]) ** 2;
        if (dist2(path[0]) < dist2(path[path.length - 1])) path = [...path].reverse();
      }
      // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
      const pts = path.map(([x, y]) => `${x},${y}`).join(" ");
      return `<polyline class="dag-edge" data-from="${escapeHtml(e.from)}" data-to="${escapeHtml(e.to)}" points="${pts}" marker-end="url(#dag-arrow)" />`;
    })
    .join("");
  const nodes = laid.nodes
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
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
  return `<svg class="revision-dag" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}" preserveAspectRatio="xMinYMin meet" role="group" aria-label="supersession graph">${defs}${edges}${nodes}</svg>`;
}

// Wire the DAG nodes into the IA: click / Enter / Space navigate to the
// revision via the router; hover/focus traces the node and its incident edges
// by class toggle (no re-render).
/**
 * @param {*} [card]
 */
function wireDagInteractions(card) {
  /**
   * @param {*} [node]
   */
  const nav = (node) => {
    const id = node.getAttribute("data-revision-id");
    if (id) navigate({ selected: { kind: "revision", id }, diff: null, diffHash: null, focus: null });
  };
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  card.querySelectorAll(".dag-node").forEach((node) => {
    node.addEventListener("click", () => nav(node));
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    node.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        nav(node);
      }
    });
    /**
     * @param {*} [on]
     */
    const trace = (on) => {
      const id = node.getAttribute("data-revision-id");
      node.classList.toggle("traced", on);
      card
        .querySelectorAll(`.dag-edge[data-from="${id}"], .dag-edge[data-to="${id}"]`)
        // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
        .forEach((edge) => {
          edge.classList.toggle("traced", on);
          // Swap the arrowhead to the accent marker via the marker-end attribute
          // (cross-browser; not CSS context paint) so it tracks the edge highlight.
          edge.setAttribute("marker-end", on ? "url(#dag-arrow-traced)" : "url(#dag-arrow)");
        });
    };
    node.addEventListener("mouseenter", () => trace(true));
    node.addEventListener("mouseleave", () => trace(false));
    node.addEventListener("focus", () => trace(true));
    node.addEventListener("blur", () => trace(false));
  });
}

// Mark the active lens on the switcher.
function renderLensSwitcher() {
  // biome-ignore lint/suspicious/useIterableCallbackReturn: legacy forEach side-effect callback; refactored when this moves to a .ts module
  document.querySelectorAll(".lens-tab").forEach((t) =>
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    t.setAttribute("aria-pressed", String(t.dataset.lens === state.lens)),
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
/** @type {*} */
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
/** @type {*} */
let shownCompositeId = null;
/**
 * @param {*} [revisionId]
 */
function showComposite(revisionId) {
  if (revisionId === shownCompositeId) return;
  shownCompositeId = revisionId;
  openUnit(revisionId);
}

/**
 * @param {*} [revisionId]
 */
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
      // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
      detail.innerHTML = `<p class="up-empty">error: ${escapeHtml(err.message)}</p>`;
    }
  }
}

/**
 * @param {*} [ca]
 */
function verdictBadge(ca) {
  const status = (ca && ca.status) || "unassessed";
  let value;
  let cls;
  if (status === "resolved") {
    value = assessmentDisplayLabel(ca.assessment);
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

/**
 * @param {*} [d]
 */
function currentAssessmentSummary(d) {
  const ca = d.currentAssessment || {};
  if (ca.status === "resolved" && ca.assessmentId) {
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    const a = (d.assessments || []).find((x) => x.id === ca.assessmentId);
    if (a && a.summary) {
      const cls = isMarkdownContentType(a.summaryContentType) ? "verdict-summary markdown-body" : "verdict-summary";
      return `<div class="${cls}">${renderContentHtml(a.summary, a.summaryContentType)}</div>`;
    }
  }
  if (ca.status === "ambiguous") {
    return `<div class="verdict-summary">${(ca.candidates || []).length} unreplaced assessments — see Assessments below.</div>`;
  }
  return "";
}

/**
 * @param {*} [t]
 */
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

/**
 * @param {*} [kind]
 * @param {*} [opts]
 */
function factCard(kind, opts) {
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  const tags = (opts.tags || []).filter(Boolean).map((t) => `<span class="badge">${escapeHtml(t)}</span>`).join(" ");
  const body = renderBodyContent(opts.body, opts.bodyContentType);
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

/**
 * @param {*} [o]
 */
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
    bodyContentType: o.bodyContentType,
    createdAt: o.createdAt,
    verify: verificationChip(o.verificationStatus),
    endorsements: endorsementsBlock(o.endorsements),
    extra,
  });
}

/**
 * @param {*} [ir]
 */
function renderInputRequestCard(ir) {
  const responses = (ir.responses || [])
    .map(
      // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
      (r) =>
        `<div class="fact-response"><span class="outcome">${escapeHtml(r.outcome)}</span>${r.reason ? renderBodyContent(r.reason, r.reasonContentType) : ""} ${verificationChip(r.verificationStatus)}${endorsementsBlock(r.endorsements)}</div>`,
    )
    .join("");
  return factCard("input-request", {
    track: ir.trackId,
    title: ir.title,
    status: ir.status,
    target: targetLabel(ir.target),
    tags: [ir.mode, ir.reasonCode],
    body: ir.body,
    bodyContentType: ir.bodyContentType,
    createdAt: ir.createdAt,
    verify: verificationChip(ir.verificationStatus),
    endorsements: endorsementsBlock(ir.endorsements),
    extra: responses ? `<div class="fact-responses">${responses}</div>` : "",
  });
}

/**
 * @param {*} [a]
 */
function renderAssessmentCard(a) {
  const rel = [];
  if ((a.replaces || []).length) rel.push(`replaces ${a.replaces.map(linkify).join(", ")}`);
  if ((a.relatedObservations || []).length) rel.push(`re ${a.relatedObservations.map(linkify).join(", ")}`);
  if ((a.relatedInputRequests || []).length) rel.push(`re ${a.relatedInputRequests.map(linkify).join(", ")}`);
  return factCard("assessment", {
    track: a.trackId,
    title: assessmentDisplayLabel(a.assessment),
    status: a.status,
    target: targetLabel(a.target),
    body: a.summary,
    bodyContentType: a.summaryContentType,
    createdAt: a.createdAt,
    verify: verificationChip(a.verificationStatus),
    endorsements: endorsementsBlock(a.endorsements),
    extra: rel.length ? `<div class="fact-rel">${rel.join(" · ")}</div>` : "",
  });
}

// Validation evidence is advisory: it renders with the shared factCard shape
// (status maps to .fact-status.<status>) but never as a verdict aggregate, and
// the unit-page section caption keeps it "context only".
/**
 * @param {*} [v]
 */
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
    bodyContentType: v.summaryContentType,
    createdAt: v.completedAt || v.createdAt,
    verify: verificationChip(v.verificationStatus),
    endorsements: endorsementsBlock(v.endorsements),
    extra: rel.length ? `<div class="fact-rel">${rel.join(" · ")}</div>` : "",
  });
}

/**
 * @param {*} [n]
 */
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

/**
 * @param {*} [revisionId]
 */
function staleFactSectionContext(revisionId) {
  const successors = supersededByRevision(revisionId);
  if (!successors.length) return "";
  return `<p class="fact-stale-context">superseded by ${successors.map(linkify).join(" ")}</p>`;
}

/**
 * @param {*} title
 * @param {any[]} items
 * @param {*} [render]
 * @param {*} [context]
 */
function factSection(title, items, render, context = "") {
  items = items || [];
  const body = items.length ? items.map(render).join("") : `<p class="up-empty">none</p>`;
  return `<section><h2>${escapeHtml(title)} (${items.length})</h2>${context}${body}</section>`;
}

/**
 * @param {*} [d]
 */
function renderUnitPage(d) {
  const ru = d.revision || {};
  const base = ru.base || {};
  const s = d.summary || {};
  const badge = supersessionBadge(ru.id);
  const title = `${shortId(ru.id)}${base.commitOid ? " · base " + shortId(base.commitOid) : ""}`;
  const staleContext = staleFactSectionContext(ru.id);

  /**
   * @param {string} label
   * @param {*} [n]
   */
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

  sections.push(factSection("Observations", d.observations, renderObservationCard, staleContext));
  sections.push(factSection("Input requests", d.inputRequests, renderInputRequestCard, staleContext));
  sections.push(factSection("Assessments", d.assessments, renderAssessmentCard, staleContext));

  // Validation checks: a first-class section after Assessments, rendered from
  // the document array (not raw events). Advisory-only — a context-only caption,
  // structurally separate from Current assessment, never a verdict aggregate.
  const validationChecks = d.validationChecks || [];
  const validationBody = validationChecks.length
    ? validationChecks.map(renderValidationCheckCard).join("") +
      `<p class="validation-note">context only — does not affect the current assessment</p>`
    : `<p class="up-empty">none</p>`;
  sections.push(`<section><h2>Validation checks (${validationChecks.length})</h2>${staleContext}${validationBody}</section>`);

  if ((d.adapterNotes || []).length) sections.push(factSection("Adapter notes", d.adapterNotes, renderAdapterNoteCard));

  $("#detail").innerHTML = `<div class="unit-page"><p class="unit-page-title">${escapeHtml(title)}</p>${sections.join("")}</div>`;

  const diffBtn = $("#up-diff-btn");
  if (diffBtn && ru.objectId) {
    diffBtn.addEventListener("click", () =>
      openDiff(ru.objectId, null, ru.objectArtifactContentHash),
    );
  }
  const tlBtn = $("#up-timeline-btn");
  if (tlBtn) tlBtn.addEventListener("click", () => navigateToUnit(ru.id));
}

/**
 * @param {*} [s]
 * @returns {string}
 */
function escapeHtml(s) {
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
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
//   ?diff=<objectId> ?focus=<factId> ?diffHash=<objectArtifactContentHash>
//                                  the route-preserving diff overlay
//   ?v=1                          grammar version (reserved)
//   ?journal= ?asof=             reserved: reported as unsupported live-state input
// ---------------------------------------------------------------------------
const LENSES = ["timeline", "list", "threads"];
const DEFAULT_LENS = "timeline";

// Guards re-deriving the view from a fragment this router just wrote.
// biome-ignore lint/correctness/noUnusedVariables: legacy write-only field; removed when this moves to a .ts module
let routerLastHash = null;

// Classify a selection id as a revision or an event (a `rev:`/`review-unit:` id
// is a revision; anything else is treated as an event).
/**
 * @param {*} [id]
 */
function selectionKind(id) {
  const info = refInfo(id);
  if (info && (info.kind === "rev" || info.kind === "review-unit")) return "revision";
  return "event";
}

/**
 * @param {*} [queryString]
 */
function parseQuery(queryString) {
  const params = {};
  for (const pair of String(queryString || "").split("&")) {
    if (!pair) continue;
    const eq = pair.indexOf("=");
    const key = decodeURIComponent(eq < 0 ? pair : pair.slice(0, eq));
    const value = eq < 0 ? "" : decodeURIComponent(pair.slice(eq + 1));
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    params[key] = value;
  }
  return params;
}

// Parse a fragment into a complete state patch. Absent params resolve to their
// defaults so the fragment fully determines the filter/selection state (so
// Back/Forward to a barer fragment clears what it omits).
/**
 * @param {*} [hash]
 */
function parseHash(hash) {
  const raw = String(hash || "").replace(/^#/, "");
  const q = raw.indexOf("?");
  const path = q < 0 ? raw : raw.slice(0, q);
  const p = parseQuery(q < 0 ? "" : raw.slice(q + 1));

  const patch = {
    selected: { kind: null, id: null },
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    filterTrack: p.track != null ? p.track : "",
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    filterObject: p.object != null ? p.object : "",
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    order: p.order === "asc" || p.order === "desc" ? p.order : "desc",
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    filterText: p.q != null ? p.q : "",
    enabledTypes:
      // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
      p.types != null ? new Set(p.types.split(",").filter(Boolean)) : new Set(presentTypes()),
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    diff: p.diff || null,
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    diffHash: p.diffHash || null,
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    focus: p.focus ? p.focus : null,
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    unsupportedAsOf: p.asof != null ? p.asof || true : null,
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    unsupportedJournal: p.journal != null ? p.journal || true : null,
  };

  const segs = path.split("/").filter(Boolean); // "/timeline" -> ["timeline"]
  if (segs.length === 0) {
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    patch.lens = DEFAULT_LENS;
  } else if (segs[0] === "revision" && segs[1]) {
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    patch.selected = { kind: "revision", id: decodeURIComponent(segs[1]) };
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    patch.lens = LENSES.includes(p.lens) ? p.lens : DEFAULT_LENS;
  } else if (segs[0] === "event" && segs[1]) {
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    patch.selected = { kind: "event", id: decodeURIComponent(segs[1]) };
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    patch.lens = LENSES.includes(p.lens) ? p.lens : DEFAULT_LENS;
  } else if (LENSES.includes(segs[0])) {
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    patch.lens = segs[0];
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    if (p.sel) patch.selected = { kind: selectionKind(p.sel), id: p.sel };
  } else {
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    patch.lens = DEFAULT_LENS;
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
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
  if (state.diff && state.diffHash) params.push("diffHash=" + encodeURIComponent(state.diffHash));
  if (state.focus) params.push("focus=" + encodeURIComponent(state.focus));
  return params.length ? path + "?" + params.join("&") : path;
}

// The single mutation + history + render choke point. Distinct navigations push
// history; refinements (search-as-you-type, cursor steps) replace it.
/**
 * @param {*} [patch]
 * @param {*} [opts]
 */
function navigate(patch, opts) {
  opts = opts || {};
  Object.assign(state, patch);
  if (!state.selected) state.selected = { kind: null, id: null };
  if (!state.diff) state.diffHash = null;
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
/**
 * @param {*} [patch]
 */
function resolve(patch) {
  const freshnessDiagnostic = liveStateDiagnostic(patch);
  if (patch.unknownPath != null) {
    showRouteDiagnostic(
      routeDiagnostic("fell back to the timeline — unknown route " + patch.unknownPath, freshnessDiagnostic),
    );
    patch.lens = DEFAULT_LENS;
    patch.selected = { kind: null, id: null };
    delete patch.unknownPath;
    return patch;
  }
  const sel = patch.selected || { kind: null, id: null };
  if (sel.kind === "revision" && sel.id && !revisionExists(sel.id)) {
    if (revisionInAnyThread(sel.id)) {
      showRouteDiagnostic(
        routeDiagnostic(
          "fell back to the threads lens — revision " + shortRef(sel.id) + " is not directly selectable",
          freshnessDiagnostic,
        ),
      );
      patch.lens = "threads";
    } else {
      // Keep the requested lens (only the selection was absent); name it in the
      // diagnostic so the message matches the lens actually shown.
      const lens = patch.lens || DEFAULT_LENS;
      showRouteDiagnostic(
        routeDiagnostic(
          "fell back to the " + lens + " lens — revision " + shortRef(sel.id) + " is not in this store",
          freshnessDiagnostic,
        ),
      );
      patch.lens = lens;
    }
    patch.selected = { kind: null, id: null };
    return patch;
  }
  if (sel.kind === "event" && sel.id && !eventExists(sel.id)) {
    showRouteDiagnostic(
      routeDiagnostic(
        "fell back to the " + (patch.lens || DEFAULT_LENS) + " lens — event " + shortRef(sel.id) + " is not in this store",
        freshnessDiagnostic,
      ),
    );
    patch.selected = { kind: null, id: null };
    return patch;
  }
  if (freshnessDiagnostic) {
    showRouteDiagnostic(freshnessDiagnostic);
    return patch;
  }
  clearRouteDiagnostic();
  return patch;
}

/**
 * @param {*} [patch]
 */
function liveStateDiagnostic(patch) {
  const unsupported = [];
  if (patch.unsupportedAsOf != null) unsupported.push("as-of links are not supported by this server");
  if (patch.unsupportedJournal != null) unsupported.push("journal links are not supported by this server");
  delete patch.unsupportedAsOf;
  delete patch.unsupportedJournal;
  return unsupported.length ? "showing live state — " + unsupported.join("; ") : "";
}

/**
 * @param {*} [primary]
 * @param {*} [secondary]
 */
function routeDiagnostic(primary, secondary) {
  return secondary ? primary + " — " + secondary : primary;
}

/**
 * @param {*} [id]
 */
function revisionExists(id) {
  return (state.units?.entries || []).some((u) => u.revisionId === id);
}
/**
 * @param {*} [id]
 */
function revisionInAnyThread(id) {
  return objectThreads().some((t) => (t.revisions || []).includes(id));
}
/**
 * @param {*} [id]
 */
function eventExists(id) {
  return (state.history?.entries || []).some((e) => e.eventId === id);
}

/**
 * @param {string} message
 */
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
/**
 * @param {*} [el]
 */
function isTypingTarget(el) {
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || el.isContentEditable;
}

// The ordered selectable entries of the active lens, for cursor stepping. Pure:
// reads state, returns { kind, id } in the lens's display order.
function lensEntryIds() {
  if (state.lens === "list") {
    return (state.units?.entries || [])
      .filter(matchesRevisionFilters)
      .map((u) => ({ kind: "revision", id: u.revisionId }));
  }
  if (state.lens === "threads") {
    const ids = [];
    for (const t of objectThreads().filter(threadMatchesRevisionFilters)) {
      for (const r of filteredThreadRevisionIds(t, threadRevisionOrder(t))) ids.push({ kind: "revision", id: r });
    }
    return ids;
  }
  let entries = (state.history?.entries || []).filter(matchesFilters);
  if (state.order === "desc") entries = entries.slice().reverse();
  return entries.map((e) => ({ kind: "event", id: e.eventId }));
}

// Move the selection by delta within the active lens (replaceState — stepping a
// cursor is a refinement, not a distinct navigation).
/**
 * @param {*} [delta]
 */
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
    openRevisionDiff(sel.id);
  } else if (sel.kind === "event") {
    const rev = entryRevisionId((state.history?.entries || []).find((e) => e.eventId === sel.id) || {});
    if (rev) openRevisionDiff(rev);
  }
}

function focusSearch() {
  if (state.lens !== "timeline") navigate({ lens: "timeline" });
  const box = $("#filter-text");
  if (box) box.focus();
}

function toggleKeyHelp() {
  const help = $("#key-help");
  if (!help) return;
  if (!help.classList.contains("hidden")) closeKeyHelp();
  else openKeyHelp();
}

function openKeyHelp() {
  if (cmdOpen) closePalette({ restoreFocus: false });
  if (state.diff) closeDiff();
  openOverlay("help", "#key-help-close");
}

/**
 * @param {*} [opts]
 */
function closeKeyHelp(opts = {}) {
  closeOverlay("help", opts);
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
    closeDiff();
    return;
  }
  const help = $("#key-help");
  if (help && !help.classList.contains("hidden")) {
    closeKeyHelp();
    return;
  }
  const active = document.activeElement;
  if (isTypingTarget(active)) {
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    active.blur();
    return;
  }
  if (state.filterText) navigate({ filterText: "" }, { replace: true });
}

// A short-lived two-key chord (g-then-…). Cleared after ~1s.
/** @type {*} */
let pendingChord = null;
/** @type {*} */
let chordTimer = null;
/**
 * @param {*} [key]
 */
function setChord(key) {
  pendingChord = key;
  if (chordTimer) clearTimeout(chordTimer);
  chordTimer = setTimeout(() => {
    pendingChord = null;
  }, 1000);
}

/**
 * @param {*} [ev]
 */
function onKey(ev) {
  if (trapOverlayFocus(ev)) return;
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

  // Diff-local jumps, active only while the overlay is open (layered Escape
  // already closes it): ]/[ step changes, n/p step review facts.
  if (state.diff) {
    if (ev.key === "]") {
      ev.preventDefault();
      jumpChange(1);
      return;
    }
    if (ev.key === "[") {
      ev.preventDefault();
      jumpChange(-1);
      return;
    }
    if (ev.key === "n") {
      ev.preventDefault();
      jumpFact(1);
      return;
    }
    if (ev.key === "p") {
      ev.preventDefault();
      jumpFact(-1);
      return;
    }
  }

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
/** @type {any[]} */
let cmdItems = [];
/** @type {any[]} */
let cmdFiltered = [];
let cmdActive = 0;

/**
 * @param {string} text
 */
function copyText(text) {
  if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(text);
}

function copyCurrentViewLink() {
  copyText(location.origin + location.pathname + serializeState());
}

/**
 * @param {any[]} cmds
 */
function assignCommandOptionIds(cmds) {
  cmds.forEach((cmd, index) => {
    cmd.domIndex = index;
  });
  return cmds;
}

// The candidate commands, built over the loaded state. (When a search index
// exists, jumps query it instead of re-deriving — the source is a single fn.)
function buildCommands() {
  const cmds = [];
  cmds.push({ kind: "Actions", label: "Copy current view link", hint: "share", run: copyCurrentViewLink });
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
    label: "Copy selected id",
    hint: "clipboard",
    run: () => {
      if (state.selected.id) copyText(state.selected.id);
    },
  });

  const current = currentSelectionCommand();
  if (current) cmds.push(current);

  for (const u of sortedRevisionEntriesForCommands()) {
    cmds.push({
      kind: "Revisions",
      label: revisionCommandLabel(u),
      hint: revisionCommandHint(u),
      run: () =>
        navigate({
          selected: { kind: "revision", id: u.revisionId },
          diff: null,
          diffHash: null,
          focus: null,
        }),
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
      run: () =>
        navigate({
          selected: { kind: "event", id: e.eventId },
          diff: null,
          diffHash: null,
          focus: null,
        }),
    });
  }
  return assignCommandOptionIds(cmds);
}

function selectedRevisionId() {
  if (state.selected.kind === "revision") return state.selected.id;
  if (state.selected.kind === "event") {
    const event = (state.history?.entries || []).find((e) => e.eventId === state.selected.id);
    return event ? entryRevisionId(event) : "";
  }
  return "";
}

function currentSelectionCommand() {
  if (!state.selected.id) return null;
  if (state.selected.kind === "revision") {
    const unit = unitForRevision(state.selected.id);
    return {
      kind: "Current",
      label: "Open current selection",
      hint: unit ? revisionCommandLabel(unit) : shortRef(state.selected.id),
      run: () =>
        navigate({
          selected: { kind: "revision", id: state.selected.id },
          diff: null,
          diffHash: null,
          focus: null,
        }),
    };
  }
  if (state.selected.kind === "event") {
    const event = (state.history?.entries || []).find((e) => e.eventId === state.selected.id);
    return {
      kind: "Current",
      label: "Open current selection",
      hint: event ? entryTitle(event) : shortRef(state.selected.id),
      run: () =>
        navigate({
          selected: { kind: "event", id: state.selected.id },
          diff: null,
          diffHash: null,
          focus: null,
        }),
    };
  }
  return null;
}

function sortedRevisionEntriesForCommands() {
  const selectedRevision = selectedRevisionId();
  return [...(state.units?.entries || [])].sort((left, right) => {
    if (left.revisionId === selectedRevision) return -1;
    if (right.revisionId === selectedRevision) return 1;
    return String(right.capturedAt || "").localeCompare(String(left.capturedAt || "")) || String(right.revisionId).localeCompare(String(left.revisionId));
  });
}

/**
 * @param {*} [u]
 */
function revisionCommandLabel(u) {
  const targetDisplay = u.targetDisplay || {};
  const overview = u.overview || {};
  const current = overview.currentAssessment || {};
  const target = targetDisplay.label || shortId(u.revisionId);
  const assessment = current.assessment ? assessmentLabel(current.assessment) : current.status || "unassessed";
  return `${target} · ${assessment} · ${shortId(u.revisionId)}`;
}

/**
 * @param {*} [u]
 */
function revisionCommandHint(u) {
  const overview = u.overview || {};
  const cues = attentionTokens(overview).map((cue) => cue.label);
  const latest = overview.latestActivity?.title;
  return [cues.join(", ") || "review context", latest, shortId(u.objectId)].filter(Boolean).join(" · ");
}

function togglePalette() {
  if (cmdOpen) closePalette();
  else openPalette();
}

function openPalette() {
  if (state.diff) closeDiff();
  closeKeyHelp({ restoreFocus: false });
  cmdOpen = true;
  cmdItems = buildCommands();
  const input = $("#cmd-input");
  input.value = "";
  filterPalette("");
  openOverlay("palette", "#cmd-input");
}

/**
 * @param {*} [opts]
 */
function closePalette(opts = {}) {
  cmdOpen = false;
  closeOverlay("palette", opts);
}

/**
 * @param {*} [query]
 */
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
  const input = $("#cmd-input");
  if (!cmdFiltered.length) {
    list.innerHTML = `<li id="cmd-option-empty" class="cmd-empty" role="option" aria-disabled="true">No matches</li>`;
    input.setAttribute("aria-activedescendant", "cmd-option-empty");
    return;
  }
  let html = "";
  /** @type {*} */
  let lastKind = null;
  cmdFiltered.forEach((c, i) => {
    if (c.kind !== lastKind) {
      lastKind = c.kind;
      html += `<li class="cmd-group" role="presentation">${escapeHtml(c.kind)}</li>`;
    }
    html += `<li id="cmd-option-${escapeHtml(String(c.domIndex ?? i))}" class="cmd-item${i === cmdActive ? " active" : ""}" role="option" data-idx="${i}" aria-selected="${i === cmdActive}"><span class="cmd-label">${escapeHtml(c.label)}</span>${c.hint ? `<span class="cmd-hint">${escapeHtml(c.hint)}</span>` : ""}</li>`;
  });
  list.innerHTML = html;
  const active = list.querySelector(".cmd-item.active");
  if (active) {
    input.setAttribute("aria-activedescendant", active.id);
    active.scrollIntoView({ block: "nearest" });
  }
}

/**
 * @param {*} [delta]
 */
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
  // biome-ignore lint/suspicious/useIterableCallbackReturn: legacy forEach side-effect callback; refactored when this moves to a .ts module
  document.querySelectorAll(".lens-tab").forEach((tab) =>
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
    tab.addEventListener("click", () => navigate({ lens: LENSES.includes(tab.dataset.lens) ? tab.dataset.lens : DEFAULT_LENS })),
  );
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
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
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  $("#diff-modal").addEventListener("click", (ev) => {
    if (ev.target === $("#diff-modal")) closeDiff();
  });
  // One delegated listener for the diff body (installed once, reads the
  // module-local diffCtx renderDiff sets) — never wired at the openDiff call
  // site, which stays route-only. A file header toggles its section; an
  // annotated row's gutter scrolls to its annotation.
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  $("#diff-body").addEventListener("click", (ev) => {
    const renderAll = ev.target.closest("[data-render-diff-file]");
    if (renderAll) {
      const section = renderAll.closest(".dfile");
      if (section) {
        ensureDiffFileBody(section);
        setDiffFileExpanded(section, true);
      }
      return;
    }
    const head = ev.target.closest(".dfile-head");
    if (head) {
      toggleDiffFile(head.closest(".dfile"));
      return;
    }
    const noted = ev.target.closest(".drow-noted[data-anno]");
    if (noted) scrollToAnno(noted.dataset.anno, { updateRoute: true });
  });
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  $("#diff-body").addEventListener("keydown", (ev) => {
    if (ev.key !== "Enter" && ev.key !== " ") return;
    const head = ev.target.closest(".dfile-head");
    if (head) {
      ev.preventDefault();
      toggleDiffFile(head.closest(".dfile"));
      return;
    }
    const noted = ev.target.closest(".drow-noted[data-anno]");
    if (noted) {
      ev.preventDefault();
      scrollToAnno(noted.dataset.anno, { updateRoute: true });
    }
  });
  // The navigator sidebar: a file entry expands + scrolls its section; an
  // unanchored-fact entry scrolls to its annotation body.
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  $("#diff-nav").addEventListener("click", (ev) => {
    const filterBtn = ev.target.closest("[data-diff-nav-filter]");
    if (filterBtn) {
      setDiffNavFilter(filterBtn.dataset.diffNavFilter);
      return;
    }
    const fileBtn = ev.target.closest("[data-nav-file]");
    if (fileBtn) {
      const idx = Number(fileBtn.dataset.navFile);
      const section = $("#diff-body").querySelector(`.dfile[data-dfile="${idx}"]`);
      if (section) {
        expandDiffFile(section);
        section.scrollIntoView({ block: "start" });
      }
      return;
    }
    const factBtn = ev.target.closest(".diff-nav-fact[data-anno]");
    if (factBtn) scrollToAnno(factBtn.dataset.anno, { updateRoute: true });
  });
  $("#key-help-close").addEventListener("click", () => closeKeyHelp());
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  $("#key-help").addEventListener("click", (ev) => {
    if (ev.target === $("#key-help")) closeKeyHelp();
  });
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
  $("#cmd-input").addEventListener("input", (ev) => filterPalette(ev.target.value));
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
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
  // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
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
    // @ts-expect-error legacy DOM/dynamic access; typed when this moves to a .ts module
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
