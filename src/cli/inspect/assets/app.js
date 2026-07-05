"use strict";
(() => {
  var __defProp = Object.defineProperty;
  var __name = (target, value) => __defProp(target, "name", { value, configurable: true });

  // src/dom.ts
  function $(sel) {
    return document.querySelector(sel);
  }
  __name($, "$");

  // src/http.ts
  function payloadError(data) {
    if (typeof data === "object" && data !== null && "error" in data && data.error) {
      return typeof data.error === "string" ? data.error : String(data.error);
    }
    return "";
  }
  __name(payloadError, "payloadError");
  async function fetchJSON(path) {
    const res = await fetch(path, { cache: "no-store" });
    const text = await res.text();
    let data;
    try {
      data = JSON.parse(text);
    } catch {
      throw new Error(`${path}: non-JSON response (${res.status})`);
    }
    const error = payloadError(data);
    if (!res.ok || error) {
      throw new Error(error || `${path}: HTTP ${res.status}`);
    }
    return data;
  }
  __name(fetchJSON, "fetchJSON");

  // src/classNames.ts
  var CLASS = {
    // App chrome, master-detail panes, lens containers, and shared chips.
    units: "units",
    timeline: "timeline",
    empty: "empty",
    badge: "badge",
    body: "body",
    title: "title",
    time: "time",
    rail: "rail",
    meta: "meta",
    type: "type",
    typeCount: "type-count",
    code: "code",
    dot: "dot",
    kv: "kv",
    ghost: "ghost",
    actions: "actions",
    // Fact cards (observation / input-request / assessment / validation / note).
    annoGroup: "anno-group",
    annoHead: "anno-head",
    annoLoc: "anno-loc",
    annoSummary: "anno-summary",
    annoTime: "anno-time",
    annoTitle: "anno-title",
    annoTrack: "anno-track",
    factBodyRemoved: "fact-body-removed",
    factRel: "fact-rel",
    factResponse: "fact-response",
    factResponses: "fact-responses",
    factStaleContext: "fact-stale-context",
    factStatus: "fact-status",
    outcome: "outcome",
    advisoryNote: "advisory-note",
    validationNote: "validation-note",
    readback: "readback",
    readbackRow: "readback-row",
    readerScopeNote: "reader-scope-note",
    // The current-assessment verdict block.
    verdictStatus: "verdict-status",
    verdictSummary: "verdict-summary",
    verdictValue: "verdict-value",
    // The advisory endorsement readback.
    endorseAttrs: "endorse-attrs",
    endorseLabel: "endorse-label",
    endorseList: "endorse-list",
    endorseWho: "endorse-who",
    endorsements: "endorsements",
    endorsementsLabel: "endorsements-label",
    // The revision-overview summary line.
    overviewAssessment: "overview-assessment",
    overviewCue: "overview-cue",
    overviewCues: "overview-cues",
    overviewLabel: "overview-label",
    overviewLatest: "overview-latest",
    overviewMain: "overview-main",
    overviewMuted: "overview-muted",
    overviewStat: "overview-stat",
    overviewStats: "overview-stats",
    overviewSummary: "overview-summary",
    // The annotated snapshot diff: files, rows, and the navigator.
    dfileBody: "dfile-body",
    dfileHead: "dfile-head",
    dfileNotes: "dfile-notes",
    dfileSummary: "dfile-summary",
    dhunk: "dhunk",
    diffBtn: "diff-btn",
    diffFactVicinity: "diff-fact-vicinity",
    diffNavFact: "diff-nav-fact",
    diffNavFile: "diff-nav-file",
    diffNavFiles: "diff-nav-files",
    diffNavFilters: "diff-nav-filters",
    diffNavReason: "diff-nav-reason",
    diffNavSummary: "diff-nav-summary",
    diffUnanchored: "diff-unanchored",
    dpath: "dpath",
    drow: "drow",
    drowMeta: "drow-meta",
    dtext: "dtext",
    emph: "emph",
    ln: "ln",
    sign: "sign",
    // Revision list, supersession threads, and the laid-out DAG.
    unitCard: "unit-card",
    unitPage: "unit-page",
    unitPageTitle: "unit-page-title",
    supersessionBadges: "supersession-badges",
    threadCompeting: "thread-competing",
    threadOverview: "thread-overview",
    threadOverviews: "thread-overviews",
    competing: "competing",
    dagEdge: "dag-edge",
    dagArrowHead: "dag-arrow-head",
    dagArrowHeadTraced: "dag-arrow-head-traced",
    revisionDag: "revision-dag",
    factDag: "fact-dag",
    head: "head",
    stale: "stale",
    superseded: "superseded",
    supersedes: "supersedes",
    upEmpty: "up-empty",
    upIdentity: "up-identity",
    upStat: "up-stat",
    upStats: "up-stats",
    // The command palette.
    cmdEmpty: "cmd-empty",
    cmdGroup: "cmd-group",
    cmdHint: "cmd-hint",
    cmdLabel: "cmd-label"
  };
  var ANNO_KINDS = [
    "observation",
    "assessment",
    "input-request",
    "validation"
  ];
  var DIFF_ROW_KINDS = ["added", "removed", "context"];
  var TOKEN_KINDS = [
    "keyword",
    "string",
    "comment",
    "number",
    "type",
    "function",
    "constant",
    "operator",
    "punctuation",
    "variable"
  ];
  var DIFF_FILE_STATUSES = [
    "added",
    "deleted",
    "modified",
    "renamed",
    "copied"
  ];
  var VERIFY_STATUSES = [
    "valid",
    "invalid",
    "unsigned",
    "untrusted_key"
  ];
  var ENDORSE_CLASSES = [
    "endorsement-trusted",
    "ambiguous_endorser",
    "unknown_endorser"
  ];
  var VERDICT_ASSESSMENTS = [
    "accepted",
    "accepted_with_follow_up",
    "ambiguous",
    "needs_changes",
    "needs_clarification",
    "unassessed"
  ];
  var FACT_STATUSES = [
    "accepted",
    "accepted_with_follow_up",
    "ambiguous",
    "current",
    "errored",
    "failed",
    "needs_changes",
    "needs_clarification",
    "open",
    "passed",
    "replaced",
    "resolved",
    "responded",
    "skipped",
    "stale",
    "superseded",
    "unassessed"
  ];
  var REF_ID_PREFIXES = [
    "input-request-response",
    "input-request",
    "obs",
    "assess",
    "rev",
    "evt",
    "note",
    "validation",
    "obj",
    "engagement",
    "checkpoint",
    "task-attempt",
    "assoc-commit",
    "assoc-ref",
    "withdraw-commit",
    "withdraw-ref"
  ];
  var REF_KINDS = [
    ...REF_ID_PREFIXES,
    "hash",
    "commit",
    "track"
  ];
  var annoContainerClass = /* @__PURE__ */ __name((kind) => `anno anno-${kind}`, "annoContainerClass");
  var annoKindClass = /* @__PURE__ */ __name((kind) => `anno-kind anno-kind-${kind}`, "annoKindClass");
  var drowClass = /* @__PURE__ */ __name((kind, noted) => `drow drow-${kind}${noted ? " drow-noted" : ""}`, "drowClass");
  var tokClass = /* @__PURE__ */ __name((kind) => `tok tok-${kind}`, "tokClass");
  var diffStatusClass = /* @__PURE__ */ __name((status) => `dstatus s-${status}`, "diffStatusClass");
  var verifyClass = /* @__PURE__ */ __name((status) => `verify verify-${status}`, "verifyClass");
  var endorseClass = /* @__PURE__ */ __name((cls) => `endorse endorse-${cls}`, "endorseClass");
  var verdictClass = /* @__PURE__ */ __name((assessment) => `verdict verdict-${assessment}`, "verdictClass");
  var factStatusClass = /* @__PURE__ */ __name((status) => `fact-status ${status}`, "factStatusClass");
  var refClass = /* @__PURE__ */ __name((kind) => `ref ref-${kind}`, "refClass");
  var dfileClass = /* @__PURE__ */ __name((lowSignal) => `dfile${lowSignal ? " dfile-lowsignal" : ""}`, "dfileClass");
  var dagNodeClass = /* @__PURE__ */ __name((o) => `dag-node${o.isHead ? " head" : ""}${o.isSuperseded ? " superseded" : ""}`, "dagNodeClass");
  var bodyClass = /* @__PURE__ */ __name((base, markdown) => `${base}${markdown ? " markdown-body" : ""}`, "bodyClass");
  var cmdItemClass = /* @__PURE__ */ __name((active) => `cmd-item${active ? " active" : ""}`, "cmdItemClass");
  var tokensOf = /* @__PURE__ */ __name((classStrings) => classStrings.flatMap((s) => s.split(" ")), "tokensOf");
  var ALL_EMITTABLE_CLASSES = [
    ...new Set(
      tokensOf([
        ...Object.values(CLASS),
        ...ANNO_KINDS.map((k) => annoContainerClass(k)),
        ...ANNO_KINDS.map((k) => annoKindClass(k)),
        ...DIFF_ROW_KINDS.map((k) => drowClass(k, true)),
        ...TOKEN_KINDS.map((k) => tokClass(k)),
        ...DIFF_FILE_STATUSES.map((s) => diffStatusClass(s)),
        ...VERIFY_STATUSES.map((s) => verifyClass(s)),
        ...ENDORSE_CLASSES.map((c) => endorseClass(c)),
        ...VERDICT_ASSESSMENTS.map((a) => verdictClass(a)),
        ...FACT_STATUSES.map((s) => factStatusClass(s)),
        ...REF_KINDS.map((k) => refClass(k)),
        dfileClass(true),
        dagNodeClass({ isHead: true, isSuperseded: true }),
        bodyClass("anno-body", true),
        bodyClass("verdict-summary", true),
        cmdItemClass(true)
      ])
    )
  ];

  // src/escape.ts
  var ENTITIES = {
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;"
  };
  function escapeHtml(value) {
    return String(value).replace(/[&<>"']/g, (char) => ENTITIES[char]);
  }
  __name(escapeHtml, "escapeHtml");

  // src/format.ts
  function parseMs(occurredAt) {
    if (typeof occurredAt !== "string") return null;
    const match = occurredAt.match(/(\d+)\s*$/);
    return match ? Number(match[1]) : null;
  }
  __name(parseMs, "parseMs");
  function fmtTime(occurredAt) {
    const ms = parseMs(occurredAt);
    if (ms == null) return occurredAt || "";
    const date = new Date(ms);
    return `${date.toLocaleTimeString([], { hour12: false })}.${String(ms % 1e3).padStart(3, "0")}`;
  }
  __name(fmtTime, "fmtTime");
  function fmtDateTime(occurredAt) {
    const ms = parseMs(occurredAt);
    if (ms == null) return occurredAt || "";
    return new Date(ms).toLocaleString([], { hour12: false });
  }
  __name(fmtDateTime, "fmtDateTime");

  // src/refs.ts
  function shortId(id) {
    if (!id) return "";
    const tail = String(id).split(":").pop() || "";
    return tail.length > 12 ? tail.slice(0, 12) : tail;
  }
  __name(shortId, "shortId");
  function shortRef(id) {
    const value = String(id);
    let match = value.match(
      /^([a-z][a-z-]*):(?:git:|worktree:)?sha256:([0-9a-f]{6,})$/i
    );
    if (match) return `${match[1]}:${match[2].slice(0, 8)}`;
    match = value.match(/^sha256:([0-9a-f]{8,})$/i);
    if (match) return `sha256:${match[1].slice(0, 8)}`;
    if (/^[0-9a-f]{40}$/i.test(value)) return value.slice(0, 10);
    return value;
  }
  __name(shortRef, "shortRef");
  function targetDisplayLabel(td) {
    if (!td) return "working tree";
    return escapeHtml(td.label || "working tree");
  }
  __name(targetDisplayLabel, "targetDisplayLabel");
  function targetHeadBadge(td) {
    const head = td?.head;
    if (!head?.label) return "";
    let inner = `@ ${escapeHtml(head.label)}`;
    if (head.liveBranch) inner += ` · ${escapeHtml(head.liveBranch)} (current)`;
    return ` <span class="${CLASS.badge}">${inner}</span>`;
  }
  __name(targetHeadBadge, "targetHeadBadge");
  var NON_CLICKABLE_KINDS = /* @__PURE__ */ new Set([
    "validation",
    "obj",
    "engagement",
    "checkpoint",
    "task-attempt",
    "assoc-commit",
    "assoc-ref",
    "withdraw-commit",
    "withdraw-ref"
  ]);
  function refInfo(token) {
    const match = token.match(
      /^([a-z][a-z-]*):(?:git:|worktree:)?sha256:[0-9a-f]+$/i
    );
    if (match) {
      const kind = match[1].toLowerCase();
      return { kind, clickable: !NON_CLICKABLE_KINDS.has(kind) };
    }
    if (/^sha256:[0-9a-f]+$/i.test(token))
      return { kind: "hash", clickable: false };
    if (/^[0-9a-f]{40}$/i.test(token))
      return { kind: "commit", clickable: false };
    if (/^(agent|human):[a-z0-9][a-z0-9_-]*$/i.test(token)) {
      return { kind: "track", clickable: true };
    }
    return null;
  }
  __name(refInfo, "refInfo");
  var REF_RE = new RegExp(
    `\\b(?:${REF_ID_PREFIXES.join("|")}):(?:git:|worktree:)?sha256:[0-9a-f]{6,}\\b|(?<!:)\\bsha256:[0-9a-f]{16,}\\b|\\b[0-9a-f]{40}\\b|\\b(?:agent|human):[a-z0-9][a-z0-9_-]*\\b`,
    "gi"
  );
  function linkifyEscaped(escaped) {
    return escaped.replace(REF_RE, (token) => {
      const info = refInfo(token);
      if (!info) return token;
      const display = escapeHtml(shortRef(token));
      if (!info.clickable) {
        return `<span class="${refClass(info.kind)}" title="${escapeHtml(token)}">${display}</span>`;
      }
      return `<span class="${refClass(info.kind)}" role="link" tabindex="0" data-ref-kind="${info.kind}" data-ref-id="${escapeHtml(token)}" title="${escapeHtml(token)}">${display}</span>`;
    });
  }
  __name(linkifyEscaped, "linkifyEscaped");
  function linkify(text) {
    return linkifyEscaped(escapeHtml(String(text ?? "")));
  }
  __name(linkify, "linkify");
  function isMarkdownContentType(contentType) {
    return contentType === "text/markdown";
  }
  __name(isMarkdownContentType, "isMarkdownContentType");
  function safeMarkdownHref(href) {
    const raw = String(href ?? "").trim();
    if (/^(https?:|mailto:)/i.test(raw) || raw.startsWith("#"))
      return escapeHtml(raw);
    return "";
  }
  __name(safeMarkdownHref, "safeMarkdownHref");

  // src/types.ts
  var TYPES = [
    { id: "review_initialized", label: "init", color: "var(--evt-init)" },
    { id: "work_object_proposed", label: "capture", color: "var(--evt-capture)" },
    {
      id: "review_observation_recorded",
      label: "observation",
      color: "var(--evt-observation)"
    },
    {
      id: "review_assessment_recorded",
      label: "assessment",
      color: "var(--evt-assessment)"
    },
    { id: "input_request_opened", label: "request", color: "var(--evt-request)" },
    {
      id: "input_request_responded",
      label: "response",
      color: "var(--evt-response)"
    },
    { id: "review_note_imported", label: "note", color: "var(--evt-note)" },
    {
      id: "validation_check_recorded",
      label: "validation",
      color: "var(--evt-validation)"
    }
  ];
  var TYPE_MAP = Object.fromEntries(TYPES.map((type) => [type.id, type]));
  function typeColor(id) {
    return TYPE_MAP[id]?.color ?? "var(--evt-note)";
  }
  __name(typeColor, "typeColor");
  function typeLabel(id) {
    return TYPE_MAP[id]?.label ?? id;
  }
  __name(typeLabel, "typeLabel");
  var VERIFICATION_LABELS = {
    valid: "signature valid",
    invalid: "signature invalid",
    untrusted_key: "untrusted key",
    unsigned: "unsigned"
  };
  var ENDORSEMENT_LABELS = {
    "endorsement-trusted": "trusted endorsement",
    unknown_endorser: "unknown endorser",
    ambiguous_endorser: "ambiguous endorser"
  };
  var ASSESSMENT_LABELS = {
    accepted: "accepted",
    accepted_with_follow_up: "accepted-with-follow-up",
    needs_changes: "needs-changes",
    needs_clarification: "needs-clarification"
  };
  var LENSES = ["timeline", "list", "threads"];
  var DEFAULT_LENS = "timeline";
  var QUERY_FIELDS = [
    "type",
    "track",
    "revision",
    "snapshot",
    "status",
    "attention"
  ];
  var DEFAULT_OPEN_FILES = 10;
  var LARGE_FILE_ROWS = 500;
  var OVERLAY_SELECTORS = {
    diff: "#diff-modal",
    palette: "#cmd-palette",
    help: "#key-help"
  };
  var SUPERSEDABLE_FACT_TYPES = /* @__PURE__ */ new Set([
    "review_observation_recorded",
    "review_assessment_recorded",
    "input_request_opened",
    "validation_check_recorded"
  ]);

  // src/projection.ts
  function entryTrack(e) {
    return e.trackId || e.writer?.actorId || "";
  }
  __name(entryTrack, "entryTrack");
  function entryRevisionId(e) {
    return e.subject?.revisionId || "";
  }
  __name(entryRevisionId, "entryRevisionId");
  function principalLabel(e) {
    const principal = e.principal;
    if (principal?.status !== "resolved" || !principal.actorId) {
      return null;
    }
    const agent = (e.writer?.actorId || "").replace(/^actor:agent:/, "");
    const principalName = principal.actorId.replace(
      /^actor:git-(email|name):/,
      ""
    );
    return `${agent} (for ${principalName})`;
  }
  __name(principalLabel, "principalLabel");
  function verificationChip(status) {
    if (!status) return "";
    const label = VERIFICATION_LABELS[status] || status;
    return `<span class="${verifyClass(escapeHtml(status))}" title="advisory signature readback — reader-relative, never gates a write">${escapeHtml(label)}</span>`;
  }
  __name(verificationChip, "verificationChip");
  function endorserDisplay(actorId) {
    return actorId.replace(/^actor:git-(email|name):/, "");
  }
  __name(endorserDisplay, "endorserDisplay");
  function endorsementRow(en) {
    const cls = en.classification || "";
    const label = ENDORSEMENT_LABELS[cls] || cls;
    const parts = [
      `<span class="${CLASS.endorseLabel}">${escapeHtml(label)}</span>`
    ];
    if (en.endorser) {
      parts.push(
        `<span class="${CLASS.endorseWho}">${escapeHtml(endorserDisplay(en.endorser))}</span>`
      );
    }
    const attrs = en.endorserAttributes || {};
    const attrBits = [];
    if (attrs.kind) attrBits.push(attrs.kind);
    const roles = attrs.roles || [];
    if (roles.length) attrBits.push(roles.join(", "));
    if (attrBits.length) {
      parts.push(
        `<span class="${CLASS.endorseAttrs}">${escapeHtml(attrBits.join(" · "))}</span>`
      );
    }
    return `<li class="${endorseClass(escapeHtml(cls))}">${parts.join(" ")}</li>`;
  }
  __name(endorsementRow, "endorsementRow");
  function endorsementsBlock(endorsements) {
    const list = endorsements || [];
    if (!list.length) return "";
    const rows = list.map(endorsementRow).join("");
    return `<div class="${CLASS.endorsements}" title="advisory endorsement readback — reader-relative, never gates a write">
    <span class="${CLASS.endorsementsLabel}">endorsements</span>
    <ul class="${CLASS.endorseList}">${rows}</ul>
  </div>`;
  }
  __name(endorsementsBlock, "endorsementsBlock");
  function assessmentDisplayLabel(value) {
    return ASSESSMENT_LABELS[value] || value || "";
  }
  __name(assessmentDisplayLabel, "assessmentDisplayLabel");
  function entryTitle(e) {
    const s = e.summary || {};
    if (s.title) return s.title;
    if (s.assessment) return assessmentDisplayLabel(s.assessment);
    if (s.outcome) return s.outcome;
    if (s.reasonCode) return s.reasonCode;
    if (e.eventType === "work_object_proposed") {
      const base = s.base?.commitOid || "";
      return base ? `capture · base ${shortId(base)}` : "capture";
    }
    if (e.eventType === "validation_check_recorded") {
      const name = s.checkName || "validation";
      return s.status ? `${name} · ${s.status}` : name;
    }
    return typeLabel(e.eventType);
  }
  __name(entryTitle, "entryTitle");
  function entryTags(e) {
    const s = e.summary || {};
    return Array.isArray(s.tags) ? s.tags : [];
  }
  __name(entryTags, "entryTags");
  function entryAnchor(e) {
    const t = e.summary?.target;
    if (!t?.filePath) return "";
    if (t.startLine)
      return `${t.filePath}:${t.startLine}-${t.endLine || t.startLine}`;
    return t.filePath;
  }
  __name(entryAnchor, "entryAnchor");
  function assessmentLabel(value) {
    if (!value) return "";
    return String(value).replaceAll("_", " ");
  }
  __name(assessmentLabel, "assessmentLabel");
  function assessmentCue(overview) {
    const currentAssessment = overview?.currentAssessment || {};
    const status = currentAssessment.status || "unassessed";
    const assessment = currentAssessment.assessment || "";
    const label = assessment || (status === "ambiguous" ? "ambiguous current assessment" : status === "resolved" ? "resolved" : "unassessed");
    const cls = assessment || status;
    return `<span class="${CLASS.overviewAssessment}"><span>current assessment</span><span class="${factStatusClass(escapeHtml(cls))}">${escapeHtml(assessmentLabel(label))}</span></span>`;
  }
  __name(assessmentCue, "assessmentCue");
  function plural(n, singular, pluralLabel = `${singular}s`) {
    return `${n} ${n === 1 ? singular : pluralLabel}`;
  }
  __name(plural, "plural");
  function attentionTokens(overview) {
    const attention = overview?.attention || {};
    const tokens = [];
    if (attention.openInputRequestCount) {
      tokens.push({
        token: "open-request",
        query: "attention:open-request",
        label: plural(attention.openInputRequestCount, "open request")
      });
    }
    if (attention.unassessed) {
      tokens.push({
        token: "unassessed",
        query: "attention:unassessed",
        label: "unassessed"
      });
    }
    const validationCount = (attention.failedValidationCount || 0) + (attention.erroredValidationCount || 0);
    if (validationCount) {
      tokens.push({
        token: "validation-context",
        query: "attention:validation-context",
        label: plural(
          validationCount,
          "validation context",
          "validation contexts"
        )
      });
    }
    if (attention.acceptedWithFollowUp) {
      tokens.push({
        token: "follow-up",
        query: "attention:follow-up",
        label: "follow-up"
      });
    }
    if (attention.staleFactCount) {
      tokens.push({
        token: "stale-fact",
        query: "attention:stale-fact",
        label: plural(attention.staleFactCount, "stale fact")
      });
    }
    return tokens;
  }
  __name(attentionTokens, "attentionTokens");
  function attentionCues(overview) {
    const tokens = attentionTokens(overview);
    if (!tokens.length)
      return `<span class="${CLASS.overviewMuted}">no attention cues</span>`;
    return tokens.map(
      (cue) => `<button class="${CLASS.overviewCue}" type="button" data-attention-query="${escapeHtml(cue.query)}" title="filter ${escapeHtml(cue.query)}">${escapeHtml(cue.label)}</button>`
    ).join("");
  }
  __name(attentionCues, "attentionCues");
  function overviewStats(overview) {
    const counts = overview?.counts || {};
    const facts = (counts.observations || 0) + (counts.inputRequests || 0) + (counts.assessments || 0) + (counts.validationChecks || 0) + (counts.adapterNotes || 0);
    const stat = /* @__PURE__ */ __name((label, value) => `<span class="${CLASS.overviewStat}"><b>${value ?? 0}</b> ${escapeHtml(label)}</span>`, "stat");
    return `<div class="${CLASS.overviewStats}">${stat("files", counts.files)}${stat("rows", counts.rows)}${stat("facts", facts)}</div>`;
  }
  __name(overviewStats, "overviewStats");
  function latestActivityLine(overview) {
    const latest = overview?.latestActivity;
    if (!latest) return "";
    const title = latest.title || latest.kind || "activity";
    return `<div class="${CLASS.overviewLatest}"><span>latest</span><b>${escapeHtml(title)}</b><span>${escapeHtml(fmtDateTime(latest.at || ""))}</span></div>`;
  }
  __name(latestActivityLine, "latestActivityLine");
  function revisionSearchIndex(r) {
    const overview = r.overview || {};
    const currentAssessment = overview.currentAssessment || {};
    const latest = overview.latestActivity || {};
    const target = r.targetDisplay || {};
    const head = target.head || {};
    const cues = attentionTokens(overview);
    const text = [
      r.revisionId,
      r.snapshotId,
      target.label,
      head.label,
      currentAssessment.status,
      currentAssessment.assessment,
      latest.kind,
      latest.title,
      ...cues.map((cue) => cue.label),
      "review cues",
      "attention"
    ].filter(Boolean).join(" ").toLowerCase();
    return {
      text,
      type: "revision",
      revision: r.revisionId,
      // The search-index key is `snapshot` (grammar renamed from `object`, #334);
      // the value is the revision's snapshot/content-object id.
      snapshot: r.snapshotId,
      status: currentAssessment.assessment || currentAssessment.status || "",
      attention: cues.map((cue) => cue.token).join(" ")
    };
  }
  __name(revisionSearchIndex, "revisionSearchIndex");
  function renderRevisionOverview(r, overview = r.overview) {
    return `<div class="${CLASS.overviewSummary}">
    <div class="${CLASS.overviewMain}">${assessmentCue(overview)}${overviewStats(overview)}</div>
    <div class="${CLASS.overviewCues}" aria-label="review cues"><span class="${CLASS.overviewLabel}">review cues</span>${attentionCues(overview)}</div>
    ${latestActivityLine(overview)}
  </div>`;
  }
  __name(renderRevisionOverview, "renderRevisionOverview");

  // src/query.ts
  function tokenizeQuery(q) {
    const out = [];
    const re = /-?(?:[a-z]+:)?"[^"]*"|\S+/gi;
    let m = re.exec(q);
    while (m !== null) {
      out.push(m[0]);
      m = re.exec(q);
    }
    return out;
  }
  __name(tokenizeQuery, "tokenizeQuery");
  function parseSearchQuery(q) {
    const clauses = [];
    for (let tok of tokenizeQuery(q || "")) {
      let negate = false;
      if (tok.length > 1 && tok[0] === "-") {
        negate = true;
        tok = tok.slice(1);
      }
      const colon = tok.indexOf(":");
      let field = colon > 0 ? tok.slice(0, colon).toLowerCase() : "";
      if (field === "object") field = "snapshot";
      if (field && QUERY_FIELDS.includes(field)) {
        const raw = tok.slice(colon + 1).replace(/^"|"$/g, "");
        clauses.push({ kind: "field", field, value: raw.toLowerCase(), negate });
      } else {
        const term = tok.replace(/^"|"$/g, "").toLowerCase();
        if (term) clauses.push({ kind: "text", value: term, negate });
      }
    }
    return clauses;
  }
  __name(parseSearchQuery, "parseSearchQuery");
  function fieldMatches(idx, field, value) {
    if (field === "type") {
      const known = TYPES.find((t) => t.label === value || t.id === value);
      return idx.type === (known ? known.id : value);
    }
    return (idx[field] || "").toLowerCase().includes(value);
  }
  __name(fieldMatches, "fieldMatches");
  function matchesQuery(idx, clauses) {
    for (const c of clauses) {
      const hit = c.kind === "field" ? fieldMatches(idx, c.field, c.value) : idx.text.includes(c.value);
      if (c.negate ? hit : !hit) return false;
    }
    return true;
  }
  __name(matchesQuery, "matchesQuery");

  // src/store.ts
  var state = {
    history: null,
    revisions: null,
    threads: null,
    lens: "timeline",
    selected: { kind: null, id: null },
    enabledTypes: new Set(TYPES.map((t) => t.id)),
    seenTypes: new Set(TYPES.map((t) => t.id)),
    filterText: "",
    filterTrack: "",
    filterSnapshot: "",
    order: "desc",
    diff: null,
    diffHash: null,
    focus: null,
    lastEventCount: null
  };
  var subscribers = /* @__PURE__ */ new Set();
  function getState() {
    return state;
  }
  __name(getState, "getState");
  function subscribe(fn) {
    subscribers.add(fn);
    return () => {
      subscribers.delete(fn);
    };
  }
  __name(subscribe, "subscribe");
  function commit(patch) {
    Object.assign(state, patch);
    if (!state.selected) state.selected = { kind: null, id: null };
    if (!state.diff) state.diffHash = null;
    for (const fn of subscribers) fn();
  }
  __name(commit, "commit");

  // src/model.ts
  function presentTypes() {
    const history2 = getState().history;
    const keys = history2?.facets ? Object.keys(history2.facets) : [];
    const present = new Set(
      keys.length ? keys : (history2?.entries ?? []).map((e) => e.eventType)
    );
    const ordered = TYPES.map((t) => t.id).filter((id) => present.has(id));
    for (const id of present) if (!TYPE_MAP[id]) ordered.push(id);
    return ordered;
  }
  __name(presentTypes, "presentTypes");
  function currentThreads() {
    return getState().threads?.threads ?? [];
  }
  __name(currentThreads, "currentThreads");
  function threadRevisionOrder(thread) {
    const revisions = thread.revisions ?? [];
    const nodes = thread.laidOut?.nodes ?? [];
    if (!nodes.length) return revisions;
    const known = new Set(revisions);
    const ordered = nodes.filter(
      (n) => typeof n.id === "string" && known.has(n.id)
    ).slice().sort((a, b) => (a.y ?? 0) - (b.y ?? 0) || (a.x ?? 0) - (b.x ?? 0)).map((n) => n.id);
    if (ordered.length === revisions.length) return ordered;
    const seen = new Set(ordered);
    return ordered.concat(revisions.filter((id) => !seen.has(id)));
  }
  __name(threadRevisionOrder, "threadRevisionOrder");
  function revisionClassification(revisionId) {
    const map = getState().threads?.revisionClassification;
    const raw = map ? map[revisionId] : void 0;
    if (raw === null || typeof raw !== "object") return null;
    return raw;
  }
  __name(revisionClassification, "revisionClassification");
  function supersededByRevision(revisionId) {
    return revisionClassification(revisionId)?.supersededBy ?? [];
  }
  __name(supersededByRevision, "supersededByRevision");
  function supersedesRevision(revisionId) {
    return revisionClassification(revisionId)?.supersedes ?? [];
  }
  __name(supersedesRevision, "supersedesRevision");
  function revisionIsHead(revisionId) {
    const klass = revisionClassification(revisionId)?.state;
    return klass === "head" || klass === "isolated";
  }
  __name(revisionIsHead, "revisionIsHead");
  function revisionForId(revisionId) {
    return (getState().revisions?.entries ?? []).find(
      (r) => r.revisionId === revisionId
    ) ?? null;
  }
  __name(revisionForId, "revisionForId");
  function snapshotIdForRevision(revisionId) {
    return revisionForId(revisionId)?.snapshotId ?? "";
  }
  __name(snapshotIdForRevision, "snapshotIdForRevision");
  function snapshotContentHashForRevision(revisionId) {
    return revisionForId(revisionId)?.snapshotContentHash ?? "";
  }
  __name(snapshotContentHashForRevision, "snapshotContentHashForRevision");
  function revisionIdForSnapshot(snapshotId, contentHash = null) {
    const entries = getState().revisions?.entries ?? [];
    const revision = entries.find(
      (r) => r.snapshotId === snapshotId && (!contentHash || r.snapshotContentHash === contentHash)
    ) ?? entries.find((r) => r.snapshotId === snapshotId);
    return revision ? revision.revisionId ?? null : null;
  }
  __name(revisionIdForSnapshot, "revisionIdForSnapshot");
  function overviewForRevision(revisionId) {
    return revisionForId(revisionId)?.overview ?? null;
  }
  __name(overviewForRevision, "overviewForRevision");
  function isSupersedableFact(e) {
    return SUPERSEDABLE_FACT_TYPES.has(e.eventType);
  }
  __name(isSupersedableFact, "isSupersedableFact");
  function supersessionStaleBadge(e) {
    if (!isSupersedableFact(e)) return "";
    const successors = supersededByRevision(entryRevisionId(e));
    if (!successors.length) return "";
    return `<span class="${CLASS.badge} ${CLASS.stale}">superseded by ${successors.map(linkify).join(" ")}</span>`;
  }
  __name(supersessionStaleBadge, "supersessionStaleBadge");
  function captureSupersedesBadge(e) {
    if (e.eventType !== "work_object_proposed") return "";
    const predecessors = supersedesRevision(entryRevisionId(e));
    if (!predecessors.length) return "";
    return `<span class="${CLASS.badge} ${CLASS.supersedes}">supersedes ${predecessors.map(linkify).join(" ")}</span>`;
  }
  __name(captureSupersedesBadge, "captureSupersedesBadge");
  function entryFactId(e) {
    if (e.eventType === "review_observation_recorded")
      return e.summary?.observationId ?? "";
    if (e.eventType === "review_assessment_recorded")
      return e.summary?.assessmentId ?? "";
    return "";
  }
  __name(entryFactId, "entryFactId");
  function factSupersessionIndex() {
    const index = /* @__PURE__ */ new Map();
    for (const e of getState().history?.entries ?? []) {
      const superseder = entryFactId(e);
      if (!superseder) continue;
      const targets = e.summary?.supersedes ?? e.summary?.replaces ?? [];
      for (const target of targets) {
        const supersedersOf = index.get(target) ?? [];
        supersedersOf.push(superseder);
        index.set(target, supersedersOf);
      }
    }
    return index;
  }
  __name(factSupersessionIndex, "factSupersessionIndex");
  function factSupersededBy(factId) {
    return factSupersessionIndex().get(factId) ?? [];
  }
  __name(factSupersededBy, "factSupersededBy");
  function factSupersessionBadge(e) {
    const factId = entryFactId(e);
    if (!factId || !factSupersededBy(factId).length) return "";
    const label = e.eventType === "review_assessment_recorded" ? "replaced" : "superseded";
    return `<span class="${CLASS.badge} ${CLASS.superseded}">${label}</span>`;
  }
  __name(factSupersessionBadge, "factSupersessionBadge");
  function supersessionBadge(revisionId) {
    if (!revisionId) return "";
    if (revisionIsHead(revisionId))
      return `<span class="${CLASS.badge} ${CLASS.head}">current in thread</span>`;
    const successors = supersededByRevision(revisionId);
    if (successors.length)
      return `<span class="${CLASS.badge} ${CLASS.superseded}">superseded by ${successors.map(linkify).join(" ")}</span>`;
    return "";
  }
  __name(supersessionBadge, "supersessionBadge");
  function annotationsForRevision(revisionId) {
    const out = [];
    for (const e of getState().history?.entries ?? []) {
      if (entryRevisionId(e) !== revisionId) continue;
      const s = e.summary ?? {};
      if (e.eventType === "review_observation_recorded") {
        out.push({
          kind: "observation",
          id: s.observationId ?? e.eventId ?? "",
          title: s.title ?? "(observation)",
          body: s.body ?? "",
          bodyContentType: s.bodyContentType,
          track: e.trackId ?? "",
          tags: Array.isArray(s.tags) ? s.tags : [],
          target: s.target ?? {}
        });
      } else if (e.eventType === "input_request_opened") {
        const meta = [s.mode, s.reasonCode].filter(Boolean).join(" · ");
        out.push({
          kind: "input-request",
          id: s.inputRequestId ?? e.eventId ?? "",
          title: s.title ?? "(input request)",
          body: s.body ?? "",
          bodyContentType: s.bodyContentType,
          track: e.trackId ?? "",
          tags: meta ? [meta] : [],
          target: s.target ?? {}
        });
      } else if (e.eventType === "review_assessment_recorded") {
        const label = assessmentDisplayLabel(s.assessment ?? "");
        out.push({
          kind: "assessment",
          id: s.assessmentId ?? e.eventId ?? "",
          title: `assessment: ${label || "?"}`,
          body: s.summary ?? "",
          bodyContentType: s.summaryContentType,
          track: e.trackId ?? "",
          tags: [],
          target: s.target ?? {}
        });
      }
    }
    return out;
  }
  __name(annotationsForRevision, "annotationsForRevision");
  function renderThreadRevisionOverview(revisionId) {
    const revision = revisionForId(revisionId);
    const overview = overviewForRevision(revisionId);
    if (!revision || !overview) return "";
    return `<div class="${CLASS.threadOverview}">
    <div><b>${targetDisplayLabel(revision.targetDisplay)}</b> <span>${escapeHtml(shortId(revisionId))}</span></div>
    ${assessmentCue(overview)}
    <div class="${CLASS.overviewCues}" aria-label="review cues"><span class="${CLASS.overviewLabel}">review cues</span>${attentionCues(overview)}</div>
  </div>`;
  }
  __name(renderThreadRevisionOverview, "renderThreadRevisionOverview");
  function matchesRevisionFilters(r) {
    const s = getState();
    if (s.filterSnapshot && r.snapshotId !== s.filterSnapshot) return false;
    return matchesQuery(revisionSearchIndex(r), parseSearchQuery(s.filterText));
  }
  __name(matchesRevisionFilters, "matchesRevisionFilters");
  function threadMatchesRevisionFilters(thread) {
    const revisions = thread.revisions ?? [];
    const s = getState();
    if (!s.filterText && !s.filterSnapshot) return true;
    return revisions.map(revisionForId).filter((r) => r !== null).some(matchesRevisionFilters);
  }
  __name(threadMatchesRevisionFilters, "threadMatchesRevisionFilters");
  function filteredThreadRevisionIds(thread, revisions = thread.revisions ?? []) {
    const s = getState();
    if (!s.filterText && !s.filterSnapshot) return revisions;
    return revisions.filter((revisionId) => {
      const revision = revisionForId(revisionId);
      return revision ? matchesRevisionFilters(revision) : false;
    });
  }
  __name(filteredThreadRevisionIds, "filteredThreadRevisionIds");
  function lensEntryIds() {
    const s = getState();
    if (s.lens === "list") {
      return (s.revisions?.entries ?? []).filter(matchesRevisionFilters).map((r) => ({ kind: "revision", id: r.revisionId ?? "" }));
    }
    if (s.lens === "threads") {
      const ids = [];
      for (const t of currentThreads().filter(threadMatchesRevisionFilters)) {
        for (const r of filteredThreadRevisionIds(t, threadRevisionOrder(t))) {
          ids.push({ kind: "revision", id: r });
        }
      }
      return ids;
    }
    return (s.history?.entries ?? []).map(
      (e) => ({ kind: "event", id: e.eventId ?? "" })
    );
  }
  __name(lensEntryIds, "lensEntryIds");
  function selectedEventId() {
    const selected = getState().selected;
    return selected && selected.kind === "event" ? selected.id : null;
  }
  __name(selectedEventId, "selectedEventId");
  function revisionExists(id) {
    return (getState().revisions?.entries ?? []).some((r) => r.revisionId === id);
  }
  __name(revisionExists, "revisionExists");
  function revisionInAnyThread(id) {
    return currentThreads().some((t) => (t.revisions ?? []).includes(id));
  }
  __name(revisionInAnyThread, "revisionInAnyThread");
  function eventExists(id) {
    return (getState().history?.entries ?? []).some((e) => e.eventId === id);
  }
  __name(eventExists, "eventExists");

  // src/data.ts
  var HISTORY_PAGE = 100;
  function historyQueryParams(s) {
    const p = new URLSearchParams();
    if (s.filterText) p.set("q", s.filterText);
    if (s.filterTrack) p.set("track", s.filterTrack);
    if (s.filterSnapshot) p.set("snapshot", s.filterSnapshot);
    if (s.order && s.order !== "asc") p.set("order", s.order);
    const present = presentTypes();
    if (present.some((id) => !s.enabledTypes.has(id))) {
      p.set("type", present.filter((id) => s.enabledTypes.has(id)).join(","));
    }
    p.set("limit", String(HISTORY_PAGE));
    return p.toString();
  }
  __name(historyQueryParams, "historyQueryParams");
  function showError(message) {
    const el = $("#error");
    if (!el) return;
    if (!message) {
      el.classList.add("hidden");
      el.textContent = "";
      return;
    }
    el.textContent = `error: ${message}`;
    el.classList.remove("hidden");
  }
  __name(showError, "showError");
  async function load() {
    try {
      const freshness = await fetchJSON("/api/freshness");
      const params = historyQueryParams(getState());
      const [historyRaw, revisionsRaw, threadsRaw] = await Promise.all([
        fetchJSON(`/api/history?${params}`),
        fetchJSON("/api/revisions"),
        fetchJSON("/api/threads")
      ]);
      showError(null);
      commit({
        history: { ...historyRaw, queryKey: params },
        revisions: revisionsRaw,
        threads: threadsRaw,
        lastEventCount: freshness.eventCount ?? null
      });
    } catch (err) {
      showError(err instanceof Error ? err.message : String(err));
    }
  }
  __name(load, "load");
  var reloading = false;
  function maybeReloadForQuery() {
    const s = getState();
    const want = historyQueryParams(s);
    if (reloading || !s.history || s.history.queryKey === want) return;
    reloading = true;
    void load().finally(() => {
      reloading = false;
    });
  }
  __name(maybeReloadForQuery, "maybeReloadForQuery");
  var pageFetches = /* @__PURE__ */ new Map();
  function pageUrl(s, selector) {
    const params = new URLSearchParams(historyQueryParams(s));
    if (selector.offset != null) params.set("offset", String(selector.offset));
    return `/api/history?${params}`;
  }
  __name(pageUrl, "pageUrl");
  async function fetchHistoryDoc(url) {
    try {
      return await fetchJSON(url);
    } catch (err) {
      showError(err instanceof Error ? err.message : String(err));
      return null;
    }
  }
  __name(fetchHistoryDoc, "fetchHistoryDoc");
  function mergeWindows(prev, page) {
    const prevOffset = prev.offset ?? 0;
    const prevEntries = prev.entries ?? [];
    const prevEnd = prevOffset + prevEntries.length;
    const pageOffset = page.offset ?? 0;
    const pageEntries = page.entries ?? [];
    const pageEnd = pageOffset + pageEntries.length;
    if (pageOffset > prevEnd || pageEnd < prevOffset) {
      return { entries: pageEntries, offset: pageOffset };
    }
    const offset = Math.min(prevOffset, pageOffset);
    const end = Math.max(prevEnd, pageEnd);
    const entries = [];
    for (let g = offset; g < end; g++) {
      entries.push(
        g >= pageOffset && g < pageEnd ? pageEntries[g - pageOffset] : prevEntries[g - prevOffset]
      );
    }
    return { entries, offset };
  }
  __name(mergeWindows, "mergeWindows");
  function commitHistoryPage(page) {
    const s = getState();
    const queryKey = historyQueryParams(s);
    const prev = s.history;
    const merged = prev && prev.queryKey === queryKey ? mergeWindows(prev, page) : { entries: page.entries ?? [], offset: page.offset ?? 0 };
    commit({
      history: {
        ...page,
        entries: merged.entries,
        offset: merged.offset,
        queryKey
      }
    });
  }
  __name(commitHistoryPage, "commitHistoryPage");
  function fetchHistoryPage(selector) {
    const s = getState();
    if (!s.history) return Promise.resolve();
    const url = pageUrl(s, selector);
    const existing = pageFetches.get(url);
    if (existing) return existing;
    const run2 = fetchHistoryDoc(url).then((doc) => {
      if (doc) commitHistoryPage(doc);
    }).finally(() => {
      pageFetches.delete(url);
    });
    pageFetches.set(url, run2);
    return run2;
  }
  __name(fetchHistoryPage, "fetchHistoryPage");
  function resetQuery(order) {
    const params = new URLSearchParams();
    if (order && order !== "asc") params.set("order", order);
    params.set("limit", String(HISTORY_PAGE));
    return params.toString();
  }
  __name(resetQuery, "resetQuery");
  async function fetchRevealPage(eventId) {
    const s = getState();
    const queryKey = resetQuery(s.order);
    const params = new URLSearchParams(queryKey);
    params.set("at", eventId);
    const doc = await fetchHistoryDoc(`/api/history?${params}`);
    if (!doc) return null;
    const present = (doc.entries ?? []).some((e) => e.eventId === eventId);
    const facetKeys = doc.facets ? Object.keys(doc.facets) : [];
    const enabledTypes = /* @__PURE__ */ new Set([...s.enabledTypes, ...facetKeys]);
    return { doc: { ...doc, queryKey }, present, enabledTypes };
  }
  __name(fetchRevealPage, "fetchRevealPage");
  function revealPatch(page, eventId) {
    return {
      lens: "timeline",
      selected: { kind: "event", id: eventId },
      filterText: "",
      filterTrack: "",
      filterSnapshot: "",
      enabledTypes: page.enabledTypes,
      diff: null,
      diffHash: null,
      focus: null,
      history: page.doc
    };
  }
  __name(revealPatch, "revealPatch");
  async function fetchEventIdForQuery(q) {
    const params = new URLSearchParams({ q, limit: "1" });
    const doc = await fetchHistoryDoc(`/api/history?${params}`);
    return doc?.entries?.[0]?.eventId ?? null;
  }
  __name(fetchEventIdForQuery, "fetchEventIdForQuery");
  async function pollFreshness() {
    try {
      const f = await fetchJSON("/api/freshness");
      const refresh = $("#refresh");
      const s = getState();
      const changed = (f.eventCount ?? null) !== s.lastEventCount;
      if (changed) {
        if (refresh) {
          refresh.textContent = "updated";
          refresh.classList.add("live");
        }
        await load();
        setTimeout(() => {
          if (refresh) {
            refresh.textContent = "watching";
            refresh.classList.remove("live");
          }
        }, 1200);
      } else if (refresh) {
        refresh.textContent = "watching";
      }
    } catch {
      const refresh = $("#refresh");
      if (refresh) refresh.textContent = "stalled";
    }
  }
  __name(pollFreshness, "pollFreshness");

  // src/markdown.ts
  function renderBodyContent(text, contentType) {
    if (!text) return "";
    const cls = bodyClass("anno-body", isMarkdownContentType(contentType));
    return `<div class="${cls}">${renderContentHtml(text, contentType)}</div>`;
  }
  __name(renderBodyContent, "renderBodyContent");
  function renderContentHtml(text, contentType) {
    return isMarkdownContentType(contentType) ? renderMarkdown(text) : linkify(text);
  }
  __name(renderContentHtml, "renderContentHtml");
  function renderMarkdown(text) {
    const lines = String(text ?? "").replace(/\r\n?/g, "\n").split("\n");
    const out = [];
    let paragraph = [];
    let listKind = null;
    let listItems = [];
    const flushParagraph = /* @__PURE__ */ __name(() => {
      if (!paragraph.length) return;
      out.push(`<p>${renderMarkdownInline(paragraph.join(" "))}</p>`);
      paragraph = [];
    }, "flushParagraph");
    const flushList = /* @__PURE__ */ __name(() => {
      if (!listKind) return;
      out.push(
        `<${listKind}>${listItems.map((item) => `<li>${renderMarkdownInline(item)}</li>`).join("")}</${listKind}>`
      );
      listKind = null;
      listItems = [];
    }, "flushList");
    const flushBlocks = /* @__PURE__ */ __name(() => {
      flushParagraph();
      flushList();
    }, "flushBlocks");
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
        out.push(
          `<h${level}>${renderMarkdownInline(heading[2].trim())}</h${level}>`
        );
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
  __name(renderMarkdown, "renderMarkdown");
  function renderMarkdownInline(text) {
    const placeholders = [];
    const stash = /* @__PURE__ */ __name((html2) => {
      const token = `\0MD${placeholders.length}\0`;
      placeholders.push([token, html2]);
      return token;
    }, "stash");
    let html = escapeHtml(String(text ?? ""));
    html = html.replace(
      /`([^`]+)`/g,
      (_, code) => stash(`<code>${code}</code>`)
    );
    html = html.replace(
      /\[([^\]]+)\]\(([^)\s]+)\)/g,
      (_, label, href) => {
        const safe = safeMarkdownHref(href);
        const labelHtml = renderMarkdownInline(label);
        return safe ? stash(
          `<a href="${safe}" target="_blank" rel="noreferrer">${labelHtml}</a>`
        ) : labelHtml;
      }
    );
    html = html.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>").replace(/\*([^*]+)\*/g, "<em>$1</em>");
    html = linkifyEscaped(html);
    for (const [token, replacement] of placeholders) {
      html = html.split(token).join(replacement);
    }
    return html;
  }
  __name(renderMarkdownInline, "renderMarkdownInline");

  // src/supersession.ts
  function renderSupersessionSvg(laid, opts) {
    const nodes = laid?.nodes ?? [];
    if (!laid || !nodes.length) return "";
    const w = laid.bounds?.w ?? 0;
    const h = laid.bounds?.h ?? 0;
    const center = new Map(
      nodes.map((n) => [
        n.id ?? "",
        [n.x ?? 0, n.y ?? 0]
      ])
    );
    const marker = /* @__PURE__ */ __name((id, cls) => `<marker id="${id}" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto" markerUnits="userSpaceOnUse"><path class="${cls}" d="M0,0 L7,4 L0,8 z" /></marker>`, "marker");
    const defs = `<defs>${marker("dag-arrow", CLASS.dagArrowHead)}${marker("dag-arrow-traced", CLASS.dagArrowHeadTraced)}</defs>`;
    const edges = (laid.edges ?? []).map((e) => {
      let path = e.path ?? [];
      const from = e.from != null ? center.get(e.from) : void 0;
      if (from && path.length > 1) {
        const dist2 = /* @__PURE__ */ __name((p) => (p[0] - from[0]) ** 2 + (p[1] - from[1]) ** 2, "dist2");
        if (dist2(path[0]) < dist2(path[path.length - 1]))
          path = [...path].reverse();
      }
      const pts = path.map(([x, y]) => `${x},${y}`).join(" ");
      return `<polyline class="${CLASS.dagEdge}" data-from="${escapeHtml(e.from ?? "")}" data-to="${escapeHtml(e.to ?? "")}" points="${pts}" marker-end="url(#dag-arrow)" />`;
    }).join("");
    const nodesHtml = nodes.map((n) => {
      const id = n.id ?? "";
      const sel = opts.isSelected(id);
      const nodeW = n.w ?? 0;
      const nodeH = n.h ?? 0;
      const nx = n.x ?? 0;
      const ny = n.y ?? 0;
      const cls = dagNodeClass({
        isHead: !!n.isHead,
        isSuperseded: !!n.isSuperseded
      });
      const interactive = opts.interactive ? ' tabindex="0" role="link"' : "";
      const selected = sel ? ' aria-selected="true"' : "";
      return `<g class="${cls}" ${opts.idAttr}="${escapeHtml(id)}"${interactive}${selected} aria-label="${escapeHtml(opts.ariaNoun)} ${escapeHtml(shortId(id))}">
        <rect x="${nx - nodeW / 2}" y="${ny - nodeH / 2}" width="${nodeW}" height="${nodeH}" rx="6" />
        <text x="${nx}" y="${ny}" text-anchor="middle" dominant-baseline="middle">${escapeHtml(shortId(id))}</text>
      </g>`;
    }).join("");
    return `<svg class="${CLASS.revisionDag}" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}" preserveAspectRatio="xMinYMin meet" role="group" aria-label="supersession graph">${defs}${edges}${nodesHtml}</svg>`;
  }
  __name(renderSupersessionSvg, "renderSupersessionSvg");

  // src/cards.ts
  function verdictBadge(ca) {
    const status = ca?.status || "unassessed";
    let value;
    let cls;
    if (status === "resolved") {
      const assessment = ca?.assessment ?? "";
      value = assessmentDisplayLabel(assessment);
      cls = verdictClass(assessment);
    } else if (status === "ambiguous") {
      value = `ambiguous (${(ca?.candidates ?? []).length} candidates)`;
      cls = verdictClass("ambiguous");
    } else {
      value = "unassessed";
      cls = verdictClass("unassessed");
    }
    return `<div class="${cls}"><span class="${CLASS.verdictStatus}">current assessment</span><span class="${CLASS.verdictValue}">${escapeHtml(value)}</span></div>`;
  }
  __name(verdictBadge, "verdictBadge");
  function currentAssessmentSummary(d) {
    const ca = d.currentAssessment || {};
    if (ca.status === "resolved" && ca.assessmentId) {
      const a = (d.assessments || []).find((x) => x.id === ca.assessmentId);
      if (a?.summary) {
        const cls = bodyClass(
          "verdict-summary",
          isMarkdownContentType(a.summaryContentType)
        );
        return `<div class="${cls}">${renderContentHtml(a.summary, a.summaryContentType)}</div>`;
      }
    }
    if (ca.status === "ambiguous") {
      return `<div class="${CLASS.verdictSummary}">${(ca.candidates || []).length} unreplaced assessments — see Assessments below.</div>`;
    }
    return "";
  }
  __name(currentAssessmentSummary, "currentAssessmentSummary");
  function targetLabel(t) {
    const tt = t ?? {};
    switch (tt.kind) {
      case "range":
        return `${escapeHtml(tt.filePath)}:${tt.startLine}-${tt.endLine ?? tt.startLine} (${escapeHtml(tt.side || "new")})`;
      case "file":
        return escapeHtml(tt.filePath || "");
      case "revision":
        return "whole revision";
      case "observation":
        return `→ ${linkify(tt.observationId)}`;
      case "input_request":
        return `→ ${linkify(tt.inputRequestId)}`;
      case "assessment":
        return `→ ${linkify(tt.assessmentId)}`;
      case "event":
        return `→ ${linkify(tt.eventId)}`;
      default:
        return escapeHtml(tt.kind || "");
    }
  }
  __name(targetLabel, "targetLabel");
  function removedBodyCue(state2) {
    if (state2 !== "suppressed_present" && state2 !== "physically_removed") {
      return null;
    }
    const title = state2 === "suppressed_present" ? "removal recorded; bytes still stored until compact" : "removed; bytes swept from the store";
    return `<div class="${CLASS.factBodyRemoved}" title="${title}">content removed</div>`;
  }
  __name(removedBodyCue, "removedBodyCue");
  function factCard(kind, opts) {
    const tags = (opts.tags || []).filter(Boolean).map((t) => `<span class="${CLASS.badge}">${escapeHtml(t)}</span>`).join(" ");
    const body = removedBodyCue(opts.bodyContentState) ?? renderBodyContent(opts.body, opts.bodyContentType);
    return `<div class="${annoContainerClass(kind)}">
    <div class="${CLASS.annoHead}">
      <span class="${annoKindClass(kind)}">${kind}</span>
      <span class="${CLASS.annoTrack}">${escapeHtml(opts.track || "")}</span>
      <span class="${CLASS.annoTitle}">${linkify(opts.title || "")}</span>
      ${opts.status ? `<span class="${factStatusClass(escapeHtml(opts.status))}">${escapeHtml(opts.status)}</span>` : ""}
      ${opts.target ? `<span class="${CLASS.annoLoc}">${opts.target}</span>` : ""}
      ${tags}
      ${opts.verify || ""}
      ${opts.createdAt ? `<span class="${CLASS.annoTime}" title="${escapeHtml(opts.createdAt)}">${escapeHtml(fmtDateTime(opts.createdAt))}</span>` : ""}
    </div>
    ${body}
    ${opts.endorsements || ""}
    ${opts.extra || ""}</div>`;
  }
  __name(factCard, "factCard");
  function renderObservationCard(o) {
    const supersedes = o.supersedes ?? [];
    const extra = supersedes.length ? `<div class="${CLASS.factRel}">supersedes ${supersedes.map(linkify).join(", ")}</div>` : "";
    return factCard("observation", {
      track: o.trackId,
      title: o.title,
      status: o.status,
      target: targetLabel(o.target),
      tags: o.tags,
      body: o.body,
      bodyContentType: o.bodyContentType,
      bodyContentState: o.bodyContentState,
      createdAt: o.createdAt,
      verify: verificationChip(o.verificationStatus ?? ""),
      endorsements: endorsementsBlock(o.endorsements),
      extra
    });
  }
  __name(renderObservationCard, "renderObservationCard");
  function renderInputRequestCard(ir) {
    const responses = (ir.responses ?? []).map(
      (r) => `<div class="${CLASS.factResponse}"><span class="${CLASS.outcome}">${escapeHtml(r.outcome)}</span>${removedBodyCue(r.reasonContentState) ?? (r.reason ? renderBodyContent(r.reason, r.reasonContentType) : "")} ${verificationChip(r.verificationStatus ?? "")}${endorsementsBlock(r.endorsements)}</div>`
    ).join("");
    return factCard("input-request", {
      track: ir.trackId,
      title: ir.title,
      status: ir.status,
      target: targetLabel(ir.target),
      tags: [ir.mode, ir.reasonCode],
      body: ir.body,
      bodyContentType: ir.bodyContentType,
      bodyContentState: ir.bodyContentState,
      createdAt: ir.createdAt,
      verify: verificationChip(ir.verificationStatus ?? ""),
      endorsements: endorsementsBlock(ir.endorsements),
      extra: responses ? `<div class="${CLASS.factResponses}">${responses}</div>` : ""
    });
  }
  __name(renderInputRequestCard, "renderInputRequestCard");
  function renderAssessmentCard(a) {
    const rel = [];
    const replaces = a.replaces ?? [];
    const relatedObservations = a.relatedObservations ?? [];
    const relatedInputRequests = a.relatedInputRequests ?? [];
    if (replaces.length) rel.push(`replaces ${replaces.map(linkify).join(", ")}`);
    if (relatedObservations.length) {
      rel.push(`re ${relatedObservations.map(linkify).join(", ")}`);
    }
    if (relatedInputRequests.length) {
      rel.push(`re ${relatedInputRequests.map(linkify).join(", ")}`);
    }
    return factCard("assessment", {
      track: a.trackId,
      title: assessmentDisplayLabel(a.assessment ?? ""),
      status: a.status,
      target: targetLabel(a.target),
      body: a.summary,
      bodyContentType: a.summaryContentType,
      bodyContentState: a.summaryContentState,
      createdAt: a.createdAt,
      verify: verificationChip(a.verificationStatus ?? ""),
      endorsements: endorsementsBlock(a.endorsements),
      extra: rel.length ? `<div class="${CLASS.factRel}">${rel.join(" · ")}</div>` : ""
    });
  }
  __name(renderAssessmentCard, "renderAssessmentCard");
  function renderValidationCheckCard(v) {
    const rel = [];
    const logs = v.logArtifactContentHashes ?? [];
    if (v.command) rel.push(escapeHtml(v.command));
    if (logs.length) rel.push(`logs ${logs.map(linkify).join(", ")}`);
    return factCard("validation", {
      track: v.trackId,
      title: v.checkName,
      status: v.status,
      // passed | failed | errored | skipped → .fact-status.<status>
      target: targetLabel(v.target),
      tags: [v.trigger, v.exitCode != null ? `exit ${v.exitCode}` : null],
      body: v.summary || "",
      bodyContentType: v.summaryContentType,
      bodyContentState: v.summaryContentState,
      createdAt: v.completedAt || v.createdAt,
      verify: verificationChip(v.verificationStatus ?? ""),
      endorsements: endorsementsBlock(v.endorsements),
      extra: rel.length ? `<div class="${CLASS.factRel}">${rel.join(" · ")}</div>` : ""
    });
  }
  __name(renderValidationCheckCard, "renderValidationCheckCard");
  function renderAdapterNoteCard(n) {
    return factCard("observation", {
      track: n.author || "imported",
      title: n.title,
      status: n.status,
      target: n.filePath ? escapeHtml(n.filePath) : "",
      body: n.body,
      bodyContentState: n.bodyContentState,
      createdAt: n.createdAt
    });
  }
  __name(renderAdapterNoteCard, "renderAdapterNoteCard");
  function factSection(title, items, render2, context = "") {
    const list = items ?? [];
    const body = list.length ? list.map(render2).join("") : `<p class="${CLASS.upEmpty}">none</p>`;
    return `<section><h2>${escapeHtml(title)} (${list.length})</h2>${context}${body}</section>`;
  }
  __name(factSection, "factSection");
  function renderFactSupersessionBlock(graph, noun) {
    const laid = graph?.laidOut;
    if (!laid || !(laid.nodes ?? []).length) return "";
    const svg = renderSupersessionSvg(laid, {
      idAttr: "data-fact-id",
      ariaNoun: noun,
      interactive: false,
      isSelected: /* @__PURE__ */ __name(() => false, "isSelected")
    });
    if (!svg) return "";
    const heads = (laid.nodes ?? []).filter((n) => n.isHead).length;
    const caption = `${noun} supersession${heads > 1 ? ` — ${heads} competing` : ""}`;
    return `<figure class="${CLASS.factDag}"><figcaption>${escapeHtml(caption)}</figcaption>${svg}</figure>`;
  }
  __name(renderFactSupersessionBlock, "renderFactSupersessionBlock");

  // src/overlay.ts
  var registry = /* @__PURE__ */ new Map();
  var activeOverlay = null;
  function activeName() {
    return activeOverlay?.name ?? null;
  }
  __name(activeName, "activeName");
  function register(name, registration) {
    registry.set(name, registration);
  }
  __name(register, "register");
  function overlayNode(name) {
    const registered = registry.get(name);
    if (registered) return registered.node;
    const selector = OVERLAY_SELECTORS[name];
    return selector ? $(selector) : null;
  }
  __name(overlayNode, "overlayNode");
  function overlayFocusable(node) {
    return Array.from(
      node.querySelectorAll(
        'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
      )
    ).filter(
      (el) => el.getClientRects().length > 0 || el === document.activeElement
    );
  }
  __name(overlayFocusable, "overlayFocusable");
  function open(name, initialSelector) {
    const node = overlayNode(name);
    if (!node) return;
    if (activeOverlay && activeOverlay.name !== name) {
      closeActive({ restoreFocus: false });
    }
    const priorFocus = activeOverlay?.name === name ? activeOverlay.priorFocus : document.activeElement;
    const onClose2 = registry.get(name)?.onClose ?? noop;
    activeOverlay = { name, node, onClose: onClose2, priorFocus };
    node.classList.remove("hidden");
    const target = initialSelector ? node.querySelector(initialSelector) : overlayFocusable(node)[0];
    target?.focus();
  }
  __name(open, "open");
  function closeActive(opts = {}) {
    if (!activeOverlay) return;
    const current = activeOverlay;
    current.node.classList.add("hidden");
    activeOverlay = null;
    current.onClose();
    if (opts.restoreFocus !== false && current.priorFocus instanceof HTMLElement && document.contains(current.priorFocus)) {
      current.priorFocus.focus();
    }
  }
  __name(closeActive, "closeActive");
  function close(name, opts = {}) {
    if (activeOverlay?.name === name) {
      closeActive(opts);
      return;
    }
    const node = overlayNode(name);
    if (node) node.classList.add("hidden");
  }
  __name(close, "close");
  function trapFocus(ev) {
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
  __name(trapFocus, "trapFocus");
  function noop() {
  }
  __name(noop, "noop");

  // src/router.ts
  var LENSES2 = ["timeline", "list", "threads"];
  var DEFAULT_LENS2 = "timeline";
  function selectionKind(id) {
    const info = refInfo(id);
    if (info && (info.kind === "rev" || info.kind === "review-unit"))
      return "revision";
    return "event";
  }
  __name(selectionKind, "selectionKind");
  function parseQuery(queryString) {
    const params = {};
    for (const pair of queryString.split("&")) {
      if (!pair) continue;
      const eq = pair.indexOf("=");
      const key = decodeURIComponent(eq < 0 ? pair : pair.slice(0, eq));
      params[key] = eq < 0 ? "" : decodeURIComponent(pair.slice(eq + 1));
    }
    return params;
  }
  __name(parseQuery, "parseQuery");
  function parseHash(hash, presentTypes2) {
    const raw = hash.replace(/^#/, "");
    const q = raw.indexOf("?");
    const path = q < 0 ? raw : raw.slice(0, q);
    const p = parseQuery(q < 0 ? "" : raw.slice(q + 1));
    const patch = {
      lens: DEFAULT_LENS2,
      selected: { kind: null, id: null },
      filterTrack: p.track != null ? p.track : "",
      // The filter param is `snapshot`; legacy `object` is still parsed for old
      // bookmarks during the transition (#334).
      filterSnapshot: p.snapshot != null ? p.snapshot : p.object != null ? p.object : "",
      order: p.order === "asc" || p.order === "desc" ? p.order : "desc",
      filterText: p.q != null ? p.q : "",
      enabledTypes: p.types != null ? new Set(p.types.split(",").filter(Boolean)) : new Set(presentTypes2),
      diff: p.diff || null,
      diffHash: p.diffHash || null,
      focus: p.focus ? p.focus : null,
      unsupportedAsOf: p.asof != null ? p.asof || true : null,
      unsupportedJournal: p.journal != null ? p.journal || true : null,
      unknownPath: null
    };
    const segs = path.split("/").filter(Boolean);
    const lensParam = p.lens ?? "";
    if (segs.length === 0) {
      patch.lens = DEFAULT_LENS2;
    } else if (segs[0] === "revision" && segs[1]) {
      patch.selected = { kind: "revision", id: decodeURIComponent(segs[1]) };
      patch.lens = LENSES2.includes(lensParam) ? lensParam : DEFAULT_LENS2;
    } else if (segs[0] === "event" && segs[1]) {
      patch.selected = { kind: "event", id: decodeURIComponent(segs[1]) };
      patch.lens = LENSES2.includes(lensParam) ? lensParam : DEFAULT_LENS2;
    } else if (LENSES2.includes(segs[0])) {
      patch.lens = segs[0];
      if (p.sel) patch.selected = { kind: selectionKind(p.sel), id: p.sel };
    } else {
      patch.lens = DEFAULT_LENS2;
      patch.unknownPath = path;
    }
    return patch;
  }
  __name(parseHash, "parseHash");
  function serializeState(snapshot, presentTypes2) {
    const params = [];
    const sel = snapshot.selected ?? { kind: null, id: null };
    let path = snapshot.lens === DEFAULT_LENS2 ? "#/timeline" : `#/${snapshot.lens}`;
    if (sel.id && (sel.kind === "revision" || sel.kind === "event")) {
      path = sel.kind === "revision" ? `#/revision/${encodeURIComponent(sel.id)}` : `#/event/${encodeURIComponent(sel.id)}`;
      if (snapshot.lens && snapshot.lens !== DEFAULT_LENS2)
        params.push(`lens=${encodeURIComponent(snapshot.lens)}`);
    } else if (sel.id) {
      params.push(`sel=${encodeURIComponent(sel.id)}`);
    }
    if (snapshot.filterTrack)
      params.push(`track=${encodeURIComponent(snapshot.filterTrack)}`);
    if (snapshot.filterSnapshot)
      params.push(`snapshot=${encodeURIComponent(snapshot.filterSnapshot)}`);
    if (snapshot.order && snapshot.order !== "desc")
      params.push(`order=${encodeURIComponent(snapshot.order)}`);
    if (presentTypes2.some((id) => !snapshot.enabledTypes.has(id))) {
      params.push(
        `types=${encodeURIComponent(
          presentTypes2.filter((id) => snapshot.enabledTypes.has(id)).join(",")
        )}`
      );
    }
    if (snapshot.filterText)
      params.push(`q=${encodeURIComponent(snapshot.filterText)}`);
    if (snapshot.diff) params.push(`diff=${encodeURIComponent(snapshot.diff)}`);
    if (snapshot.diff && snapshot.diffHash)
      params.push(`diffHash=${encodeURIComponent(snapshot.diffHash)}`);
    if (snapshot.focus)
      params.push(`focus=${encodeURIComponent(snapshot.focus)}`);
    return params.length ? `${path}?${params.join("&")}` : path;
  }
  __name(serializeState, "serializeState");
  function navigate(patch, opts = {}) {
    commit(patch);
    const hash = serializeState(getState(), presentTypes());
    if (opts.replace) history.replaceState({}, "", hash);
    else history.pushState({}, "", hash);
  }
  __name(navigate, "navigate");
  function applyHash() {
    const patch = resolve(parseHash(location.hash, presentTypes()));
    commit(patch);
    const sel = getState().selected;
    if (sel.kind === "event" && sel.id && !eventExists(sel.id)) {
      void revealSelectedEvent(sel.id, patch.lens ?? DEFAULT_LENS2);
    }
  }
  __name(applyHash, "applyHash");
  async function revealSelectedEvent(eventId, lens) {
    const page = await fetchRevealPage(eventId);
    if (!page) return;
    if (page.present) {
      commit(revealPatch(page, eventId));
      clearRouteDiagnostic();
      return;
    }
    commit({ selected: { kind: null, id: null } });
    showRouteDiagnostic(
      `fell back to the ${lens} lens — event ${shortRef(eventId)} is not in this store`
    );
  }
  __name(revealSelectedEvent, "revealSelectedEvent");
  function resolve(patch) {
    const freshnessDiagnostic = liveStateDiagnostic(patch);
    const next = statePatchFrom(patch);
    if (patch.unknownPath != null) {
      showRouteDiagnostic(
        routeDiagnostic(
          `fell back to the timeline — unknown route ${patch.unknownPath}`,
          freshnessDiagnostic
        )
      );
      next.lens = DEFAULT_LENS2;
      next.selected = { kind: null, id: null };
      return next;
    }
    const sel = patch.selected ?? { kind: null, id: null };
    if (sel.kind === "revision" && sel.id && !revisionExists(sel.id)) {
      if (revisionInAnyThread(sel.id)) {
        showRouteDiagnostic(
          routeDiagnostic(
            `fell back to the threads lens — revision ${shortRef(sel.id)} is not directly selectable`,
            freshnessDiagnostic
          )
        );
        next.lens = "threads";
      } else {
        const lens = patch.lens || DEFAULT_LENS2;
        showRouteDiagnostic(
          routeDiagnostic(
            `fell back to the ${lens} lens — revision ${shortRef(sel.id)} is not in this store`,
            freshnessDiagnostic
          )
        );
        next.lens = lens;
      }
      next.selected = { kind: null, id: null };
      return next;
    }
    if (freshnessDiagnostic) {
      showRouteDiagnostic(freshnessDiagnostic);
      return next;
    }
    clearRouteDiagnostic();
    return next;
  }
  __name(resolve, "resolve");
  function statePatchFrom(patch) {
    return {
      lens: patch.lens,
      selected: patch.selected,
      filterTrack: patch.filterTrack,
      filterSnapshot: patch.filterSnapshot,
      order: patch.order,
      filterText: patch.filterText,
      enabledTypes: patch.enabledTypes,
      diff: patch.diff,
      diffHash: patch.diffHash,
      focus: patch.focus
    };
  }
  __name(statePatchFrom, "statePatchFrom");
  function liveStateDiagnostic(patch) {
    const unsupported = [];
    if (patch.unsupportedAsOf != null)
      unsupported.push("as-of links are not supported by this server");
    if (patch.unsupportedJournal != null)
      unsupported.push("journal links are not supported by this server");
    return unsupported.length ? `showing live state — ${unsupported.join("; ")}` : "";
  }
  __name(liveStateDiagnostic, "liveStateDiagnostic");
  function routeDiagnostic(primary, secondary) {
    return secondary ? `${primary} — ${secondary}` : primary;
  }
  __name(routeDiagnostic, "routeDiagnostic");
  function showRouteDiagnostic(message) {
    const el = $("#route-diagnostic");
    if (!el) return;
    el.textContent = message;
    el.classList.remove("hidden");
  }
  __name(showRouteDiagnostic, "showRouteDiagnostic");
  function clearRouteDiagnostic() {
    const el = $("#route-diagnostic");
    if (!el) return;
    el.textContent = "";
    el.classList.add("hidden");
  }
  __name(clearRouteDiagnostic, "clearRouteDiagnostic");

  // src/diff/highlight.ts
  function validChannel(spans, len) {
    let cursor = 0;
    for (const span of spans) {
      if (!Number.isInteger(span.start) || !Number.isInteger(span.end) || span.start < cursor || span.end < span.start || span.end > len) {
        return false;
      }
      cursor = span.end;
    }
    return true;
  }
  __name(validChannel, "validChannel");
  function segClass(kind, isEmph) {
    const parts = [
      kind ? tokClass(kind) : null,
      isEmph ? CLASS.emph : null
    ].filter(Boolean);
    return parts.length > 0 ? parts.join(" ") : null;
  }
  __name(segClass, "segClass");
  function highlightRowText(text, tokens, emphasis) {
    const toks = tokens && validChannel(tokens, text.length) ? tokens : [];
    const emph = emphasis && validChannel(emphasis, text.length) ? emphasis : [];
    if (toks.length === 0 && emph.length === 0) return escapeHtml(text);
    const points = [
      .../* @__PURE__ */ new Set([
        0,
        text.length,
        ...toks.flatMap((t) => [t.start, t.end]),
        ...emph.flatMap((e) => [e.start, e.end])
      ])
    ].sort((a, b) => a - b);
    let out = "";
    for (let i = 0; i + 1 < points.length; i++) {
      const a = points[i];
      const b = points[i + 1];
      if (a >= b) continue;
      const seg = escapeHtml(text.slice(a, b));
      const kind = toks.find((t) => t.start <= a && a < t.end)?.kind;
      const isEmph = emph.some((e) => e.start <= a && a < e.end);
      const cls = segClass(kind, isEmph);
      out += cls ? `<span class="${cls}">${seg}</span>` : seg;
    }
    return out;
  }
  __name(highlightRowText, "highlightRowText");

  // src/diff/render.ts
  function filePathLabel(f) {
    const oldp = f.old_path;
    const newp = f.new_path;
    return oldp && newp && oldp !== newp ? `${oldp} → ${newp}` : newp || oldp || "(unknown path)";
  }
  __name(filePathLabel, "filePathLabel");
  function fileRowCount(f) {
    return (f.hunks ?? []).reduce((n, h) => n + (h.rows ? h.rows.length : 0), 0);
  }
  __name(fileRowCount, "fileRowCount");
  function classifyLowSignal(f) {
    if (f.is_binary) return "binary";
    if (f.is_mode_only) return "mode change only";
    const hunks = f.hunks ?? [];
    const renamed = f.status === "renamed" || !!f.old_path && !!f.new_path && f.old_path !== f.new_path;
    if (renamed && !hunks.length) {
      return f.similarity != null ? `rename ${f.similarity}%` : "rename";
    }
    if (fileRowCount(f) > LARGE_FILE_ROWS) return "large file";
    return null;
  }
  __name(classifyLowSignal, "classifyLowSignal");
  function fileFactCount(f, anchored) {
    const oldp = f.old_path;
    const newp = f.new_path;
    let n = 0;
    for (const a of anchored) {
      const p = a.target?.filePath;
      if (p === newp || p === oldp) n += 1;
    }
    return n;
  }
  __name(fileFactCount, "fileFactCount");
  function fileForFact(files, filePath) {
    return files.find((f) => f.new_path === filePath || f.old_path === filePath) ?? null;
  }
  __name(fileForFact, "fileForFact");
  function rangeTouchesCapturedRows(a, file) {
    if (!file) return false;
    const t = a.target ?? {};
    if (t.kind !== "range" || t.startLine == null) return true;
    const start = t.startLine;
    const side = t.side === "old" ? "old" : "new";
    const end = t.endLine ?? start;
    for (const h of file.hunks ?? []) {
      for (const r of h.rows ?? []) {
        const line = side === "old" ? r.old_line : r.new_line;
        if (line != null && line >= start && line <= end) return true;
      }
    }
    return false;
  }
  __name(rangeTouchesCapturedRows, "rangeTouchesCapturedRows");
  function renderAnnotation(a, showLocation) {
    const tags = (a.tags ?? []).map((t2) => `<span class="${CLASS.badge}">${escapeHtml(t2)}</span>`).join(" ");
    const body = renderBodyContent(a.body, a.bodyContentType);
    const t = a.target ?? {};
    const loc = showLocation && t.filePath ? `<span class="${CLASS.annoLoc}">${escapeHtml(t.filePath)}${t.startLine ? `:${t.startLine}-${t.endLine || t.startLine}` : ""}</span>` : "";
    return `<div class="${annoContainerClass(a.kind)}" data-anno="${escapeHtml(a.id)}">
    <div class="${CLASS.annoHead}"><span class="${annoKindClass(a.kind)}">${a.kind}</span><span class="${CLASS.annoTrack}">${escapeHtml(a.track)}</span><span class="${CLASS.annoTitle}">${linkify(a.title)}</span> ${tags} ${loc}</div>${body}</div>`;
  }
  __name(renderAnnotation, "renderAnnotation");
  function renderDiffFactVicinity(f, anchored) {
    const facts = anchored.filter((a) => {
      const p = a.target?.filePath;
      return p === f.new_path || p === f.old_path;
    });
    return `<div class="${CLASS.diffFactVicinity}" data-fact-vicinity="true">
    <p>Large annotated file: showing review facts first.</p>
    <button type="button" data-render-diff-file="true">Render all rows</button>
    ${facts.map((a) => renderAnnotation(a, true)).join("")}
  </div>`;
  }
  __name(renderDiffFactVicinity, "renderDiffFactVicinity");
  function renderDiffFileHeader(f, anchored, reason, open3) {
    const n = fileFactCount(f, anchored);
    const summary = reason ? `<span class="${CLASS.dfileSummary}">${escapeHtml(reason)}</span>` : "";
    return `<header class="${CLASS.dfileHead}" role="button" tabindex="0" aria-expanded="${open3}">
    <span class="${diffStatusClass(escapeHtml(f.status))}">${escapeHtml(f.status)}</span>
    <span class="${CLASS.dpath}">${escapeHtml(filePathLabel(f))}</span>${summary}
    ${n ? `<span class="${CLASS.dfileNotes}">${n} note${n === 1 ? "" : "s"}</span>` : ""}</header>`;
  }
  __name(renderDiffFileHeader, "renderDiffFileHeader");
  function renderDiffFileBody(f, anchored) {
    const oldp = f.old_path;
    const newp = f.new_path;
    const fileFacts = anchored.filter((a) => {
      const p = a.target?.filePath;
      return p === newp || p === oldp;
    });
    const rangeFacts = fileFacts.filter((a) => a.target?.kind === "range");
    const fileLevelFacts = fileFacts.filter((a) => a.target?.kind === "file");
    const emitted = /* @__PURE__ */ new Set();
    let html = "";
    for (const a of fileLevelFacts) {
      html += renderAnnotation(a, false);
      emitted.add(a.id);
    }
    for (const m of f.metadata_rows ?? []) {
      html += `<div class="${CLASS.drow} ${CLASS.drowMeta}"><span class="${CLASS.dtext}">${escapeHtml(m.text)}</span></div>`;
    }
    const factsByLine = /* @__PURE__ */ new Map();
    for (const a of rangeFacts) {
      const t = a.target ?? {};
      if (t.startLine == null) continue;
      const start = t.startLine;
      const side = t.side === "old" ? "old" : "new";
      const end = t.endLine ?? start;
      for (let line = start; line <= end; line++) {
        const key = `${side}:${line}`;
        const bucket = factsByLine.get(key);
        if (bucket) bucket.push(a);
        else factsByLine.set(key, [a]);
      }
    }
    const hunks = f.hunks ?? [];
    for (const h of hunks) {
      html += `<div class="${CLASS.dhunk}">${escapeHtml(h.header)}</div>`;
      for (const r of h.rows ?? []) {
        const matching = [];
        const seen = /* @__PURE__ */ new Set();
        const collect = /* @__PURE__ */ __name((key) => {
          const bucket = factsByLine.get(key);
          if (!bucket) return;
          for (const a of bucket) {
            if (!seen.has(a.id)) {
              seen.add(a.id);
              matching.push(a);
            }
          }
        }, "collect");
        if (r.old_line != null) collect(`old:${r.old_line}`);
        if (r.new_line != null) collect(`new:${r.new_line}`);
        const sign = r.kind === "added" ? "+" : r.kind === "removed" ? "-" : " ";
        const noted = matching.length > 0;
        const notedAttrs = noted ? ` data-anno="${escapeHtml(matching[0].id)}" tabindex="0" role="button"` : "";
        html += `<div class="${drowClass(escapeHtml(r.kind), noted)}"${notedAttrs}>
        <span class="${CLASS.ln}">${r.old_line ?? ""}</span>
        <span class="${CLASS.ln}">${r.new_line ?? ""}</span>
        <span class="${CLASS.sign}">${sign}</span>
        <span class="${CLASS.dtext}">${highlightRowText(r.text, r.tokens, r.emphasis)}</span></div>`;
        for (const a of matching) {
          if (!emitted.has(a.id)) {
            html += renderAnnotation(a, false);
            emitted.add(a.id);
          }
        }
      }
    }
    for (const a of rangeFacts) {
      if (!emitted.has(a.id)) {
        html += renderAnnotation(a, true);
        emitted.add(a.id);
      }
    }
    if (!hunks.length && !(f.metadata_rows ?? []).length) {
      if (!classifyLowSignal(f)) {
        html += `<div class="${CLASS.drow} ${CLASS.drowMeta}"><span class="${CLASS.dtext}">(no captured content)</span></div>`;
      }
    }
    return html;
  }
  __name(renderDiffFileBody, "renderDiffFileBody");
  function renderDiff(snapshotId, artifact, annotations) {
    const annos = annotations ?? [];
    const files = artifact.snapshot?.files ?? [];
    const filePaths = /* @__PURE__ */ new Set();
    for (const f of files) {
      if (f.new_path) filePaths.add(f.new_path);
      if (f.old_path) filePaths.add(f.old_path);
    }
    const anchored = [];
    const unanchored = [];
    for (const a of annos) {
      const t = a.target ?? {};
      if ((t.kind === "range" || t.kind === "file") && t.filePath && filePaths.has(t.filePath)) {
        const file = fileForFact(files, t.filePath);
        if (t.kind === "range" && !rangeTouchesCapturedRows(a, file)) {
          unanchored.push(a);
        } else {
          anchored.push(a);
        }
      } else {
        unanchored.push(a);
      }
    }
    const ctx = { snapshotId, files, anchored, unanchored, filePaths };
    const counts = {};
    for (const a of annos) {
      counts[a.kind] = (counts[a.kind] ?? 0) + 1;
    }
    const breakdown = Object.entries(counts).map(([k, n]) => `${n} ${k}${n === 1 ? "" : "s"}`).join(", ");
    let html = `<div class="${CLASS.annoSummary}">${annos.length} review fact${annos.length === 1 ? "" : "s"} on this revision${breakdown ? ` · ${breakdown}` : ""}${unanchored.length ? ` · ${unanchored.length} not anchored to a diff line` : ""}</div>`;
    if (unanchored.length) {
      html += `<div class="${CLASS.annoGroup}">${unanchored.map((a) => renderAnnotation(a, true)).join("")}</div>`;
    }
    if (!files.length) {
      return {
        html: `${html}<p class="${CLASS.empty}">No files captured in this snapshot.</p>`,
        ctx
      };
    }
    let openBudget = DEFAULT_OPEN_FILES;
    html += files.map((f, i) => {
      const reason = classifyLowSignal(f);
      const annotated = fileFactCount(f, anchored) > 0;
      const annotatedLarge = annotated && fileRowCount(f) > LARGE_FILE_ROWS;
      const open3 = annotated && !annotatedLarge || (reason ? false : openBudget-- > 0);
      const expanded = annotatedLarge || open3;
      const body = annotatedLarge ? renderDiffFactVicinity(f, anchored) : open3 ? renderDiffFileBody(f, anchored) : "";
      const lowAttr = reason ? ` data-lowsignal="${escapeHtml(reason)}"` : "";
      const bodyAttr = annotatedLarge ? ` data-fact-vicinity="true"` : open3 ? ` data-rendered="1"` : "";
      return `<section class="${dfileClass(!!reason)}" data-dfile="${i}" data-expanded="${expanded}"${lowAttr}>${renderDiffFileHeader(f, anchored, reason, expanded)}<div class="${CLASS.dfileBody}" data-dfile-body="${i}"${bodyAttr}>${body}</div></section>`;
    }).join("");
    return { html, ctx };
  }
  __name(renderDiff, "renderDiff");
  function renderDiffNavSummary(summary) {
    return `<div class="${CLASS.diffNavSummary}" aria-label="diff summary">
    <span><b>${summary.fileCount}</b> files</span>
    <span><b>${summary.factCount}</b> facts</span>
    <span><b>${summary.unanchoredCount}</b> unanchored</span>
  </div>`;
  }
  __name(renderDiffNavSummary, "renderDiffNavSummary");
  function renderDiffNavFilters(activeFilter) {
    return `<div class="${CLASS.diffNavFilters}" role="group" aria-label="diff navigator filters">
    <button type="button" data-diff-nav-filter="all" aria-pressed="${activeFilter === "all"}">all</button>
    <button type="button" data-diff-nav-filter="with-facts" aria-pressed="${activeFilter === "with-facts"}">with facts</button>
    <button type="button" data-diff-nav-filter="unanchored" aria-pressed="${activeFilter === "unanchored"}">unanchored</button>
  </div>`;
  }
  __name(renderDiffNavFilters, "renderDiffNavFilters");
  function unanchoredReason(a, filePaths) {
    const t = a.target ?? {};
    if (a.kind === "assessment") return "broad assessment";
    if (t.kind === "revision" || !t.filePath) return "revision-level";
    if (t.kind === "range" && filePaths.has(t.filePath)) {
      return "line outside captured rows";
    }
    if (!filePaths.has(t.filePath)) return "file missing from snapshot";
    return "not anchored to a diff line";
  }
  __name(unanchoredReason, "unanchoredReason");

  // src/diff/controller.ts
  var shownDiffSnapshot = null;
  var shownDiffHash = null;
  var diffCtx = null;
  var diffFactCursor = -1;
  var diffChangeCursor = -1;
  var diffNavFilter = "all";
  var DIFF_NAV_FILTERS = [
    "all",
    "with-facts",
    "unanchored"
  ];
  function isDiffNavFilter(value) {
    return DIFF_NAV_FILTERS.includes(value);
  }
  __name(isDiffNavFilter, "isDiffNavFilter");
  function openDiff(snapshotId, focusId = null, contentHash = null) {
    navigate({
      diff: snapshotId,
      diffHash: contentHash || null,
      focus: focusId || null
    });
  }
  __name(openDiff, "openDiff");
  function openRevisionDiff(revisionId, focusId = null) {
    const snapshotId = snapshotIdForRevision(revisionId);
    if (snapshotId)
      openDiff(snapshotId, focusId, snapshotContentHashForRevision(revisionId));
  }
  __name(openRevisionDiff, "openRevisionDiff");
  function closeDiff() {
    const modal = $("#diff-modal");
    if (!getState().diff && modal?.classList.contains("hidden")) return;
    navigate({ diff: null, diffHash: null, focus: null }, { replace: true });
  }
  __name(closeDiff, "closeDiff");
  function renderDiffOverlay() {
    const state2 = getState();
    if (!state2.diff) {
      close("diff");
      shownDiffSnapshot = null;
      shownDiffHash = null;
      diffCtx = null;
      return Promise.resolve();
    }
    if (state2.diff === shownDiffSnapshot && state2.diffHash === shownDiffHash) {
      if (activeName() !== "diff") open("diff", "#diff-close");
      applyDiffFocus();
      return Promise.resolve();
    }
    shownDiffSnapshot = state2.diff;
    shownDiffHash = state2.diffHash;
    const snapshotId = state2.diff;
    const contentHash = state2.diffHash;
    const revisionId = revisionIdForSnapshot(snapshotId, contentHash);
    const label = revisionId ? shortId(revisionId) : "";
    const title = $("#diff-title");
    if (title)
      title.textContent = label ? `${label} · snapshot ${shortId(snapshotId)}` : shortId(snapshotId);
    const body = $("#diff-body");
    if (body) body.innerHTML = `<p class="${CLASS.empty}">loading snapshot…</p>`;
    const nav = $("#diff-nav");
    if (nav) nav.innerHTML = "";
    open("diff", "#diff-close");
    let snapshotUrl = `/api/snapshots/${encodeURIComponent(snapshotId)}`;
    if (contentHash)
      snapshotUrl += `?contentHash=${encodeURIComponent(contentHash)}`;
    return fetchJSON(snapshotUrl).then((artifact) => {
      if (state2.diff !== snapshotId || state2.diffHash !== contentHash) return;
      const annotations = revisionId ? annotationsForRevision(revisionId) : [];
      const { html, ctx } = renderDiff(
        snapshotId,
        artifact,
        annotations
      );
      const liveBody = $("#diff-body");
      if (liveBody) liveBody.innerHTML = html;
      diffCtx = ctx;
      diffFactCursor = -1;
      diffChangeCursor = -1;
      diffNavFilter = "all";
      const liveNav = $("#diff-nav");
      if (liveNav) liveNav.innerHTML = renderDiffNav();
      applyDiffFocus();
    }).catch((err) => {
      if (state2.diff !== snapshotId || state2.diffHash !== contentHash) return;
      const liveBody = $("#diff-body");
      if (liveBody)
        liveBody.innerHTML = `<p class="${CLASS.empty}">error: ${escapeHtml(
          err instanceof Error ? err.message : String(err)
        )}</p>`;
    });
  }
  __name(renderDiffOverlay, "renderDiffOverlay");
  function applyDiffFocus() {
    const focusId = getState().focus;
    if (focusId) scrollToAnno(focusId);
  }
  __name(applyDiffFocus, "applyDiffFocus");
  function focusDiffFactRoute(id) {
    if (!id || getState().focus === id) return false;
    navigate({ focus: id }, { replace: true });
    return true;
  }
  __name(focusDiffFactRoute, "focusDiffFactRoute");
  function scrollToAnno(id, opts = {}) {
    if (opts.updateRoute && focusDiffFactRoute(id)) return;
    const sel = `.anno[data-anno="${id}"]`;
    const body = $("#diff-body");
    let target = body?.querySelector(sel) ?? null;
    if (!target && diffCtx) {
      const fact = diffCtx.anchored.find((a) => a.id === id);
      const filePath = fact?.target?.filePath;
      if (filePath) {
        const idx = diffCtx.files.findIndex(
          (f) => f.new_path === filePath || f.old_path === filePath
        );
        if (idx >= 0) {
          const section = body?.querySelector(
            `.dfile[data-dfile="${idx}"]`
          );
          if (section) {
            expandDiffFile(section);
            target = body?.querySelector(sel) ?? null;
          }
        }
      }
    }
    if (target) {
      target.scrollIntoView({ block: "center" });
      flashAnno(target);
    }
  }
  __name(scrollToAnno, "scrollToAnno");
  function flashAnno(el) {
    el.classList.remove("anno-flash");
    void el.offsetWidth;
    el.classList.add("anno-flash");
  }
  __name(flashAnno, "flashAnno");
  function ensureDiffFileBody(section) {
    if (!diffCtx) return;
    const body = section.querySelector("[data-dfile-body]");
    if (!body || body.dataset.rendered) return;
    const idx = Number(section.dataset.dfile);
    body.innerHTML = renderDiffFileBody(diffCtx.files[idx], diffCtx.anchored);
    body.removeAttribute("data-fact-vicinity");
    body.dataset.rendered = "1";
  }
  __name(ensureDiffFileBody, "ensureDiffFileBody");
  function diffFileHeader(section) {
    return section.querySelector(".dfile-head");
  }
  __name(diffFileHeader, "diffFileHeader");
  function diffFileExpanded(section) {
    const head = diffFileHeader(section);
    return head ? head.getAttribute("aria-expanded") === "true" : false;
  }
  __name(diffFileExpanded, "diffFileExpanded");
  function setDiffFileExpanded(section, open3) {
    const value = String(open3);
    section.dataset.expanded = value;
    const head = diffFileHeader(section);
    if (head) head.setAttribute("aria-expanded", value);
  }
  __name(setDiffFileExpanded, "setDiffFileExpanded");
  function expandDiffFile(section) {
    ensureDiffFileBody(section);
    setDiffFileExpanded(section, true);
  }
  __name(expandDiffFile, "expandDiffFile");
  function toggleDiffFile(section) {
    const isOpen = diffFileExpanded(section);
    if (!isOpen) ensureDiffFileBody(section);
    setDiffFileExpanded(section, !isOpen);
  }
  __name(toggleDiffFile, "toggleDiffFile");
  function renderDiffNav() {
    if (!diffCtx) return "";
    const { files, anchored, unanchored, filePaths } = diffCtx;
    const visibleFiles = files.map((f, i) => ({ f, i, factCount: fileFactCount(f, anchored) })).filter((item) => {
      if (diffNavFilter === "with-facts") return item.factCount > 0;
      if (diffNavFilter === "unanchored") return false;
      return true;
    });
    const fileItems = visibleFiles.map(({ f, i, factCount: n }) => {
      const badge = n ? `<span class="${CLASS.dfileNotes}">${n}</span>` : "";
      return `<li><button class="${CLASS.diffNavFile}" data-nav-file="${i}">
        <span class="${diffStatusClass(escapeHtml(f.status ?? ""))}">${escapeHtml(f.status ?? "")}</span>
        <span class="${CLASS.dpath}">${escapeHtml(filePathLabel(f))}</span>${badge}</button></li>`;
    }).join("");
    let html = renderDiffNavSummary(diffNavSummary()) + renderDiffNavFilters(diffNavFilter);
    if (diffNavFilter !== "unanchored") {
      html += `<ol class="${CLASS.diffNavFiles}">${fileItems}</ol>`;
    }
    if (unanchored.length && diffNavFilter !== "with-facts") {
      const entries = unanchored.map(
        (a) => `<li><button class="${CLASS.diffNavFact}" data-anno="${escapeHtml(a.id)}"><span>${escapeHtml(a.title)}</span><span class="${CLASS.diffNavReason}">${escapeHtml(unanchoredReason(a, filePaths))}</span></button></li>`
      ).join("");
      html += `<section class="${CLASS.diffUnanchored}" aria-label="unanchored review facts">
      <h3>${unanchored.length} not anchored to a diff line</h3>
      <ol>${entries}</ol></section>`;
    }
    return html;
  }
  __name(renderDiffNav, "renderDiffNav");
  function diffNavSummary() {
    if (!diffCtx) return { fileCount: 0, factCount: 0, unanchoredCount: 0 };
    return {
      fileCount: diffCtx.files.length,
      factCount: diffCtx.anchored.length + diffCtx.unanchored.length,
      unanchoredCount: diffCtx.unanchored.length
    };
  }
  __name(diffNavSummary, "diffNavSummary");
  function setDiffNavFilter(filter) {
    if (!isDiffNavFilter(filter)) return;
    diffNavFilter = filter;
    const nav = $("#diff-nav");
    if (nav) nav.innerHTML = renderDiffNav();
  }
  __name(setDiffNavFilter, "setDiffNavFilter");
  function diffFactTargets() {
    return Array.from(
      $("#diff-body")?.querySelectorAll(".anno[data-anno]") ?? []
    );
  }
  __name(diffFactTargets, "diffFactTargets");
  function diffChangeTargets() {
    return Array.from(
      $("#diff-body")?.querySelectorAll(".dhunk") ?? []
    );
  }
  __name(diffChangeTargets, "diffChangeTargets");
  function jumpToTarget(targets, cursor, dir) {
    if (!targets.length) return cursor;
    const next = (cursor + dir + targets.length) % targets.length;
    const el = targets[next];
    const section = el.closest(".dfile");
    if (section && !diffFileExpanded(section)) expandDiffFile(section);
    el.scrollIntoView({ block: "center" });
    return next;
  }
  __name(jumpToTarget, "jumpToTarget");
  function jumpFact(dir) {
    const targets = diffFactTargets();
    if (!targets.length) return;
    diffFactCursor = (diffFactCursor + dir + targets.length) % targets.length;
    const el = targets[diffFactCursor];
    if (el) {
      const section = el.closest(".dfile");
      if (section && !diffFileExpanded(section)) expandDiffFile(section);
      const id = el.dataset.anno;
      if (id && focusDiffFactRoute(id)) return;
      el.scrollIntoView({ block: "center" });
      flashAnno(el);
    }
  }
  __name(jumpFact, "jumpFact");
  function jumpChange(dir) {
    diffChangeCursor = jumpToTarget(diffChangeTargets(), diffChangeCursor, dir);
  }
  __name(jumpChange, "jumpChange");
  function initControls() {
    const modal = $("#diff-modal");
    if (modal) register("diff", { node: modal, onClose: closeDiff });
    $("#diff-close")?.addEventListener("click", () => closeDiff());
    modal?.addEventListener("click", (ev) => {
      if (ev.target === modal) closeDiff();
    });
    const body = $("#diff-body");
    body?.addEventListener("click", (ev) => {
      const t = ev.target;
      if (!(t instanceof Element)) return;
      const renderAll = t.closest("[data-render-diff-file]");
      if (renderAll) {
        const section = renderAll.closest(".dfile");
        if (section) {
          ensureDiffFileBody(section);
          setDiffFileExpanded(section, true);
        }
        return;
      }
      const head = t.closest(".dfile-head");
      if (head) {
        const section = head.closest(".dfile");
        if (section) toggleDiffFile(section);
        return;
      }
      const noted = t.closest(".drow-noted[data-anno]");
      if (noted) {
        const id = noted.dataset.anno;
        if (id) scrollToAnno(id, { updateRoute: true });
      }
    });
    body?.addEventListener("keydown", (ev) => {
      if (ev.key !== "Enter" && ev.key !== " ") return;
      const t = ev.target;
      if (!(t instanceof Element)) return;
      const head = t.closest(".dfile-head");
      if (head) {
        ev.preventDefault();
        const section = head.closest(".dfile");
        if (section) toggleDiffFile(section);
        return;
      }
      const noted = t.closest(".drow-noted[data-anno]");
      if (noted) {
        ev.preventDefault();
        const id = noted.dataset.anno;
        if (id) scrollToAnno(id, { updateRoute: true });
      }
    });
    const nav = $("#diff-nav");
    nav?.addEventListener("click", (ev) => {
      const t = ev.target;
      if (!(t instanceof Element)) return;
      const filterBtn = t.closest("[data-diff-nav-filter]");
      if (filterBtn) {
        const filter = filterBtn.dataset.diffNavFilter;
        if (filter) setDiffNavFilter(filter);
        return;
      }
      const fileBtn = t.closest("[data-nav-file]");
      if (fileBtn) {
        const idx = Number(fileBtn.dataset.navFile);
        const section = $("#diff-body")?.querySelector(
          `.dfile[data-dfile="${idx}"]`
        );
        if (section) {
          expandDiffFile(section);
          section.scrollIntoView({ block: "start" });
        }
        return;
      }
      const factBtn = t.closest(".diff-nav-fact[data-anno]");
      if (factBtn) {
        const id = factBtn.dataset.anno;
        if (id) scrollToAnno(id, { updateRoute: true });
      }
    });
  }
  __name(initControls, "initControls");

  // src/detail.ts
  var shownCompositeId = null;
  function eventBodyBlock(e) {
    const s = e.summary ?? {};
    if (s.body) return renderBodyContent(s.body, s.bodyContentType);
    if (s.summary) return renderBodyContent(s.summary, s.summaryContentType);
    if (s.reason) return renderBodyContent(s.reason, s.reasonContentType);
    return "";
  }
  __name(eventBodyBlock, "eventBodyBlock");
  function renderDetail() {
    shownCompositeId = null;
    const el = $("#detail");
    if (!el) return;
    const entries = getState().history?.entries ?? [];
    const e = entries.find((x) => x.eventId === selectedEventId());
    if (!e) {
      el.innerHTML = `<p class="${CLASS.empty}">Select an event or revision to inspect.</p>`;
      return;
    }
    const revisionId = entryRevisionId(e);
    const kv = [
      ["type", `${typeLabel(e.eventType)} (${e.eventType})`],
      ["occurredAt", fmtDateTime(e.occurredAt ?? "")],
      ["eventId", e.eventId ?? ""],
      ["payloadHash", e.payloadHash ?? ""],
      ["revision", revisionId || "—"],
      ["track", entryTrack(e) || "—"],
      ["writer", principalLabel(e) || (e.writer ? e.writer.actorId || "—" : "—")]
    ];
    const snapshotId = revisionId ? snapshotIdForRevision(revisionId) : "";
    const s = e.summary ?? {};
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
      focusId = s.observationId ?? null;
      focusNoun = "observation";
    } else if (e.eventType === "review_assessment_recorded") {
      focusId = s.assessmentId ?? null;
      focusNoun = "assessment";
    } else if (e.eventType === "input_request_opened") {
      focusId = s.inputRequestId ?? null;
      focusNoun = "input request";
    }
    const bodyBlock = eventBodyBlock(e);
    const btnLabel = focusId ? `show this ${focusNoun} in the diff` : "view snapshot diff";
    const verifyChip = verificationChip(e.verificationStatus ?? "");
    const endorse = endorsementsBlock(e.endorsements);
    const readback = verifyChip || endorse ? `<div class="${CLASS.readback}"><p class="${CLASS.readerScopeNote}">reader-relative — computed against your enrolled keys</p>${verifyChip ? `<div class="${CLASS.readbackRow}">${verifyChip}</div>` : ""}${endorse}</div>` : "";
    const diffButton = snapshotId ? `<button class="${CLASS.ghost} ${CLASS.diffBtn}" id="detail-diff-btn" data-open-diff="${escapeHtml(snapshotId)}" data-diff-hash="${escapeHtml(snapshotContentHashForRevision(revisionId))}" data-diff-focus="${escapeHtml(focusId ?? "")}">${escapeHtml(btnLabel)}</button>` : "";
    el.innerHTML = `
    <h2>${linkify(entryTitle(e))}</h2>
    <dl class="${CLASS.kv}">${kv.map(([k, v]) => `<dt>${escapeHtml(k)}</dt><dd>${linkify(v)}</dd>`).join("")}</dl>
    ${readback}
    ${diffButton}
    ${bodyBlock}
    <pre>${escapeHtml(JSON.stringify(e, null, 2))}</pre>`;
  }
  __name(renderDetail, "renderDetail");
  function staleFactSectionContext(revisionId) {
    const successors = supersededByRevision(revisionId);
    if (!successors.length) return "";
    return `<p class="${CLASS.factStaleContext}">superseded by ${successors.map(linkify).join(" ")}</p>`;
  }
  __name(staleFactSectionContext, "staleFactSectionContext");
  function renderRevisionPage(d) {
    const ru = d.revision ?? {};
    const base = ru.base ?? {};
    const s = d.summary ?? {};
    const revisionId = ru.id ?? "";
    const badge = supersessionBadge(revisionId);
    const title = `${shortId(ru.id)}${base.commitOid ? ` · base ${shortId(base.commitOid)}` : ""}`;
    const staleContext = staleFactSectionContext(revisionId);
    const observationContext = renderFactSupersessionBlock(
      d.factSupersession?.observations,
      "observation"
    ) + staleContext;
    const assessmentContext = renderFactSupersessionBlock(d.factSupersession?.assessments, "assessment") + staleContext;
    const stat = /* @__PURE__ */ __name((label, n) => `<span class="${CLASS.upStat}"><b>${n ?? 0}</b> ${label}</span>`, "stat");
    const sections = [];
    sections.push(`<section><h2>Revision</h2><dl class="${CLASS.upIdentity}">
    <dt>id</dt><dd>${linkify(ru.id)}</dd>
    <dt>base</dt><dd>${base.commitOid ? linkify(base.commitOid) : "—"} ${base.kind ? `<span class="${CLASS.factStatus}">${escapeHtml(base.kind)}</span>` : ""}</dd>
    <dt>target</dt><dd>${targetDisplayLabel(ru.targetDisplay)}${targetHeadBadge(ru.targetDisplay)}</dd>
    <dt>worktree</dt><dd>${escapeHtml(ru.targetDisplay?.label ?? "working tree")}</dd>
    <dt>head</dt><dd>${escapeHtml(ru.targetDisplay?.head?.label ?? "—")}</dd>
    <dt>supersession</dt><dd>${badge || "—"}</dd>
    <dt>snapshot</dt><dd>${linkify(ru.objectId)}</dd>
  </dl></section>`);
    sections.push(
      `<section><h2>Current assessment</h2>${verdictBadge(d.currentAssessment)}${currentAssessmentSummary(d)}<p class="${CLASS.advisoryNote}">advisory — a recorded judgement, not a merge gate</p></section>`
    );
    sections.push(`<section><h2>Summary</h2><div class="${CLASS.upStats}">
    ${stat("files", s.fileCount)}${stat("rows", s.rowCount)}${stat("observations", s.observationCount)}${stat("input requests", s.inputRequestCount)}${stat("assessments", s.assessmentCount)}${stat("validation checks", s.validationCheckCount)}${stat("adapter notes", s.adapterNoteCount)}
  </div>
  <div style="margin-top:10px">
    <button class="${CLASS.ghost} ${CLASS.diffBtn}" id="up-diff-btn" data-open-diff="${escapeHtml(ru.objectId ?? "")}" data-diff-hash="${escapeHtml(ru.objectArtifactContentHash ?? "")}">view annotated diff</button>
    <button class="${CLASS.ghost}" id="up-timeline-btn" data-reveal-revision="${escapeHtml(revisionId)}" style="margin-left:6px">show in timeline</button>
  </div></section>`);
    sections.push(
      factSection(
        "Observations",
        d.observations,
        renderObservationCard,
        observationContext
      )
    );
    sections.push(
      factSection(
        "Input requests",
        d.inputRequests,
        renderInputRequestCard,
        staleContext
      )
    );
    sections.push(
      factSection(
        "Assessments",
        d.assessments,
        renderAssessmentCard,
        assessmentContext
      )
    );
    const validationChecks = d.validationChecks ?? [];
    const validationBody = validationChecks.length ? `${validationChecks.map(renderValidationCheckCard).join("")}<p class="${CLASS.validationNote}">context only — does not affect the current assessment</p>` : `<p class="${CLASS.upEmpty}">none</p>`;
    sections.push(
      `<section><h2>Validation checks (${validationChecks.length})</h2>${staleContext}${validationBody}</section>`
    );
    if ((d.adapterNotes ?? []).length) {
      sections.push(
        factSection("Adapter notes", d.adapterNotes, renderAdapterNoteCard)
      );
    }
    const el = $("#detail");
    if (el)
      el.innerHTML = `<div class="${CLASS.unitPage}"><p class="${CLASS.unitPageTitle}">${escapeHtml(title)}</p>${sections.join("")}</div>`;
  }
  __name(renderRevisionPage, "renderRevisionPage");
  async function openRevision(revisionId) {
    const el = $("#detail");
    if (el) el.innerHTML = `<p class="${CLASS.upEmpty}">loading…</p>`;
    try {
      const d = await fetchJSON(
        `/api/revisions/${encodeURIComponent(revisionId)}`
      );
      const sel = getState().selected;
      if (sel.kind !== "revision" || sel.id !== revisionId) return;
      renderRevisionPage(d);
    } catch (err) {
      const sel = getState().selected;
      if (sel.kind === "revision" && sel.id === revisionId) {
        const live = $("#detail");
        if (live)
          live.innerHTML = `<p class="${CLASS.upEmpty}">error: ${escapeHtml(
            err instanceof Error ? err.message : String(err)
          )}</p>`;
      }
    }
  }
  __name(openRevision, "openRevision");
  function showComposite(revisionId) {
    if (revisionId === shownCompositeId) return Promise.resolve();
    shownCompositeId = revisionId;
    return openRevision(revisionId);
  }
  __name(showComposite, "showComposite");
  function initControls2() {
    const el = $("#detail");
    el?.addEventListener("click", (ev) => {
      const t = ev.target;
      if (!(t instanceof Element)) return;
      const diffBtn = t.closest("[data-open-diff]");
      if (diffBtn) {
        const snapshotId = diffBtn.dataset.openDiff;
        if (snapshotId)
          openDiff(
            snapshotId,
            diffBtn.dataset.diffFocus || null,
            diffBtn.dataset.diffHash || null
          );
      }
    });
  }
  __name(initControls2, "initControls");

  // src/help-overlay.ts
  function onClose() {
  }
  __name(onClose, "onClose");
  function closeKeyHelp(opts = {}) {
    close("help", opts);
  }
  __name(closeKeyHelp, "closeKeyHelp");
  function initControls3() {
    const node = $("#key-help");
    if (!node) return;
    register("help", { node, onClose });
    $("#key-help-close")?.addEventListener("click", () => closeKeyHelp());
    node.addEventListener("click", (ev) => {
      if (ev.target === node) closeKeyHelp();
    });
  }
  __name(initControls3, "initControls");

  // src/lenses/timeline.ts
  var ROW_H = 52;
  var OVERSCAN = 8;
  function timelineRows() {
    return getState().history?.entries ?? [];
  }
  __name(timelineRows, "timelineRows");
  function loadedWindow(state2) {
    const h = state2.history;
    const entries = h?.entries ?? [];
    const offset = h?.offset ?? 0;
    const matchCount = h?.matchCount ?? entries.length;
    return { offset, count: entries.length, matchCount };
  }
  __name(loadedWindow, "loadedWindow");
  function visibleRange(scrollTop, viewportH, rowCount) {
    if (viewportH <= 0 || rowCount === 0) return { start: 0, end: rowCount };
    const maxScroll = Math.max(0, rowCount * ROW_H - viewportH);
    const clamped = Math.min(Math.max(0, scrollTop), maxScroll);
    const start = Math.max(0, Math.floor(clamped / ROW_H) - OVERSCAN);
    const end = Math.min(
      rowCount,
      Math.ceil((clamped + viewportH) / ROW_H) + OVERSCAN
    );
    return { start, end };
  }
  __name(visibleRange, "visibleRange");
  function spacer(height) {
    const li = document.createElement("li");
    li.dataset.spacer = "1";
    li.setAttribute("aria-hidden", "true");
    li.style.height = `${height}px`;
    return li;
  }
  __name(spacer, "spacer");
  function eventRow(e, selected) {
    const li = document.createElement("li");
    li.className = "event";
    li.dataset.eventId = e.eventId ?? "";
    if (e.eventId && e.eventId === selected)
      li.setAttribute("aria-selected", "true");
    const tags = entryTags(e).map((t) => `<span class="${CLASS.badge}">${escapeHtml(t)}</span>`).join(" ");
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
  __name(eventRow, "eventRow");
  function ensureScrollListener(list) {
    if (list.dataset.virtualized) return;
    list.dataset.virtualized = "1";
    list.addEventListener("scroll", () => renderTimeline());
  }
  __name(ensureScrollListener, "ensureScrollListener");
  function renderTimeline() {
    const list = $("#timeline");
    if (!list) return;
    const state2 = getState();
    const rows = timelineRows();
    const { offset, matchCount } = loadedWindow(state2);
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
    maybeExtendWindow(viewportH, start, end, offset, loadEnd, matchCount);
  }
  __name(renderTimeline, "renderTimeline");
  function maybeExtendWindow(viewportH, visibleStart, visibleEnd, loadStart, loadEnd, matchCount) {
    if (viewportH <= 0) return;
    if (loadEnd < matchCount && visibleEnd >= loadEnd - OVERSCAN) {
      void fetchHistoryPage({ offset: loadEnd });
    }
    if (loadStart > 0 && visibleStart <= loadStart + OVERSCAN) {
      void fetchHistoryPage({ offset: Math.max(0, loadStart - HISTORY_PAGE) });
    }
  }
  __name(maybeExtendWindow, "maybeExtendWindow");
  function scrollTimelineSelectionIntoView(eventId) {
    const list = $("#timeline");
    if (!list) return;
    const local = timelineRows().findIndex((e) => e.eventId === eventId);
    if (local < 0) return;
    const global = loadedWindow(getState()).offset + local;
    const centered = global * ROW_H - Math.max(0, (list.clientHeight - ROW_H) / 2);
    list.scrollTop = Math.max(0, centered);
    renderTimeline();
    const el = list.querySelector(`li[data-event-id="${eventId}"]`);
    if (el) el.scrollIntoView({ block: "center" });
  }
  __name(scrollTimelineSelectionIntoView, "scrollTimelineSelectionIntoView");

  // src/navigation.ts
  function navigateToRevision(id) {
    navigate({
      lens: "timeline",
      filterText: `revision:${id}`,
      filterTrack: "",
      filterSnapshot: ""
    });
  }
  __name(navigateToRevision, "navigateToRevision");
  function navigateToTrack(id) {
    navigate({
      lens: "timeline",
      filterTrack: id,
      diff: null,
      diffHash: null,
      focus: null
    });
  }
  __name(navigateToTrack, "navigateToTrack");
  async function revealEvent(eventId) {
    const page = await fetchRevealPage(eventId);
    if (!page?.present) return;
    navigate(revealPatch(page, eventId));
  }
  __name(revealEvent, "revealEvent");
  async function revealByQuery(id) {
    const eventId = await fetchEventIdForQuery(id);
    if (eventId) await revealEvent(eventId);
  }
  __name(revealByQuery, "revealByQuery");
  function resolveRef(kind, id) {
    void resolveRefAsync(kind, id);
  }
  __name(resolveRef, "resolveRef");
  async function resolveRefAsync(kind, id) {
    switch (kind) {
      // The revision and the (retired) review-unit prefix both address a revision's
      // composite — their identity is unified onto the revision id.
      case "rev":
      case "review-unit":
        navigate({
          selected: { kind: "revision", id },
          diff: null,
          diffHash: null,
          focus: null
        });
        break;
      case "track":
        navigateToTrack(id);
        break;
      case "snap":
        openDiff(id);
        break;
      case "obs":
      case "assess":
      case "input-request":
        await revealByQuery(id);
        break;
      case "evt":
        await revealEvent(id);
        break;
      default:
        break;
    }
  }
  __name(resolveRefAsync, "resolveRefAsync");
  function onDocumentClick(ev) {
    const t = ev.target;
    if (!(t instanceof Element)) return;
    const ref = t.closest("[data-ref-kind]");
    if (ref) {
      ev.preventDefault();
      resolveRef(ref.dataset.refKind ?? "", ref.dataset.refId ?? "");
      return;
    }
    const reveal = t.closest("[data-reveal-revision]");
    if (reveal) {
      const id = reveal.dataset.revealRevision;
      if (id) navigateToRevision(id);
    }
  }
  __name(onDocumentClick, "onDocumentClick");

  // src/palette.ts
  var cmdItems = [];
  var cmdFiltered = [];
  var cmdActive = 0;
  function copyText(text) {
    const clip = navigator.clipboard;
    if (clip?.writeText) void clip.writeText(text);
  }
  __name(copyText, "copyText");
  function copyCurrentViewLink() {
    copyText(
      location.origin + location.pathname + serializeState(getState(), presentTypes())
    );
  }
  __name(copyCurrentViewLink, "copyCurrentViewLink");
  function assignCommandOptionIds(cmds) {
    cmds.forEach((cmd, index) => {
      cmd.domIndex = index;
    });
    return cmds;
  }
  __name(assignCommandOptionIds, "assignCommandOptionIds");
  function selectedRevisionId() {
    const sel = getState().selected;
    if (sel.kind === "revision") return sel.id ?? "";
    if (sel.kind === "event") {
      const event = (getState().history?.entries ?? []).find(
        (e) => e.eventId === sel.id
      );
      return event ? entryRevisionId(event) : "";
    }
    return "";
  }
  __name(selectedRevisionId, "selectedRevisionId");
  function revisionCommandLabel(u) {
    const targetDisplay = u.targetDisplay ?? {};
    const overview = u.overview ?? {};
    const current = overview.currentAssessment ?? {};
    const target = targetDisplay.label || shortId(u.revisionId);
    const assessment = current.assessment ? assessmentLabel(current.assessment) : current.status || "unassessed";
    return `${target} · ${assessment} · ${shortId(u.revisionId)}`;
  }
  __name(revisionCommandLabel, "revisionCommandLabel");
  function revisionCommandHint(u) {
    const overview = u.overview ?? {};
    const cues = attentionTokens(overview).map((cue) => cue.label);
    const latest = overview.latestActivity?.title;
    return [cues.join(", ") || "review context", latest, shortId(u.snapshotId)].filter(Boolean).join(" · ");
  }
  __name(revisionCommandHint, "revisionCommandHint");
  function currentSelectionCommand() {
    const sel = getState().selected;
    if (!sel.id) return null;
    if (sel.kind === "revision") {
      const unit = revisionForId(sel.id);
      return {
        kind: "Current",
        label: "Open current selection",
        hint: unit ? revisionCommandLabel(unit) : shortRef(sel.id),
        run: /* @__PURE__ */ __name(() => navigate({
          selected: { kind: "revision", id: sel.id },
          diff: null,
          diffHash: null,
          focus: null
        }), "run")
      };
    }
    if (sel.kind === "event") {
      const event = (getState().history?.entries ?? []).find(
        (e) => e.eventId === sel.id
      );
      return {
        kind: "Current",
        label: "Open current selection",
        hint: event ? entryTitle(event) : shortRef(sel.id),
        run: /* @__PURE__ */ __name(() => navigate({
          selected: { kind: "event", id: sel.id },
          diff: null,
          diffHash: null,
          focus: null
        }), "run")
      };
    }
    return null;
  }
  __name(currentSelectionCommand, "currentSelectionCommand");
  function sortedRevisionEntriesForCommands() {
    const selectedRevision = selectedRevisionId();
    return [...getState().revisions?.entries ?? []].sort((left, right) => {
      if (left.revisionId === selectedRevision) return -1;
      if (right.revisionId === selectedRevision) return 1;
      return String(right.capturedAt || "").localeCompare(
        String(left.capturedAt || "")
      ) || String(right.revisionId).localeCompare(String(left.revisionId));
    });
  }
  __name(sortedRevisionEntriesForCommands, "sortedRevisionEntriesForCommands");
  function buildCommands() {
    const cmds = [];
    const state2 = getState();
    cmds.push({
      kind: "Actions",
      label: "Copy current view link",
      hint: "share",
      run: copyCurrentViewLink
    });
    cmds.push({
      kind: "Actions",
      label: "Clear filters",
      hint: "filters",
      run: /* @__PURE__ */ __name(() => navigate(
        {
          filterText: "",
          filterTrack: "",
          filterSnapshot: "",
          enabledTypes: new Set(presentTypes())
        },
        { replace: true }
      ), "run")
    });
    cmds.push({
      kind: "Actions",
      label: "Switch to timeline lens",
      hint: "lens",
      run: /* @__PURE__ */ __name(() => navigate({ lens: "timeline" }), "run")
    });
    cmds.push({
      kind: "Actions",
      label: "Switch to list lens",
      hint: "lens",
      run: /* @__PURE__ */ __name(() => navigate({ lens: "list" }), "run")
    });
    cmds.push({
      kind: "Actions",
      label: "Switch to threads lens",
      hint: "lens",
      run: /* @__PURE__ */ __name(() => navigate({ lens: "threads" }), "run")
    });
    cmds.push({
      kind: "Actions",
      label: "Toggle timeline order",
      hint: "order",
      run: /* @__PURE__ */ __name(() => navigate(
        { order: getState().order === "desc" ? "asc" : "desc" },
        { replace: true }
      ), "run")
    });
    cmds.push({
      kind: "Actions",
      label: "Copy selected id",
      hint: "clipboard",
      run: /* @__PURE__ */ __name(() => {
        const id = getState().selected.id;
        if (id) copyText(id);
      }, "run")
    });
    const current = currentSelectionCommand();
    if (current) cmds.push(current);
    for (const u of sortedRevisionEntriesForCommands()) {
      cmds.push({
        kind: "Revisions",
        label: revisionCommandLabel(u),
        hint: revisionCommandHint(u),
        run: /* @__PURE__ */ __name(() => navigate({
          selected: { kind: "revision", id: u.revisionId ?? "" },
          diff: null,
          diffHash: null,
          focus: null
        }), "run")
      });
    }
    for (const o of [
      ...new Set(
        (state2.revisions?.entries ?? []).map((u) => u.snapshotId).filter((x) => Boolean(x))
      )
    ]) {
      cmds.push({
        kind: "Snapshots",
        label: shortRef(o),
        hint: "open diff",
        run: /* @__PURE__ */ __name(() => openDiff(o), "run")
      });
    }
    for (const t of [
      ...new Set((state2.history?.entries ?? []).map(entryTrack).filter(Boolean))
    ].sort()) {
      cmds.push({
        kind: "Tracks",
        label: t,
        hint: "filter timeline",
        run: /* @__PURE__ */ __name(() => navigate({ lens: "timeline", filterTrack: t }), "run")
      });
    }
    for (const e of state2.history?.entries ?? []) {
      cmds.push({
        kind: "Events",
        label: entryTitle(e),
        hint: typeLabel(e.eventType),
        run: /* @__PURE__ */ __name(() => navigate({
          selected: { kind: "event", id: e.eventId ?? "" },
          diff: null,
          diffHash: null,
          focus: null
        }), "run")
      });
    }
    return assignCommandOptionIds(cmds);
  }
  __name(buildCommands, "buildCommands");
  function open2() {
    cmdItems = buildCommands();
    const input = $("#cmd-input");
    if (input) input.value = "";
    filterPalette("");
    open("palette", "#cmd-input");
  }
  __name(open2, "open");
  function close2(opts = {}) {
    close("palette", opts);
  }
  __name(close2, "close");
  function toggle() {
    const palette = $("#cmd-palette");
    if (palette && !palette.classList.contains("hidden")) close2();
    else open2();
  }
  __name(toggle, "toggle");
  function filterPalette(query) {
    const needle = query.trim().toLowerCase();
    cmdFiltered = needle ? cmdItems.filter(
      (c) => `${c.label} ${c.hint || ""}`.toLowerCase().includes(needle)
    ) : cmdItems.slice();
    cmdActive = 0;
    renderPalette();
  }
  __name(filterPalette, "filterPalette");
  function renderPalette() {
    const list = $("#cmd-results");
    const input = $("#cmd-input");
    if (!list || !input) return;
    if (!cmdFiltered.length) {
      list.innerHTML = `<li id="cmd-option-empty" class="${CLASS.cmdEmpty}" role="option" aria-disabled="true">No matches</li>`;
      input.setAttribute("aria-activedescendant", "cmd-option-empty");
      return;
    }
    let html = "";
    let lastKind = null;
    cmdFiltered.forEach((c, i) => {
      if (c.kind !== lastKind) {
        lastKind = c.kind;
        html += `<li class="${CLASS.cmdGroup}" role="presentation">${escapeHtml(c.kind)}</li>`;
      }
      html += `<li id="cmd-option-${escapeHtml(String(c.domIndex ?? i))}" class="${cmdItemClass(i === cmdActive)}" role="option" data-idx="${i}" aria-selected="${i === cmdActive}"><span class="${CLASS.cmdLabel}">${escapeHtml(c.label)}</span>${c.hint ? `<span class="${CLASS.cmdHint}">${escapeHtml(c.hint)}</span>` : ""}</li>`;
    });
    list.innerHTML = html;
    const active = list.querySelector(".cmd-item.active");
    if (active) {
      input.setAttribute("aria-activedescendant", active.id);
      active.scrollIntoView({ block: "nearest" });
    }
  }
  __name(renderPalette, "renderPalette");
  function move(delta) {
    if (!cmdFiltered.length) return;
    cmdActive = (cmdActive + delta + cmdFiltered.length) % cmdFiltered.length;
    renderPalette();
  }
  __name(move, "move");
  function run() {
    const cmd = cmdFiltered[cmdActive];
    close2();
    if (cmd) cmd.run();
  }
  __name(run, "run");
  function initControls4() {
    const node = $("#cmd-palette");
    if (node)
      register("palette", {
        node,
        onClose: /* @__PURE__ */ __name(() => {
          cmdActive = 0;
        }, "onClose")
      });
    const input = $("#cmd-input");
    input?.addEventListener("input", () => filterPalette(input.value));
    input?.addEventListener("keydown", (ev) => {
      if (ev.key === "ArrowDown") {
        ev.preventDefault();
        move(1);
      } else if (ev.key === "ArrowUp") {
        ev.preventDefault();
        move(-1);
      } else if (ev.key === "Enter") {
        ev.preventDefault();
        run();
      }
    });
    node?.addEventListener("click", (ev) => {
      const t = ev.target;
      if (!(t instanceof Element)) return;
      if (t === node) {
        close2();
        return;
      }
      const item = t.closest(".cmd-item");
      if (item) {
        cmdActive = Number(item.dataset.idx);
        run();
      }
    });
  }
  __name(initControls4, "initControls");

  // src/keyboard.ts
  var pendingChord = null;
  var chordTimer = null;
  function setChord(keyName) {
    pendingChord = keyName;
    if (chordTimer) clearTimeout(chordTimer);
    chordTimer = setTimeout(() => {
      pendingChord = null;
    }, 1e3);
  }
  __name(setChord, "setChord");
  function isTypingTarget(el) {
    if (!el) return false;
    return el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el instanceof HTMLElement && el.isContentEditable;
  }
  __name(isTypingTarget, "isTypingTarget");
  function stepSelection(delta) {
    void stepSelectionAsync(delta);
  }
  __name(stepSelection, "stepSelection");
  function stepList(delta) {
    const ids = lensEntryIds();
    if (!ids.length) return;
    let idx = ids.findIndex((x) => x.id === getState().selected.id);
    if (idx < 0) idx = delta > 0 ? -1 : 0;
    const next = Math.max(0, Math.min(ids.length - 1, idx + delta));
    navigate({ selected: ids[next] }, { replace: true });
  }
  __name(stepList, "stepList");
  async function stepTimeline(delta) {
    const state2 = getState();
    const { offset, count, matchCount } = loadedWindow(state2);
    const ids = lensEntryIds();
    if (!ids.length || matchCount === 0) return;
    const local = ids.findIndex((x) => x.id === state2.selected.id);
    if (local < 0) {
      navigate({ selected: ids[0] }, { replace: true });
      return;
    }
    const cur = offset + local;
    const target = Math.max(0, Math.min(matchCount - 1, cur + delta));
    if (target === cur) return;
    if (target >= offset && target < offset + count) {
      navigate({ selected: ids[target - offset] }, { replace: true });
      return;
    }
    await fetchHistoryPage({
      offset: target >= offset + count ? offset + count : Math.max(0, offset - HISTORY_PAGE)
    });
    const w = loadedWindow(getState());
    const loaded = lensEntryIds();
    const localAfter = target - w.offset;
    if (localAfter >= 0 && localAfter < loaded.length)
      navigate({ selected: loaded[localAfter] }, { replace: true });
  }
  __name(stepTimeline, "stepTimeline");
  async function stepSelectionAsync(delta) {
    if (getState().lens === "timeline") {
      await stepTimeline(delta);
      return;
    }
    stepList(delta);
  }
  __name(stepSelectionAsync, "stepSelectionAsync");
  function activateSelection() {
    const sel = getState().selected;
    if (sel.kind === "revision" && sel.id) {
      openRevisionDiff(sel.id);
    } else if (sel.kind === "event" && sel.id) {
      const event = (getState().history?.entries ?? []).find(
        (e) => e.eventId === sel.id
      );
      const rev = event ? entryRevisionId(event) : "";
      if (rev) openRevisionDiff(rev);
    }
  }
  __name(activateSelection, "activateSelection");
  function focusSearch() {
    if (getState().lens !== "timeline") navigate({ lens: "timeline" });
    $("#filter-text")?.focus();
  }
  __name(focusSearch, "focusSearch");
  function toggleHelp() {
    if (activeName() === "help") closeActive();
    else open("help", "#key-help-close");
  }
  __name(toggleHelp, "toggleHelp");
  function handleEscape() {
    if (activeName()) {
      closeActive();
      return;
    }
    const active = document.activeElement;
    if (isTypingTarget(active)) {
      if (active instanceof HTMLElement) active.blur();
      return;
    }
    if (getState().filterText) navigate({ filterText: "" }, { replace: true });
  }
  __name(handleEscape, "handleEscape");
  function onKey(ev) {
    if (trapFocus(ev)) return;
    const target = ev.target;
    const chip = target instanceof Element ? target.closest("[data-ref-kind]") : null;
    if (chip && (ev.key === "Enter" || ev.key === " ")) {
      ev.preventDefault();
      resolveRef(chip.dataset.refKind ?? "", chip.dataset.refId ?? "");
      return;
    }
    if ((ev.metaKey || ev.ctrlKey) && ev.key.toLowerCase() === "k") {
      ev.preventDefault();
      toggle();
      return;
    }
    if (ev.ctrlKey && ev.shiftKey && ev.key.toLowerCase() === "p") {
      ev.preventDefault();
      toggle();
      return;
    }
    if (ev.key === "Escape") {
      handleEscape();
      return;
    }
    if (isTypingTarget(document.activeElement)) return;
    if (getState().diff) {
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
      if (ev.key === "t") {
        navigate({ lens: "timeline" });
        return;
      }
      if (ev.key === "l") {
        navigate({ lens: "list" });
        return;
      }
      if (ev.key === "r") {
        navigate({ lens: "threads" });
        return;
      }
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
        toggleHelp();
        return;
      default:
        return;
    }
  }
  __name(onKey, "onKey");

  // src/prefs.ts
  var THEME_KEY = "shore-inspect-theme";
  var DENSITY_KEY = "shore-inspect-density";
  var liveMediaQueries = [];
  function hasPinnedTheme() {
    const stored = localStorage.getItem(THEME_KEY);
    return stored === "light" || stored === "dark";
  }
  __name(hasPinnedTheme, "hasPinnedTheme");
  function preferredTheme() {
    if (hasPinnedTheme()) return localStorage.getItem(THEME_KEY);
    return window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
  }
  __name(preferredTheme, "preferredTheme");
  function applyTheme(theme) {
    document.documentElement.setAttribute("data-theme", theme);
  }
  __name(applyTheme, "applyTheme");
  function toggleTheme() {
    const next = document.documentElement.getAttribute("data-theme") === "light" ? "dark" : "light";
    localStorage.setItem(THEME_KEY, next);
    applyTheme(next);
  }
  __name(toggleTheme, "toggleTheme");
  function preferredDensity() {
    return localStorage.getItem(DENSITY_KEY) || "comfortable";
  }
  __name(preferredDensity, "preferredDensity");
  function applyDensity(mode) {
    document.documentElement.classList.toggle("compact", mode === "compact");
  }
  __name(applyDensity, "applyDensity");
  function toggleDensity() {
    const next = document.documentElement.classList.contains("compact") ? "comfortable" : "compact";
    localStorage.setItem(DENSITY_KEY, next);
    applyDensity(next);
  }
  __name(toggleDensity, "toggleDensity");
  function applyPrefs() {
    applyTheme(preferredTheme());
    applyDensity(preferredDensity());
  }
  __name(applyPrefs, "applyPrefs");
  function watchColorScheme() {
    const query = window.matchMedia("(prefers-color-scheme: light)");
    liveMediaQueries.push(query);
    query.addEventListener("change", () => {
      if (hasPinnedTheme()) return;
      applyTheme(preferredTheme());
    });
  }
  __name(watchColorScheme, "watchColorScheme");
  function initControls5() {
    $("#theme-toggle")?.addEventListener("click", toggleTheme);
    $("#density-toggle")?.addEventListener("click", toggleDensity);
    watchColorScheme();
  }
  __name(initControls5, "initControls");

  // src/lenses/revisions.ts
  function renderRevisionList() {
    const el = $("#units");
    if (!el) return;
    const state2 = getState();
    const entries = (state2.revisions?.entries ?? []).filter(
      matchesRevisionFilters
    );
    if (!entries.length) {
      el.innerHTML = `<p class="${CLASS.empty}" style="color:var(--fg-dim)">${state2.filterText || state2.filterSnapshot ? "No revisions match the current filters." : "No captured revisions in this store."}</p>`;
      return;
    }
    const selected = state2.selected;
    const kv = /* @__PURE__ */ __name(([k, v]) => `<span>${escapeHtml(k)}</span><b>${escapeHtml(v)}</b>`, "kv");
    el.innerHTML = entries.map((u) => {
      const base = u.base ?? {};
      const overview = u.overview ?? overviewForRevision(u.revisionId ?? "");
      const revisionId = u.revisionId ?? "";
      const isSelected = selected.kind === "revision" && selected.id === revisionId;
      const badge = supersessionBadge(revisionId);
      const rows = [
        ["captured", fmtDateTime(u.capturedAt ?? "")],
        [
          "base",
          base.commitOid ? `${shortId(base.commitOid)} (${base.kind ?? ""})` : base.kind ?? "—"
        ]
      ];
      const tail = [["snapshot", shortId(u.snapshotId)]];
      const targetCell = `<span>target</span><b>${targetDisplayLabel(u.targetDisplay)}${targetHeadBadge(u.targetDisplay)}</b>`;
      return `<div class="${CLASS.unitCard}" data-revision-id="${escapeHtml(revisionId)}"${isSelected ? ' aria-selected="true"' : ""} title="${escapeHtml(revisionId)}
click to open the revision page">
      <h3>${escapeHtml(shortId(revisionId))}</h3>
      ${badge ? `<div class="${CLASS.supersessionBadges}">${badge}</div>` : ""}
      ${renderRevisionOverview(u, overview)}
      <div class="${CLASS.kv}">${rows.map(kv).join("")}${targetCell}${tail.map(kv).join("")}</div>
      <div class="${CLASS.actions}"><button class="${CLASS.ghost} ${CLASS.diffBtn}" data-open-diff="${escapeHtml(u.snapshotId ?? "")}" data-diff-hash="${escapeHtml(u.snapshotContentHash ?? "")}">view snapshot diff</button></div>
    </div>`;
    }).join("");
  }
  __name(renderRevisionList, "renderRevisionList");
  function renderRevisions() {
    const el = $("#revisions");
    if (!el) return;
    const state2 = getState();
    const threads = currentThreads().filter(threadMatchesRevisionFilters);
    if (!threads.length) {
      el.innerHTML = `<p class="${CLASS.empty}" style="color:var(--fg-dim)">${state2.filterText || state2.filterSnapshot ? "No revision threads match the current filters." : "No captured revisions in this store."}</p>`;
      return;
    }
    el.innerHTML = "";
    for (const thread of threads) el.appendChild(renderThreadCard(thread));
  }
  __name(renderRevisions, "renderRevisions");
  function threadLabel(thread) {
    const heads = thread.heads ?? [];
    if (thread.competing)
      return `revision thread · ${heads.length} competing heads`;
    if (heads.length === 1)
      return `revision thread · current in thread ${shortId(heads[0])}`;
    return "revision thread";
  }
  __name(threadLabel, "threadLabel");
  function renderThreadCard(thread) {
    const revisions = thread.revisions ?? [];
    const heads = thread.heads ?? [];
    const superseded = thread.superseded ?? [];
    const card = document.createElement("div");
    card.className = `unit-card thread-card${thread.competing ? " competing" : ""}`;
    const competingBadge = thread.competing ? `<div class="${CLASS.threadCompeting}"><span class="${CLASS.factStatus} ${CLASS.competing}">competing revisions (${heads.length})</span> ${heads.map((h) => linkify(h)).join(" ")}</div>` : "";
    const overviewBlocks = heads.map((h) => renderThreadRevisionOverview(h)).filter(Boolean).join("");
    card.innerHTML = `
    <h3>${escapeHtml(threadLabel(thread))}</h3>
    ${competingBadge}
    ${overviewBlocks ? `<div class="${CLASS.threadOverviews}">${overviewBlocks}</div>` : ""}
    <div class="${CLASS.kv}">
      <span>revisions</span><b>${escapeHtml(String(revisions.length))}</b>
      <span>heads</span><b>${escapeHtml(String(heads.length))}</b>
      <span>superseded</span><b>${escapeHtml(String(superseded.length))}</b>
    </div>
    ${renderThreadSvg(thread.laidOut)}`;
    wireDagInteractions(card);
    return card;
  }
  __name(renderThreadCard, "renderThreadCard");
  function renderThreadSvg(laid) {
    return renderSupersessionSvg(laid, {
      idAttr: "data-revision-id",
      ariaNoun: "revision",
      interactive: true,
      isSelected: /* @__PURE__ */ __name((id) => {
        const s = getState().selected;
        return s.kind === "revision" && s.id === id;
      }, "isSelected")
    });
  }
  __name(renderThreadSvg, "renderThreadSvg");
  function wireDagInteractions(card) {
    const nav = /* @__PURE__ */ __name((node) => {
      const id = node.getAttribute("data-revision-id");
      if (id)
        navigate({
          selected: { kind: "revision", id },
          diff: null,
          diffHash: null,
          focus: null
        });
    }, "nav");
    for (const node of Array.from(
      card.querySelectorAll(".dag-node")
    )) {
      node.addEventListener("click", () => nav(node));
      node.addEventListener("keydown", (ev) => {
        if (ev.key === "Enter" || ev.key === " ") {
          ev.preventDefault();
          nav(node);
        }
      });
      const trace = /* @__PURE__ */ __name((on) => {
        const id = node.getAttribute("data-revision-id");
        node.classList.toggle("traced", on);
        for (const edge of Array.from(
          card.querySelectorAll(
            `.dag-edge[data-from="${id}"], .dag-edge[data-to="${id}"]`
          )
        )) {
          edge.classList.toggle("traced", on);
          edge.setAttribute(
            "marker-end",
            on ? "url(#dag-arrow-traced)" : "url(#dag-arrow)"
          );
        }
      }, "trace");
      node.addEventListener("mouseenter", () => trace(true));
      node.addEventListener("mouseleave", () => trace(false));
      node.addEventListener("focus", () => trace(true));
      node.addEventListener("blur", () => trace(false));
    }
  }
  __name(wireDagInteractions, "wireDagInteractions");

  // src/render.ts
  var lastMasterLens = null;
  function renderStats() {
    const h = getState().history;
    const r = getState().revisions;
    const o = getState().threads;
    const events = $("#stat-events");
    if (events) events.textContent = `${h?.eventCount ?? "—"} events`;
    const units = $("#stat-units");
    if (units) units.textContent = `${r?.revisionCount ?? "—"} units`;
    const threads = $("#stat-threads");
    if (threads) threads.textContent = `${o?.threadCount ?? "—"} threads`;
    const hash = $("#stat-hash");
    if (hash) hash.textContent = shortId(h?.eventSetHash);
  }
  __name(renderStats, "renderStats");
  function renderDiagnostics() {
    const el = $("#diagnostics");
    if (!el) return;
    const diags = getState().history?.diagnostics ?? [];
    if (!diags.length) {
      el.classList.add("hidden");
      el.innerHTML = "";
      return;
    }
    el.classList.remove("hidden");
    el.innerHTML = diags.map((raw) => {
      const d = raw ?? {};
      return `<div><span class="${CLASS.code}">${escapeHtml(d.code || "diagnostic")}</span>${escapeHtml(d.message || "")}</div>`;
    }).join("");
  }
  __name(renderDiagnostics, "renderDiagnostics");
  function renderTypeToggles() {
    const container = $("#filter-types");
    if (!container) return;
    container.innerHTML = "";
    const counts = getState().history?.facets ?? {};
    const state2 = getState();
    for (const id of presentTypes()) {
      if (!state2.seenTypes.has(id)) {
        state2.seenTypes.add(id);
        state2.enabledTypes.add(id);
      }
      const enabled = state2.enabledTypes.has(id);
      const count = counts[id] ?? 0;
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = `type-toggle${enabled ? "" : " off"}`;
      btn.dataset.type = id;
      btn.setAttribute("aria-pressed", String(enabled));
      btn.setAttribute(
        "aria-label",
        `${enabled ? "Hide" : "Show"} ${typeLabel(id)} events (${count})`
      );
      btn.innerHTML = `<span class="${CLASS.dot}" style="background:${typeColor(id)}"></span>${escapeHtml(typeLabel(id))}<span class="${CLASS.typeCount}">${count}</span>`;
      btn.title = id;
      container.appendChild(btn);
    }
  }
  __name(renderTypeToggles, "renderTypeToggles");
  function renderLensSwitcher() {
    const lens = getState().lens;
    for (const tab of document.querySelectorAll(".lens-tab")) {
      tab.setAttribute("aria-pressed", String(tab.dataset.lens === lens));
    }
  }
  __name(renderLensSwitcher, "renderLensSwitcher");
  function syncControls() {
    const state2 = getState();
    const text = $("#filter-text");
    if (text && text.value !== state2.filterText) text.value = state2.filterText;
    const order = $("#order-toggle");
    if (order)
      order.textContent = state2.order === "desc" ? "newest first" : "oldest first";
    const toolbar = $("#toolbar");
    if (toolbar) toolbar.classList.toggle("hidden", state2.lens !== "timeline");
  }
  __name(syncControls, "syncControls");
  function renderMaster() {
    const master = $("#master");
    if (!master) return;
    const lens = getState().lens;
    if (lens !== lastMasterLens) {
      lastMasterLens = lens;
      if (lens === "list") {
        master.innerHTML = `<div id="units" class="${CLASS.units}"></div>`;
      } else if (lens === "threads") {
        master.innerHTML = `<div id="revisions" class="${CLASS.units}" aria-label="supersession threads"></div>`;
      } else {
        master.innerHTML = `<ol id="timeline" class="${CLASS.timeline}" aria-label="event timeline"></ol>`;
      }
    }
    if (lens === "list") renderRevisionList();
    else if (lens === "threads") renderRevisions();
    else renderTimeline();
  }
  __name(renderMaster, "renderMaster");
  function renderSelected() {
    const sel = getState().selected;
    if (sel.kind === "revision" && sel.id) void showComposite(sel.id);
    else renderDetail();
  }
  __name(renderSelected, "renderSelected");
  function scrollSelectionIntoView() {
    const sel = getState().selected;
    if (!sel.id) return;
    if (sel.kind === "event") {
      scrollTimelineSelectionIntoView(sel.id);
      return;
    }
    const master = $("#master");
    if (!master) return;
    const el = master.querySelector(`[data-revision-id="${sel.id}"]`);
    if (el) el.scrollIntoView({ block: "center" });
  }
  __name(scrollSelectionIntoView, "scrollSelectionIntoView");
  function render() {
    renderStats();
    renderDiagnostics();
    renderLensSwitcher();
    syncControls();
    renderTypeToggles();
    renderMaster();
    renderSelected();
    scrollSelectionIntoView();
    void renderDiffOverlay();
  }
  __name(render, "render");
  function onTypeToggleClick(ev) {
    const t = ev.target;
    if (!(t instanceof Element)) return;
    const btn = t.closest("[data-type]");
    const id = btn?.dataset.type;
    if (!id) return;
    const types = new Set(getState().enabledTypes);
    if (types.has(id)) types.delete(id);
    else types.add(id);
    navigate({ enabledTypes: types }, { replace: true });
  }
  __name(onTypeToggleClick, "onTypeToggleClick");
  function onMasterClick(ev) {
    const t = ev.target;
    if (!(t instanceof Element)) return;
    if (t.closest("[data-ref-kind]")) return;
    const cue = t.closest("[data-attention-query]");
    if (cue) {
      const query = cue.dataset.attentionQuery;
      if (query) navigate({ filterText: query });
      return;
    }
    const diffBtn = t.closest("[data-open-diff]");
    if (diffBtn) {
      const snapshotId = diffBtn.dataset.openDiff;
      if (snapshotId)
        openDiff(snapshotId, null, diffBtn.dataset.diffHash || null);
      return;
    }
    const eventEl = t.closest("[data-event-id]");
    if (eventEl) {
      const id = eventEl.dataset.eventId;
      if (id) navigate({ selected: { kind: "event", id } });
      return;
    }
    const revEl = t.closest(".unit-card[data-revision-id]");
    if (revEl) {
      const id = revEl.dataset.revisionId;
      if (id) navigate({ selected: { kind: "revision", id } });
    }
  }
  __name(onMasterClick, "onMasterClick");
  function initControls6() {
    $("#master")?.addEventListener("click", onMasterClick);
    $("#filter-types")?.addEventListener("click", onTypeToggleClick);
  }
  __name(initControls6, "initControls");

  // src/main.ts
  function wireToolbar() {
    for (const tab of document.querySelectorAll(".lens-tab")) {
      tab.addEventListener("click", () => {
        const lens = tab.dataset.lens;
        navigate({ lens: lens && LENSES.includes(lens) ? lens : DEFAULT_LENS });
      });
    }
    const filterText = $("#filter-text");
    filterText?.addEventListener("input", () => {
      navigate({ filterText: filterText.value }, { replace: true });
    });
    $("#filter-clear")?.addEventListener("click", () => {
      navigate(
        {
          filterText: "",
          filterTrack: "",
          filterSnapshot: "",
          enabledTypes: new Set(presentTypes())
        },
        { replace: true }
      );
    });
    $("#order-toggle")?.addEventListener("click", () => {
      navigate(
        { order: getState().order === "desc" ? "asc" : "desc" },
        { replace: true }
      );
    });
  }
  __name(wireToolbar, "wireToolbar");
  function main() {
    applyPrefs();
    subscribe(render);
    subscribe(maybeReloadForQuery);
    initControls5();
    initControls();
    initControls4();
    initControls3();
    initControls6();
    initControls2();
    wireToolbar();
    document.addEventListener("keydown", onKey);
    document.addEventListener("click", onDocumentClick);
    window.addEventListener("popstate", applyHash);
    window.addEventListener("hashchange", applyHash);
    return load().then(() => {
      applyHash();
      const refresh = $("#refresh");
      if (refresh) refresh.textContent = "watching";
      setInterval(() => {
        void pollFreshness();
      }, 3e3);
    });
  }
  __name(main, "main");

  // src/entry.ts
  void main();
})();
