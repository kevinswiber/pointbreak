// The single source of truth for the class names the inspector emits. A pure leaf
// (no DOM, no state, imports nothing): every rendering module references `CLASS`
// for static tokens and the helpers below for the dynamic compositions, so a
// mistyped class string becomes a compile error instead of a silent CSS miss.
//
// Every constant and helper resolves to the exact string the hand-written emitter
// produced, so the emitted HTML is byte-identical. Sites that escape a server-
// supplied value keep `escapeHtml(...)` at the call site (e.g.
// `diffStatusClass(escapeHtml(f.status))`); the helpers compose only the class.
//
// `ALL_EMITTABLE_CLASSES` enumerates every class the module can emit (every `CLASS`
// value plus every dynamic-family member through its helper). The cross-artifact
// class-vs-CSS drift test reads it as the authoritative emitted-class set.

// ---------------------------------------------------------------------------
// Static tokens — one entry per unique static class emitted from a `class="…"`
// site. camelCase key, kebab-case value. Multi-class statics interpolate two
// entries at the call site (e.g. `${CLASS.badge} ${CLASS.stale}`).
// ---------------------------------------------------------------------------

export const CLASS = {
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
  cmdLabel: "cmd-label",
} as const;

// ---------------------------------------------------------------------------
// Dynamic-family vocabularies — the kinds/statuses the renderers compose into
// class names. Each `as const` array is the single source for both the derived
// type and the `ALL_EMITTABLE_CLASSES` enumeration, derived from its producer:
//
//   ANNO_KINDS          ← cards `factCard` + diff/render `renderAnnotation`
//   DIFF_ROW_KINDS      ← diff/render row kinds (drow-meta is a static token)
//   DIFF_FILE_STATUSES  ← diff file `status`
//   VERIFY_STATUSES     ← projection `verificationChip` (VERIFICATION_LABELS keys)
//   ENDORSE_CLASSES     ← projection `endorsementRow` (ENDORSEMENT_LABELS keys)
//   VERDICT_ASSESSMENTS ← cards `verdictBadge`
//   FACT_STATUSES       ← cards `factCard` + projection `assessmentCue`
//   REF_KINDS           ← refs `refInfo` + the `REF_RE` prefix alternation
//
// The status/kind values are server-supplied strings, so the helpers below take
// `string`; these arrays name the known value space the drift test enumerates.
// ---------------------------------------------------------------------------

export const ANNO_KINDS = [
  "observation",
  "assessment",
  "input-request",
  "validation",
] as const;

export const DIFF_ROW_KINDS = ["added", "removed", "context"] as const;

export const DIFF_FILE_STATUSES = [
  "added",
  "deleted",
  "modified",
  "renamed",
  "copied",
] as const;

export const VERIFY_STATUSES = [
  "valid",
  "invalid",
  "unsigned",
  "untrusted_key",
] as const;

export const ENDORSE_CLASSES = [
  "endorsement-trusted",
  "ambiguous_endorser",
  "unknown_endorser",
] as const;

export const VERDICT_ASSESSMENTS = [
  "accepted",
  "accepted_with_follow_up",
  "ambiguous",
  "needs_changes",
  "needs_clarification",
  "unassessed",
] as const;

// Spans every fact producer (observation/input-request/assessment/validation/
// adapter) plus the projection assessment cue, cross-checked against app.css
// `.fact-status.<x>` + `.replaced`. `resolved` is CSS-less (a PR2 allowlist
// entry). The runtime value is a server string; this is the known set.
export const FACT_STATUSES = [
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
  "unassessed",
] as const;

// The short prefixes `refInfo`/`REF_RE` classify a token into — not the long
// `revision`/`event`/`object` forms. Only `.ref-commit`/`.ref-hash` have a CSS
// rule; the rest use base `.ref` styling (PR2 allowlist).
export const REF_KINDS = [
  "review-unit",
  "input-request-response",
  "input-request",
  "obs",
  "assess",
  "snap",
  "rev",
  "evt",
  "note",
  "validation",
  "hash",
  "commit",
  "track",
] as const;

export type AnnoKind = (typeof ANNO_KINDS)[number];
export type DiffRowKind = (typeof DIFF_ROW_KINDS)[number];
export type DiffFileStatus = (typeof DIFF_FILE_STATUSES)[number];
export type VerifyStatus = (typeof VERIFY_STATUSES)[number];
export type EndorseClass = (typeof ENDORSE_CLASSES)[number];
export type VerdictAssessment = (typeof VERDICT_ASSESSMENTS)[number];
export type FactStatus = (typeof FACT_STATUSES)[number];
export type RefKind = (typeof REF_KINDS)[number];

// ---------------------------------------------------------------------------
// Typed helpers — each returns the exact legacy literal. The kind/status params
// take the data-model `string` (the values are server-supplied); the caller
// escapes where the original did (e.g. `diffStatusClass(escapeHtml(f.status))`).
// ---------------------------------------------------------------------------

/** `anno anno-<kind>` — the fact-card container class. */
export const annoContainerClass = (kind: string): string => `anno anno-${kind}`;

/** `anno-kind anno-kind-<kind>` — the fact-card kind chip class. */
export const annoKindClass = (kind: string): string =>
  `anno-kind anno-kind-${kind}`;

/** `drow drow-<kind>[ drow-noted]` — a diff row class (the noted gutter affordance). */
export const drowClass = (kind: string, noted: boolean): string =>
  `drow drow-${kind}${noted ? " drow-noted" : ""}`;

/** `dstatus s-<status>` — a diff file's status chip class. */
export const diffStatusClass = (status: string): string =>
  `dstatus s-${status}`;

/** `verify verify-<status>` — the advisory signature-readback chip class. */
export const verifyClass = (status: string): string =>
  `verify verify-${status}`;

/** `endorse endorse-<cls>` — an endorsement row class. */
export const endorseClass = (cls: string): string => `endorse endorse-${cls}`;

/** `verdict verdict-<assessment>` — the current-assessment verdict class. */
export const verdictClass = (assessment: string): string =>
  `verdict verdict-${assessment}`;

/** `fact-status <status>` — a fact's status chip class. */
export const factStatusClass = (status: string): string =>
  `fact-status ${status}`;

/** `ref ref-<kind>` — a reference chip class. */
export const refClass = (kind: string): string => `ref ref-${kind}`;

/** `dfile[ dfile-lowsignal]` — a diff file section class. */
export const dfileClass = (lowSignal: boolean): string =>
  `dfile${lowSignal ? " dfile-lowsignal" : ""}`;

/** `dag-node[ head][ superseded]` — a supersession-DAG node class. */
export const dagNodeClass = (o: {
  isHead: boolean;
  isSuperseded: boolean;
}): string =>
  `dag-node${o.isHead ? " head" : ""}${o.isSuperseded ? " superseded" : ""}`;

/** `<base>[ markdown-body]` — a rendered-body wrapper class. */
export const bodyClass = (
  base: "anno-body" | "verdict-summary",
  markdown: boolean,
): string => `${base}${markdown ? " markdown-body" : ""}`;

/** `cmd-item[ active]` — a command-palette option class. */
export const cmdItemClass = (active: boolean): string =>
  `cmd-item${active ? " active" : ""}`;

// ---------------------------------------------------------------------------
// The exhaustive emitted-class set: every `CLASS` value plus every dynamic-family
// member through its helper, split on spaces and deduped. The cross-artifact
// drift test reads this as the authoritative set the JS can emit.
// ---------------------------------------------------------------------------

const tokensOf = (classStrings: string[]): string[] =>
  classStrings.flatMap((s) => s.split(" "));

export const ALL_EMITTABLE_CLASSES: readonly string[] = [
  ...new Set(
    tokensOf([
      ...Object.values(CLASS),
      ...ANNO_KINDS.map((k) => annoContainerClass(k)),
      ...ANNO_KINDS.map((k) => annoKindClass(k)),
      ...DIFF_ROW_KINDS.map((k) => drowClass(k, true)),
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
      cmdItemClass(true),
    ]),
  ),
];
