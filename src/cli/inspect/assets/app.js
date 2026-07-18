"use strict";
(() => {
  var __defProp = Object.defineProperty;
  var __name = (target, value) => __defProp(target, "name", { value, configurable: true });

  // src/auth.ts
  var SESSION_TOKEN_PREFIX = "pointbreak.inspect-token.v1:";
  var credentialVersion = 0;
  function sessionTokenKey(origin = location.origin) {
    return `${SESSION_TOKEN_PREFIX}${origin}`;
  }
  __name(sessionTokenKey, "sessionTokenKey");
  function getSessionToken() {
    return sessionStorage.getItem(sessionTokenKey());
  }
  __name(getSessionToken, "getSessionToken");
  function setSessionToken(token) {
    if (!token) throw new Error("invalid capability");
    sessionStorage.setItem(sessionTokenKey(), token);
    credentialVersion += 1;
  }
  __name(setSessionToken, "setSessionToken");
  function sessionCredentialVersion() {
    return credentialVersion;
  }
  __name(sessionCredentialVersion, "sessionCredentialVersion");
  function decoded(value) {
    try {
      return decodeURIComponent(value.replace(/\+/g, "%20"));
    } catch {
      throw new Error("invalid capability");
    }
  }
  __name(decoded, "decoded");
  function extractCapability(hash) {
    const prefixed = hash.startsWith("#") ? hash : `#${hash}`;
    const queryAt = prefixed.indexOf("?");
    if (queryAt < 0) return { token: null, cleanedHash: prefixed };
    const route = prefixed.slice(0, queryAt);
    const kept = [];
    const tokens = [];
    for (const pair of prefixed.slice(queryAt + 1).split("&")) {
      if (!pair) continue;
      const separator = pair.indexOf("=");
      const rawKey = separator < 0 ? pair : pair.slice(0, separator);
      const rawValue = separator < 0 ? "" : pair.slice(separator + 1);
      if (decoded(rawKey) === "token") tokens.push(decoded(rawValue));
      else kept.push(pair);
    }
    if (tokens.length > 1 || tokens.length === 1 && !tokens[0]) {
      throw new Error("invalid capability");
    }
    return {
      token: tokens[0] ?? null,
      cleanedHash: kept.length ? `${route}?${kept.join("&")}` : route
    };
  }
  __name(extractCapability, "extractCapability");
  function bootstrapCapability() {
    const result = extractCapability(location.hash);
    if (result.token !== null) {
      setSessionToken(result.token);
      history.replaceState(
        history.state,
        "",
        `${location.pathname}${location.search}${result.cleanedHash}`
      );
    }
    return result;
  }
  __name(bootstrapCapability, "bootstrapCapability");
  function isLoopbackLiteral(hostname) {
    const unbracketed = hostname.replace(/^\[|\]$/g, "").toLowerCase();
    if (unbracketed === "::1") return true;
    const octets = unbracketed.split(".");
    return octets.length === 4 && octets.every((octet) => /^\d+$/.test(octet) && Number(octet) <= 255) && Number(octets[0]) === 127;
  }
  __name(isLoopbackLiteral, "isLoopbackLiteral");
  function routeWithToken(route, token) {
    const cleaned = extractCapability(route).cleanedHash;
    const separator = cleaned.includes("?") ? "&" : "?";
    return `${cleaned}${separator}token=${encodeURIComponent(token)}`;
  }
  __name(routeWithToken, "routeWithToken");
  function resolveReconnectInput(input, currentOrigin, currentRoute) {
    const value = input.trim();
    if (!value) throw new Error("invalid capability URL");
    if (!/^[a-z][a-z0-9+.-]*:/i.test(value)) {
      return { kind: "retry", token: value };
    }
    let url;
    try {
      url = new URL(value);
    } catch {
      throw new Error("invalid capability URL");
    }
    if (url.protocol !== "http:" || !isLoopbackLiteral(url.hostname) || url.username || url.password) {
      throw new Error("invalid capability URL");
    }
    let extraction;
    try {
      extraction = extractCapability(url.hash);
    } catch {
      throw new Error("invalid capability URL");
    }
    if (!extraction.token) throw new Error("invalid capability URL");
    if (url.origin === currentOrigin) {
      return { kind: "retry", token: extraction.token };
    }
    return {
      kind: "navigate",
      url: `${url.origin}/${routeWithToken(currentRoute, extraction.token)}`
    };
  }
  __name(resolveReconnectInput, "resolveReconnectInput");
  var AuthCoordinator = class {
    constructor(options) {
      this.options = options;
    }
    options;
    static {
      __name(this, "AuthCoordinator");
    }
    recovery = null;
    recoverUnauthorized() {
      if (this.recovery) return this.recovery;
      this.recovery = this.promptAndApply().finally(() => {
        this.recovery = null;
      });
      return this.recovery;
    }
    reconnect() {
      return this.recoverUnauthorized();
    }
    async promptAndApply() {
      while (true) {
        const input = await this.options.prompt();
        if (input === null) {
          clearReconnectError();
          return false;
        }
        let target;
        try {
          target = resolveReconnectInput(
            input,
            this.options.currentOrigin(),
            this.options.currentRoute()
          );
        } catch {
          showReconnectError("Enter a token or an HTTP loopback capability URL.");
          continue;
        }
        clearReconnectError();
        if (target.kind === "retry") {
          setSessionToken(target.token);
          return true;
        }
        this.options.navigate(target.url);
        return false;
      }
    }
  };
  var installedCoordinator = null;
  function installAuthCoordinator(coordinator) {
    installedCoordinator = coordinator;
  }
  __name(installAuthCoordinator, "installAuthCoordinator");
  function recoverUnauthorized() {
    return installedCoordinator?.recoverUnauthorized() ?? Promise.resolve(false);
  }
  __name(recoverUnauthorized, "recoverUnauthorized");
  function requestReconnect() {
    return installedCoordinator?.reconnect() ?? Promise.resolve(false);
  }
  __name(requestReconnect, "requestReconnect");
  function showReconnectError(message) {
    const error = document.querySelector("#reconnect-error");
    if (!error) return;
    error.textContent = message;
    error.classList.remove("hidden");
  }
  __name(showReconnectError, "showReconnectError");
  function clearReconnectError() {
    const error = document.querySelector("#reconnect-error");
    if (!error) return;
    error.textContent = "";
    error.classList.add("hidden");
  }
  __name(clearReconnectError, "clearReconnectError");
  function promptForCredential() {
    const dialog = document.querySelector("#reconnect-dialog");
    const form = dialog?.querySelector("form");
    const input = document.querySelector("#reconnect-input");
    const cancel = document.querySelector("#reconnect-cancel");
    if (!dialog || !form || !input || !cancel) return Promise.resolve(null);
    dialog.classList.remove("hidden");
    input.value = "";
    input.focus();
    return new Promise((resolve2) => {
      let settled = false;
      const finish = /* @__PURE__ */ __name((value) => {
        if (settled) return;
        settled = true;
        form.removeEventListener("submit", onSubmit);
        cancel.removeEventListener("click", onCancel);
        input.value = "";
        dialog.classList.add("hidden");
        resolve2(value);
      }, "finish");
      const onSubmit = /* @__PURE__ */ __name((event) => {
        event.preventDefault();
        finish(input.value);
      }, "onSubmit");
      const onCancel = /* @__PURE__ */ __name(() => finish(null), "onCancel");
      form.addEventListener("submit", onSubmit);
      cancel.addEventListener("click", onCancel);
    });
  }
  __name(promptForCredential, "promptForCredential");

  // src/classNames.ts
  var CLASS = {
    // App chrome, master-detail panes, lens containers, and shared chips.
    units: "units",
    timeline: "timeline",
    empty: "empty",
    badge: "badge",
    tierMedium: "tier-medium",
    body: "body",
    title: "title",
    time: "time",
    eventDate: "event-date",
    rail: "rail",
    meta: "meta",
    type: "type",
    typeCount: "type-count",
    code: "code",
    dot: "dot",
    kv: "kv",
    ghost: "ghost",
    actions: "actions",
    timelineShell: "timeline-shell",
    timelineNewPill: "timeline-new-pill",
    // (The app-shell store-identity chip + detail popover is static markup in
    // index.html — `store-identity*` classes live there and in app.css, not here —
    // and its rows are `renderIdentity`-filled <dt>/<dd> styled via element selectors.
    // Issue #391.)
    // Fact cards (observation / input-request / assessment / validation / note).
    annoGroup: "anno-group",
    annoHead: "anno-head",
    annoLoc: "anno-loc",
    annoSummary: "anno-summary",
    annoTime: "anno-time",
    annoTitle: "anno-title",
    annoTrack: "anno-track",
    actorAttribution: "actor-attribution",
    factBodyRemoved: "fact-body-removed",
    factRel: "fact-rel",
    factResponse: "fact-response",
    factResponses: "fact-responses",
    factStaleContext: "fact-stale-context",
    factStatus: "fact-status",
    outcome: "outcome",
    advisoryNote: "advisory-note",
    validationNote: "validation-note",
    validationContinuity: "validation-continuity",
    validationContinuityNeutral: "validation-continuity-neutral",
    validationContinuityOutstanding: "validation-continuity-outstanding",
    readback: "readback",
    readbackRow: "readback-row",
    readerScopeNote: "reader-scope-note",
    rawEvent: "raw-event",
    rawEventActions: "raw-event-actions",
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
    overviewHistoryCue: "overview-history-cue",
    overviewCues: "overview-cues",
    overviewLabel: "overview-label",
    overviewLatest: "overview-latest",
    overviewMain: "overview-main",
    overviewMuted: "overview-muted",
    revisionDiagnostic: "revision-diagnostic",
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
    diffAnchorReason: "diff-anchor-reason",
    diffDecisionContext: "diff-decision-context",
    diffDecisionContextNav: "diff-decision-context-nav",
    diffFactVicinity: "diff-fact-vicinity",
    diffFileNotice: "diff-file-notice",
    diffNavFact: "diff-nav-fact",
    diffNavFile: "diff-nav-file",
    diffNavFiles: "diff-nav-files",
    diffNavReason: "diff-nav-reason",
    diffNavSummary: "diff-nav-summary",
    diffUnanchored: "diff-unanchored",
    diffUnanchoredFacts: "diff-unanchored-facts",
    dpath: "dpath",
    drow: "drow",
    drowMeta: "drow-meta",
    dtext: "dtext",
    emph: "emph",
    ln: "ln",
    sign: "sign",
    // Revision list, supersession badges, and the laid-out DAG.
    unitCard: "unit-card",
    unitPage: "unit-page",
    unitPageTitle: "unit-page-title",
    supersessionBadges: "supersession-badges",
    competing: "competing",
    revisionSupersession: "revision-supersession",
    revisionHeads: "revision-heads",
    revisionSelf: "revision-self",
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
    // The applied-filter chip row (the toolbar's pure view of filterText).
    filterChips: "filter-chips",
    filterChipRemove: "filter-chip-remove",
    // The type facet section (the Timeline-only ?type= page-set control): static
    // container/list classes in index.html; rows are emitted via typeFacetRowClass.
    typeFacet: "type-facet",
    typeFacetMenu: "type-facet-menu",
    // The search-bar suggestion popover: static list container in index.html;
    // the rows are emitted via suggestionClass below.
    filterSuggestions: "filter-suggestions",
    suggestion: "suggestion",
    suggestionActive: "suggestion-active",
    // The command palette.
    cmdEmpty: "cmd-empty",
    cmdGroup: "cmd-group",
    cmdHint: "cmd-hint",
    cmdLabel: "cmd-label",
    // The attention lens: tiered cards over the outstanding review state.
    attentionCard: "attention-card",
    attentionTier: "attention-tier",
    attentionEmpty: "attention-empty",
    attentionOrderLabel: "attention-order-label",
    attentionKind: "attention-kind",
    attentionMeta: "attention-meta",
    attentionFreshness: "attention-freshness",
    attentionFocus: "attention-focus",
    attentionDelta: "attention-delta",
    // The attention tab's judgment-queue count badge (absent when both tiers are
    // empty) and the muted advisory count beside the needs-input number.
    attentionBadge: "attention-badge",
    attentionBadgeSecondary: "attention-badge-secondary",
    // The detail page's per-revision outstanding set (the scoped attention read);
    // absent when nothing is outstanding on the shown revision.
    outstandingSet: "outstanding-set",
    // Copyable CLI command handoffs (workflow-handoff.ts): the block, its label,
    // the command code, the visible placeholder marker, the clipboard-only copy
    // control, and the detail page's stage-template section host.
    workflowHandoff: "workflow-handoff",
    workflowHandoffLabel: "workflow-handoff-label",
    workflowCommand: "workflow-command",
    workflowPlaceholder: "workflow-placeholder",
    workflowCopy: "workflow-copy",
    workflowHandoffs: "workflow-handoffs"
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
    "track",
    "actor"
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
  var cmdItemClass = /* @__PURE__ */ __name((active2) => `cmd-item${active2 ? " active" : ""}`, "cmdItemClass");
  var filterChipClass = /* @__PURE__ */ __name((negated) => `filter-chip${negated ? " filter-chip-negated" : ""}`, "filterChipClass");
  var typeFacetRowClass = /* @__PURE__ */ __name((enabled) => `type-facet-row${enabled ? "" : " type-facet-row-off"}`, "typeFacetRowClass");
  var suggestionClass = /* @__PURE__ */ __name((active2) => `suggestion${active2 ? " suggestion-active" : ""}`, "suggestionClass");
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
        filterChipClass(true),
        typeFacetRowClass(true),
        typeFacetRowClass(false),
        suggestionClass(true),
        dagNodeClass({ isHead: true, isSuperseded: true }),
        bodyClass("anno-body", true),
        bodyClass("verdict-summary", true),
        cmdItemClass(true)
      ])
    )
  ];

  // src/dom.ts
  function $(sel) {
    return document.querySelector(sel);
  }
  __name($, "$");

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
  var RFC3339_UTC = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?Z$/;
  function parseRfc3339UtcMillis(value) {
    const match = value.match(RFC3339_UTC);
    if (!match) return null;
    const [
      ,
      yearText,
      monthText,
      dayText,
      hourText,
      minuteText,
      secondText,
      fraction
    ] = match;
    const year = Number(yearText);
    const month = Number(monthText);
    const day = Number(dayText);
    const hour = Number(hourText);
    const minute = Number(minuteText);
    const second = Number(secondText);
    const leapYear = year % 4 === 0 && year % 100 !== 0 || year % 400 === 0;
    const daysInMonth = [
      31,
      leapYear ? 29 : 28,
      31,
      30,
      31,
      30,
      31,
      31,
      30,
      31,
      30,
      31
    ];
    if (month < 1 || month > 12 || day < 1 || day > daysInMonth[month - 1] || hour > 23 || minute > 59 || second > 60) {
      return null;
    }
    const millis = Number((fraction ?? "").padEnd(3, "0").slice(0, 3));
    const date = /* @__PURE__ */ new Date(0);
    date.setUTCFullYear(year, month - 1, day);
    date.setUTCHours(hour, minute, Math.min(second, 59), millis);
    return date.getTime() + (second === 60 ? 1e3 : 0);
  }
  __name(parseRfc3339UtcMillis, "parseRfc3339UtcMillis");
  function parseMs(occurredAt) {
    if (typeof occurredAt !== "string") return null;
    if (occurredAt.startsWith("unix-ms:")) {
      const unixMillis = occurredAt.match(/^unix-ms:([+-]?\d+)$/);
      return unixMillis ? Number(unixMillis[1]) : null;
    }
    if (/^\d{4}-\d{2}-\d{2}T/.test(occurredAt))
      return parseRfc3339UtcMillis(occurredAt);
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
  function fmtDate(occurredAt) {
    const ms = parseMs(occurredAt);
    if (ms == null) return occurredAt || "";
    return new Date(ms).toLocaleDateString();
  }
  __name(fmtDate, "fmtDate");

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
  var LENSES = ["timeline", "list", "attention"];
  var DEFAULT_LENS = "timeline";
  var EVENT_QUERY_FIELDS = [
    "type",
    "track",
    "actor",
    "revision",
    "snapshot",
    "check",
    "assessment",
    "is",
    "tag",
    "before",
    "after"
  ];
  var REVISION_QUERY_FIELDS = [
    "track",
    "actor",
    "revision",
    "snapshot",
    "assessment",
    "is",
    "tag",
    "attention",
    "before",
    "after"
  ];
  var KNOWN_QUERY_KEYS = [
    "type",
    "track",
    "actor",
    "revision",
    "snapshot",
    "check",
    "assessment",
    "is",
    "tag",
    "attention",
    "before",
    "after",
    "status",
    "object"
  ];
  var REVISION_ATTENTION_VALUES = [
    "open-request",
    "unassessed",
    "validation-context",
    "follow-up",
    "stale-fact"
  ];
  var DEFAULT_OPEN_FILES = 10;
  var LARGE_FILE_ROWS = 500;
  var OVERLAY_SELECTORS = {
    palette: "#cmd-palette",
    help: "#key-help"
  };
  var SUPERSEDABLE_FACT_TYPES = /* @__PURE__ */ new Set([
    "review_observation_recorded",
    "review_assessment_recorded",
    "input_request_opened",
    "validation_check_recorded"
  ]);

  // src/query.ts
  var RANGE_ANCHOR_FIELD = "occurred_at";
  function normalizeTimeSlot(raw) {
    if (typeof raw !== "string" || raw === "") return "";
    const epoch = /Z$/.test(raw) ? Date.parse(raw) : parseMs(raw);
    return epoch == null || Number.isNaN(epoch) ? "" : new Date(epoch).toISOString();
  }
  __name(normalizeTimeSlot, "normalizeTimeSlot");
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
  var EVENT_VALUE_SETS = {
    is: ["open", "answered"]
  };
  var REVISION_VALUE_SETS = {
    is: [
      "open",
      "answered",
      "unassessed",
      "stale",
      "follow-up",
      "contested",
      "superseded"
    ],
    attention: REVISION_ATTENTION_VALUES
  };
  function parseSearchQueryFor(q, surface) {
    const fields = surface === "revision" ? REVISION_QUERY_FIELDS : EVENT_QUERY_FIELDS;
    const valueSets = surface === "revision" ? REVISION_VALUE_SETS : EVENT_VALUE_SETS;
    const clauses = [];
    const diagnostics = [];
    for (let tok of tokenizeQuery(q || "")) {
      let negate = false;
      if (tok.length > 1 && tok[0] === "-") {
        negate = true;
        tok = tok.slice(1);
      }
      const colon = tok.indexOf(":");
      const key = colon > 0 ? tok.slice(0, colon).toLowerCase() : "";
      if (!key) {
        pushText(clauses, tok, negate);
        continue;
      }
      const value = tok.slice(colon + 1).replace(/^"|"$/g, "").toLowerCase();
      const [field, deprecatedFrom] = resolveAlias(key, surface);
      if (fields.includes(field)) {
        const allowed = valueSets[field];
        if (allowed && !allowed.includes(value)) {
          diagnostics.push({
            code: "unsupported-value",
            key: field,
            message: `\`${field}:${value}\` — expected one of: ${allowed.join(", ")}`
          });
          continue;
        }
        if (deprecatedFrom)
          diagnostics.push({
            code: "deprecated-qualifier",
            key: deprecatedFrom,
            message: `\`${deprecatedFrom}:\` is deprecated; use \`${field}:\``
          });
        clauses.push({
          kind: "field",
          field,
          value: canonicalizeFieldValue(field, value),
          negate
        });
      } else if (KNOWN_QUERY_KEYS.includes(key)) {
        diagnostics.push({
          code: "unsupported-qualifier",
          key,
          message: `\`${key}:\` is not a filter on the ${surface === "revision" ? "revisions" : "timeline"} view`
        });
      } else {
        pushText(clauses, tok, negate);
      }
    }
    return { clauses, diagnostics };
  }
  __name(parseSearchQueryFor, "parseSearchQueryFor");
  function canonicalizeFieldValue(field, value) {
    if (field === "actor" && value && !value.startsWith("actor:") && !value.startsWith("did:key:"))
      return `actor:${value}`;
    return value;
  }
  __name(canonicalizeFieldValue, "canonicalizeFieldValue");
  function pushText(clauses, tok, negate) {
    const term = tok.replace(/^"|"$/g, "").toLowerCase();
    if (term) clauses.push({ kind: "text", value: term, negate });
  }
  __name(pushText, "pushText");
  function resolveAlias(key, surface) {
    if (key === "object") return ["snapshot", null];
    if (key === "status")
      return [surface === "revision" ? "assessment" : "check", "status"];
    return [key, null];
  }
  __name(resolveAlias, "resolveAlias");
  function fieldMatches(idx, field, value) {
    switch (matchKindFor(field)) {
      case "exact":
        return exactMatches(idx, field, value);
      case "set":
        return (idx[field] || "").includes(` ${value} `);
      case "range-after": {
        const anchor = (idx[RANGE_ANCHOR_FIELD] || "").toLowerCase();
        return anchor !== "" && anchor > rangeBound(value);
      }
      case "range-before": {
        const anchor = (idx[RANGE_ANCHOR_FIELD] || "").toLowerCase();
        return anchor !== "" && anchor < rangeBound(value);
      }
      default:
        return (idx[field] || "").toLowerCase().includes(value);
    }
  }
  __name(fieldMatches, "fieldMatches");
  var RFC3339_BOUND = /^\d{4}-\d{2}-\d{2}t\d{2}:\d{2}:\d{2}(\.\d+)?z$/;
  function rangeBound(value) {
    if (!RFC3339_BOUND.test(value)) return value;
    const upper = value.toUpperCase();
    const iso = normalizeTimeSlot(upper);
    if (!iso || iso.slice(0, 19) !== upper.slice(0, 19)) return value;
    return iso.toLowerCase();
  }
  __name(rangeBound, "rangeBound");
  function matchKindFor(field) {
    if (["type", "check", "assessment"].includes(field)) return "exact";
    if (["track", "actor", "is", "tag", "attention"].includes(field))
      return "set";
    if (field === "before") return "range-before";
    if (field === "after") return "range-after";
    return "substring";
  }
  __name(matchKindFor, "matchKindFor");
  function exactMatches(idx, field, value) {
    if (field === "type") {
      return value.split(",").some((v) => {
        const known = TYPES.find((t) => t.label === v || t.id === v);
        return (idx.type || "") === (known ? known.id : v);
      });
    }
    if (field === "assessment")
      return (idx.assessment || "") === resolveAssessmentValue(value);
    return (idx[field] || "").toLowerCase() === value;
  }
  __name(exactMatches, "exactMatches");
  function resolveAssessmentValue(value) {
    for (const [wire, label] of Object.entries(ASSESSMENT_LABELS)) {
      if (wire === value || label === value) return wire;
    }
    return value;
  }
  __name(resolveAssessmentValue, "resolveAssessmentValue");
  function matchesQuery(idx, clauses) {
    for (const c of clauses) {
      const hit = c.kind === "field" ? fieldMatches(idx, c.field, c.value) : idx.text.includes(c.value);
      if (c.negate ? hit : !hit) return false;
    }
    return true;
  }
  __name(matchesQuery, "matchesQuery");

  // src/refs.ts
  function workLabelText(td) {
    return escapeHtml(td?.workLabel?.text || "working-tree changes");
  }
  __name(workLabelText, "workLabelText");
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
  function linkifyEscaped(escaped, opts = {}) {
    const tabIndex = typeof opts === "object" ? opts.tabIndex ?? 0 : 0;
    return escaped.replace(REF_RE, (token) => {
      const info = refInfo(token);
      if (!info) return token;
      const display = escapeHtml(shortRef(token));
      if (!info.clickable) {
        return `<span class="${refClass(info.kind)}" title="${escapeHtml(token)}">${display}</span>`;
      }
      return `<span class="${refClass(info.kind)}" role="link" tabindex="${tabIndex}" data-ref-kind="${info.kind}" data-ref-id="${escapeHtml(token)}" title="${escapeHtml(token)}">${display}</span>`;
    });
  }
  __name(linkifyEscaped, "linkifyEscaped");
  function linkify(text, opts = {}) {
    return linkifyEscaped(escapeHtml(String(text ?? "")), opts);
  }
  __name(linkify, "linkify");
  function actorChip(actorId, opts = {}) {
    if (!actorId) return "";
    const tabIndex = typeof opts === "object" ? opts.tabIndex ?? 0 : opts;
    const display = escapeHtml(actorId);
    return `<span class="${refClass("actor")}" role="link" tabindex="${tabIndex}" data-ref-kind="actor" data-ref-id="${escapeHtml(actorId)}" title="filter to ${escapeHtml(actorId)}">${display}</span>`;
  }
  __name(actorChip, "actorChip");
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

  // src/projection.ts
  var SNAPSHOT_CONTENT_UNAVAILABLE = "snapshot_content_unavailable";
  function revisionSnapshotUnavailable(r) {
    return (r.diagnostics ?? []).some(
      (diagnostic) => diagnostic.code === SNAPSHOT_CONTENT_UNAVAILABLE
    );
  }
  __name(revisionSnapshotUnavailable, "revisionSnapshotUnavailable");
  function revisionDiagnostics(r) {
    const diagnostics = r.diagnostics ?? [];
    return diagnostics.map(
      (diagnostic) => `<div class="${CLASS.revisionDiagnostic}" role="status"><b>${escapeHtml(diagnostic.code)}</b><span>${escapeHtml(diagnostic.message)}</span></div>`
    ).join("");
  }
  __name(revisionDiagnostics, "revisionDiagnostics");
  function entryTrack(e) {
    return e.trackId || "";
  }
  __name(entryTrack, "entryTrack");
  function entryActor(e) {
    return e.writer?.actorId || "";
  }
  __name(entryActor, "entryActor");
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
      if (s.summary) return s.summary;
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
  var [
    ATTENTION_OPEN_REQUEST,
    ATTENTION_UNASSESSED,
    ATTENTION_VALIDATION_CONTEXT,
    ATTENTION_FOLLOW_UP,
    ATTENTION_STALE_FACT
  ] = REVISION_ATTENTION_VALUES;
  function attentionTokens(overview) {
    const attention = overview?.attention || {};
    const tokens = [];
    if (attention.openInputRequestCount) {
      tokens.push({
        token: ATTENTION_OPEN_REQUEST,
        query: `attention:${ATTENTION_OPEN_REQUEST}`,
        label: plural(attention.openInputRequestCount, "open request")
      });
    }
    if (attention.unassessed) {
      tokens.push({
        token: ATTENTION_UNASSESSED,
        query: `attention:${ATTENTION_UNASSESSED}`,
        label: "unassessed"
      });
    }
    const validationCount = (attention.failedValidationCount || 0) + (attention.erroredValidationCount || 0);
    if (validationCount) {
      tokens.push({
        token: ATTENTION_VALIDATION_CONTEXT,
        query: `attention:${ATTENTION_VALIDATION_CONTEXT}`,
        label: plural(
          validationCount,
          "outstanding validation check",
          "outstanding validation checks"
        )
      });
    }
    if (attention.acceptedWithFollowUp) {
      tokens.push({
        token: ATTENTION_FOLLOW_UP,
        query: `attention:${ATTENTION_FOLLOW_UP}`,
        label: "follow-up"
      });
    }
    if (attention.staleFactCount) {
      tokens.push({
        token: ATTENTION_STALE_FACT,
        query: `attention:${ATTENTION_STALE_FACT}`,
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
  function validationContinuityCue(overview) {
    const recovered = overview?.validationContinuity?.recoveredCount || 0;
    if (!recovered) return "";
    return `<span class="${CLASS.overviewHistoryCue}">${recovered} failed then passed</span>`;
  }
  __name(validationContinuityCue, "validationContinuityCue");
  function overviewStats(overview) {
    const counts = overview?.counts || {};
    const facts = (counts.observations || 0) + (counts.inputRequests || 0) + (counts.assessments || 0) + (counts.validationChecks || 0);
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
  function tokenSet(values) {
    return values.length ? ` ${values.map((v) => v.toLowerCase()).join(" ")} ` : "";
  }
  __name(tokenSet, "tokenSet");
  function tagTokenSet(tags) {
    const tokens = /* @__PURE__ */ new Set();
    for (const tag of tags) {
      if (!tag) continue;
      const lowered = tag.toLowerCase();
      const colon = lowered.indexOf(":");
      if (colon > 0) tokens.add(lowered.slice(0, colon));
      tokens.add(lowered);
    }
    return tokens.size ? ` ${[...tokens].join(" ")} ` : "";
  }
  __name(tagTokenSet, "tagTokenSet");
  function revisionSearchIndex(r, classification = null) {
    const overview = r.overview || {};
    const currentAssessment = overview.currentAssessment || {};
    const attention = overview.attention || {};
    const latest = overview.latestActivity || {};
    const target = r.targetDisplay || {};
    const head = target.head || {};
    const cues = attentionTokens(overview);
    const text = [
      r.revisionId,
      r.snapshotId,
      r.summary,
      target.label,
      target.workLabel?.text,
      head.label,
      currentAssessment.status,
      currentAssessment.assessment,
      latest.kind,
      latest.title,
      ...(r.diagnostics ?? []).flatMap((diagnostic) => [
        diagnostic.code,
        diagnostic.message
      ]),
      ...cues.map((cue) => cue.label),
      "review cues",
      "attention"
    ].filter(Boolean).join(" ").toLowerCase();
    const isTokens = [];
    if ((attention.openInputRequestCount ?? 0) > 0) isTokens.push("open");
    if ((attention.respondedInputRequestCount ?? 0) > 0)
      isTokens.push("answered");
    if (attention.unassessed) isTokens.push("unassessed");
    if ((attention.staleFactCount ?? 0) > 0) isTokens.push("stale");
    if (attention.acceptedWithFollowUp) isTokens.push("follow-up");
    if (classification?.competing) isTokens.push("contested");
    if (classification?.state === "superseded") isTokens.push("superseded");
    return {
      text,
      type: "revision",
      revision: r.revisionId,
      // The search-index key is `snapshot` (grammar renamed from `object`, #334);
      // the value is the revision's snapshot/content-object id.
      snapshot: r.snapshotId,
      // The revision grammar's assessment: field. Resolved-only, mirroring the
      // Rust revision-record builder: the wire value ONLY when the current
      // assessment is resolved; unassessed and ambiguous both emit "" — an
      // ambiguous revision can carry a stale assessment value that must not
      // leak through here.
      assessment: currentAssessment.status === "resolved" ? currentAssessment.assessment || "" : "",
      // The attention token set in the space-wrapped membership encoding.
      attention: cues.length ? ` ${cues.map((cue) => cue.token).join(" ")} ` : "",
      track: tokenSet(overview.tracks ?? []),
      actor: tokenSet(overview.actors ?? []),
      tag: tagTokenSet(overview.tags ?? []),
      is: tokenSet(isTokens),
      // The range anchor: the revision's capturedAt, normalized to the shared
      // fixed-width form under the one canonical occurred_at key.
      [RANGE_ANCHOR_FIELD]: normalizeTimeSlot(r.capturedAt)
    };
  }
  __name(revisionSearchIndex, "revisionSearchIndex");
  function renderRevisionOverview(r, overview = r.overview) {
    return `<div class="${CLASS.overviewSummary}">
    <div class="${CLASS.overviewMain}">${assessmentCue(overview)}${overviewStats(overview)}</div>
    <div class="${CLASS.overviewCues}" aria-label="review cues"><span class="${CLASS.overviewLabel}">review cues</span>${attentionCues(overview)}${validationContinuityCue(overview)}</div>
    ${revisionDiagnostics(r)}
    ${latestActivityLine(overview)}
  </div>`;
  }
  __name(renderRevisionOverview, "renderRevisionOverview");

  // src/store.ts
  var state = {
    history: null,
    revisions: null,
    threads: null,
    attention: null,
    identity: null,
    lens: "timeline",
    selected: { kind: null, id: null },
    open: false,
    attentionFocus: null,
    reading: false,
    followByLens: { timeline: true, list: false, attention: false },
    timelineHeadAnchor: null,
    timelineNewCount: 0,
    attentionDelta: null,
    enabledTypes: new Set(TYPES.map((t) => t.id)),
    // A type is "seen" only after it has actually appeared in the loaded facet
    // distribution. That lets a type which first arrives while follow is enabled
    // become visible without re-enabling a type the reader deliberately hid earlier.
    seenTypes: /* @__PURE__ */ new Set(),
    filterText: "",
    filterTrack: "",
    filterSnapshot: "",
    order: "desc",
    sortKey: "captured",
    diff: null,
    diffHash: null,
    focus: null,
    diffPage: false,
    diffRevision: null,
    diffFile: null,
    diffFileQuery: "",
    lastEventCount: null,
    lastCommitGraphStamp: null
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
    if (!state.selected.id) state.open = false;
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
  function revisionCapturedMs(r) {
    return parseMs(r.capturedAt) ?? Number.NEGATIVE_INFINITY;
  }
  __name(revisionCapturedMs, "revisionCapturedMs");
  function revisionActivityMs(r) {
    return parseMs(r.overview?.latestActivity?.at) ?? Number.NEGATIVE_INFINITY;
  }
  __name(revisionActivityMs, "revisionActivityMs");
  function byOrder(order) {
    return order === "asc" ? (a, b) => a - b : (a, b) => b - a;
  }
  __name(byOrder, "byOrder");
  function revisionSortMs(r, sortKey) {
    return sortKey === "activity" ? revisionActivityMs(r) : revisionCapturedMs(r);
  }
  __name(revisionSortMs, "revisionSortMs");
  function orderedRevisionEntries(entries, order, sortKey) {
    const cmp = byOrder(order);
    return [...entries].sort(
      (a, b) => cmp(revisionSortMs(a, sortKey), revisionSortMs(b, sortKey))
    );
  }
  __name(orderedRevisionEntries, "orderedRevisionEntries");
  function isSupersedableFact(e) {
    return SUPERSEDABLE_FACT_TYPES.has(e.eventType);
  }
  __name(isSupersedableFact, "isSupersedableFact");
  function supersessionStaleBadge(e, opts = {}) {
    if (!isSupersedableFact(e)) return "";
    const successors = supersededByRevision(entryRevisionId(e));
    if (!successors.length) return "";
    return `<span class="${CLASS.badge} ${CLASS.stale}">superseded by ${successors.map((id) => linkify(id, opts)).join(" ")}</span>`;
  }
  __name(supersessionStaleBadge, "supersessionStaleBadge");
  function captureSupersedesBadge(e, opts = {}) {
    if (e.eventType !== "work_object_proposed") return "";
    const predecessors = supersedesRevision(entryRevisionId(e));
    if (!predecessors.length) return "";
    return `<span class="${CLASS.badge} ${CLASS.supersedes}">supersedes ${predecessors.map((id) => linkify(id, opts)).join(" ")}</span>`;
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
  function matchesRevisionFilters(r) {
    const s = getState();
    if (s.filterSnapshot && r.snapshotId !== s.filterSnapshot) return false;
    const revisionId = r.revisionId ?? "";
    const classification = revisionClassification(revisionId);
    const competing = currentThreads().some(
      (t) => t.competing && (t.revisions ?? []).includes(revisionId)
    );
    return matchesQuery(
      revisionSearchIndex(r, { state: classification?.state, competing }),
      parseSearchQueryFor(s.filterText, "revision").clauses
    );
  }
  __name(matchesRevisionFilters, "matchesRevisionFilters");
  function lensEntryIds() {
    const s = getState();
    if (s.lens === "list") {
      return orderedRevisionEntries(
        (s.revisions?.entries ?? []).filter(matchesRevisionFilters),
        s.order,
        s.sortKey
      ).map((r) => ({ kind: "revision", id: r.revisionId ?? "" }));
    }
    return (s.history?.entries ?? []).map(
      (e) => ({ kind: "event", id: e.eventId ?? "" })
    );
  }
  __name(lensEntryIds, "lensEntryIds");
  function attentionEntryKeys(state2) {
    return (state2.attention?.items ?? []).map((item) => item.id);
  }
  __name(attentionEntryKeys, "attentionEntryKeys");
  function selectedEventId() {
    const selected = getState().selected;
    return selected && selected.kind === "event" ? selected.id : null;
  }
  __name(selectedEventId, "selectedEventId");
  function eventForId(id) {
    const history2 = getState().history;
    return history2?.entries.find((entry) => entry.eventId === id) ?? (history2?.retainedEntry?.eventId === id ? history2.retainedEntry : void 0);
  }
  __name(eventForId, "eventForId");
  function revisionExists(id) {
    return (getState().revisions?.entries ?? []).some((r) => r.revisionId === id);
  }
  __name(revisionExists, "revisionExists");
  function revisionInAnyThread(id) {
    return currentThreads().some((t) => (t.revisions ?? []).includes(id));
  }
  __name(revisionInAnyThread, "revisionInAnyThread");
  function eventExists(id) {
    return eventForId(id) != null;
  }
  __name(eventExists, "eventExists");

  // src/connection.ts
  var snapshot = {
    connection: "connecting",
    refresh: "idle"
  };
  function connectionPresentation(state2) {
    const refreshLabel = state2.refresh === "degraded" ? "response error" : state2.refresh;
    switch (state2.connection) {
      case "unauthorized":
        return {
          serverLabel: "local server",
          connectionLabel: "authentication required",
          refreshLabel,
          action: "Reconnect",
          canConnectAnother: false
        };
      case "unreachable":
        return {
          serverLabel: "local server",
          connectionLabel: "server unavailable",
          refreshLabel,
          action: "Retry",
          canConnectAnother: true
        };
      case "connected":
        return {
          serverLabel: "local server",
          connectionLabel: "connected",
          refreshLabel,
          action: state2.refresh === "degraded" ? "Retry" : null,
          canConnectAnother: false
        };
      case "connecting":
        return {
          serverLabel: "local server",
          connectionLabel: "connecting",
          refreshLabel,
          action: null,
          canConnectAnother: false
        };
    }
  }
  __name(connectionPresentation, "connectionPresentation");
  function markRequestSuccess() {
    snapshot = {
      connection: "connected",
      refresh: snapshot.refresh
    };
    renderConnectionChrome();
  }
  __name(markRequestSuccess, "markRequestSuccess");
  function markRequestFailure(kind) {
    snapshot = kind === "protocol" ? { connection: "connected", refresh: "degraded" } : { ...snapshot, connection: kind };
    renderConnectionChrome();
  }
  __name(markRequestFailure, "markRequestFailure");
  function setRefreshState(refresh) {
    snapshot = { ...snapshot, refresh };
    renderConnectionChrome();
  }
  __name(setRefreshState, "setRefreshState");
  var actions = null;
  function configureConnectionActions(next) {
    actions = next;
  }
  __name(configureConnectionActions, "configureConnectionActions");
  function initConnectionControls() {
    document.querySelector("#connection-action")?.addEventListener("click", () => {
      if (!actions) return;
      if (snapshot.connection === "unauthorized") void actions.reconnect();
      else void actions.retry();
    });
    document.querySelector("#connect-another")?.addEventListener("click", () => {
      if (actions) void actions.reconnect();
    });
    renderConnectionChrome();
  }
  __name(initConnectionControls, "initConnectionControls");
  function renderConnectionChrome() {
    const presentation = connectionPresentation(snapshot);
    const root = document.querySelector("#store-identity");
    root?.classList.remove("hidden");
    const connection = document.querySelector("#connection-status");
    if (connection) connection.textContent = presentation.connectionLabel;
    const refresh = document.querySelector("#refresh-status");
    if (refresh) refresh.textContent = presentation.refreshLabel;
    const legacyRefresh = document.querySelector("#stat-live");
    if (legacyRefresh) {
      legacyRefresh.textContent = presentation.refreshLabel;
      legacyRefresh.dataset.state = snapshot.refresh;
    }
    const dot = document.querySelector("#refresh");
    if (dot) {
      dot.dataset.connection = snapshot.connection;
      dot.dataset.state = snapshot.refresh;
      dot.title = `${presentation.connectionLabel}; refresh ${presentation.refreshLabel}`;
    }
    const action = document.querySelector("#connection-action");
    if (action) {
      action.textContent = presentation.action ?? "";
      action.classList.toggle("hidden", presentation.action === null);
    }
    document.querySelector("#connect-another")?.classList.toggle("hidden", !presentation.canConnectAnother);
    const word = document.querySelector("#refresh-word");
    if (word) {
      word.textContent = snapshot.connection === "unauthorized" ? "authentication required" : snapshot.connection === "unreachable" ? "server unavailable" : snapshot.refresh === "degraded" ? "response error" : "";
    }
  }
  __name(renderConnectionChrome, "renderConnectionChrome");

  // src/follow.ts
  function followState(timeline) {
    return { ...getState().followByLens, timeline };
  }
  __name(followState, "followState");
  var intentGeneration = 0;
  function timelineFollowGeneration() {
    return intentGeneration;
  }
  __name(timelineFollowGeneration, "timelineFollowGeneration");
  function headAnchor() {
    const head = getState().history?.entries?.[0];
    const occurredAt = head?.occurredAt;
    const eventId = head?.eventId;
    return occurredAt && eventId ? { occurredAt, eventId } : null;
  }
  __name(headAnchor, "headAnchor");
  function isFollowingTimeline() {
    return getState().followByLens.timeline === true;
  }
  __name(isFollowingTimeline, "isFollowingTimeline");
  function parkTimelineRead() {
    const state2 = getState();
    if (state2.timelineHeadAnchor) return;
    const anchor = headAnchor();
    if (!anchor) return;
    intentGeneration += 1;
    commit({ timelineHeadAnchor: anchor, timelineNewCount: 0 });
  }
  __name(parkTimelineRead, "parkTimelineRead");
  async function catchUpTimeline() {
    const state2 = getState();
    if (!state2.followByLens.timeline || state2.order !== "desc" || !state2.timelineHeadAnchor)
      return;
    const generation = timelineFollowGeneration();
    const queryKey = historyQueryParams(state2);
    if (!await loadHistoryHead(() => {
      const current = getState();
      return timelineFollowGeneration() === generation && current.followByLens.timeline && current.order === "desc" && current.timelineHeadAnchor != null && historyQueryParams(current) === queryKey;
    }))
      return;
    intentGeneration += 1;
    commit({ timelineHeadAnchor: null, timelineNewCount: 0 });
    const timeline = $("#timeline");
    if (timeline) timeline.scrollTop = 0;
  }
  __name(catchUpTimeline, "catchUpTimeline");
  async function toggleTimelineFollow() {
    intentGeneration += 1;
    if (isFollowingTimeline()) {
      commit({
        followByLens: followState(false),
        timelineNewCount: 0
      });
      return;
    }
    commit({ followByLens: followState(true) });
    const state2 = getState();
    if (state2.order !== "desc") return;
    if (state2.timelineHeadAnchor) {
      await probeNewCount();
      return;
    }
    const generation = timelineFollowGeneration();
    const queryKey = historyQueryParams(state2);
    await loadHistoryHead(() => {
      const current = getState();
      return timelineFollowGeneration() === generation && current.followByLens.timeline && current.order === "desc" && current.timelineHeadAnchor == null && historyQueryParams(current) === queryKey;
    });
  }
  __name(toggleTimelineFollow, "toggleTimelineFollow");
  function resetTimelineReadForQueryChange() {
    intentGeneration += 1;
    const state2 = getState();
    const engaged = state2.selected.kind === "event" && Boolean(state2.selected.id);
    const atDescendingHead = state2.order === "desc" && !engaged;
    commit({
      timelineHeadAnchor: atDescendingHead ? null : headAnchor(),
      timelineNewCount: 0
    });
  }
  __name(resetTimelineReadForQueryChange, "resetTimelineReadForQueryChange");

  // src/http.ts
  var RequestFailure = class extends Error {
    constructor(kind, status) {
      super(
        kind === "unauthorized" ? "authentication required" : kind === "unreachable" ? "server unavailable" : "server response error"
      );
      this.kind = kind;
      this.status = status;
      this.name = "RequestFailure";
    }
    kind;
    status;
    static {
      __name(this, "RequestFailure");
    }
  };
  function failure(kind, status) {
    markRequestFailure(kind);
    return new RequestFailure(kind, status);
  }
  __name(failure, "failure");
  function expectedDocument(path) {
    const pathname = new URL(path, location.origin).pathname;
    const collections = {
      "/api/attention": { schema: "pointbreak.inspect-attention" },
      "/api/freshness": {
        schema: "pointbreak.inspect-freshness",
        version: 1
      },
      "/api/history": { schema: "pointbreak.inspect-history" },
      "/api/history/new-count": {
        schema: "pointbreak.inspect-history-new-count"
      },
      "/api/identity": { schema: "pointbreak.inspect-identity" },
      "/api/revisions": { schema: "pointbreak.inspect-revisions" },
      "/api/threads": { schema: "pointbreak.inspect-threads" },
      "/api/version": { schema: "pointbreak.version", version: 1 }
    };
    if (collections[pathname]) return collections[pathname];
    if (/^\/api\/revisions\/[^/]+$/.test(pathname)) {
      return { schema: "pointbreak.review-revision", version: 2 };
    }
    if (/^\/api\/snapshots\/[^/]+$/.test(pathname)) {
      return { schema: "pointbreak.review-snapshot", version: 1 };
    }
    return null;
  }
  __name(expectedDocument, "expectedDocument");
  function isExpectedDocument(data, expected) {
    if (typeof data !== "object" || data === null) return false;
    const document2 = data;
    return document2.schema === expected.schema && (expected.version === void 0 || document2.version === expected.version);
  }
  __name(isExpectedDocument, "isExpectedDocument");
  function hasPayloadError(data) {
    return typeof data === "object" && data !== null && "error" in data && Boolean(data.error);
  }
  __name(hasPayloadError, "hasPayloadError");
  async function fetchOnce(path) {
    const headers = {};
    const token = getSessionToken();
    if (token) headers.Authorization = `Bearer ${token}`;
    let response;
    try {
      response = await fetch(path, {
        cache: "no-store",
        credentials: "omit",
        referrerPolicy: "no-referrer",
        headers
      });
    } catch {
      throw failure("unreachable");
    }
    if (response.status === 401) throw new RequestFailure("unauthorized", 401);
    let text;
    try {
      text = await response.text();
    } catch {
      throw failure("protocol", response.status);
    }
    if (!response.ok) throw failure("protocol", response.status);
    let data;
    try {
      data = JSON.parse(text);
    } catch {
      throw failure("protocol", response.status);
    }
    const expected = expectedDocument(path);
    if (hasPayloadError(data) || expected !== null && !isExpectedDocument(data, expected)) {
      throw failure("protocol", response.status);
    }
    markRequestSuccess();
    return data;
  }
  __name(fetchOnce, "fetchOnce");
  async function fetchJSON(path) {
    const requestCredentialVersion = sessionCredentialVersion();
    try {
      return await fetchOnce(path);
    } catch (error) {
      if (!(error instanceof RequestFailure) || error.kind !== "unauthorized") {
        throw error;
      }
    }
    const credentialAlreadyRenewed = sessionCredentialVersion() !== requestCredentialVersion;
    if (credentialAlreadyRenewed || await recoverUnauthorized()) {
      try {
        return await fetchOnce(path);
      } catch (error) {
        if (error instanceof RequestFailure && error.kind === "unauthorized") {
          throw failure("unauthorized", 401);
        }
        throw error;
      }
    }
    throw failure("unauthorized", 401);
  }
  __name(fetchJSON, "fetchJSON");

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
  async function probeNewCount() {
    const s = getState();
    const generation = timelineFollowGeneration();
    const anchor = s.timelineHeadAnchor;
    const queryKey = historyQueryParams(s);
    if (!s.followByLens.timeline || s.order !== "desc" || !anchor) return true;
    const params = new URLSearchParams(queryKey);
    params.delete("limit");
    params.delete("offset");
    params.delete("at");
    params.set("sinceOccurredAt", anchor.occurredAt);
    params.set("sinceEventId", anchor.eventId);
    let doc;
    try {
      doc = await fetchJSON(
        `/api/history/new-count?${params.toString()}`
      );
    } catch (err) {
      showLoadError(err);
      return false;
    }
    const current = getState();
    const currentAnchor = current.timelineHeadAnchor;
    if (timelineFollowGeneration() !== generation || !current.followByLens.timeline || current.order !== "desc" || historyQueryParams(current) !== queryKey || currentAnchor?.occurredAt !== anchor.occurredAt || currentAnchor?.eventId !== anchor.eventId)
      return true;
    commit({ timelineNewCount: doc.newCount ?? 0 });
    return true;
  }
  __name(probeNewCount, "probeNewCount");
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
  function showLoadError(err) {
    showError(err instanceof Error ? err.message : String(err));
  }
  __name(showLoadError, "showLoadError");
  function commitFreshnessBaseline(freshness) {
    commit({
      lastEventCount: freshness.eventCount ?? null,
      lastCommitGraphStamp: freshness.commitGraphStamp ?? null
    });
  }
  __name(commitFreshnessBaseline, "commitFreshnessBaseline");
  async function loadHistoryHead(isCurrent = () => true) {
    try {
      const params = historyQueryParams(getState());
      const freshness = await fetchJSON("/api/freshness");
      const historyRaw = await fetchJSON(`/api/history?${params}`);
      if (!isCurrent()) return false;
      showError(null);
      commitHistoryHead(historyRaw, params);
      commitFreshnessBaseline(freshness);
      return true;
    } catch (err) {
      showLoadError(err);
      return false;
    }
  }
  __name(loadHistoryHead, "loadHistoryHead");
  async function loadWholeDocuments() {
    try {
      const previousAttentionCount = getState().attention?.items?.length;
      const revisionsScrollTop = $("#units")?.scrollTop;
      const [revisionsRaw, threadsRaw, attentionRaw] = await Promise.all([
        fetchJSON("/api/revisions"),
        fetchJSON("/api/threads"),
        fetchJSON("/api/attention")
      ]);
      const attention = attentionRaw;
      showError(null);
      commit({
        revisions: revisionsRaw,
        threads: threadsRaw,
        attention,
        attentionDelta: previousAttentionCount == null ? null : attention.items.length - previousAttentionCount
      });
      if (revisionsScrollTop != null) {
        const units = $("#units");
        if (units) units.scrollTop = revisionsScrollTop;
      }
      return true;
    } catch (err) {
      showLoadError(err);
      return false;
    }
  }
  __name(loadWholeDocuments, "loadWholeDocuments");
  async function load() {
    if (!await loadHistoryHead()) return false;
    return loadWholeDocuments();
  }
  __name(load, "load");
  async function loadIdentity() {
    try {
      const doc = await fetchJSON("/api/identity");
      commit({ identity: doc });
    } catch {
    }
  }
  __name(loadIdentity, "loadIdentity");
  var reloading = false;
  async function reloadHistoryForQuery() {
    const queryKey = historyQueryParams(getState());
    const doc = await fetchHistoryDoc(`/api/history?${queryKey}`);
    if (!doc) return false;
    showError(null);
    commitHistoryPage(doc, queryKey);
    resetTimelineReadForQueryChange();
    return true;
  }
  __name(reloadHistoryForQuery, "reloadHistoryForQuery");
  function maybeReloadForQuery() {
    const s = getState();
    const want = historyQueryParams(s);
    if (reloading || !s.history || s.history.queryKey === want) return;
    reloading = true;
    void reloadHistoryForQuery().then((reloaded) => {
      reloading = false;
      if (reloaded) maybeReloadForQuery();
    }).catch(() => {
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
  function commitHistoryPage(page, requestedQueryKey) {
    const s = getState();
    const queryKey = requestedQueryKey ?? historyQueryParams(s);
    const prev = s.history;
    const merged = prev && prev.queryKey === queryKey ? mergeWindows(prev, page) : { entries: page.entries ?? [], offset: page.offset ?? 0 };
    const selected = s.selected.kind === "event" && s.selected.id ? s.selected.id : null;
    const selectedIsVisible = selected != null && merged.entries.some((entry) => entry.eventId === selected);
    const retainedEntry = selected != null && !selectedIsVisible ? prev?.entries.find((entry) => entry.eventId === selected) ?? (prev?.retainedEntry?.eventId === selected ? prev.retainedEntry : void 0) : void 0;
    commit({
      history: {
        ...page,
        entries: merged.entries,
        offset: merged.offset,
        queryKey,
        retainedEntry
      }
    });
  }
  __name(commitHistoryPage, "commitHistoryPage");
  function commitHistoryHead(page, queryKey) {
    const state2 = getState();
    const selected = state2.selected.kind === "event" && state2.selected.id ? state2.selected.id : null;
    const selectedIsVisible = selected != null && (page.entries ?? []).some((entry) => entry.eventId === selected);
    const retainedEntry = selected != null && !selectedIsVisible ? state2.history?.entries.find((entry) => entry.eventId === selected) ?? (state2.history?.retainedEntry?.eventId === selected ? state2.history.retainedEntry : void 0) : void 0;
    commit({ history: { ...page, queryKey, retainedEntry } });
  }
  __name(commitHistoryHead, "commitHistoryHead");
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
  var pollSettleTimer;
  function settlePoll(markWatching) {
    clearTimeout(pollSettleTimer);
    pollSettleTimer = setTimeout(() => {
      commit({ attentionDelta: null });
      if (markWatching) setRefreshState("watching");
    }, 1200);
  }
  __name(settlePoll, "settlePoll");
  async function pollFreshness() {
    let documentsLoaded = false;
    try {
      const f = await fetchJSON("/api/freshness");
      const s = getState();
      const stampChanged = f.commitGraphStamp != null && (s.lastCommitGraphStamp == null || f.commitGraphStamp !== s.lastCommitGraphStamp);
      const changed = (f.eventCount ?? null) !== s.lastEventCount || stampChanged;
      if (changed) {
        clearTimeout(pollSettleTimer);
        setRefreshState("updated");
        documentsLoaded = await loadWholeDocuments();
        if (!documentsLoaded) {
          setRefreshState("degraded");
          commit({ attentionDelta: null });
          return;
        }
        let historyLoaded = true;
        const timeline = getState();
        if (timeline.followByLens.timeline && timeline.order === "desc" && timeline.timelineHeadAnchor == null) {
          const generation = timelineFollowGeneration();
          const queryKey = historyQueryParams(getState());
          const isCurrent = /* @__PURE__ */ __name(() => {
            const current = getState();
            return timelineFollowGeneration() === generation && current.followByLens.timeline && current.order === "desc" && current.timelineHeadAnchor == null && historyQueryParams(current) === queryKey;
          }, "isCurrent");
          historyLoaded = await loadHistoryHead(isCurrent);
          if (!historyLoaded && !isCurrent()) {
            historyLoaded = true;
            historyLoaded = await probeNewCount();
          }
        } else if (timeline.followByLens.timeline && timeline.order === "desc" && timeline.timelineHeadAnchor != null) {
          historyLoaded = await probeNewCount();
        }
        if (!historyLoaded) {
          setRefreshState("degraded");
          settlePoll(false);
          return;
        }
        commitFreshnessBaseline(f);
        settlePoll(true);
      } else {
        setRefreshState("watching");
      }
    } catch {
      setRefreshState("degraded");
      if (documentsLoaded) settlePoll(false);
      else {
        clearTimeout(pollSettleTimer);
        commit({ attentionDelta: null });
      }
    }
  }
  __name(pollFreshness, "pollFreshness");

  // src/router.ts
  var LENSES2 = ["timeline", "list", "attention"];
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
      filterTrack: p.track != null ? p.track : "",
      // The filter param is `snapshot`; legacy `object` is still parsed for old
      // bookmarks during the transition (#334).
      filterSnapshot: p.snapshot != null ? p.snapshot : p.object != null ? p.object : "",
      order: p.order === "asc" || p.order === "desc" ? p.order : "desc",
      sortKey: p.sort === "activity" ? "activity" : "captured",
      filterText: p.q != null ? p.q : "",
      enabledTypes: p.types != null ? new Set(p.types.split(",").filter(Boolean)) : new Set(presentTypes2),
      diff: p.diff || null,
      diffHash: p.diffHash || null,
      focus: p.focus ? p.focus : null,
      diffPage: false,
      diffRevision: null,
      diffFile: p.file ? p.file : null,
      unsupportedAsOf: p.asof != null ? p.asof || true : null,
      unsupportedJournal: p.journal != null ? p.journal || true : null,
      unknownPath: null,
      migrated: null
    };
    const segs = path.split("/").filter(Boolean);
    const lensParam = p.lens ?? "";
    if (segs[0] === "revision" && segs[1] && segs[2] === "diff") {
      patch.diffPage = true;
      patch.diffRevision = decodeURIComponent(segs[1]);
      if (p.fq != null) {
        patch.diffFileQuery = p.fq;
      } else {
        switch (p.nav) {
          case "with-facts":
            patch.diffFileQuery = "has:facts";
            patch.migrated = "legacy-diff-nav";
            break;
          case "unanchored":
            patch.diffFileQuery = "is:unanchored";
            patch.migrated = "legacy-diff-nav";
            break;
          case "all":
            patch.diffFileQuery = "";
            patch.migrated = "legacy-diff-nav";
            break;
          default:
            patch.diffFileQuery = "";
        }
      }
      return patch;
    }
    patch.selected = { kind: null, id: null };
    patch.open = false;
    if (segs.length === 0) {
      patch.lens = DEFAULT_LENS2;
    } else if (segs[0] === "revision" && segs[1]) {
      patch.selected = { kind: "revision", id: decodeURIComponent(segs[1]) };
      patch.open = true;
      patch.lens = LENSES2.includes(lensParam) ? lensParam : DEFAULT_LENS2;
    } else if (segs[0] === "event" && segs[1]) {
      patch.selected = { kind: "event", id: decodeURIComponent(segs[1]) };
      patch.open = true;
      patch.lens = LENSES2.includes(lensParam) ? lensParam : DEFAULT_LENS2;
    } else if (LENSES2.includes(segs[0]) || segs[0] === "threads") {
      patch.lens = segs[0] === "threads" ? "list" : segs[0];
      if (segs[0] === "threads") patch.migrated = "threads-alias";
      if (p.sel) patch.selected = { kind: selectionKind(p.sel), id: p.sel };
    } else {
      patch.lens = DEFAULT_LENS2;
      patch.unknownPath = path;
    }
    return patch;
  }
  __name(parseHash, "parseHash");
  function serializeState(snapshot2, presentTypes2) {
    if (snapshot2.diffPage && snapshot2.diffRevision) {
      const pageParams = [];
      if (snapshot2.focus)
        pageParams.push(`focus=${encodeURIComponent(snapshot2.focus)}`);
      if (snapshot2.diffFile)
        pageParams.push(`file=${encodeURIComponent(snapshot2.diffFile)}`);
      if (snapshot2.diffFileQuery)
        pageParams.push(`fq=${encodeURIComponent(snapshot2.diffFileQuery)}`);
      const pagePath = `#/revision/${encodeURIComponent(snapshot2.diffRevision)}/diff`;
      return pageParams.length ? `${pagePath}?${pageParams.join("&")}` : pagePath;
    }
    const params = [];
    const sel = snapshot2.selected ?? { kind: null, id: null };
    let path = snapshot2.lens === DEFAULT_LENS2 ? "#/timeline" : `#/${snapshot2.lens}`;
    if (sel.id && snapshot2.open && (sel.kind === "revision" || sel.kind === "event")) {
      path = sel.kind === "revision" ? `#/revision/${encodeURIComponent(sel.id)}` : `#/event/${encodeURIComponent(sel.id)}`;
      if (snapshot2.lens && snapshot2.lens !== DEFAULT_LENS2)
        params.push(`lens=${encodeURIComponent(snapshot2.lens)}`);
    } else if (sel.id) {
      params.push(`sel=${encodeURIComponent(sel.id)}`);
    }
    if (snapshot2.filterTrack)
      params.push(`track=${encodeURIComponent(snapshot2.filterTrack)}`);
    if (snapshot2.filterSnapshot)
      params.push(`snapshot=${encodeURIComponent(snapshot2.filterSnapshot)}`);
    if (snapshot2.order && snapshot2.order !== "desc")
      params.push(`order=${encodeURIComponent(snapshot2.order)}`);
    if (snapshot2.lens === "list" && snapshot2.sortKey !== "captured")
      params.push(`sort=${encodeURIComponent(snapshot2.sortKey)}`);
    if (presentTypes2.some((id) => !snapshot2.enabledTypes.has(id))) {
      params.push(
        `types=${encodeURIComponent(
          presentTypes2.filter((id) => snapshot2.enabledTypes.has(id)).join(",")
        )}`
      );
    }
    if (snapshot2.filterText)
      params.push(`q=${encodeURIComponent(snapshot2.filterText)}`);
    if (snapshot2.diff) params.push(`diff=${encodeURIComponent(snapshot2.diff)}`);
    if (snapshot2.diff && snapshot2.diffHash)
      params.push(`diffHash=${encodeURIComponent(snapshot2.diffHash)}`);
    if (snapshot2.focus)
      params.push(`focus=${encodeURIComponent(snapshot2.focus)}`);
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
    const parsed = parseHash(location.hash, presentTypes());
    const patch = resolve(parsed);
    if (patch.selected?.kind === "event" && patch.selected.id) parkTimelineRead();
    commit(patch);
    if (parsed.migrated === "threads-alias") {
      history.replaceState(
        {},
        "",
        location.hash.replace(/^#\/threads/, "#/list")
      );
    } else if (parsed.migrated === "legacy-diff" || parsed.migrated === "legacy-diff-nav") {
      history.replaceState({}, "", serializeState(getState(), presentTypes()));
    }
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
    if (patch.diff && !patch.diffPage) {
      next.diffPage = true;
      const mapped = revisionIdForSnapshot(patch.diff, patch.diffHash);
      if (mapped) {
        next.diffRevision = mapped;
        patch.migrated = "legacy-diff";
      }
    }
    const sel = patch.selected ?? { kind: null, id: null };
    if (sel.kind === "revision" && sel.id && !revisionExists(sel.id)) {
      if (revisionInAnyThread(sel.id)) {
        next.open = true;
      } else {
        const lens = patch.lens || DEFAULT_LENS2;
        showRouteDiagnostic(
          routeDiagnostic(
            `fell back to the ${lens} lens — revision ${shortRef(sel.id)} is not in this store`,
            freshnessDiagnostic
          )
        );
        next.lens = lens;
        next.selected = { kind: null, id: null };
        return next;
      }
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
    const next = {
      lens: patch.lens,
      filterTrack: patch.filterTrack,
      filterSnapshot: patch.filterSnapshot,
      order: patch.order,
      // sortKey, like order/filterText, is set in parseHash's base patch object
      // before any path-arm branches (including the diff-page early return), so it
      // is always present on a full parse — unlike selected/open, which the
      // diff-page branch deliberately omits. Unconditional copy is therefore correct.
      sortKey: patch.sortKey,
      filterText: patch.filterText,
      enabledTypes: patch.enabledTypes,
      diff: patch.diff,
      diffHash: patch.diffHash,
      focus: patch.focus,
      diffPage: patch.diffPage,
      diffRevision: patch.diffRevision,
      diffFile: patch.diffFile
    };
    if (patch.selected !== void 0) next.selected = patch.selected;
    if (patch.open !== void 0) next.open = patch.open;
    if (patch.diffFileQuery !== void 0)
      next.diffFileQuery = patch.diffFileQuery;
    return next;
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

  // src/autocomplete.ts
  var CHECK_VALUES = ["passed", "failed", "errored", "skipped"];
  var EVENT_IS_VALUES = ["open", "answered"];
  var REVISION_IS_VALUES = [
    "open",
    "answered",
    "unassessed",
    "stale",
    "follow-up",
    "contested",
    "superseded"
  ];
  function keysFor(surface) {
    return surface === "revision" ? REVISION_QUERY_FIELDS : EVENT_QUERY_FIELDS;
  }
  __name(keysFor, "keysFor");
  function valuesForKey(field, surface, distinct, presentTypeIds) {
    switch (field) {
      case "type": {
        if (!presentTypeIds) return TYPES.map((t) => t.label);
        const labels = TYPES.filter((t) => presentTypeIds.has(t.id)).map(
          (t) => t.label
        );
        for (const id of presentTypeIds) {
          if (!TYPES.some((t) => t.id === id)) labels.push(id);
        }
        return labels;
      }
      case "track":
        return distinct.track;
      case "actor":
        return distinct.actor;
      case "tag":
        return distinct.tag;
      case "check":
        return CHECK_VALUES;
      case "assessment":
        return Object.keys(ASSESSMENT_LABELS);
      case "is":
        return surface === "revision" ? REVISION_IS_VALUES : EVENT_IS_VALUES;
      case "attention":
        return REVISION_ATTENTION_VALUES;
      default:
        return [];
    }
  }
  __name(valuesForKey, "valuesForKey");
  function suggestionsFor(filterText, surface, distinct, presentTypeIds) {
    const tokens = filterText.split(/\s+/);
    const active2 = tokens[tokens.length - 1] ?? "";
    if (!active2) return [];
    const colon = active2.indexOf(":");
    if (colon < 0) {
      const prefix = active2.toLowerCase();
      return keysFor(surface).filter((k) => k.startsWith(prefix)).map((k) => ({ insertText: `${k}:`, label: `${k}:` }));
    }
    const field = active2.slice(0, colon).toLowerCase();
    const valuePrefix = active2.slice(colon + 1).toLowerCase().replace(/^"/, "");
    if (!keysFor(surface).includes(field)) return [];
    return valuesForKey(field, surface, distinct, presentTypeIds).flatMap(
      (full) => {
        const value = field === "actor" ? full.replace(/^actor:/, "") : full;
        const matches = value.toLowerCase().includes(valuePrefix) || field === "actor" && full.toLowerCase().includes(valuePrefix);
        if (!matches) return [];
        const clause = /\s/.test(value) ? `${field}:"${value}"` : `${field}:${value}`;
        return [{ insertText: clause, label: clause }];
      }
    );
  }
  __name(suggestionsFor, "suggestionsFor");
  function acceptSuggestion(filterText, insertText) {
    const tokens = filterText.split(/\s+/);
    tokens[tokens.length - 1] = insertText;
    return `${tokens.join(" ")} `;
  }
  __name(acceptSuggestion, "acceptSuggestion");
  function currentSurface() {
    return getState().lens === "list" ? "revision" : "event";
  }
  __name(currentSurface, "currentSurface");
  function distinctValuesFromRevisions() {
    const track = /* @__PURE__ */ new Set();
    const actor = /* @__PURE__ */ new Set();
    const tag = /* @__PURE__ */ new Set();
    for (const r of getState().revisions?.entries ?? []) {
      const overview = r.overview ?? {};
      for (const id of overview.tracks ?? []) if (id) track.add(id.toLowerCase());
      for (const id of overview.actors ?? []) if (id) actor.add(id.toLowerCase());
      for (const full of overview.tags ?? []) {
        const key = full.split(":")[0] ?? full;
        if (key) tag.add(key.toLowerCase());
      }
    }
    return { track: [...track], actor: [...actor], tag: [...tag] };
  }
  __name(distinctValuesFromRevisions, "distinctValuesFromRevisions");
  function activeDistinctValues() {
    if (currentSurface() === "event") {
      return getState().history?.distinctValues ?? { track: [], actor: [], tag: [] };
    }
    return distinctValuesFromRevisions();
  }
  __name(activeDistinctValues, "activeDistinctValues");
  var seenPresentTypeIds = /* @__PURE__ */ new Set();
  function presentTypeVocabulary() {
    for (const id of presentTypes()) seenPresentTypeIds.add(id);
    return seenPresentTypeIds;
  }
  __name(presentTypeVocabulary, "presentTypeVocabulary");
  function currentSuggestions(input) {
    return suggestionsFor(
      input.value,
      currentSurface(),
      activeDistinctValues(),
      presentTypeVocabulary()
    );
  }
  __name(currentSuggestions, "currentSuggestions");
  var activeIndex = -1;
  function suggestionListEl() {
    return $("#filter-suggestions");
  }
  __name(suggestionListEl, "suggestionListEl");
  function dismiss() {
    const list = suggestionListEl();
    if (list) {
      list.classList.add("hidden");
      list.innerHTML = "";
    }
    activeIndex = -1;
  }
  __name(dismiss, "dismiss");
  function paint(input) {
    const list = suggestionListEl();
    const suggestions = currentSuggestions(input);
    if (!list) return suggestions;
    activeIndex = -1;
    if (!suggestions.length) {
      dismiss();
      return suggestions;
    }
    list.classList.remove("hidden");
    list.innerHTML = suggestions.map(
      (s, i) => `<li class="${suggestionClass(false)}" data-index="${i}">${escapeHtml(s.label)}</li>`
    ).join("");
    return suggestions;
  }
  __name(paint, "paint");
  function updateActive(items) {
    items.forEach((el, i) => {
      el.classList.toggle(CLASS.suggestionActive, i === activeIndex);
    });
  }
  __name(updateActive, "updateActive");
  function accept(input, suggestion) {
    const next = acceptSuggestion(input.value, suggestion.insertText);
    input.value = next;
    navigate({ filterText: next }, { replace: true });
    dismiss();
    input.focus();
  }
  __name(accept, "accept");
  function onDocumentClickForSuggestions(ev) {
    const list = suggestionListEl();
    if (!list || list.classList.contains("hidden")) return;
    if (ev.target instanceof Node && (list.contains(ev.target) || $("#filter-text")?.contains(ev.target))) {
      return;
    }
    dismiss();
  }
  __name(onDocumentClickForSuggestions, "onDocumentClickForSuggestions");
  function onSuggestionListClick(input, ev) {
    const t = ev.target;
    if (!(t instanceof Element)) return;
    const row = t.closest("[data-index]");
    const indexAttr = row?.dataset.index;
    if (indexAttr == null) return;
    const chosen = currentSuggestions(input)[Number(indexAttr)];
    if (chosen) accept(input, chosen);
  }
  __name(onSuggestionListClick, "onSuggestionListClick");
  function initControls() {
    const input = $("#filter-text");
    if (!input) return;
    input.addEventListener("input", () => paint(input));
    input.addEventListener("keydown", (ev) => {
      const list2 = suggestionListEl();
      if (!list2 || list2.classList.contains("hidden")) {
        return;
      }
      const items = list2.querySelectorAll(`.${CLASS.suggestion}`);
      if (ev.key === "ArrowDown") {
        ev.preventDefault();
        activeIndex = Math.min(items.length - 1, activeIndex + 1);
        updateActive(items);
      } else if (ev.key === "ArrowUp") {
        ev.preventDefault();
        activeIndex = Math.max(0, activeIndex - 1);
        updateActive(items);
      } else if (ev.key === "Enter" || ev.key === "Tab") {
        if (activeIndex < 0) return;
        ev.preventDefault();
        ev.stopPropagation();
        const chosen = currentSuggestions(input)[activeIndex];
        if (chosen) accept(input, chosen);
      } else if (ev.key === "Escape") {
        ev.preventDefault();
        ev.stopPropagation();
        dismiss();
      }
    });
    input.addEventListener("blur", (ev) => {
      const list2 = suggestionListEl();
      if (list2 && ev.relatedTarget instanceof Node && list2.contains(ev.relatedTarget)) {
        return;
      }
      dismiss();
    });
    const list = suggestionListEl();
    list?.addEventListener("mousedown", (ev) => {
      ev.preventDefault();
    });
    list?.addEventListener("click", (ev) => onSuggestionListClick(input, ev));
    document.addEventListener("click", onDocumentClickForSuggestions, true);
  }
  __name(initControls, "initControls");

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
    html = protectBackslashEscapes(html, stash, (character) => character === "`");
    html = html.replace(
      /`([^`]+)`/g,
      (_, code) => stash(`<code>${code}</code>`)
    );
    html = protectBackslashEscapes(html, stash);
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
    for (const [token, replacement] of placeholders.reverse()) {
      html = html.split(token).join(replacement);
    }
    return html;
  }
  __name(renderMarkdownInline, "renderMarkdownInline");
  function protectBackslashEscapes(html, stash, shouldProtect = isAsciiPunctuation) {
    let protectedHtml = "";
    for (let index = 0; index < html.length; index++) {
      const character = html[index];
      const escaped = html[index + 1];
      if (character === "\\" && escaped && shouldProtect(escaped)) {
        protectedHtml += stash(escaped);
        index++;
      } else {
        protectedHtml += character;
      }
    }
    return protectedHtml;
  }
  __name(protectBackslashEscapes, "protectBackslashEscapes");
  function isAsciiPunctuation(character) {
    const codePoint = character.codePointAt(0) ?? 0;
    return codePoint >= 33 && codePoint <= 47 || codePoint >= 58 && codePoint <= 64 || codePoint >= 91 && codePoint <= 96 || codePoint >= 123 && codePoint <= 126;
  }
  __name(isAsciiPunctuation, "isAsciiPunctuation");

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
  var VALIDATION_DISPOSITION_LABELS = {
    outstanding: "outstanding",
    current: "current result",
    resolved_by_later_pass: "resolved by strictly later pass",
    historical: "historical",
    skipped: "skipped"
  };
  function renderActorAttribution(label, writer) {
    const actorId = writer?.actorId ?? "";
    if (!actorId) return "";
    return `<span class="${CLASS.actorAttribution}">${label} ${actorChip(actorId)}</span>`;
  }
  __name(renderActorAttribution, "renderActorAttribution");
  function renderRecordedTime(createdAt) {
    if (!createdAt) return "";
    return `<span class="${CLASS.annoTime}" title="${escapeHtml(createdAt)}">${escapeHtml(fmtDateTime(createdAt))}</span>`;
  }
  __name(renderRecordedTime, "renderRecordedTime");
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
      const cue = removedBodyCue(a?.summaryContentState);
      if (cue) return cue;
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
    const annotationId = opts.annotationId ? ` data-anno="${escapeHtml(opts.annotationId)}"` : "";
    return `<div class="${annoContainerClass(kind)}"${annotationId}>
    <div class="${CLASS.annoHead}">
      <span class="${annoKindClass(kind)}">${kind}</span>
      <span class="${CLASS.annoTrack}">${escapeHtml(opts.track || "")}</span>
      ${renderActorAttribution("writer", opts.writer)}
      <span class="${CLASS.annoTitle}">${linkify(opts.title || "")}</span>
      ${opts.status ? `<span class="${factStatusClass(escapeHtml(opts.status))}">${escapeHtml(opts.status)}</span>` : ""}
      ${opts.target ? `<span class="${CLASS.annoLoc}">${opts.target}</span>` : ""}
      ${tags}
      ${opts.verify || ""}
      ${renderRecordedTime(opts.createdAt)}
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
      annotationId: o.id,
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
      writer: o.writer,
      extra
    });
  }
  __name(renderObservationCard, "renderObservationCard");
  function renderInputRequestResponse(r) {
    const reason = removedBodyCue(r.reasonContentState) ?? (r.reason ? renderBodyContent(r.reason, r.reasonContentType) : "");
    return `<div class="${CLASS.factResponse}">
    <div class="${CLASS.annoHead}">
      <span class="${CLASS.outcome}">${escapeHtml(r.outcome)}</span>
      ${r.id ? `<span class="${CLASS.annoLoc}">${linkify(r.id)}</span>` : ""}
      ${renderActorAttribution("answered by", r.writer)}
      ${verificationChip(r.verificationStatus ?? "")}
      ${renderRecordedTime(r.createdAt)}
    </div>
    ${reason}
    ${endorsementsBlock(r.endorsements)}
  </div>`;
  }
  __name(renderInputRequestResponse, "renderInputRequestResponse");
  function renderInputRequestCard(ir) {
    const responses = (ir.responses ?? []).map(renderInputRequestResponse).join("");
    return factCard("input-request", {
      annotationId: ir.id,
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
      writer: ir.writer,
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
      annotationId: a.id,
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
      writer: a.writer,
      extra: rel.length ? `<div class="${CLASS.factRel}">${rel.join(" · ")}</div>` : ""
    });
  }
  __name(renderAssessmentCard, "renderAssessmentCard");
  function renderValidationCheckCard(v, disposition) {
    const rel = [];
    const logs = v.logArtifactContentHashes ?? [];
    if (v.command) rel.push(escapeHtml(v.command));
    if (logs.length) rel.push(`logs ${logs.map(linkify).join(", ")}`);
    const continuity = disposition ? `<div class="${CLASS.validationContinuity} ${disposition === "outstanding" ? CLASS.validationContinuityOutstanding : CLASS.validationContinuityNeutral}" title="server-projected validation continuity">${escapeHtml(VALIDATION_DISPOSITION_LABELS[disposition])}</div>` : "";
    const related = rel.length ? `<div class="${CLASS.factRel}">${rel.join(" · ")}</div>` : "";
    return factCard("validation", {
      annotationId: v.id,
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
      writer: v.writer,
      extra: continuity + related
    });
  }
  __name(renderValidationCheckCard, "renderValidationCheckCard");
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
    const target = showLocation && a.target?.filePath ? a.target : void 0;
    switch (a.kind) {
      case "observation":
        return renderObservationCard({
          id: a.id,
          trackId: a.track,
          title: a.title,
          status: a.status,
          target,
          tags: a.tags,
          body: a.body,
          bodyContentType: a.bodyContentType,
          bodyContentState: a.bodyContentState,
          createdAt: a.createdAt,
          verificationStatus: a.verificationStatus,
          endorsements: a.endorsements,
          supersedes: a.supersedes,
          writer: a.writer
        });
      case "input-request":
        return renderInputRequestCard({
          id: a.id,
          trackId: a.track,
          title: a.title,
          status: a.status,
          target,
          mode: a.mode,
          reasonCode: a.reasonCode,
          body: a.body,
          bodyContentType: a.bodyContentType,
          bodyContentState: a.bodyContentState,
          createdAt: a.createdAt,
          verificationStatus: a.verificationStatus,
          endorsements: a.endorsements,
          responses: a.responses,
          writer: a.writer
        });
      case "assessment":
        return renderAssessmentCard({
          id: a.id,
          trackId: a.track,
          assessment: a.assessment ?? a.title.replace(/^assessment:\s*/, "").trim(),
          status: a.status,
          target,
          summary: a.body,
          summaryContentType: a.bodyContentType,
          summaryContentState: a.bodyContentState,
          createdAt: a.createdAt,
          verificationStatus: a.verificationStatus,
          endorsements: a.endorsements,
          replaces: a.replaces,
          relatedObservations: a.relatedObservations,
          relatedInputRequests: a.relatedInputRequests,
          writer: a.writer
        });
      case "validation":
        return renderValidationCheckCard(
          {
            id: a.id,
            trackId: a.track,
            checkName: a.title,
            status: a.status,
            target,
            trigger: a.trigger,
            exitCode: a.exitCode,
            summary: a.body,
            summaryContentType: a.bodyContentType,
            summaryContentState: a.bodyContentState,
            completedAt: a.completedAt,
            createdAt: a.createdAt,
            verificationStatus: a.verificationStatus,
            endorsements: a.endorsements,
            command: a.command,
            logArtifactContentHashes: a.logArtifactContentHashes,
            writer: a.writer
          },
          a.continuity
        );
      default:
        return factCard(a.kind, {
          annotationId: a.id,
          track: a.track,
          title: a.title,
          status: a.status,
          body: a.body,
          bodyContentType: a.bodyContentType,
          bodyContentState: a.bodyContentState,
          createdAt: a.createdAt,
          writer: a.writer,
          tags: a.tags
        });
    }
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
  function partitionAnnotations(files, annotations) {
    const anchored = [];
    const decisionContext = [];
    const unanchored = [];
    for (const annotation of annotations) {
      const target = annotation.target ?? {};
      if (annotation.kind === "validation") {
        decisionContext.push(annotation);
        continue;
      }
      if (target.kind !== "range" && target.kind !== "file") {
        decisionContext.push(annotation);
        continue;
      }
      if (!target.filePath) {
        unanchored.push(annotation);
        continue;
      }
      const file = fileForFact(files, target.filePath);
      if (!file || target.kind === "range" && !rangeTouchesCapturedRows(annotation, file)) {
        unanchored.push(annotation);
        continue;
      }
      anchored.push(annotation);
    }
    return { anchored, decisionContext, unanchored };
  }
  __name(partitionAnnotations, "partitionAnnotations");
  function renderDecisionContext(annotations) {
    if (!annotations.length) return "";
    return `<section class="${CLASS.diffDecisionContext}" aria-label="Decision context">
    <h2>Decision context (${annotations.length})</h2>
    <p>Review evidence and recorded assessments for this revision. Validation remains context only.</p>
    <div class="${CLASS.annoGroup}">${annotations.map((annotation) => renderAnnotation(annotation, true)).join("")}</div>
  </section>`;
  }
  __name(renderDecisionContext, "renderDecisionContext");
  function renderUnanchoredFacts(annotations, filePaths) {
    if (!annotations.length) return "";
    return `<section class="${CLASS.diffUnanchoredFacts}" aria-label="Unanchored facts">
    <h2>Unanchored facts (${annotations.length})</h2>
    <div class="${CLASS.annoGroup}">${annotations.map(
      (annotation) => `<div><p class="${CLASS.diffAnchorReason}">${escapeHtml(unanchoredReason(annotation, filePaths))}</p>${renderAnnotation(annotation, true)}</div>`
    ).join("")}</div>
  </section>`;
  }
  __name(renderUnanchoredFacts, "renderUnanchoredFacts");
  function renderDiff(snapshotId, artifact, annotations) {
    const annos = annotations ?? [];
    const files = artifact.snapshot?.files ?? [];
    const filePaths = /* @__PURE__ */ new Set();
    for (const f of files) {
      if (f.new_path) filePaths.add(f.new_path);
      if (f.old_path) filePaths.add(f.old_path);
    }
    const { anchored, decisionContext, unanchored } = partitionAnnotations(
      files,
      annos
    );
    const ctx = {
      snapshotId,
      files,
      anchored,
      decisionContext,
      unanchored,
      filePaths
    };
    const counts = {};
    for (const a of annos) {
      counts[a.kind] = (counts[a.kind] ?? 0) + 1;
    }
    const breakdown = Object.entries(counts).map(([k, n]) => `${n} ${k}${n === 1 ? "" : "s"}`).join(", ");
    let html = `<div class="${CLASS.annoSummary}">${annos.length} review fact${annos.length === 1 ? "" : "s"} on this revision${breakdown ? ` · ${breakdown}` : ""}${unanchored.length ? ` · ${unanchored.length} not anchored to a diff line` : ""}</div>`;
    html += renderDecisionContext(decisionContext);
    html += renderUnanchoredFacts(unanchored, filePaths);
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
    <span><b>${summary.decisionContextCount}</b> context</span>
    <span><b>${summary.unanchoredCount}</b> unanchored</span>
  </div>`;
  }
  __name(renderDiffNavSummary, "renderDiffNavSummary");
  function unanchoredReason(a, filePaths) {
    const t = a.target ?? {};
    if (t.kind !== "range" && t.kind !== "file") {
      return "not a file or range target";
    }
    if (!t.filePath) return "target missing file path";
    if (t.kind === "range" && filePaths.has(t.filePath)) {
      return "line outside captured rows";
    }
    if (!filePaths.has(t.filePath)) return "file missing from snapshot";
    return "not anchored to a diff line";
  }
  __name(unanchoredReason, "unanchoredReason");
  var DIFF_FILE_QUERY_KEYS = ["path", "change", "has", "is"];
  var DIFF_FILE_CHANGE_VALUES = [
    "added",
    "deleted",
    "modified",
    "renamed",
    "copied"
  ];
  var DIFF_FILE_HAS_VALUES = ["facts"];
  var DIFF_FILE_IS_VALUES = ["unanchored"];
  function parseDiffFileQuery(query) {
    const clauses = [];
    const freeText = [];
    const diagnostics = [];
    for (const tok of tokenizeQuery(query || "")) {
      const colon = tok.indexOf(":");
      const field = colon > 0 ? tok.slice(0, colon).toLowerCase() : "";
      const rawValue = colon > 0 ? tok.slice(colon + 1).replace(/^"|"$/g, "").toLowerCase() : "";
      if (field === "status") {
        diagnostics.push({
          code: "unsupported-qualifier",
          key: "status",
          message: "status: isn't valid in the diff file search — use change: (added, deleted, modified, renamed, copied)"
        });
        continue;
      }
      if (DIFF_FILE_QUERY_KEYS.includes(field)) {
        const key = field;
        if (key === "change" && !DIFF_FILE_CHANGE_VALUES.includes(rawValue)) {
          diagnostics.push({
            code: "unsupported-value",
            key: "change",
            message: `change: has no value "${rawValue}" — expected one of ${DIFF_FILE_CHANGE_VALUES.join(", ")}`
          });
          continue;
        }
        if (key === "has" && !DIFF_FILE_HAS_VALUES.includes(rawValue)) {
          diagnostics.push({
            code: "unsupported-value",
            key: "has",
            message: `has: has no value "${rawValue}" — expected "facts"`
          });
          continue;
        }
        if (key === "is" && !DIFF_FILE_IS_VALUES.includes(rawValue)) {
          diagnostics.push({
            code: "unsupported-value",
            key: "is",
            message: `is: has no value "${rawValue}" — expected "unanchored"`
          });
          continue;
        }
        clauses.push({ field: key, value: rawValue });
        continue;
      }
      const term = tok.replace(/^"|"$/g, "").toLowerCase();
      if (term) freeText.push(term);
    }
    return { clauses, freeText, diagnostics };
  }
  __name(parseDiffFileQuery, "parseDiffFileQuery");
  function matchDiffFiles(ctx, query) {
    const { clauses, freeText, diagnostics } = parseDiffFileQuery(query);
    const files = ctx.files.filter((f) => {
      const label = filePathLabel(f).toLowerCase();
      for (const term of freeText) {
        if (!label.includes(term)) return false;
      }
      for (const c of clauses) {
        if (c.field === "path" && !label.includes(c.value)) return false;
        if (c.field === "change" && f.status !== c.value) return false;
        if (c.field === "has" && fileFactCount(f, ctx.anchored) <= 0)
          return false;
        if (c.field === "is" && fileFactCount(f, ctx.unanchored) <= 0)
          return false;
      }
      return true;
    });
    return { files, diagnostics };
  }
  __name(matchDiffFiles, "matchDiffFiles");

  // src/diff/controller.ts
  var PAGE_SURFACE = {
    title: "#diff-page-title",
    nav: "#diff-page-nav",
    // outer host — click delegation only, never rebuilt
    navList: "#diff-page-nav-list",
    // swappable content — renderDiffNav's target
    body: "#diff-page-body"
  };
  function surfaceBody() {
    return $(PAGE_SURFACE.body);
  }
  __name(surfaceBody, "surfaceBody");
  function surfaceNavList() {
    return $(PAGE_SURFACE.navList);
  }
  __name(surfaceNavList, "surfaceNavList");
  var shownDiffKey = null;
  var diffCtx = null;
  var diffFactCursor = -1;
  var diffChangeCursor = -1;
  var shownDiffFileQuery = "";
  var shownDiffFile = null;
  function syncDiffFileQueryInput() {
    const input = $("#diff-file-query");
    const value = getState().diffFileQuery;
    if (input && input.value !== value) input.value = value;
    shownDiffFileQuery = value;
  }
  __name(syncDiffFileQueryInput, "syncDiffFileQueryInput");
  var DIFF_ROUTE_CLEARED = {
    diff: null,
    diffHash: null,
    focus: null,
    diffPage: false,
    diffRevision: null,
    diffFile: null,
    diffFileQuery: ""
  };
  function openDiff(snapshotId, focusId = null, contentHash = null) {
    navigate({
      diffPage: true,
      diffRevision: revisionIdForSnapshot(snapshotId, contentHash),
      diff: snapshotId,
      diffHash: contentHash || null,
      focus: focusId || null
    });
  }
  __name(openDiff, "openDiff");
  function openRevisionDiff(revisionId, focusId = null) {
    navigate({
      diffPage: true,
      diffRevision: revisionId,
      diff: null,
      diffHash: null,
      focus: focusId || null
    });
  }
  __name(openRevisionDiff, "openRevisionDiff");
  function closeDiff() {
    const state2 = getState();
    if (!state2.diffPage && !state2.diff) return;
    navigate({ ...DIFF_ROUTE_CLEARED });
  }
  __name(closeDiff, "closeDiff");
  async function paintDiffPage(opts) {
    const title = $(PAGE_SURFACE.title);
    if (title) title.textContent = opts.title;
    const body = surfaceBody();
    if (body) body.innerHTML = `<p class="${CLASS.empty}">loading snapshot…</p>`;
    const nav = surfaceNavList();
    if (nav) nav.innerHTML = "";
    let snapshotUrl = `/api/snapshots/${encodeURIComponent(opts.snapshotId)}`;
    if (opts.contentHash)
      snapshotUrl += `?contentHash=${encodeURIComponent(opts.contentHash)}`;
    try {
      const artifact = await fetchJSON(snapshotUrl);
      if (!opts.stillCurrent()) return false;
      const { html, ctx } = renderDiff(
        opts.snapshotId,
        artifact,
        opts.annotations
      );
      const note = opts.factsNote ? `<p class="${CLASS.empty}">${escapeHtml(opts.factsNote)}</p>` : "";
      const liveBody = surfaceBody();
      if (liveBody) liveBody.innerHTML = note + html;
      diffCtx = ctx;
      diffFactCursor = -1;
      diffChangeCursor = -1;
      const liveNav = surfaceNavList();
      if (liveNav) liveNav.innerHTML = renderDiffNav();
      applyDiffFocus();
      return true;
    } catch (err) {
      if (!opts.stillCurrent()) return false;
      const liveBody = surfaceBody();
      if (liveBody)
        liveBody.innerHTML = `<p class="${CLASS.empty}">error: ${escapeHtml(
          err instanceof Error ? err.message : String(err)
        )}</p>`;
      return false;
    }
  }
  __name(paintDiffPage, "paintDiffPage");
  function applyDiffFileScroll() {
    const path = getState().diffFile;
    shownDiffFile = path;
    if (!path || !diffCtx) return;
    const idx = diffCtx.files.findIndex(
      (f) => f.new_path === path || f.old_path === path
    );
    if (idx < 0) return;
    const section = surfaceBody()?.querySelector(
      `.dfile[data-dfile="${idx}"]`
    );
    if (section) {
      expandDiffFile(section);
      section.scrollIntoView({ block: "start" });
    }
  }
  __name(applyDiffFileScroll, "applyDiffFileScroll");
  async function renderDiffPageFromRevision(revisionId) {
    const stillCurrent = /* @__PURE__ */ __name(() => getState().diffPage && getState().diffRevision === revisionId, "stillCurrent");
    const doc = await ensureRevisionComposite(revisionId);
    if (!stillCurrent()) return;
    if (!doc) {
      const body = $(PAGE_SURFACE.body);
      if (body)
        body.innerHTML = `<p class="${CLASS.empty}">error: revision ${escapeHtml(
          shortId(revisionId)
        )} could not be loaded</p>`;
      return;
    }
    const revision = doc.revision ?? {};
    const snapshotId = revision.objectId;
    if (!snapshotId) {
      const body = $(PAGE_SURFACE.body);
      if (body)
        body.innerHTML = `<p class="${CLASS.empty}">this revision names no captured snapshot</p>`;
      return;
    }
    const summary = revision.summary;
    const workLabel = revision.targetDisplay?.workLabel?.text;
    const discoveryLabel = summary || workLabel;
    const provenance = summary && workLabel ? ` · ${workLabel}` : "";
    const titlePrefix = discoveryLabel ? `${discoveryLabel}${provenance} · ` : "";
    const painted = await paintDiffPage({
      snapshotId,
      contentHash: revision.objectArtifactContentHash ?? null,
      annotations: compositeAnnotations(doc),
      title: `${titlePrefix}${shortId(revisionId)} · snapshot ${shortId(snapshotId)}`,
      stillCurrent,
      factsNote: null
    });
    if (painted) {
      syncDiffFileQueryInput();
      applyDiffFileScroll();
    }
  }
  __name(renderDiffPageFromRevision, "renderDiffPageFromRevision");
  async function renderDiffPageFromSnapshot(snapshotId, contentHash) {
    const stillCurrent = /* @__PURE__ */ __name(() => getState().diffPage && !getState().diffRevision && getState().diff === snapshotId && getState().diffHash === contentHash, "stillCurrent");
    const painted = await paintDiffPage({
      snapshotId,
      contentHash,
      annotations: [],
      title: `snapshot ${shortId(snapshotId)}`,
      stillCurrent,
      factsNote: "no review facts — this link names a snapshot the record cannot map to a revision"
    });
    if (painted) {
      syncDiffFileQueryInput();
      applyDiffFileScroll();
    }
  }
  __name(renderDiffPageFromSnapshot, "renderDiffPageFromSnapshot");
  function renderDiffPage() {
    const state2 = getState();
    if (!state2.diffPage) {
      shownDiffKey = null;
      diffCtx = null;
      return Promise.resolve();
    }
    const key = state2.diffRevision ? `page:rev:${state2.diffRevision}` : state2.diff ? `page:snap:${state2.diff}|${state2.diffHash ?? ""}` : null;
    if (!key) {
      const body = $(PAGE_SURFACE.body);
      if (body)
        body.innerHTML = `<p class="${CLASS.empty}">nothing to diff — this link names no snapshot</p>`;
      return Promise.resolve();
    }
    if (key === shownDiffKey) {
      if (getState().diffFileQuery !== shownDiffFileQuery) {
        syncDiffFileQueryInput();
        const nav = surfaceNavList();
        if (nav) nav.innerHTML = renderDiffNav();
      }
      if (getState().diffFile !== shownDiffFile) applyDiffFileScroll();
      applyDiffFocus();
      return Promise.resolve();
    }
    shownDiffKey = key;
    if (state2.diffRevision) return renderDiffPageFromRevision(state2.diffRevision);
    return renderDiffPageFromSnapshot(
      state2.diff,
      state2.diffHash ?? null
    );
  }
  __name(renderDiffPage, "renderDiffPage");
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
    const body = surfaceBody();
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
    const { files, anchored, decisionContext, unanchored, filePaths } = diffCtx;
    const { files: matchedFiles, diagnostics } = matchDiffFiles(
      diffCtx,
      getState().diffFileQuery
    );
    const matched = new Set(matchedFiles);
    const fileItems = files.map((f, i) => ({ f, i, factCount: fileFactCount(f, anchored) })).filter((item) => matched.has(item.f)).map(({ f, i, factCount: n }) => {
      const badge = n ? `<span class="${CLASS.dfileNotes}">${n}</span>` : "";
      return `<li><button class="${CLASS.diffNavFile}" data-nav-file="${i}">
        <span class="${diffStatusClass(escapeHtml(f.status ?? ""))}">${escapeHtml(f.status ?? "")}</span>
        <span class="${CLASS.dpath}">${escapeHtml(filePathLabel(f))}</span>${badge}</button></li>`;
    }).join("");
    let html = renderDiffNavSummary(diffNavSummary());
    if (diagnostics.length) {
      html += `<div class="${CLASS.diffFileNotice}" role="status">${diagnostics.map((d) => escapeHtml(d.message)).join(" ")}</div>`;
    }
    if (decisionContext.length) {
      const entries = decisionContext.map(
        (annotation) => `<li><button class="${CLASS.diffNavFact}" data-anno="${escapeHtml(annotation.id)}"><span>${escapeHtml(annotation.title)}</span><span class="${CLASS.diffNavReason}">${escapeHtml(annotation.kind)}</span></button></li>`
      ).join("");
      html += `<section class="${CLASS.diffDecisionContextNav}" aria-label="Decision context">
      <h3>Decision context (${decisionContext.length})</h3>
      <ol>${entries}</ol></section>`;
    }
    html += `<ol class="${CLASS.diffNavFiles}">${fileItems}</ol>`;
    if (unanchored.length) {
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
    if (!diffCtx)
      return {
        fileCount: 0,
        factCount: 0,
        decisionContextCount: 0,
        unanchoredCount: 0
      };
    return {
      fileCount: diffCtx.files.length,
      factCount: diffCtx.anchored.length + diffCtx.decisionContext.length + diffCtx.unanchored.length,
      decisionContextCount: diffCtx.decisionContext.length,
      unanchoredCount: diffCtx.unanchored.length
    };
  }
  __name(diffNavSummary, "diffNavSummary");
  function diffFactTargets() {
    return Array.from(
      surfaceBody()?.querySelectorAll(".anno[data-anno]") ?? []
    );
  }
  __name(diffFactTargets, "diffFactTargets");
  function diffChangeTargets() {
    return Array.from(
      surfaceBody()?.querySelectorAll(".dhunk") ?? []
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
  function onDiffBodyClick(ev) {
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
  }
  __name(onDiffBodyClick, "onDiffBodyClick");
  function onDiffBodyKeydown(ev) {
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
  }
  __name(onDiffBodyKeydown, "onDiffBodyKeydown");
  function onDiffNavClick(ev) {
    const t = ev.target;
    if (!(t instanceof Element)) return;
    const fileBtn = t.closest("[data-nav-file]");
    if (fileBtn) {
      const idx = Number(fileBtn.dataset.navFile);
      const section = surfaceBody()?.querySelector(
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
  }
  __name(onDiffNavClick, "onDiffNavClick");
  function initControls2() {
    $("#diff-page-close")?.addEventListener("click", () => closeDiff());
    const body = $(PAGE_SURFACE.body);
    body?.addEventListener("click", onDiffBodyClick);
    body?.addEventListener("keydown", onDiffBodyKeydown);
    $(PAGE_SURFACE.nav)?.addEventListener("click", onDiffNavClick);
    $("#diff-file-query")?.addEventListener("input", (ev) => {
      const value = ev.target.value;
      navigate({ diffFileQuery: value }, { replace: true });
      void renderDiffPage();
    });
  }
  __name(initControls2, "initControls");

  // src/workflow-handoff.ts
  var TRACK = "<your-track>";
  var OUTCOMES = "<approved|rejected|dismissed|superseded|abandoned>";
  var STATUSES = "<passed|failed|errored|skipped>";
  var CALLS = "<accepted|accepted-with-follow-up|needs-changes|needs-clarification>";
  var SHELL_SAFE = /^[A-Za-z0-9@%+:,._/-]+$/;
  function shellQuote(value) {
    if (SHELL_SAFE.test(value)) return value;
    return `'${value.replace(/'/g, `'\\''`)}'`;
  }
  __name(shellQuote, "shellQuote");
  function firstReviewHandoff() {
    return {
      label: "Capture your first revision",
      command: 'pointbreak capture --summary "<what changed>"',
      placeholders: ["<what changed>"]
    };
  }
  __name(firstReviewHandoff, "firstReviewHandoff");
  function revisionHandoffs(revisionId) {
    if (!revisionId) return [];
    const rev = shellQuote(revisionId);
    return [
      {
        label: "Add a claim (observation)",
        command: `pointbreak observation add --exact-revision ${rev} --track ${TRACK} --title "<claim title>" --body "<why it matters>"`,
        placeholders: [TRACK, "<claim title>", "<why it matters>"]
      },
      {
        label: "Record evidence (validation)",
        command: `pointbreak validation add --exact-revision ${rev} --track ${TRACK} --check-name "<check name>" --status ${STATUSES} --command "<command you ran>" --exit-code <exit-code> --summary "<what the run showed>"`,
        placeholders: [
          TRACK,
          "<check name>",
          STATUSES,
          "<command you ran>",
          "<exit-code>",
          "<what the run showed>"
        ]
      },
      {
        label: "Ask a question (input request)",
        command: `pointbreak input-request open --revision ${rev} --track ${TRACK} --title "<question>" --reason manual-decision-required --mode advisory --body "<what needs an answer>"`,
        placeholders: [TRACK, "<question>", "<what needs an answer>"]
      },
      {
        label: "Make the call (assessment)",
        command: `pointbreak assessment add --exact-revision ${rev} --track ${TRACK} --assessment ${CALLS} --summary "<why this call>"`,
        placeholders: [TRACK, CALLS, "<why this call>"]
      },
      {
        label: "Record a landed commit on this same revision",
        command: `pointbreak association record --revision ${rev} --track ${TRACK} --commit <landed-commit>`,
        placeholders: [TRACK, "<landed-commit>"]
      }
    ];
  }
  __name(revisionHandoffs, "revisionHandoffs");
  function respondHandoff(inputRequestId, label) {
    return {
      label,
      command: `pointbreak input-request respond ${shellQuote(inputRequestId)} --outcome ${OUTCOMES} --reason "<answer>"`,
      placeholders: [OUTCOMES, "<answer>"]
    };
  }
  __name(respondHandoff, "respondHandoff");
  function attentionHandoffs(item) {
    switch (item.kind) {
      case "open_input_request": {
        if (!item.inputRequestId) return [];
        return [respondHandoff(item.inputRequestId, "Respond to this request")];
      }
      case "ambiguous_assessment": {
        const candidates = item.assessments ?? [];
        const ids = candidates.map((record) => record.assessmentId ?? "");
        if (!item.revisionId || ids.length < 2 || ids.some((id) => !id))
          return [];
        const replaces = ids.map((id) => ` --replaces ${shellQuote(id)}`).join("");
        return [
          {
            label: "Record one current call replacing the competing assessments",
            command: `pointbreak assessment add --exact-revision ${shellQuote(item.revisionId)} --track ${TRACK} --assessment ${CALLS} --summary "<why this call>"${replaces}`,
            placeholders: [TRACK, CALLS, "<why this call>"]
          }
        ];
      }
      case "failed_validation": {
        if (!item.revisionId || !item.trackId || !item.checkName) return [];
        return [
          {
            label: "Record the re-run of this check",
            command: `pointbreak validation add --exact-revision ${shellQuote(item.revisionId)} --track ${shellQuote(item.trackId)} --check-name ${shellQuote(item.checkName)} --status ${STATUSES} --command "<command you ran>" --exit-code <exit-code> --summary "<what the re-run showed>"`,
            placeholders: [
              STATUSES,
              "<command you ran>",
              "<exit-code>",
              "<what the re-run showed>"
            ]
          }
        ];
      }
      case "follow_up_outstanding": {
        const ids = item.openInputRequestIds ?? [];
        if (!ids.length || ids.some((id) => !id)) return [];
        return ids.map(
          (id, index) => respondHandoff(
            id,
            ids.length === 1 ? "Respond to the open follow-up request" : `Respond to open follow-up request ${index + 1} of ${ids.length}`
          )
        );
      }
      case "competing_heads": {
        const heads = item.headRevisionIds ?? [];
        if (heads.length < 2 || heads.some((id) => !id)) return [];
        const supersedes = heads.map((id) => ` --supersedes ${shellQuote(id)}`).join("");
        return [
          {
            label: "Capture a replacement only when genuinely new content replaces every head",
            command: `pointbreak capture --summary "<what changed>"${supersedes}`,
            placeholders: ["<what changed>"]
          }
        ];
      }
      default:
        return [];
    }
  }
  __name(attentionHandoffs, "attentionHandoffs");
  function commandHtml(handoff) {
    let html = escapeHtml(handoff.command);
    for (const token of new Set(handoff.placeholders)) {
      const escaped = escapeHtml(token);
      html = html.split(escaped).join(`<span class="${CLASS.workflowPlaceholder}">${escaped}</span>`);
    }
    return html;
  }
  __name(commandHtml, "commandHtml");
  function renderWorkflowHandoff(handoff) {
    return `<div class="${CLASS.workflowHandoff}" data-workflow-handoff>
    <span class="${CLASS.workflowHandoffLabel}">${escapeHtml(handoff.label)}</span>
    <code class="${CLASS.workflowCommand}" data-workflow-command>${commandHtml(handoff)}</code>
    <button type="button" class="${CLASS.ghost} ${CLASS.workflowCopy}" data-copy-workflow-command aria-label="copy command: ${escapeHtml(handoff.label)}">copy</button>
  </div>`;
  }
  __name(renderWorkflowHandoff, "renderWorkflowHandoff");
  function renderWorkflowHandoffs(handoffs) {
    return handoffs.map(renderWorkflowHandoff).join("");
  }
  __name(renderWorkflowHandoffs, "renderWorkflowHandoffs");
  async function copyWorkflowCommand(button) {
    const text = button.closest(`.${CLASS.workflowHandoff}`)?.querySelector("[data-workflow-command]")?.textContent;
    if (!text) return;
    const previous = button.textContent ?? "copy";
    try {
      if (!navigator.clipboard?.writeText) {
        throw new Error("clipboard unavailable");
      }
      await navigator.clipboard.writeText(text);
      button.textContent = "copied";
    } catch {
      button.textContent = "copy failed";
    } finally {
      window.setTimeout(() => {
        button.textContent = previous;
      }, 1200);
    }
  }
  __name(copyWorkflowCommand, "copyWorkflowCommand");

  // src/lenses/attention.ts
  function partitionAttentionTiers(items) {
    return {
      primary: items.filter((item) => item.tier !== "secondary"),
      secondary: items.filter((item) => item.tier === "secondary")
    };
  }
  __name(partitionAttentionTiers, "partitionAttentionTiers");
  function renderAttention() {
    const el = $("#attention");
    if (!el) return;
    const items = getState().attention?.items ?? [];
    if (!items.length) {
      el.innerHTML = `<p class="${CLASS.attentionEmpty}" style="color:var(--fg-dim)">Nothing needs attention in this store.</p>`;
      return;
    }
    const { primary, secondary } = partitionAttentionTiers(items);
    const focus = getState().attentionFocus;
    el.innerHTML = `<div class="${CLASS.attentionOrderLabel}">longest waiting first</div>` + renderTier("Needs input", primary, focus) + renderTier("Advisory", secondary, focus);
  }
  __name(renderAttention, "renderAttention");
  function renderTier(label, items, focus) {
    if (!items.length) return "";
    return `<h3 class="${CLASS.attentionTier}">${escapeHtml(label)} (${items.length})</h3>${items.map((item) => renderAttentionCard(item, focus)).join("")}`;
  }
  __name(renderTier, "renderTier");
  function anchorRevision(item) {
    if (item.revisionId) return item.revisionId;
    return item.headRevisionIds?.[0] ?? "";
  }
  __name(anchorRevision, "anchorRevision");
  function renderAttentionCard(item, focus) {
    const anchor = anchorRevision(item);
    const focusClass = item.id === focus ? ` ${CLASS.attentionFocus}` : "";
    const kind = escapeHtml(item.kind.replace(/_/g, "-"));
    const subject = anchor ? shortRef(anchor) : "thread";
    const freshness = item.freshness?.state === "superseded" ? `<span class="${CLASS.attentionFreshness}">superseded${item.freshness.supersededBy?.length ? ` by ${item.freshness.supersededBy.map((id) => linkify(id)).join(", ")}` : ""}</span>` : "";
    const rows = [];
    const push = /* @__PURE__ */ __name((k, v, medium = false) => {
      const tier = medium ? ` class="${CLASS.tierMedium}"` : "";
      rows.push(`<span${tier}>${escapeHtml(k)}</span><b${tier}>${v}</b>`);
    }, "push");
    push("subject", escapeHtml(subject));
    for (const [k, v] of detailRows(item))
      push(k, v, k === "reason" || k === "track" || k === "actor");
    push("observed", escapeHtml(fmtDateTime(item.observedAt ?? "")), true);
    return `<div class="${CLASS.unitCard} ${CLASS.attentionCard}${focusClass}" data-entry-id="${escapeHtml(item.id)}" data-revision-id="${escapeHtml(anchor)}" title="${escapeHtml(item.id)}">
      <h3><span class="${CLASS.attentionKind}">${kind}</span> ${escapeHtml(askLabel(item))}</h3>
      ${freshness}
      <div class="${CLASS.kv}">${rows.join("")}</div>
      ${renderWorkflowHandoffs(attentionHandoffs(item))}
    </div>`;
  }
  __name(renderAttentionCard, "renderAttentionCard");
  function askLabel(item) {
    switch (item.kind) {
      case "open_input_request":
        return item.title ?? "open input request";
      case "ambiguous_assessment":
        return `${item.assessments?.length ?? 0} competing assessments`;
      case "competing_heads":
        return `${item.headRevisionIds?.length ?? 0} competing heads`;
      case "stale_assessment":
        return `stale ${item.assessment ?? "assessment"}`;
      case "failed_validation":
        return `${item.checkName ?? "check"} ${item.status ?? "failed"}`;
      case "follow_up_outstanding":
        return "follow-up outstanding";
      default:
        return item.kind.replace(/_/g, "-");
    }
  }
  __name(askLabel, "askLabel");
  function detailRows(item) {
    const actor = item.openedBy ?? item.recordedBy;
    const rows = [];
    if (item.reasonCode) rows.push(["reason", escapeHtml(item.reasonCode)]);
    if (item.mode) rows.push(["mode", escapeHtml(item.mode)]);
    if (item.trackId) rows.push(["track", escapeHtml(item.trackId)]);
    if (actor) rows.push(["actor", linkify(actor)]);
    if (item.kind === "competing_heads" && item.headRevisionIds) {
      rows.push([
        "heads",
        item.headRevisionIds.map((id) => linkify(id)).join(" ")
      ]);
    }
    if (item.kind === "ambiguous_assessment" && item.assessments) {
      rows.push([
        "assessments",
        item.assessments.map(
          (a) => `${escapeHtml(a.assessment ?? "")} (${escapeHtml(a.trackId ?? "")})`
        ).join(", ")
      ]);
    }
    if (item.kind === "failed_validation" && item.exitCode != null) {
      rows.push(["exit", escapeHtml(String(item.exitCode))]);
    }
    if (item.kind === "follow_up_outstanding" && item.openInputRequestIds) {
      rows.push([
        "requests",
        item.openInputRequestIds.map((id) => linkify(id)).join(" ")
      ]);
    }
    return rows;
  }
  __name(detailRows, "detailRows");

  // src/detail.ts
  function commitConditionLabel(condition) {
    return condition?.condition ?? "unknown";
  }
  __name(commitConditionLabel, "commitConditionLabel");
  function commitLivenessLabel(item) {
    let label = commitConditionLabel(item);
    if (item?.retention === "reflog") label += " (reflog-retained)";
    else if (item?.retention === "none") label += " (reflog expired)";
    return label;
  }
  __name(commitLivenessLabel, "commitLivenessLabel");
  function refContinuityLabel(entry) {
    if (!entry?.continuity) return "";
    let label = entry.continuity;
    if (entry.continuity === "rewritten" && entry.rewriteAction)
      label += ` by ${entry.rewriteAction}`;
    if (entry.sameTree === true) label += " (same tree)";
    if (entry.currentTipOid && entry.currentTipOid !== entry.recordedHeadOid)
      label += ` → ${shortId(entry.currentTipOid)}`;
    return label;
  }
  __name(refContinuityLabel, "refContinuityLabel");
  function shortGitRef(reference) {
    return (reference || "").replace(/^refs\/heads\//, "").replace(/^refs\/remotes\//, "");
  }
  __name(shortGitRef, "shortGitRef");
  function associationRows(label, rows) {
    const content = rows.length ? `<ul>${rows.join("")}</ul>` : `<p class="${CLASS.upEmpty}">none</p>`;
    return `<div><h3>${escapeHtml(label)}</h3>${content}</div>`;
  }
  __name(associationRows, "associationRows");
  function renderAssociationAndLanding(commitRange, diagnostics) {
    const range = commitRange ?? {};
    const currentCommits = range.currentCommits ?? [];
    const currentRefs = range.currentRefs ?? [];
    const withdrawnCommits = range.withdrawnCommits ?? [];
    const withdrawnRefs = range.withdrawnRefs ?? [];
    const livenessByCommit = new Map(
      (range.liveness?.perCommit ?? []).map((item) => [item.commitOid, item])
    );
    const hasLanding = currentCommits.some(
      (commit2) => commit2.source === "association"
    );
    const divergent = (diagnostics ?? []).some(
      (diagnostic) => diagnostic.code === "divergent_commit_association"
    );
    let headline;
    let captureQualifier = "";
    if (!currentCommits.length) {
      headline = "floating revision — no landing commit association recorded";
    } else if (hasLanding) {
      if (divergent) {
        headline = "landing ambiguous";
      } else if (range.liveness) {
        headline = `landing ${commitConditionLabel(range.liveness.headline)}`;
      } else {
        headline = "landing unknown — Git reachability unavailable";
      }
    } else {
      headline = range.liveness ? `anchored capture target ${commitConditionLabel(range.liveness.headline)}` : "anchored capture target unknown — Git reachability unavailable";
      captureQualifier = `<p class="${CLASS.advisoryNote}">no landing commit association recorded</p>`;
    }
    const commitRow = /* @__PURE__ */ __name((commit2) => {
      const liveness = livenessByCommit.get(commit2.commitOid);
      const association = commit2.commitAssociationId ? ` ${linkify(commit2.commitAssociationId)}` : "";
      return `<li>${linkify(commit2.commitOid)} <span class="${CLASS.factStatus}">${escapeHtml(commit2.source || "unknown")}</span>${association} <span>${escapeHtml(commitLivenessLabel(liveness))}</span></li>`;
    }, "commitRow");
    const continuityByRef = new Map(
      (range.liveness?.refContinuity ?? []).map((entry) => [
        `${entry.refName} ${entry.recordedHeadOid}`,
        entry
      ])
    );
    const refRow = /* @__PURE__ */ __name((reference) => {
      const continuity = refContinuityLabel(
        continuityByRef.get(`${reference.refName} ${reference.headOid}`)
      );
      const continuityCell = continuity ? ` <span class="${CLASS.factStatus}">${escapeHtml(continuity)}</span>` : "";
      return `<li>${escapeHtml(shortGitRef(reference.refName))} @ ${linkify(reference.headOid)}${continuityCell} ${linkify(reference.refAssociationId)}</li>`;
    }, "refRow");
    const withdrawnCommitRow = /* @__PURE__ */ __name((commit2) => `<li>${linkify(commit2.commitOid)} ${linkify(commit2.commitAssociationId)} ${linkify(commit2.commitWithdrawalId)}</li>`, "withdrawnCommitRow");
    const withdrawnRefRow = /* @__PURE__ */ __name((reference) => `<li>${escapeHtml(shortGitRef(reference.refName))} @ ${linkify(reference.headOid)} ${linkify(reference.refAssociationId)} ${linkify(reference.refWithdrawalId)}</li>`, "withdrawnRefRow");
    return `<section><h2>Association and landing</h2>
    <dl class="${CLASS.upIdentity}"><dt>anchored</dt><dd>${range.anchored ? "yes" : "no"}</dd><dt>state</dt><dd>${escapeHtml(headline)}</dd></dl>
    ${captureQualifier}
    ${associationRows("current commits", currentCommits.map(commitRow))}
    ${associationRows("current refs", currentRefs.map(refRow))}
    ${associationRows("withdrawn commits", withdrawnCommits.map(withdrawnCommitRow))}
    ${associationRows("withdrawn refs", withdrawnRefs.map(withdrawnRefRow))}
  </section>`;
  }
  __name(renderAssociationAndLanding, "renderAssociationAndLanding");
  var shownCompositeId = null;
  var compositeCache = /* @__PURE__ */ new Map();
  var compositeInFlight = /* @__PURE__ */ new Map();
  function ensureRevisionComposite(revisionId) {
    const eventSetHash = getState().history?.eventSetHash;
    const cached = compositeCache.get(revisionId);
    if (cached && cached.eventSetHash === eventSetHash)
      return Promise.resolve(cached.doc);
    const pending = compositeInFlight.get(revisionId);
    if (pending) return pending;
    const read = fetchJSON(`/api/revisions/${encodeURIComponent(revisionId)}`).then((d) => {
      const doc = d;
      compositeCache.set(revisionId, { doc, eventSetHash });
      return doc;
    }).catch(() => null).finally(() => {
      compositeInFlight.delete(revisionId);
    });
    compositeInFlight.set(revisionId, read);
    return read;
  }
  __name(ensureRevisionComposite, "ensureRevisionComposite");
  function compositeAnnotations(doc) {
    const out = [];
    for (const o of doc.observations ?? []) {
      out.push({
        kind: "observation",
        id: o.id ?? "",
        title: o.title ?? "(observation)",
        body: o.body ?? "",
        bodyContentType: o.bodyContentType,
        bodyContentState: o.bodyContentState,
        track: o.trackId ?? "",
        tags: Array.isArray(o.tags) ? o.tags : [],
        target: o.target ?? {},
        status: o.status,
        writer: o.writer,
        createdAt: o.createdAt,
        verificationStatus: o.verificationStatus,
        endorsements: o.endorsements,
        supersedes: o.supersedes
      });
    }
    for (const r of doc.inputRequests ?? []) {
      const meta = [r.mode, r.reasonCode].filter(Boolean).join(" · ");
      out.push({
        kind: "input-request",
        id: r.id ?? "",
        title: r.title ?? "(input request)",
        body: r.body ?? "",
        bodyContentType: r.bodyContentType,
        bodyContentState: r.bodyContentState,
        track: r.trackId ?? "",
        tags: meta ? [meta] : [],
        target: r.target ?? {},
        status: r.status,
        writer: r.writer,
        createdAt: r.createdAt,
        verificationStatus: r.verificationStatus,
        endorsements: r.endorsements,
        mode: r.mode,
        reasonCode: r.reasonCode,
        responses: (r.responses ?? []).map((response) => ({ ...response }))
      });
    }
    for (const a of doc.assessments ?? []) {
      const label = assessmentDisplayLabel(a.assessment ?? "");
      out.push({
        kind: "assessment",
        id: a.id ?? "",
        title: `assessment: ${label || "?"}`,
        body: a.summary ?? "",
        bodyContentType: a.summaryContentType,
        bodyContentState: a.summaryContentState,
        track: a.trackId ?? "",
        tags: [],
        target: a.target ?? {},
        status: a.status,
        writer: a.writer,
        createdAt: a.createdAt,
        verificationStatus: a.verificationStatus,
        endorsements: a.endorsements,
        assessment: a.assessment,
        replaces: a.replaces,
        relatedObservations: a.relatedObservations,
        relatedInputRequests: a.relatedInputRequests
      });
    }
    for (const v of doc.validationChecks ?? []) {
      out.push({
        kind: "validation",
        id: v.id ?? "",
        title: v.checkName ?? "(validation check)",
        body: v.summary ?? "",
        bodyContentType: v.summaryContentType,
        bodyContentState: v.summaryContentState,
        track: v.trackId ?? "",
        tags: [],
        target: v.target ?? {},
        status: v.status,
        writer: v.writer,
        createdAt: v.createdAt,
        completedAt: v.completedAt,
        verificationStatus: v.verificationStatus,
        endorsements: v.endorsements,
        trigger: v.trigger,
        exitCode: v.exitCode,
        command: v.command,
        logArtifactContentHashes: v.logArtifactContentHashes,
        continuity: v.id ? doc.validationContinuity?.checks?.[v.id] : void 0
      });
    }
    return out;
  }
  __name(compositeAnnotations, "compositeAnnotations");
  var SCROLL_MEMORY_CAP = 50;
  var scrollMemory = /* @__PURE__ */ new Map();
  var shownDetailKey = null;
  function rememberScroll() {
    const pane = $("#detail");
    if (!pane || shownDetailKey === null) return;
    scrollMemory.set(shownDetailKey, pane.scrollTop);
    if (scrollMemory.size > SCROLL_MEMORY_CAP) {
      const oldest = scrollMemory.keys().next().value;
      if (oldest !== void 0) scrollMemory.delete(oldest);
    }
  }
  __name(rememberScroll, "rememberScroll");
  function projectScroll(newKey) {
    const pane = $("#detail");
    if (!pane) {
      shownDetailKey = newKey;
      return;
    }
    if (shownDetailKey === newKey) return;
    pane.scrollTop = (newKey ? scrollMemory.get(newKey) : void 0) ?? 0;
    shownDetailKey = newKey;
  }
  __name(projectScroll, "projectScroll");
  function entityAnchor(kind, id, label) {
    return `<a href="#/${kind}/${encodeURIComponent(id)}" title="${escapeHtml(id)}">${escapeHtml(label ?? shortRef(id))}</a>`;
  }
  __name(entityAnchor, "entityAnchor");
  function eventBodyBlock(e) {
    const s = e.summary ?? {};
    if (s.body) return renderBodyContent(s.body, s.bodyContentType);
    if (s.summary) return renderBodyContent(s.summary, s.summaryContentType);
    if (s.reason) return renderBodyContent(s.reason, s.reasonContentType);
    return "";
  }
  __name(eventBodyBlock, "eventBodyBlock");
  function addRow(rows, label, value) {
    if (value === void 0 || value === null || value === "") return;
    rows.push([label, String(value)]);
  }
  __name(addRow, "addRow");
  function addListRow(rows, label, values) {
    if (!Array.isArray(values) || values.length === 0) return;
    rows.push([label, values.join(", ")]);
  }
  __name(addListRow, "addListRow");
  function addContentRows(rows, label, byteSize, hash, state2) {
    addRow(rows, `${label}Bytes`, byteSize);
    addRow(rows, `${label}Hash`, hash);
    addRow(rows, `${label}State`, state2);
  }
  __name(addContentRows, "addContentRows");
  function endpointSummary(endpoint) {
    if (!endpoint) return "";
    switch (endpoint.kind) {
      case "git_commit":
        return [
          "git_commit",
          endpoint.commitOid,
          endpoint.treeOid ? `tree ${endpoint.treeOid}` : ""
        ].filter(Boolean).join(" · ");
      case "git_tree":
        return ["git_tree", endpoint.treeOid].filter(Boolean).join(" · ");
      case "git_index":
        return ["git_index", endpoint.treeOid].filter(Boolean).join(" · ");
      case "git_working_tree":
        return "git_working_tree";
      default:
        return endpoint.kind ?? "";
    }
  }
  __name(endpointSummary, "endpointSummary");
  function sourceSummary(source) {
    if (!source) return "";
    const parts = [source.kind, source.mode];
    if (source.includeUntracked !== void 0) {
      parts.push(source.includeUntracked ? "includes untracked" : "tracked only");
    }
    if (source.pathspecs?.length) {
      parts.push(`pathspecs ${source.pathspecs.join(", ")}`);
    }
    return parts.filter(Boolean).join(" · ");
  }
  __name(sourceSummary, "sourceSummary");
  function targetSummary(target) {
    if (!target) return "";
    const kind = target.kind || "target";
    const line = target.filePath && target.startLine ? `${target.filePath}:${target.startLine}-${target.endLine || target.startLine}` : target.filePath;
    switch (kind) {
      case "revision":
        return ["revision", target.revisionId].filter(Boolean).join(" · ");
      case "file":
        return ["file", target.revisionId, line].filter(Boolean).join(" · ");
      case "range":
        return ["range", target.revisionId, line, target.side].filter(Boolean).join(" · ");
      case "observation":
        return ["observation", target.observationId, target.revisionId].filter(Boolean).join(" · ");
      case "input_request":
        return ["input request", target.inputRequestId, target.revisionId].filter(Boolean).join(" · ");
      case "assessment":
        return ["assessment", target.assessmentId, target.revisionId].filter(Boolean).join(" · ");
      case "event":
        return ["event", target.eventId, target.revisionId].filter(Boolean).join(" · ");
      default:
        return [kind, target.revisionId, line].filter(Boolean).join(" · ");
    }
  }
  __name(targetSummary, "targetSummary");
  function pushEventTypeRows(e, rows) {
    const s = e.summary ?? {};
    switch (e.eventType) {
      case "review_initialized":
        addRow(rows, "summary", "review initialized");
        break;
      case "work_object_proposed":
        addRow(rows, "snapshot", s.objectId);
        addRow(rows, "engagement", s.engagementId);
        addRow(rows, "artifactHash", s.objectArtifactContentHash);
        addRow(rows, "source", sourceSummary(s.source));
        addRow(rows, "base", endpointSummary(s.base));
        addRow(rows, "targetEndpoint", endpointSummary(s.target));
        break;
      case "review_observation_recorded":
        addRow(rows, "observationId", s.observationId);
        addRow(rows, "target", targetSummary(s.target));
        addRow(rows, "confidence", s.confidence);
        addListRow(rows, "tags", s.tags);
        addListRow(rows, "supersedes", s.supersedes);
        addListRow(rows, "respondsTo", s.respondsTo);
        addContentRows(
          rows,
          "body",
          s.bodyByteSize,
          s.bodyContentHash,
          s.bodyContentState
        );
        break;
      case "review_assessment_recorded":
        addRow(rows, "assessmentId", s.assessmentId);
        addRow(rows, "assessment", s.assessment);
        addRow(rows, "target", targetSummary(s.target));
        addListRow(rows, "replaces", s.replaces);
        addListRow(rows, "relatedObservations", s.relatedObservations);
        addListRow(rows, "relatedInputRequests", s.relatedInputRequests);
        addContentRows(
          rows,
          "summary",
          s.summaryByteSize,
          s.summaryContentHash,
          s.summaryContentState
        );
        break;
      case "input_request_opened":
        addRow(rows, "inputRequestId", s.inputRequestId);
        addRow(rows, "mode", s.mode);
        addRow(rows, "reasonCode", s.reasonCode);
        addRow(rows, "target", targetSummary(s.target));
        addContentRows(
          rows,
          "body",
          s.bodyByteSize,
          s.bodyContentHash,
          s.bodyContentState
        );
        break;
      case "input_request_responded":
        addRow(rows, "inputRequestResponseId", s.inputRequestResponseId);
        addRow(rows, "inputRequestId", s.inputRequestId);
        addRow(rows, "outcome", s.outcome);
        addContentRows(
          rows,
          "reason",
          s.reasonByteSize,
          s.reasonContentHash,
          s.reasonContentState
        );
        break;
      case "review_note_imported":
        addRow(rows, "summary", "retired note import");
        break;
      case "validation_check_recorded":
        addRow(rows, "validationCheckId", s.validationCheckId);
        addRow(rows, "target", targetSummary(s.target));
        addRow(rows, "check", s.checkName);
        addRow(rows, "status", s.status);
        addRow(rows, "trigger", s.trigger);
        addRow(rows, "exitCode", s.exitCode);
        addRow(rows, "command", s.command);
        addRow(rows, "sourceFingerprint", s.sourceFingerprint);
        addRow(rows, "startedAt", s.startedAt);
        addRow(rows, "completedAt", s.completedAt);
        addListRow(rows, "logArtifacts", s.logArtifactContentHashes);
        addContentRows(
          rows,
          "summary",
          void 0,
          s.summaryContentHash,
          s.summaryContentState
        );
        break;
      case "revision_ref_associated":
        addRow(rows, "refAssociationId", s.refAssociationId);
        addRow(rows, "refName", s.refName);
        addRow(rows, "headOid", s.headOid);
        break;
      case "revision_ref_withdrawn":
        addRow(rows, "refWithdrawalId", s.refWithdrawalId);
        addRow(rows, "refAssociationId", s.refAssociationId);
        break;
      case "revision_commit_associated":
        addRow(rows, "commitAssociationId", s.commitAssociationId);
        addRow(rows, "commitOid", s.commitOid);
        addRow(rows, "treeOid", s.treeOid);
        break;
      case "revision_commit_withdrawn":
        addRow(rows, "commitWithdrawalId", s.commitWithdrawalId);
        addRow(rows, "commitAssociationId", s.commitAssociationId);
        break;
      default:
        addRow(rows, "summaryKind", s.kind);
        break;
    }
  }
  __name(pushEventTypeRows, "pushEventTypeRows");
  function rawEventBlock(e) {
    const raw = escapeHtml(JSON.stringify(e, null, 2));
    return `<details class="${CLASS.rawEvent}">
    <summary>Raw event</summary>
    <div class="${CLASS.rawEventActions}"><button class="${CLASS.ghost}" type="button" data-copy-raw-event>copy</button></div>
    <pre data-raw-event>${raw}</pre>
  </details>`;
  }
  __name(rawEventBlock, "rawEventBlock");
  function renderDetail() {
    shownCompositeId = null;
    const el = $("#detail-body");
    if (!el) return;
    rememberScroll();
    const selected = selectedEventId();
    const e = selected ? eventForId(selected) : void 0;
    if (!e) {
      el.innerHTML = `<p class="${CLASS.empty}">Select an event or revision to inspect.</p>`;
      projectScroll(null);
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
      ["actor", entryActor(e) || "—"]
    ];
    const principal = principalLabel(e);
    if (principal) kv.push(["principal", principal]);
    const snapshotId = revisionId ? snapshotIdForRevision(revisionId) : "";
    const s = e.summary ?? {};
    if (e.eventType === "work_object_proposed") {
      const predecessors = supersedesRevision(revisionId);
      if (predecessors.length) kv.push(["supersedes", predecessors.join(", ")]);
    }
    pushEventTypeRows(e, kv);
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
    const kvValue = /* @__PURE__ */ __name((k, v) => {
      if (k === "eventId" && e.eventId) return entityAnchor("event", e.eventId);
      if (k === "revision" && revisionId)
        return entityAnchor("revision", revisionId);
      if (k === "actor" && v !== "—") return actorChip(v);
      return linkify(v);
    }, "kvValue");
    el.innerHTML = `
    <h2>${e.eventId ? entityAnchor("event", e.eventId, entryTitle(e)) : linkify(entryTitle(e))}</h2>
    <dl class="${CLASS.kv}">${kv.map(([k, v]) => `<dt>${escapeHtml(k)}</dt><dd>${kvValue(k, v)}</dd>`).join("")}</dl>
    ${readback}
    ${diffButton}
    ${bodyBlock}
    ${rawEventBlock(e)}`;
    projectScroll(e.eventId ?? null);
  }
  __name(renderDetail, "renderDetail");
  function renderRevisionSupersessionBlock(thread, selfId) {
    const laid = thread?.laidOut;
    if (!thread || !laid || !(laid.nodes ?? []).length) return "";
    const svg = renderSupersessionSvg(laid, {
      idAttr: "data-revision-id",
      ariaNoun: "revision",
      interactive: true,
      isSelected: /* @__PURE__ */ __name((id) => id === selfId, "isSelected")
    });
    if (!svg) return "";
    const heads = thread.heads ?? [];
    const chips = thread.competing ? `<div class="${CLASS.revisionHeads}"><span class="${CLASS.factStatus} ${CLASS.competing}">competing revisions (${heads.length})</span> ${heads.map(
      (h) => linkify(h) + (h === selfId ? `<span class="${CLASS.revisionSelf}">you are here</span>` : "")
    ).join(" ")}</div>` : "";
    const caption = `revision supersession${thread.competing ? ` — ${heads.length} competing` : ""}`;
    return `<figure class="${CLASS.revisionSupersession}"><figcaption>${escapeHtml(caption)}</figcaption>${chips}${svg}</figure>`;
  }
  __name(renderRevisionSupersessionBlock, "renderRevisionSupersessionBlock");
  function wireDagInteractions(scope) {
    const nav = /* @__PURE__ */ __name((node) => {
      const id = node.getAttribute("data-revision-id");
      if (id)
        navigate({
          selected: { kind: "revision", id },
          ...DIFF_ROUTE_CLEARED
        });
    }, "nav");
    for (const node of Array.from(
      scope.querySelectorAll(".dag-node")
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
          scope.querySelectorAll(
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
  var scopedAttention = null;
  var scopedAttentionPending = null;
  var scopedAttentionGeneration = 0;
  function scopedAttentionFresh(revisionId) {
    const eventSetHash = getState().attention?.eventSetHash;
    const hit = /* @__PURE__ */ __name((s) => s?.revisionId === revisionId && s.eventSetHash === eventSetHash, "hit");
    return hit(scopedAttention) || hit(scopedAttentionPending);
  }
  __name(scopedAttentionFresh, "scopedAttentionFresh");
  async function fetchScopedAttention(revisionId) {
    const eventSetHash = getState().attention?.eventSetHash;
    const generation = ++scopedAttentionGeneration;
    scopedAttentionPending = { revisionId, eventSetHash };
    let items;
    try {
      const doc = await fetchJSON(
        `/api/attention?revision=${encodeURIComponent(revisionId)}`
      );
      items = doc.items ?? [];
    } catch {
      items = null;
    }
    if (generation !== scopedAttentionGeneration) return;
    scopedAttentionPending = null;
    scopedAttention = { revisionId, eventSetHash, items };
  }
  __name(fetchScopedAttention, "fetchScopedAttention");
  function renderOutstandingBlock(revisionId) {
    const items = scopedAttention?.revisionId === revisionId ? scopedAttention.items : null;
    if (!items?.length) return "";
    const rows = items.map((item) => {
      const anchor = anchorRevision(item);
      const kind = escapeHtml(item.kind.replace(/_/g, "-"));
      return `<li><span class="${CLASS.attentionKind}">${kind}</span> ${escapeHtml(askLabel(item))}${anchor ? ` ${linkify(anchor)}` : ""}${renderWorkflowHandoffs(attentionHandoffs(item))}</li>`;
    }).join("");
    return `<section class="${CLASS.outstandingSet}"><h2>Outstanding (${items.length})</h2><ul>${rows}</ul></section>`;
  }
  __name(renderOutstandingBlock, "renderOutstandingBlock");
  async function refreshOutstandingIfStale(revisionId) {
    if (scopedAttentionFresh(revisionId)) return;
    await fetchScopedAttention(revisionId);
    if (revisionId !== shownCompositeId) return;
    const host = $("#detail-body")?.querySelector(
      "[data-outstanding-host]"
    );
    if (host) host.innerHTML = renderOutstandingBlock(revisionId);
  }
  __name(refreshOutstandingIfStale, "refreshOutstandingIfStale");
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
    const title = ru.summary || ru.targetDisplay?.workLabel?.text || `${shortId(ru.id)}${base.commitOid ? ` · base ${shortId(base.commitOid)}` : ""}`;
    const staleContext = staleFactSectionContext(revisionId);
    const observationContext = renderFactSupersessionBlock(
      d.factSupersession?.observations,
      "observation"
    ) + staleContext;
    const assessmentContext = renderFactSupersessionBlock(d.factSupersession?.assessments, "assessment") + staleContext;
    const snapshotUnavailable = revisionSnapshotUnavailable(d);
    const stat = /* @__PURE__ */ __name((label, n) => `<span class="${CLASS.upStat}"><b>${n ?? 0}</b> ${label}</span>`, "stat");
    const sections = [];
    sections.push(`<section><h2>Revision</h2><dl class="${CLASS.upIdentity}">
    <dt>id</dt><dd>${linkify(ru.id)}</dd>
    <dt>summary</dt><dd>${escapeHtml(ru.summary ?? "—")}</dd>
    <dt>work</dt><dd>${workLabelText(ru.targetDisplay)}</dd>
    <dt>base</dt><dd>${base.commitOid ? linkify(base.commitOid) : "—"} ${base.kind ? `<span class="${CLASS.factStatus}">${escapeHtml(base.kind)}</span>` : ""}</dd>
    <dt>target</dt><dd>${targetDisplayLabel(ru.targetDisplay)}${targetHeadBadge(ru.targetDisplay)}</dd>
    <dt>worktree</dt><dd>${escapeHtml(ru.targetDisplay?.label ?? "working tree")}</dd>
    <dt>head</dt><dd>${escapeHtml(ru.targetDisplay?.head?.label ?? "—")}</dd>
    <dt>supersession</dt><dd>${badge || "—"}</dd>
    <dt>snapshot</dt><dd>${linkify(ru.objectId)}</dd>
  </dl>${revisionDiagnostics(d)}${renderRevisionSupersessionBlock(d.revisionSupersession, revisionId)}</section>`);
    sections.push(renderAssociationAndLanding(d.commitRange, d.diagnostics));
    sections.push(
      `<section><h2>Current assessment</h2>${verdictBadge(d.currentAssessment)}${currentAssessmentSummary(d)}<p class="${CLASS.advisoryNote}">advisory — a recorded judgement, not a merge gate</p></section>`
    );
    sections.push(
      `<div data-outstanding-host>${renderOutstandingBlock(revisionId)}</div>`
    );
    if (revisionId) {
      sections.push(
        `<details class="${CLASS.workflowHandoffs}"><summary>Record with the CLI</summary>
      <p class="${CLASS.advisoryNote}">copy a command into your terminal and replace each &lt;placeholder&gt; before running — Review never runs commands</p>
      ${renderWorkflowHandoffs(revisionHandoffs(revisionId))}</details>`
      );
    }
    sections.push(`<section><h2>Summary</h2><div class="${CLASS.upStats}">
    ${stat("files", s.fileCount)}${stat("rows", s.rowCount)}${stat("observations", s.observationCount)}${stat("input requests", s.inputRequestCount)}${stat("assessments", s.assessmentCount)}${stat("validation checks", s.validationCheckCount)}
  </div>
  <div style="margin-top:10px">
    ${snapshotUnavailable ? `<button class="${CLASS.ghost} ${CLASS.diffBtn}" id="up-diff-btn" type="button" disabled title="captured snapshot content is unavailable">snapshot unavailable</button>` : `<button class="${CLASS.ghost} ${CLASS.diffBtn}" id="up-diff-btn" type="button" data-open-diff="${escapeHtml(ru.objectId ?? "")}" data-diff-hash="${escapeHtml(ru.objectArtifactContentHash ?? "")}">view annotated diff</button>`}
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
    const validationDispositions = d.validationContinuity?.checks;
    const renderedValidationChecks = validationChecks.map(
      (check) => renderValidationCheckCard(
        check,
        check.id ? validationDispositions?.[check.id] : void 0
      )
    ).join("");
    const validationBody = validationChecks.length ? `${renderedValidationChecks}<p class="${CLASS.validationNote}">context only — does not affect the current assessment</p>` : `<p class="${CLASS.upEmpty}">none</p>`;
    sections.push(
      `<section><h2>Validation checks (${validationChecks.length})</h2>${staleContext}${validationBody}</section>`
    );
    const el = $("#detail-body");
    if (el) {
      el.innerHTML = `<div class="${CLASS.unitPage}"><p class="${CLASS.unitPageTitle}">${escapeHtml(title)}</p>${sections.join("")}</div>`;
      const block = el.querySelector(
        `.${CLASS.revisionSupersession}`
      );
      if (block) wireDagInteractions(block);
    }
    projectScroll(revisionId || null);
  }
  __name(renderRevisionPage, "renderRevisionPage");
  async function openRevision(revisionId) {
    const el = $("#detail-body");
    rememberScroll();
    if (el) el.innerHTML = `<p class="${CLASS.upEmpty}">loading…</p>`;
    const [d] = await Promise.all([
      ensureRevisionComposite(revisionId),
      fetchScopedAttention(revisionId)
    ]);
    const sel = getState().selected;
    if (sel.kind !== "revision" || sel.id !== revisionId) return;
    if (!d) {
      const live = $("#detail-body");
      if (live)
        live.innerHTML = `<p class="${CLASS.upEmpty}">error: revision ${escapeHtml(
          shortRef(revisionId)
        )} could not be loaded</p>`;
      return;
    }
    renderRevisionPage(d);
  }
  __name(openRevision, "openRevision");
  function showComposite(revisionId) {
    if (revisionId === shownCompositeId)
      return refreshOutstandingIfStale(revisionId);
    shownCompositeId = revisionId;
    return openRevision(revisionId);
  }
  __name(showComposite, "showComposite");
  async function copyRawEvent(button) {
    const raw = button.closest(`.${CLASS.rawEvent}`)?.querySelector("[data-raw-event]")?.textContent;
    if (!raw) return;
    const previous = button.textContent ?? "copy";
    try {
      if (!navigator.clipboard?.writeText) {
        throw new Error("clipboard unavailable");
      }
      await navigator.clipboard.writeText(raw);
      button.textContent = "copied";
    } catch {
      button.textContent = "copy failed";
    } finally {
      window.setTimeout(() => {
        button.textContent = previous;
      }, 1200);
    }
  }
  __name(copyRawEvent, "copyRawEvent");
  function initControls3() {
    const el = $("#detail");
    el?.addEventListener("click", (ev) => {
      const t = ev.target;
      if (!(t instanceof Element)) return;
      const rawCopyBtn = t.closest("[data-copy-raw-event]");
      if (rawCopyBtn) {
        void copyRawEvent(rawCopyBtn);
        return;
      }
      const workflowCopyBtn = t.closest(
        "[data-copy-workflow-command]"
      );
      if (workflowCopyBtn) {
        void copyWorkflowCommand(workflowCopyBtn);
        return;
      }
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
  __name(initControls3, "initControls");

  // src/disclosure.ts
  var active = null;
  function createDisclosure({
    container,
    trigger,
    panel
  }) {
    let open3 = false;
    const controller = {
      isOpen: /* @__PURE__ */ __name(() => open3, "isOpen"),
      open: /* @__PURE__ */ __name(() => {
        if (active && active !== controller) active.close();
        open3 = true;
        active = controller;
        controller.sync();
      }, "open"),
      close: /* @__PURE__ */ __name((returnFocus = false) => {
        open3 = false;
        if (active === controller) active = null;
        controller.sync();
        if (returnFocus) $(trigger)?.focus();
      }, "close"),
      toggle: /* @__PURE__ */ __name(() => {
        if (open3) controller.close();
        else controller.open();
      }, "toggle"),
      sync: /* @__PURE__ */ __name(() => {
        $(panel)?.classList.toggle("hidden", !open3);
        $(trigger)?.setAttribute("aria-expanded", String(open3));
      }, "sync")
    };
    $(trigger)?.addEventListener("click", (event) => {
      event.stopPropagation();
      controller.toggle();
    });
    $(container)?.addEventListener("keydown", (event) => {
      if (event.key !== "Escape" || !open3) return;
      event.preventDefault();
      event.stopPropagation();
      controller.close(true);
    });
    document.addEventListener(
      "click",
      (event) => {
        if (!open3) return;
        const root = $(container);
        if (event.target instanceof Node && root?.contains(event.target)) return;
        controller.close();
      },
      true
    );
    controller.sync();
    return controller;
  }
  __name(createDisclosure, "createDisclosure");

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
  function handleOverlayKey(ev) {
    if (!activeOverlay) return false;
    if (ev.key === "Tab") {
      trapFocus(ev);
      return true;
    }
    if (ev.key === "Escape") {
      ev.preventDefault();
      closeActive();
      return true;
    }
    const reg = registry.get(activeOverlay.name);
    reg?.onKey?.(ev);
    return true;
  }
  __name(handleOverlayKey, "handleOverlayKey");
  function noop() {
  }
  __name(noop, "noop");

  // src/help-overlay.ts
  function onClose() {
  }
  __name(onClose, "onClose");
  function closeKeyHelp(opts = {}) {
    close("help", opts);
  }
  __name(closeKeyHelp, "closeKeyHelp");
  function initControls4() {
    const node = $("#key-help");
    if (!node) return;
    register("help", {
      node,
      onClose,
      // ? toggles the cheat sheet, so the open sheet owns the key: pressing it
      // again closes. Every other key is the manager's business (Tab trap,
      // Escape, deliberate inertness).
      onKey: /* @__PURE__ */ __name((ev) => {
        if (ev.key !== "?") return false;
        ev.preventDefault();
        closeKeyHelp();
        return true;
      }, "onKey")
    });
    $("#key-help-close")?.addEventListener("click", () => closeKeyHelp());
    node.addEventListener("click", (ev) => {
      if (ev.target === node) closeKeyHelp();
    });
  }
  __name(initControls4, "initControls");

  // src/prefs.ts
  var THEME_KEY = "shore-inspect-theme";
  var DENSITY_KEY = "shore-inspect-density";
  var SPLIT_KEY = "shore-inspect-split";
  var SPLIT_MIN = 25;
  var SPLIT_MAX = 75;
  var liveMediaQueries = [];
  var densityListeners = [];
  function registerDensityListener(listener) {
    densityListeners.push(listener);
  }
  __name(registerDensityListener, "registerDensityListener");
  function notifyDensityListeners() {
    for (const listener of densityListeners) listener();
  }
  __name(notifyDensityListeners, "notifyDensityListeners");
  function preferredThemeMode() {
    const stored = localStorage.getItem(THEME_KEY);
    return stored === "light" || stored === "dark" ? stored : "system";
  }
  __name(preferredThemeMode, "preferredThemeMode");
  function hasPinnedTheme() {
    return preferredThemeMode() !== "system";
  }
  __name(hasPinnedTheme, "hasPinnedTheme");
  function osTheme() {
    return window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
  }
  __name(osTheme, "osTheme");
  function preferredTheme() {
    const mode = preferredThemeMode();
    return mode === "system" ? osTheme() : mode;
  }
  __name(preferredTheme, "preferredTheme");
  function syncChoice(name, value) {
    for (const input of document.querySelectorAll(
      `input[name="${name}"]`
    )) {
      input.checked = input.value === value;
    }
  }
  __name(syncChoice, "syncChoice");
  function applyTheme(theme) {
    document.documentElement.setAttribute("data-theme", theme);
    syncChoice("theme-mode", preferredThemeMode());
  }
  __name(applyTheme, "applyTheme");
  function setThemeMode(mode) {
    const next = mode === "light" || mode === "dark" ? mode : "system";
    localStorage.setItem(THEME_KEY, next);
    applyTheme(preferredTheme());
  }
  __name(setThemeMode, "setThemeMode");
  function preferredDensity() {
    return localStorage.getItem(DENSITY_KEY) || "comfortable";
  }
  __name(preferredDensity, "preferredDensity");
  function applyDensity(mode) {
    const value = mode === "compact" ? "compact" : "comfortable";
    document.documentElement.classList.toggle("compact", value === "compact");
    syncChoice("density-mode", value);
  }
  __name(applyDensity, "applyDensity");
  function setDensity(mode) {
    const next = mode === "compact" ? "compact" : "comfortable";
    localStorage.setItem(DENSITY_KEY, next);
    applyDensity(next);
  }
  __name(setDensity, "setDensity");
  function preferredSplit() {
    const raw = localStorage.getItem(SPLIT_KEY);
    const n = raw === null ? Number.NaN : Number.parseInt(raw, 10);
    return Number.isInteger(n) && n >= SPLIT_MIN && n <= SPLIT_MAX ? n : null;
  }
  __name(preferredSplit, "preferredSplit");
  function applySplit(pct) {
    if (pct === null) {
      document.documentElement.style.removeProperty("--split-master");
      localStorage.removeItem(SPLIT_KEY);
      return;
    }
    const clamped = Math.round(Math.min(SPLIT_MAX, Math.max(SPLIT_MIN, pct)));
    document.documentElement.style.setProperty("--split-master", `${clamped}%`);
    localStorage.setItem(SPLIT_KEY, String(clamped));
  }
  __name(applySplit, "applySplit");
  function applyPrefs() {
    applyTheme(preferredTheme());
    applyDensity(preferredDensity());
    const split = preferredSplit();
    if (split !== null) applySplit(split);
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
    $("#view-panel")?.addEventListener("change", (event) => {
      const input = event.target;
      if (!(input instanceof HTMLInputElement) || !input.checked) return;
      if (input.name === "theme-mode") setThemeMode(input.value);
      if (input.name === "density-mode") {
        setDensity(input.value);
        notifyDensityListeners();
      }
    });
    watchColorScheme();
  }
  __name(initControls5, "initControls");

  // src/lenses/timeline.ts
  var ROW_H = 52;
  var rowH = ROW_H;
  function timelineRowHeight() {
    return rowH;
  }
  __name(timelineRowHeight, "timelineRowHeight");
  var OVERSCAN = 8;
  var REMEASURE_SETTLE_MS = 150;
  var remeasureTimer;
  var everMeasured = false;
  function remeasureTimelineRows() {
    const list = $("#timeline");
    if (!list) return;
    const rows = list.querySelectorAll("li.event[data-event-id]");
    if (rows.length === 0) return;
    let total = 0;
    for (const row of rows) total += row.getBoundingClientRect().height;
    const mean = total / rows.length;
    if (!Number.isFinite(mean) || mean <= 0) return;
    everMeasured = true;
    if (Math.abs(mean - rowH) < 0.5) return;
    const anchored = anchoredScrollTop(list, rowH, mean);
    rowH = mean;
    list.scrollTop = anchored;
    renderTimeline();
  }
  __name(remeasureTimelineRows, "remeasureTimelineRows");
  function anchoredScrollTop(list, prevRowH, nextRowH) {
    const listTop = list.getBoundingClientRect().top;
    const first = list.firstElementChild;
    const leadingPx = first?.dataset.spacer === "1" ? Number.parseFloat(first.style.height) || 0 : 0;
    const paintStart = Math.round(leadingPx / prevRowH);
    const rows = list.querySelectorAll("li.event[data-event-id]");
    let idx = 0;
    for (const row of rows) {
      const r = row.getBoundingClientRect();
      if (r.height > 0 && r.bottom > listTop)
        return Math.max(0, (paintStart + idx) * nextRowH - (r.top - listTop));
      idx++;
    }
    return list.scrollTop / prevRowH * nextRowH;
  }
  __name(anchoredScrollTop, "anchoredScrollTop");
  function scheduleTimelineRemeasure() {
    clearTimeout(remeasureTimer);
    remeasureTimer = setTimeout(remeasureTimelineRows, REMEASURE_SETTLE_MS);
  }
  __name(scheduleTimelineRemeasure, "scheduleTimelineRemeasure");
  registerDensityListener(scheduleTimelineRemeasure);
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
    const maxScroll = Math.max(0, rowCount * rowH - viewportH);
    const clamped = Math.min(Math.max(0, scrollTop), maxScroll);
    const start = Math.max(0, Math.floor(clamped / rowH) - OVERSCAN);
    const end = Math.min(
      rowCount,
      Math.ceil((clamped + viewportH) / rowH) + OVERSCAN
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
    const tags = entryTags(e).map(
      (t) => `<span class="${CLASS.badge} ${CLASS.tierMedium}">${escapeHtml(t)}</span>`
    ).join(" ");
    const revisionId = entryRevisionId(e);
    const verification = verificationChip(e.verificationStatus ?? "");
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
          ${revisionId ? `<span class="${CLASS.tierMedium}">revision ${escapeHtml(shortId(revisionId))}</span>` : ""}
          ${entryAnchor(e) ? `<span class="${CLASS.tierMedium}">${escapeHtml(entryAnchor(e))}</span>` : ""}
          ${verification ? `<span class="${CLASS.tierMedium}">${verification}</span>` : ""}
        </span>
      </span>`;
    return li;
  }
  __name(eventRow, "eventRow");
  function ensureScrollListener(list) {
    if (list.dataset.virtualized) return;
    list.dataset.virtualized = "1";
    list.addEventListener("scroll", () => {
      if (getState().order === "desc") {
        if (list.scrollTop > 0) parkTimelineRead();
        else if (isFollowingTimeline()) void catchUpTimeline();
      }
      renderTimeline();
    });
    if (typeof ResizeObserver !== "undefined")
      new ResizeObserver(scheduleTimelineRemeasure).observe(list);
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
    if (paintStart > 0) list.appendChild(spacer(paintStart * rowH));
    for (let i = paintStart; i < paintEnd; i++)
      list.appendChild(eventRow(rows[i - offset], selected));
    if (paintEnd < matchCount)
      list.appendChild(spacer((matchCount - paintEnd) * rowH));
    maybeExtendWindow(viewportH, start, end, offset, loadEnd, matchCount);
    if (!everMeasured && paintEnd > paintStart) remeasureTimelineRows();
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
    remeasureTimelineRows();
    const global = loadedWindow(getState()).offset + local;
    const centered = global * rowH - Math.max(0, (list.clientHeight - rowH) / 2);
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
      ...DIFF_ROUTE_CLEARED
    });
  }
  __name(navigateToTrack, "navigateToTrack");
  function navigateToActor(id) {
    const current = getState().filterText.trim();
    const short = id.replace(/^actor:/, "");
    const clause = /\s/.test(short) ? `actor:"${short}"` : `actor:${short}`;
    const already = parseSearchQueryFor(current, "event").clauses.some(
      (c) => c.kind === "field" && c.field === "actor" && c.value === id.toLowerCase() && !c.negate
    );
    navigate({
      lens: "timeline",
      filterText: already ? current : current ? `${current} ${clause}` : clause,
      ...DIFF_ROUTE_CLEARED
    });
  }
  __name(navigateToActor, "navigateToActor");
  async function revealEvent(eventId) {
    const page = await fetchRevealPage(eventId);
    if (!page?.present) return;
    parkTimelineRead();
    navigate({ ...revealPatch(page, eventId), ...DIFF_ROUTE_CLEARED });
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
          ...DIFF_ROUTE_CLEARED
        });
        break;
      case "track":
        navigateToTrack(id);
        break;
      case "actor":
        navigateToActor(id);
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

  // src/split.ts
  var MIN_PCT = 25;
  var MAX_PCT = 75;
  function currentPct(divider) {
    const aria = Number(divider.getAttribute("aria-valuenow"));
    if (Number.isFinite(aria) && aria >= MIN_PCT && aria <= MAX_PCT) return aria;
    return preferredSplit() ?? 50;
  }
  __name(currentPct, "currentPct");
  function setPct(divider, pct) {
    const clamped = pct === null ? null : Math.round(Math.min(MAX_PCT, Math.max(MIN_PCT, pct)));
    applySplit(clamped);
    divider.setAttribute("aria-valuenow", String(clamped ?? 50));
  }
  __name(setPct, "setPct");
  function stepPct(split) {
    const w = split.getBoundingClientRect().width;
    return w > 0 ? 24 / w * 100 : 3;
  }
  __name(stepPct, "stepPct");
  function initControls6() {
    const split = $(".split");
    const divider = $(".divider");
    if (!split || !divider) return;
    divider.setAttribute("aria-valuenow", String(preferredSplit() ?? 50));
    divider.addEventListener("pointerdown", (ev) => {
      ev.preventDefault();
      divider.focus();
      divider.setPointerCapture?.(ev.pointerId);
      divider.classList.add("dragging");
    });
    divider.addEventListener("pointermove", (ev) => {
      if (!divider.classList.contains("dragging")) return;
      const r = split.getBoundingClientRect();
      if (r.width <= 0) return;
      const pct = (ev.clientX - r.left) / r.width * 100;
      if (pct < MIN_PCT * 0.6) {
        divider.classList.remove("dragging");
        divider.releasePointerCapture?.(ev.pointerId);
        commit({ reading: true });
        return;
      }
      setPct(divider, pct);
    });
    divider.addEventListener("pointerup", (ev) => {
      divider.classList.remove("dragging");
      divider.releasePointerCapture?.(ev.pointerId);
    });
    divider.addEventListener("dblclick", () => setPct(divider, null));
    divider.addEventListener("keydown", (ev) => {
      if (ev.key === "ArrowLeft") {
        ev.preventDefault();
        ev.stopPropagation();
        const next = currentPct(divider) - stepPct(split);
        if (next < MIN_PCT) commit({ reading: true });
        else setPct(divider, next);
      } else if (ev.key === "ArrowRight") {
        ev.preventDefault();
        ev.stopPropagation();
        setPct(divider, currentPct(divider) + stepPct(split));
      } else if (ev.key === "Enter") {
        ev.preventDefault();
        ev.stopPropagation();
        setPct(divider, null);
      }
    });
  }
  __name(initControls6, "initControls");
  function stepSplit(dir) {
    if (!getState().open) return false;
    const split = $(".split");
    const divider = $(".divider");
    if (!split || !divider) return false;
    const reading = getState().reading;
    if (dir < 0) {
      if (reading) return false;
      const next = currentPct(divider) - stepPct(split);
      if (next < MIN_PCT) commit({ reading: true });
      else setPct(divider, next);
      return true;
    }
    if (reading) {
      commit({ reading: false });
      return true;
    }
    setPct(divider, currentPct(divider) + stepPct(split));
    return true;
  }
  __name(stepSplit, "stepSplit");

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
      const event = sel.id ? eventForId(sel.id) : void 0;
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
          ...DIFF_ROUTE_CLEARED
        }), "run")
      };
    }
    if (sel.kind === "event") {
      const event = sel.id ? eventForId(sel.id) : void 0;
      return {
        kind: "Current",
        label: "Open current selection",
        hint: event ? entryTitle(event) : shortRef(sel.id),
        run: /* @__PURE__ */ __name(() => navigate({
          selected: { kind: "event", id: sel.id },
          ...DIFF_ROUTE_CLEARED
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
      run: /* @__PURE__ */ __name(() => navigate({ lens: "timeline", ...DIFF_ROUTE_CLEARED }), "run")
    });
    cmds.push({
      kind: "Actions",
      label: "Switch to list lens",
      hint: "lens",
      run: /* @__PURE__ */ __name(() => navigate({ lens: "list", ...DIFF_ROUTE_CLEARED }), "run")
    });
    cmds.push({
      kind: "Actions",
      label: "Switch to attention lens",
      hint: "lens",
      run: /* @__PURE__ */ __name(() => navigate({ lens: "attention", ...DIFF_ROUTE_CLEARED }), "run")
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
      label: "Shrink timeline pane",
      hint: "split",
      run: /* @__PURE__ */ __name(() => {
        stepSplit(-1);
      }, "run")
    });
    cmds.push({
      kind: "Actions",
      label: "Grow timeline pane",
      hint: "split",
      run: /* @__PURE__ */ __name(() => {
        stepSplit(1);
      }, "run")
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
          ...DIFF_ROUTE_CLEARED
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
        run: /* @__PURE__ */ __name(() => navigate({ lens: "timeline", filterTrack: t, ...DIFF_ROUTE_CLEARED }), "run")
      });
    }
    for (const e of state2.history?.entries ?? []) {
      cmds.push({
        kind: "Events",
        label: entryTitle(e),
        hint: typeLabel(e.eventType),
        run: /* @__PURE__ */ __name(() => {
          parkTimelineRead();
          navigate({
            selected: { kind: "event", id: e.eventId ?? "" },
            ...DIFF_ROUTE_CLEARED
          });
        }, "run")
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
    const active2 = list.querySelector(".cmd-item.active");
    if (active2) {
      input.setAttribute("aria-activedescendant", active2.id);
      active2.scrollIntoView({ block: "nearest" });
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
  function initControls7() {
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
  __name(initControls7, "initControls");

  // src/keyboard.ts
  var lastTimelineViewportRows = 10;
  var lastRevisionViewportRows = 10;
  function isTypingTarget(el) {
    if (!el) return false;
    return el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el instanceof HTMLElement && el.isContentEditable;
  }
  __name(isTypingTarget, "isTypingTarget");
  function stepSelection(delta) {
    void stepSelectionAsync(delta);
  }
  __name(stepSelection, "stepSelection");
  function focusTimelineTabStop() {
    const state2 = getState();
    if (state2.lens !== "timeline" || state2.reading) return;
    $("#timeline")?.focus({ preventScroll: true });
  }
  __name(focusTimelineTabStop, "focusTimelineTabStop");
  function isTimelineSearchInput(target) {
    return target instanceof HTMLInputElement && target.id === "filter-text";
  }
  __name(isTimelineSearchInput, "isTimelineSearchInput");
  function focusTimelineAfterSearch() {
    const state2 = getState();
    if (state2.lens !== "timeline") navigate({ lens: "timeline" });
    if (state2.reading) commit({ reading: false });
    focusTimelineTabStop();
  }
  __name(focusTimelineAfterSearch, "focusTimelineAfterSearch");
  function timelineIsActive() {
    return getState().lens === "timeline";
  }
  __name(timelineIsActive, "timelineIsActive");
  function revisionLensIsActive() {
    return getState().lens === "list";
  }
  __name(revisionLensIsActive, "revisionLensIsActive");
  function attentionLensIsActive() {
    return getState().lens === "attention";
  }
  __name(attentionLensIsActive, "attentionLensIsActive");
  function setAttentionFocus(key) {
    commit({ attentionFocus: key });
    $("#master")?.querySelector(".attention-card.attention-focus")?.scrollIntoView({ block: "nearest" });
  }
  __name(setAttentionFocus, "setAttentionFocus");
  function stepAttention(delta) {
    const keys = attentionEntryKeys(getState());
    if (!keys.length) return;
    const current = getState().attentionFocus;
    let idx = current ? keys.indexOf(current) : -1;
    if (idx < 0) idx = delta > 0 ? -1 : 0;
    const next = Math.max(0, Math.min(keys.length - 1, idx + delta));
    setAttentionFocus(keys[next]);
  }
  __name(stepAttention, "stepAttention");
  function jumpAttentionBoundary(target) {
    const keys = attentionEntryKeys(getState());
    if (!keys.length) return;
    setAttentionFocus(target === "first" ? keys[0] : keys[keys.length - 1]);
  }
  __name(jumpAttentionBoundary, "jumpAttentionBoundary");
  function attentionViewportRows() {
    const el = $("#attention");
    const viewportH = el?.clientHeight ?? 0;
    const card = el?.querySelector(".attention-card");
    const itemH = card?.getBoundingClientRect().height ?? 0;
    const measured = viewportH > 0 && itemH > 0 ? Math.floor(viewportH / itemH) : 0;
    return Math.max(1, measured);
  }
  __name(attentionViewportRows, "attentionViewportRows");
  function activateAttentionFocus() {
    const revisionId = $("#master")?.querySelector(".attention-card.attention-focus")?.getAttribute("data-revision-id");
    if (revisionId)
      navigate({ selected: { kind: "revision", id: revisionId }, open: true });
  }
  __name(activateAttentionFocus, "activateAttentionFocus");
  function timelineViewportRows() {
    const list = $("#timeline");
    const viewportH = list?.clientHeight ?? 0;
    const rowH2 = timelineRowHeight();
    const measured = viewportH > 0 && rowH2 > 0 ? Math.floor(viewportH / rowH2) : 0;
    if (measured > 0) {
      lastTimelineViewportRows = Math.max(1, measured);
      return lastTimelineViewportRows;
    }
    const { count } = loadedWindow(getState());
    return Math.max(
      1,
      Math.min(count || lastTimelineViewportRows, lastTimelineViewportRows)
    );
  }
  __name(timelineViewportRows, "timelineViewportRows");
  function revisionLensViewportRows() {
    const list = $("#units");
    const item = list?.querySelector(".unit-card");
    const viewportH = list?.clientHeight ?? 0;
    const itemH = item?.getBoundingClientRect().height ?? 0;
    const measured = viewportH > 0 && itemH > 0 ? Math.floor(viewportH / itemH) : 0;
    if (measured > 0) {
      lastRevisionViewportRows = Math.max(1, measured);
      return lastRevisionViewportRows;
    }
    const count = lensEntryIds().length;
    return Math.max(
      1,
      Math.min(count || lastRevisionViewportRows, lastRevisionViewportRows)
    );
  }
  __name(revisionLensViewportRows, "revisionLensViewportRows");
  function loadedLensIndex(delta) {
    const ids = lensEntryIds();
    if (!ids.length) return null;
    let idx = ids.findIndex((x) => x.id === getState().selected.id);
    if (idx < 0) idx = delta > 0 ? -1 : 0;
    return Math.max(0, Math.min(ids.length - 1, idx + delta));
  }
  __name(loadedLensIndex, "loadedLensIndex");
  function selectLoadedLensIndex(index) {
    const ids = lensEntryIds();
    if (!ids.length) return;
    const target = Math.max(0, Math.min(ids.length - 1, index));
    navigate({ selected: ids[target] }, { replace: true });
  }
  __name(selectLoadedLensIndex, "selectLoadedLensIndex");
  function stepList(delta) {
    const next = loadedLensIndex(delta);
    if (next !== null) selectLoadedLensIndex(next);
  }
  __name(stepList, "stepList");
  function jumpLoadedLensBoundary(target) {
    const ids = lensEntryIds();
    if (!ids.length) return;
    selectLoadedLensIndex(target === "first" ? 0 : ids.length - 1);
  }
  __name(jumpLoadedLensBoundary, "jumpLoadedLensBoundary");
  function pageLoadedLens(deltaRows) {
    const next = loadedLensIndex(deltaRows);
    if (next !== null) selectLoadedLensIndex(next);
  }
  __name(pageLoadedLens, "pageLoadedLens");
  async function stepTimeline(delta) {
    parkTimelineRead();
    const state2 = getState();
    const { offset, count, matchCount } = loadedWindow(state2);
    const ids = lensEntryIds();
    if (!ids.length || matchCount === 0) return;
    const local = ids.findIndex((x) => x.id === state2.selected.id);
    if (local < 0) {
      navigate({ selected: ids[0] }, { replace: true });
      focusTimelineTabStop();
      return;
    }
    const cur = offset + local;
    const target = Math.max(0, Math.min(matchCount - 1, cur + delta));
    if (target === cur) {
      focusTimelineTabStop();
      return;
    }
    if (target >= offset && target < offset + count) {
      navigate({ selected: ids[target - offset] }, { replace: true });
      focusTimelineTabStop();
      return;
    }
    await fetchHistoryPage({
      offset: target >= offset + count ? offset + count : Math.max(0, offset - HISTORY_PAGE)
    });
    const w = loadedWindow(getState());
    const loaded = lensEntryIds();
    const localAfter = target - w.offset;
    if (localAfter >= 0 && localAfter < loaded.length) {
      navigate({ selected: loaded[localAfter] }, { replace: true });
      focusTimelineTabStop();
    }
  }
  __name(stepTimeline, "stepTimeline");
  function pageOffsetContaining(target) {
    return Math.floor(target / HISTORY_PAGE) * HISTORY_PAGE;
  }
  __name(pageOffsetContaining, "pageOffsetContaining");
  async function selectTimelineIndex(targetIndex) {
    parkTimelineRead();
    const state2 = getState();
    const { offset, count, matchCount } = loadedWindow(state2);
    const ids = lensEntryIds();
    if (!ids.length || matchCount === 0) return;
    const target = Math.max(0, Math.min(matchCount - 1, targetIndex));
    if (target >= offset && target < offset + count) {
      navigate({ selected: ids[target - offset] }, { replace: true });
      focusTimelineTabStop();
      return;
    }
    await fetchHistoryPage({ offset: pageOffsetContaining(target) });
    const w = loadedWindow(getState());
    const loaded = lensEntryIds();
    const localAfter = target - w.offset;
    if (localAfter >= 0 && localAfter < loaded.length) {
      navigate({ selected: loaded[localAfter] }, { replace: true });
      focusTimelineTabStop();
    }
  }
  __name(selectTimelineIndex, "selectTimelineIndex");
  async function jumpTimelineBoundary(target) {
    const { matchCount } = loadedWindow(getState());
    if (matchCount === 0) return;
    await selectTimelineIndex(target === "first" ? 0 : matchCount - 1);
  }
  __name(jumpTimelineBoundary, "jumpTimelineBoundary");
  async function pageTimeline(deltaRows) {
    const state2 = getState();
    const { offset, matchCount } = loadedWindow(state2);
    if (matchCount === 0) return;
    const ids = lensEntryIds();
    if (!ids.length) return;
    const local = ids.findIndex((x) => x.id === state2.selected.id);
    const cur = local < 0 ? offset : offset + local;
    await selectTimelineIndex(cur + deltaRows);
  }
  __name(pageTimeline, "pageTimeline");
  function jumpLensBoundary(target) {
    if (timelineIsActive()) void jumpTimelineBoundary(target);
    else if (revisionLensIsActive()) jumpLoadedLensBoundary(target);
    else if (attentionLensIsActive()) jumpAttentionBoundary(target);
  }
  __name(jumpLensBoundary, "jumpLensBoundary");
  function pageLensRows(deltaRows) {
    if (timelineIsActive()) {
      void pageTimeline(deltaRows);
      return;
    }
    if (revisionLensIsActive()) pageLoadedLens(deltaRows);
    else if (attentionLensIsActive()) stepAttention(deltaRows);
  }
  __name(pageLensRows, "pageLensRows");
  function pageLensFullPage(direction) {
    if (timelineIsActive()) {
      pageLensRows(direction * timelineViewportRows());
      return;
    }
    if (revisionLensIsActive()) {
      pageLensRows(direction * revisionLensViewportRows());
      return;
    }
    if (attentionLensIsActive()) {
      pageLensRows(direction * attentionViewportRows());
    }
  }
  __name(pageLensFullPage, "pageLensFullPage");
  function pageLensHalfPage(direction) {
    if (timelineIsActive()) {
      pageLensRows(
        direction * Math.max(1, Math.floor(timelineViewportRows() / 2))
      );
      return;
    }
    if (attentionLensIsActive()) {
      pageLensRows(
        direction * Math.max(1, Math.floor(attentionViewportRows() / 2))
      );
      return;
    }
    if (revisionLensIsActive()) {
      pageLensRows(
        direction * Math.max(1, Math.floor(revisionLensViewportRows() / 2))
      );
    }
  }
  __name(pageLensHalfPage, "pageLensHalfPage");
  async function stepSelectionAsync(delta) {
    if (attentionLensIsActive()) {
      stepAttention(delta);
      return;
    }
    if (getState().lens === "timeline") {
      await stepTimeline(delta);
      return;
    }
    stepList(delta);
  }
  __name(stepSelectionAsync, "stepSelectionAsync");
  function activateSelection() {
    if (attentionLensIsActive()) {
      activateAttentionFocus();
      return;
    }
    const sel = getState().selected;
    if (!getState().open) {
      if (!sel.id) return;
      navigate({ open: true });
      focusTimelineTabStop();
      return;
    }
    if (sel.kind === "revision" && sel.id) {
      openRevisionDiff(sel.id);
    } else if (sel.kind === "event" && sel.id) {
      const event = eventForId(sel.id);
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
    const active2 = document.activeElement;
    if (isTypingTarget(active2)) {
      if (active2 instanceof HTMLElement) active2.blur();
      return;
    }
    if (getState().reading) {
      commit({ reading: false });
      return;
    }
    if (getState().open) {
      navigate({ open: false });
      return;
    }
    if (getState().selected.id) {
      navigate({ selected: { kind: null, id: null } });
      return;
    }
    if (getState().filterText) navigate({ filterText: "" }, { replace: true });
  }
  __name(handleEscape, "handleEscape");
  function onKey(ev) {
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
    if (activeName() !== null) {
      handleOverlayKey(ev);
      return;
    }
    if (getState().diffPage) {
      if (ev.key === "Escape") {
        ev.preventDefault();
        closeDiff();
        return;
      }
      if (isTypingTarget(document.activeElement)) return;
      switch (ev.key) {
        case "]":
          ev.preventDefault();
          jumpChange(1);
          return;
        case "[":
          ev.preventDefault();
          jumpChange(-1);
          return;
        case "n":
          ev.preventDefault();
          jumpFact(1);
          return;
        case "p":
          ev.preventDefault();
          jumpFact(-1);
          return;
        default:
          return;
      }
    }
    if (ev.metaKey || ev.ctrlKey || ev.altKey) return;
    if (ev.key === "Escape") {
      handleEscape();
      return;
    }
    if (ev.key === "Enter" && isTimelineSearchInput(ev.target)) {
      ev.preventDefault();
      focusTimelineAfterSearch();
      return;
    }
    if (isTypingTarget(document.activeElement)) return;
    switch (ev.key) {
      case "1":
        ev.preventDefault();
        navigate({ lens: "timeline" });
        return;
      case "2":
        ev.preventDefault();
        navigate({ lens: "list" });
        return;
      case "3":
        ev.preventDefault();
        navigate({ lens: "attention" });
        return;
      case "g":
        if (timelineIsActive() || revisionLensIsActive() || attentionLensIsActive()) {
          ev.preventDefault();
          jumpLensBoundary("first");
        }
        return;
      case "G":
        if (timelineIsActive() || revisionLensIsActive() || attentionLensIsActive()) {
          ev.preventDefault();
          jumpLensBoundary("last");
        }
        return;
      case "/":
        ev.preventDefault();
        focusSearch();
        return;
      case "f":
        if (timelineIsActive() || revisionLensIsActive() || attentionLensIsActive()) {
          ev.preventDefault();
          pageLensFullPage(1);
        }
        return;
      case "b":
        if (timelineIsActive() || revisionLensIsActive() || attentionLensIsActive()) {
          ev.preventDefault();
          pageLensFullPage(-1);
        }
        return;
      case "d":
        if (timelineIsActive() || revisionLensIsActive() || attentionLensIsActive()) {
          ev.preventDefault();
          pageLensHalfPage(1);
        }
        return;
      case "u":
        if (timelineIsActive() || revisionLensIsActive() || attentionLensIsActive()) {
          ev.preventDefault();
          pageLensHalfPage(-1);
        }
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
      // h/l resize the split from anywhere (the divider's ArrowLeft/Right without
      // focusing it): h shrinks the timeline pane, l grows it. preventDefault only a
      // keystroke stepSplit consumed — so an inert h/l (pane closed, or h already at
      // the reading rail) still lets the browser's own type-ahead find fire.
      case "h":
        if (stepSplit(-1)) ev.preventDefault();
        return;
      case "l":
        if (stepSplit(1)) ev.preventDefault();
        return;
      case "Enter": {
        const t = ev.target;
        if (t instanceof Element && t.closest("a[href], button")) return;
        ev.preventDefault();
        activateSelection();
        return;
      }
      case " ": {
        const t = ev.target;
        if (t instanceof Element && t.closest("a[href], button")) return;
        if (!getState().open) return;
        const pane = $("#detail");
        if (!pane) return;
        ev.preventDefault();
        const page = pane.clientHeight > 0 ? pane.clientHeight * 0.85 : 400;
        pane.scrollTop += ev.shiftKey ? -page : page;
        return;
      }
      case "?":
        ev.preventDefault();
        toggleHelp();
        return;
      default:
        return;
    }
  }
  __name(onKey, "onKey");

  // src/chips.ts
  function filterChipsFor(filterText, surface) {
    const chips = [];
    tokenizeQuery(filterText).forEach((raw, tokenIndex) => {
      const clause = parseSearchQueryFor(raw, surface).clauses[0];
      if (clause && clause.kind === "field") {
        chips.push({
          tokenIndex,
          field: clause.field,
          value: clause.value,
          negate: clause.negate
        });
      }
    });
    return chips;
  }
  __name(filterChipsFor, "filterChipsFor");
  function removeFilterChipToken(filterText, tokenIndex) {
    const tokens = tokenizeQuery(filterText);
    tokens.splice(tokenIndex, 1);
    return tokens.join(" ");
  }
  __name(removeFilterChipToken, "removeFilterChipToken");

  // src/lenses/revisions.ts
  function renderRevisionList() {
    const el = $("#units");
    if (!el) return;
    const state2 = getState();
    const entries = orderedRevisionEntries(
      (state2.revisions?.entries ?? []).filter(matchesRevisionFilters),
      state2.order,
      state2.sortKey
    );
    if (!entries.length) {
      const filtered = Boolean(state2.filterText || state2.filterSnapshot);
      const genuinelyEmpty = !filtered && state2.revisions != null && (state2.revisions.entries ?? []).length === 0;
      el.innerHTML = `<p class="${CLASS.empty}" style="color:var(--fg-dim)">${filtered ? "No revisions match the current filters." : "No captured revisions in this store."}</p>${genuinelyEmpty ? renderWorkflowHandoff(firstReviewHandoff()) : ""}`;
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
      const snapshotUnavailable = revisionSnapshotUnavailable(u);
      const rows = [
        ["captured", fmtDateTime(u.capturedAt ?? "")],
        [
          "base",
          base.commitOid ? `${shortId(base.commitOid)} (${base.kind ?? ""})` : base.kind ?? "—"
        ]
      ];
      const tail = [
        ["revision", shortId(revisionId)],
        ["landing", u.mergeStatus || "unknown"],
        ["snapshot", shortId(u.snapshotId)]
      ];
      const targetCell = `<span>target</span><b>${targetDisplayLabel(u.targetDisplay)}${targetHeadBadge(u.targetDisplay)}</b>`;
      return `<div class="${CLASS.unitCard}" data-revision-id="${escapeHtml(revisionId)}"${isSelected ? ' aria-selected="true"' : ""} title="${escapeHtml(revisionId)}
click to open the revision page">
      <h3>${typeof u.summary === "string" && u.summary ? escapeHtml(u.summary) : workLabelText(u.targetDisplay)}</h3>
      ${badge ? `<div class="${CLASS.supersessionBadges}">${badge}</div>` : ""}
      ${renderRevisionOverview(u, overview)}
      <div class="${CLASS.kv} ${CLASS.tierMedium}">${rows.map(kv).join("")}${targetCell}${tail.map(kv).join("")}</div>
      <div class="${CLASS.actions}">${snapshotUnavailable ? `<button class="${CLASS.ghost} ${CLASS.diffBtn}" type="button" disabled title="captured snapshot content is unavailable">snapshot unavailable</button>` : `<button class="${CLASS.ghost} ${CLASS.diffBtn}" type="button" data-open-diff="${escapeHtml(u.snapshotId ?? "")}" data-diff-hash="${escapeHtml(u.snapshotContentHash ?? "")}">view snapshot diff</button>`}</div>
    </div>`;
    }).join("");
  }
  __name(renderRevisionList, "renderRevisionList");

  // src/render.ts
  var INSPECTOR_TITLE = "Pointbreak Review";
  var lastMasterLens = null;
  var lastSelectionScrollKey = null;
  function renderIdentity() {
    const root = $("#store-identity");
    if (!root) return;
    const id = getState().identity;
    if (!id) {
      root.classList.remove("hidden");
      const repoEl2 = $("#store-chip-repo");
      if (repoEl2) repoEl2.textContent = "local server";
      $("#store-chip")?.setAttribute("aria-label", "local review server");
      document.title = INSPECTOR_TITLE;
      return;
    }
    root.classList.remove("hidden");
    const rows = [
      ["repository", id.repository],
      ["store", id.placement.label]
    ];
    if (id.family) rows.push(["family", id.family.id]);
    if (id.worktree) rows.push(["worktree", id.worktree]);
    const rowsEl = $("#store-identity-rows");
    if (rowsEl) {
      rowsEl.innerHTML = rows.map(([k, v]) => `<dt>${escapeHtml(k)}</dt><dd>${escapeHtml(v)}</dd>`).join("");
    }
    const repoEl = $("#store-chip-repo");
    if (repoEl) repoEl.textContent = id.repository;
    $("#store-chip")?.setAttribute(
      "aria-label",
      rows.map(([k, v]) => `${k} ${v}`).join(", ")
    );
    document.title = `${id.repository} · ${INSPECTOR_TITLE}`;
  }
  __name(renderIdentity, "renderIdentity");
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
    const menu = $("#filter-types-menu");
    if (!container || !menu) return;
    const visible = getState().lens === "timeline";
    container.classList.toggle("hidden", !visible);
    if (!visible) return;
    menu.innerHTML = "";
    const counts = getState().history?.facets ?? {};
    const state2 = getState();
    const present = presentTypes();
    for (const id of present) {
      if (!state2.seenTypes.has(id)) {
        state2.seenTypes.add(id);
        state2.enabledTypes.add(id);
      }
      const enabled = state2.enabledTypes.has(id);
      const count = counts[id] ?? 0;
      const li = document.createElement("li");
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = typeFacetRowClass(enabled);
      btn.dataset.type = id;
      btn.setAttribute("aria-pressed", String(enabled));
      btn.setAttribute(
        "aria-label",
        `${enabled ? "Hide" : "Show"} ${typeLabel(id)} events (${count})`
      );
      btn.innerHTML = `<span class="${CLASS.dot}" style="background:${typeColor(id)}"></span>${escapeHtml(typeLabel(id))}<span class="${CLASS.typeCount}">${count}</span>`;
      btn.title = id;
      li.appendChild(btn);
      menu.appendChild(li);
    }
  }
  __name(renderTypeToggles, "renderTypeToggles");
  function renderLensSwitcher() {
    const lens = getState().lens;
    for (const tab of document.querySelectorAll(".lens-tab")) {
      tab.setAttribute("aria-pressed", String(tab.dataset.lens === lens));
      if (tab.dataset.lens === "attention") renderAttentionBadge(tab);
    }
  }
  __name(renderLensSwitcher, "renderLensSwitcher");
  function renderAttentionBadge(tab) {
    const { primary, secondary } = partitionAttentionTiers(
      getState().attention?.items ?? []
    );
    tab.querySelector(`.${CLASS.attentionBadge}`)?.remove();
    tab.querySelector(`.${CLASS.attentionDelta}`)?.remove();
    if (primary.length || secondary.length) {
      const badge = document.createElement("span");
      badge.className = CLASS.attentionBadge;
      const needsInput = primary.length === 1 ? "1 item needs input" : `${primary.length} items need input`;
      badge.setAttribute(
        "aria-label",
        [
          primary.length ? needsInput : "",
          secondary.length ? `${secondary.length} advisory` : ""
        ].filter(Boolean).join(", ")
      );
      badge.innerHTML = `${primary.length ? primary.length : ""}${secondary.length ? `<span class="${CLASS.attentionBadgeSecondary}">+${secondary.length}</span>` : ""}`;
      tab.appendChild(badge);
    }
    const delta = getState().attentionDelta;
    if (delta == null || delta === 0) return;
    const chip = document.createElement("span");
    chip.className = CLASS.attentionDelta;
    chip.textContent = `changed ${delta > 0 ? `+${delta}` : `−${Math.abs(delta)}`}`;
    chip.setAttribute("role", "status");
    tab.appendChild(chip);
  }
  __name(renderAttentionBadge, "renderAttentionBadge");
  function syncStreamPositionControls() {
    const state2 = getState();
    const follow = $("#follow-toggle");
    if (follow) {
      follow.classList.toggle(
        "hidden",
        state2.lens !== "timeline" || state2.order !== "desc"
      );
      follow.setAttribute("aria-pressed", String(state2.followByLens.timeline));
      follow.textContent = state2.followByLens.timeline ? "Following" : "Follow";
    }
  }
  __name(syncStreamPositionControls, "syncStreamPositionControls");
  function syncTimelineNewPill() {
    const state2 = getState();
    const pill = $("#timeline-new-pill");
    if (!pill) return;
    const visible = state2.order === "desc" && state2.followByLens.timeline && state2.timelineHeadAnchor != null && state2.timelineNewCount > 0;
    pill.classList.toggle("hidden", !visible);
    pill.textContent = `Show ${state2.timelineNewCount} new ${state2.timelineNewCount === 1 ? "event" : "events"}`;
  }
  __name(syncTimelineNewPill, "syncTimelineNewPill");
  function syncControls() {
    const state2 = getState();
    const text = $("#filter-text");
    if (text && text.value !== state2.filterText) text.value = state2.filterText;
    if (text)
      text.placeholder = `search — text or field:value (${keysFor2(state2.lens).map((k) => `${k}:`).join(" ")})`;
    const onAttention = state2.lens === "attention";
    const viewToggle = $("#view-toggle");
    if (viewToggle) {
      viewToggle.textContent = onAttention ? "View" : `View · ${state2.order === "desc" ? "newest" : "oldest"}`;
    }
    $("#view-order-section")?.classList.toggle("hidden", onAttention);
    const newest = $("#order-newest");
    const oldest = $("#order-oldest");
    if (newest) newest.checked = state2.order === "desc";
    if (oldest) oldest.checked = state2.order === "asc";
    const onList = state2.lens === "list";
    $("#view-sort-section")?.classList.toggle("hidden", !onList);
    const picker = $("#sort-picker");
    if (picker) {
      if (picker.value !== state2.sortKey) picker.value = state2.sortKey;
    }
    const toolbar = $("#toolbar");
    if (toolbar) toolbar.classList.toggle("hidden", onAttention);
  }
  __name(syncControls, "syncControls");
  function keysFor2(lens) {
    return lens === "list" ? REVISION_QUERY_FIELDS : EVENT_QUERY_FIELDS;
  }
  __name(keysFor2, "keysFor");
  function currentQuerySurface() {
    return getState().lens === "list" ? "revision" : "event";
  }
  __name(currentQuerySurface, "currentQuerySurface");
  function renderFilterChips() {
    const container = $("#filter-chips");
    if (!container) return;
    const chips = filterChipsFor(getState().filterText, currentQuerySurface());
    const rendered = chips.map((c) => {
      const value = c.field === "actor" ? c.value.replace(/^actor:/, "") : c.value;
      const label = `${escapeHtml(c.field)}:${escapeHtml(value)}`;
      return `<span class="${filterChipClass(c.negate)}" data-token-index="${c.tokenIndex}">${c.negate ? "− " : ""}${label}<button type="button" class="${CLASS.filterChipRemove}" data-token-index="${c.tokenIndex}" aria-label="remove ${label} filter">✕</button></span>`;
    });
    const state2 = getState();
    for (const [scope, value] of [
      ["track", state2.filterTrack],
      ["snapshot", state2.filterSnapshot]
    ]) {
      if (!value) continue;
      const label = `${escapeHtml(scope)}:${escapeHtml(value)}`;
      rendered.push(
        `<span class="${filterChipClass(false)}" data-filter-scope="${scope}">${label}<button type="button" class="${CLASS.filterChipRemove}" data-filter-scope="${scope}" aria-label="remove ${label} filter">✕</button></span>`
      );
    }
    container.innerHTML = rendered.join("");
    $("#filter-chips-empty")?.classList.toggle("hidden", rendered.length > 0);
  }
  __name(renderFilterChips, "renderFilterChips");
  function syncFilterControls() {
    const state2 = getState();
    const structuredCount = filterChipsFor(
      state2.filterText,
      currentQuerySurface()
    ).length;
    const scopedCount = Number(Boolean(state2.filterTrack)) + Number(Boolean(state2.filterSnapshot));
    const present = presentTypes();
    const typesFiltered = state2.lens === "timeline" && present.some((type) => !state2.enabledTypes.has(type));
    const count = structuredCount + scopedCount + Number(typesFiltered);
    const toggle2 = $("#filters-toggle");
    if (toggle2) {
      toggle2.textContent = count > 0 ? `Filters · ${count}` : "Filters";
      toggle2.setAttribute(
        "aria-label",
        count > 0 ? `Filters — ${count} active` : "Filters — none active"
      );
    }
    const clearable = Boolean(state2.filterText.trim()) || Boolean(state2.filterTrack) || Boolean(state2.filterSnapshot) || typesFiltered;
    $("#filter-footer")?.classList.toggle("hidden", !clearable);
  }
  __name(syncFilterControls, "syncFilterControls");
  var lastQueryNotice = "";
  function syncQueryNotices() {
    const el = $("#route-diagnostic");
    if (!el) return;
    const state2 = getState();
    const parsed = parseSearchQueryFor(state2.filterText, currentQuerySurface());
    const server = state2.history?.queryNotices ?? [];
    const message = dedupeNotices([...parsed.diagnostics, ...server]).map((d) => d.message).join(" · ");
    const current = el.classList.contains("hidden") ? "" : el.textContent ?? "";
    if (current !== "" && current !== lastQueryNotice) return;
    if (message) {
      showRouteDiagnostic(message);
      lastQueryNotice = message;
    } else if (lastQueryNotice) {
      clearRouteDiagnostic();
      lastQueryNotice = "";
    }
  }
  __name(syncQueryNotices, "syncQueryNotices");
  function dedupeNotices(notices) {
    const seen = /* @__PURE__ */ new Set();
    return notices.filter((n) => {
      const key = `${n.code}\0${n.key}\0${n.message}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  }
  __name(dedupeNotices, "dedupeNotices");
  function renderMaster() {
    const master = $("#master");
    if (!master) return;
    const lens = getState().lens;
    if (lens !== lastMasterLens) {
      lastMasterLens = lens;
      if (lens === "list") {
        master.innerHTML = `<div id="units" class="${CLASS.units}"></div>`;
      } else if (lens === "attention") {
        master.innerHTML = `<div id="attention" class="${CLASS.units}" aria-label="attention"></div>`;
      } else {
        master.innerHTML = `<div class="${CLASS.timelineShell}"><button id="timeline-new-pill" class="ghost ${CLASS.timelineNewPill} hidden" type="button" aria-live="polite">Show 0 new events</button><ol id="timeline" class="${CLASS.timeline}" aria-label="event timeline" tabindex="0"></ol></div>`;
      }
    }
    if (lens === "list") renderRevisionList();
    else if (lens === "attention") renderAttention();
    else {
      syncTimelineNewPill();
      renderTimeline();
    }
  }
  __name(renderMaster, "renderMaster");
  function applySplitMode() {
    const split = $(".split");
    if (!split) return;
    const s = getState();
    split.classList.toggle("split-closed", !s.open);
    const reading = s.reading && s.open;
    split.classList.toggle("reading", reading);
    const readBtn = $("#detail-read");
    if (readBtn) {
      readBtn.textContent = reading ? "⤡" : "⤢";
      const label = reading ? "Restore split" : "Reading mode";
      readBtn.setAttribute("aria-label", label);
      readBtn.setAttribute("title", label);
    }
  }
  __name(applySplitMode, "applySplitMode");
  function renderSelected() {
    if (!getState().open) return;
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
  function selectionScrollKey() {
    const state2 = getState();
    const selected = state2.selected;
    if (!selected.id) return `${state2.lens}:none`;
    let present = false;
    if (selected.kind === "event") {
      present = (state2.history?.entries ?? []).some(
        (entry) => entry.eventId === selected.id
      );
    } else if (selected.kind === "revision") {
      present = (state2.revisions?.entries ?? []).some(
        (revision) => revision.revisionId === selected.id
      );
    }
    return `${state2.lens}:${selected.kind}:${selected.id}:${present ? "present" : "absent"}`;
  }
  __name(selectionScrollKey, "selectionScrollKey");
  function scrollChangedSelectionIntoView() {
    const key = selectionScrollKey();
    if (key === lastSelectionScrollKey) return;
    lastSelectionScrollKey = key;
    scrollSelectionIntoView();
  }
  __name(scrollChangedSelectionIntoView, "scrollChangedSelectionIntoView");
  function applyDiffPageMode() {
    const onPage = getState().diffPage;
    $("#diff-page")?.classList.toggle("hidden", !onPage);
    for (const sel of [
      "#toolbar",
      "#master",
      "#detail",
      "#master-rail",
      ".divider"
    ]) {
      $(sel)?.classList.toggle("hidden", onPage);
    }
    return onPage;
  }
  __name(applyDiffPageMode, "applyDiffPageMode");
  function render() {
    renderIdentity();
    renderStats();
    renderDiagnostics();
    renderLensSwitcher();
    syncStreamPositionControls();
    if (applyDiffPageMode()) {
      void renderDiffPage();
      return;
    }
    syncControls();
    syncQueryNotices();
    renderFilterChips();
    renderTypeToggles();
    syncFilterControls();
    applySplitMode();
    renderMaster();
    renderSelected();
    scrollChangedSelectionIntoView();
    void renderDiffPage();
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
  function onFilterChipsClick(ev) {
    const t = ev.target;
    if (!(t instanceof Element)) return;
    const btn = t.closest(`.${CLASS.filterChipRemove}`);
    const scope = btn?.dataset.filterScope;
    if (scope === "track") {
      navigate({ filterTrack: "" }, { replace: true });
      return;
    }
    if (scope === "snapshot") {
      navigate({ filterSnapshot: "" }, { replace: true });
      return;
    }
    const indexAttr = btn?.dataset.tokenIndex;
    if (indexAttr == null) return;
    const next = removeFilterChipToken(getState().filterText, Number(indexAttr));
    navigate({ filterText: next }, { replace: true });
  }
  __name(onFilterChipsClick, "onFilterChipsClick");
  function onMasterClick(ev) {
    const t = ev.target;
    if (!(t instanceof Element)) return;
    if (t.closest("#timeline-new-pill")) {
      void catchUpTimeline();
      return;
    }
    if (t.closest("[data-ref-kind]")) return;
    const workflowCopyBtn = t.closest(
      "[data-copy-workflow-command]"
    );
    if (workflowCopyBtn) {
      void copyWorkflowCommand(workflowCopyBtn);
      return;
    }
    if (t.closest("[data-workflow-handoff]")) return;
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
      if (id) {
        parkTimelineRead();
        navigate({ selected: { kind: "event", id }, open: true });
      }
      return;
    }
    const revEl = t.closest(".unit-card[data-revision-id]");
    if (revEl) {
      const id = revEl.dataset.revisionId;
      if (id) navigate({ selected: { kind: "revision", id }, open: true });
    }
  }
  __name(onMasterClick, "onMasterClick");
  function initControls8() {
    $("#master")?.addEventListener("click", onMasterClick);
    $("#filter-types")?.addEventListener("click", onTypeToggleClick);
    $("#filter-chips")?.addEventListener(
      "click",
      onFilterChipsClick
    );
    $("#detail-close")?.addEventListener(
      "click",
      () => navigate({ open: false })
    );
    $("#detail-back")?.addEventListener(
      "click",
      () => navigate({ open: false })
    );
    $("#detail-read")?.addEventListener(
      "click",
      () => commit({ reading: !getState().reading })
    );
    $("#master-rail")?.addEventListener(
      "click",
      () => commit({ reading: false })
    );
  }
  __name(initControls8, "initControls");

  // src/main.ts
  var pollTimer = null;
  var unsubscribers = [];
  function stopPolling() {
    if (pollTimer !== null) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
    for (const unsubscribe of unsubscribers) unsubscribe();
    unsubscribers = [];
  }
  __name(stopPolling, "stopPolling");
  function startPolling() {
    setRefreshState("watching");
    if (pollTimer !== null) return;
    pollTimer = setInterval(() => {
      void pollFreshness();
    }, 3e3);
  }
  __name(startPolling, "startPolling");
  function boundaryTarget(kind) {
    const state2 = getState();
    if (state2.lens === "attention") return kind === "latest" ? "last" : "first";
    const latestIsFirst = state2.order === "desc";
    return kind === "latest" === latestIsFirst ? "first" : "last";
  }
  __name(boundaryTarget, "boundaryTarget");
  function wireToolbar() {
    const viewDisclosure = createDisclosure({
      container: "#view-controls",
      trigger: "#view-toggle",
      panel: "#view-panel"
    });
    createDisclosure({
      container: "#filter-controls",
      trigger: "#filters-toggle",
      panel: "#filters-panel"
    });
    for (const tab of document.querySelectorAll(".lens-tab")) {
      tab.addEventListener("click", () => {
        const lens = tab.dataset.lens;
        navigate({
          lens: lens && LENSES.includes(lens) ? lens : DEFAULT_LENS,
          ...DIFF_ROUTE_CLEARED
        });
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
    $("#view-panel")?.addEventListener("change", (event) => {
      const input = event.target;
      if (!(input instanceof HTMLInputElement) || !input.checked) return;
      if (input.name === "view-order") {
        navigate(
          { order: input.value === "asc" ? "asc" : "desc" },
          { replace: true }
        );
      }
    });
    $("#sort-picker")?.addEventListener("change", (e) => {
      const value = e.target.value;
      navigate(
        { sortKey: value === "activity" ? "activity" : "captured" },
        { replace: true }
      );
    });
    $("#jump-latest")?.addEventListener("click", () => {
      jumpLensBoundary(boundaryTarget("latest"));
      viewDisclosure.close(true);
    });
    $("#jump-oldest")?.addEventListener("click", () => {
      jumpLensBoundary(boundaryTarget("oldest"));
      viewDisclosure.close(true);
    });
    $("#follow-toggle")?.addEventListener("click", () => {
      void toggleTimelineFollow();
    });
  }
  __name(wireToolbar, "wireToolbar");
  function main(options = {}) {
    stopPolling();
    const capability = bootstrapCapability();
    if (capability.token !== null) {
      (options.reload ?? (() => location.reload()))();
      return Promise.resolve();
    }
    applyPrefs();
    unsubscribers.push(subscribe(render));
    unsubscribers.push(subscribe(maybeReloadForQuery));
    initControls5();
    initControls2();
    initControls7();
    initControls4();
    initControls8();
    initControls3();
    initControls6();
    initControls();
    initConnectionControls();
    wireToolbar();
    document.addEventListener("keydown", onKey);
    document.addEventListener("click", onDocumentClick);
    window.addEventListener("popstate", applyHash);
    window.addEventListener("hashchange", applyHash);
    const coordinator = new AuthCoordinator({
      prompt: promptForCredential,
      navigate: /* @__PURE__ */ __name((url) => location.replace(url), "navigate"),
      currentOrigin: /* @__PURE__ */ __name(() => location.origin, "currentOrigin"),
      currentRoute: /* @__PURE__ */ __name(() => location.hash, "currentRoute")
    });
    installAuthCoordinator(coordinator);
    const retry = /* @__PURE__ */ __name(async () => {
      const [loaded] = await Promise.all([load(), loadIdentity()]);
      if (loaded) {
        applyHash();
        startPolling();
      }
    }, "retry");
    configureConnectionActions({
      retry,
      reconnect: /* @__PURE__ */ __name(async () => {
        if (await requestReconnect()) await retry();
      }, "reconnect")
    });
    render();
    renderConnectionChrome();
    return Promise.all([load(), loadIdentity()]).then(([loaded]) => {
      if (!loaded) return;
      applyHash();
      startPolling();
    });
  }
  __name(main, "main");

  // src/entry.ts
  void main();
})();
