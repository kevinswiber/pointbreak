// Shared constants and type definitions for the inspector front-end.
// Ported from the served app.js constants/types cluster. This module is the
// shared type-definition home; later modules import from it.

/** An event type's display metadata: stable id, short label, and CSS palette colour. */
export interface EventType {
  id: string;
  label: string;
  color: string;
}

// Event types in canonical (timeline) order. Colours are CSS var() references so
// the palette themes from one place rather than hard-coded hex here.
export const TYPES: readonly EventType[] = [
  { id: "review_initialized", label: "init", color: "var(--evt-init)" },
  { id: "work_object_proposed", label: "capture", color: "var(--evt-capture)" },
  {
    id: "review_observation_recorded",
    label: "observation",
    color: "var(--evt-observation)",
  },
  {
    id: "review_assessment_recorded",
    label: "assessment",
    color: "var(--evt-assessment)",
  },
  { id: "input_request_opened", label: "request", color: "var(--evt-request)" },
  {
    id: "input_request_responded",
    label: "response",
    color: "var(--evt-response)",
  },
  { id: "review_note_imported", label: "note", color: "var(--evt-note)" },
  {
    id: "validation_check_recorded",
    label: "validation",
    color: "var(--evt-validation)",
  },
];

/** Event types indexed by id; unknown ids resolve to `undefined`. */
export const TYPE_MAP: Record<string, EventType | undefined> =
  Object.fromEntries(TYPES.map((type): [string, EventType] => [type.id, type]));

/** Resolve an event type's palette colour, falling back to the note colour. */
export function typeColor(id: string): string {
  return TYPE_MAP[id]?.color ?? "var(--evt-note)";
}

/** Resolve an event type's label, falling back to the raw id. */
export function typeLabel(id: string): string {
  return TYPE_MAP[id]?.label ?? id;
}

// Advisory signature/endorsement readback labels (reader-relative, never a gate).
export const VERIFICATION_LABELS: Record<string, string> = {
  valid: "signature valid",
  invalid: "signature invalid",
  untrusted_key: "untrusted key",
  unsigned: "unsigned",
};

export const ENDORSEMENT_LABELS: Record<string, string> = {
  "endorsement-trusted": "trusted endorsement",
  unknown_endorser: "unknown endorser",
  ambiguous_endorser: "ambiguous endorser",
};

export const ASSESSMENT_LABELS: Record<string, string> = {
  accepted: "accepted",
  accepted_with_follow_up: "accepted-with-follow-up",
  needs_changes: "needs-changes",
  needs_clarification: "needs-clarification",
};

// The master-pane projections, serialized into the URL fragment by the router.
export const LENSES: readonly string[] = ["timeline", "list", "attention"];
export const DEFAULT_LENS = "timeline";

// The surface a query is parsed for — the per-surface key sets and value sets
// hang off this. Mirrors the Rust QuerySurface.
export type QuerySurface = "event" | "revision";

// One parse diagnostic: the code, the user-typed key it concerns, and a
// human-readable message. Mirrors the Rust QueryDiagnostic (kebab-case codes).
export interface QueryDiagnostic {
  code: "unsupported-qualifier" | "deprecated-qualifier" | "unsupported-value";
  key: string;
  message: string;
}

// The per-surface qualifier sets — the single authority every consumer (both
// client surfaces, autocomplete) resolves supported keys from. Parity-tested
// against the Rust arrays.
export const EVENT_QUERY_FIELDS: readonly string[] = [
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
  "after",
];
// `type:`/`check:` are event-only (known-but-unsupported here — diagnosed,
// never silent-empty); every other key matches a revision-index slot.
export const REVISION_QUERY_FIELDS: readonly string[] = [
  "track",
  "actor",
  "revision",
  "snapshot",
  "assessment",
  "is",
  "tag",
  "attention",
  "before",
  "after",
];
// Every key the grammar knows across surfaces plus the aliases — the
// known-but-unsupported distinction: a key here but not in a surface's set
// diagnoses instead of falling back to free text.
export const KNOWN_QUERY_KEYS: readonly string[] = [
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
  "object",
];
// The revision attention: value set — single source for the validator, the revision
// index builder, and autocomplete. Exactly the projection.ts attentionTokens tokens.
export const REVISION_ATTENTION_VALUES: readonly string[] = [
  "open-request",
  "unassessed",
  "validation-context",
  "follow-up",
  "stale-fact",
];
// Legacy alias: the exported event field set. Existing importers keep working.
export const QUERY_FIELDS: readonly string[] = EVENT_QUERY_FIELDS;

// How many diff file bodies render eagerly; the rest stay collapsed until opened.
export const DEFAULT_OPEN_FILES = 10;
// A file body over this many rows is treated as large/generated and collapsed.
export const LARGE_FILE_ROWS = 500;

// Overlay name → root selector, for the single-overlay manager.
export const OVERLAY_SELECTORS: Record<string, string> = {
  palette: "#cmd-palette",
  help: "#key-help",
};

// Review facts whose currency depends on the revision they target: a fact on a
// superseded revision is stale (named by all superseding successors).
export const SUPERSEDABLE_FACT_TYPES: ReadonlySet<string> = new Set([
  "review_observation_recorded",
  "review_assessment_recorded",
  "input_request_opened",
  "validation_check_recorded",
]);

// ---------------------------------------------------------------------------
// Shared wire record shapes
//
// A view over the `/api/*` JSON the pure layer reads — not an exhaustive model
// of the wire. Fields the renderers tolerate as absent are optional; deeper
// modules (query, diff-render, cards) reuse these rather than re-declaring them.
// ---------------------------------------------------------------------------

/** The captured base commit a revision was taken against. */
export interface EntryBase {
  commitOid?: string;
  // The base kind (e.g. `git_commit`), shown beside the short commit in a
  // revision card's base cell.
  kind?: string;
}

/** A file/line target a review fact addresses, when it has one. */
export interface FactTarget {
  filePath?: string;
  startLine?: number;
  endLine?: number;
}

/** Advisory attributes carried by a single endorsement attestation. */
export interface EndorserAttributes {
  kind?: string;
  roles?: string[];
}

/** One endorsement attestation (one per endorsing signer/key). */
export interface Endorsement {
  classification?: string;
  endorser?: string;
  endorserAttributes?: EndorserAttributes;
}

/** The typed, type-specific detail of a history entry. */
export interface EntrySummary {
  kind?: string;
  title?: string;
  body?: string;
  summary?: string;
  assessment?: string;
  outcome?: string;
  reasonCode?: string;
  base?: EntryBase;
  source?: EntrySource;
  target?: EntryTarget;
  checkName?: string;
  command?: string;
  status?: string;
  tags?: string[];
  confidence?: string;
  revisionId?: string;
  objectId?: string;
  engagementId?: string;
  objectArtifactContentHash?: string;
  observationId?: string;
  assessmentId?: string;
  inputRequestId?: string;
  inputRequestResponseId?: string;
  validationCheckId?: string;
  refAssociationId?: string;
  refWithdrawalId?: string;
  commitAssociationId?: string;
  commitWithdrawalId?: string;
  refName?: string;
  headOid?: string;
  commitOid?: string;
  treeOid?: string;
  // Event-authored fact-level supersession forward pointers, already on the
  // /api/history wire (skipped when empty): the observation ids this observation
  // supersedes, and the assessment ids this assessment replaces. Consumed by the
  // client-side fact-status reverse index (model.ts) for the advisory timeline pill.
  supersedes?: string[];
  respondsTo?: string[];
  replaces?: string[];
  relatedObservations?: string[];
  relatedInputRequests?: string[];
  // The content type and request mode the annotation gatherer reads when it
  // folds a fact into the review-annotation list.
  bodyContentType?: string;
  summaryContentType?: string;
  mode?: string;
  // The validation-check trigger and exit code surfaced in the event detail
  // table, and the input-request reason fields the detail body block falls back
  // to. Optional where the carrier tolerates absence (the detail renderer reads
  // a validation event's `trigger`/`exitCode`; `reason`/`reasonContentType` are
  // the third body fallback).
  trigger?: string;
  exitCode?: number;
  reason?: string;
  reasonContentType?: string;
  bodyByteSize?: number;
  bodyContentHash?: string;
  bodyContentState?: string;
  summaryByteSize?: number;
  summaryContentHash?: string;
  summaryContentState?: string;
  reasonByteSize?: number;
  reasonContentHash?: string;
  reasonContentState?: string;
  sourceFingerprint?: string;
  startedAt?: string;
  completedAt?: string;
  logArtifactContentHashes?: string[];
}

/** The capture source selector in a revision-captured history summary. */
export interface EntrySource {
  kind?: string;
  mode?: string;
  includeUntracked?: boolean;
  pathspecs?: string[];
}

/** A review endpoint or review/validation target in a history summary. */
export interface EntryTarget extends FactTarget {
  kind?: string;
  revisionId?: string;
  side?: string;
  observationId?: string;
  inputRequestId?: string;
  assessmentId?: string;
  eventId?: string;
  commitOid?: string;
  treeOid?: string;
  worktreeRoot?: string;
}

/** The actor (and producer) that wrote a history entry. */
export interface EntryWriter {
  actorId?: string;
}

/** The review subject a history entry addresses (keys on the revision). */
export interface EntrySubject {
  revisionId?: string;
}

/** The structured principal resolved client-side (ADR-0010 structured-first). */
export interface EntryPrincipal {
  status?: string;
  actorId?: string;
}

/** A single `/api/history` timeline entry. */
export interface HistoryEntry {
  eventType: string;
  eventId?: string;
  trackId?: string;
  writer?: EntryWriter;
  subject?: EntrySubject;
  principal?: EntryPrincipal;
  summary?: EntrySummary;
  // The timeline reads the occurrence timestamp (rendered as a short time) and the
  // advisory signature-status chip off each entry.
  occurredAt?: string;
  verificationStatus?: string;
  // The payload hash shown in the event detail table, and the advisory
  // endorsement readback the detail readback region renders (optional where the
  // carrier tolerates absence — an unsigned event carries no endorsements).
  payloadHash?: string;
  endorsements?: Endorsement[];
  // Computed once per load (not a wire field): the search record the state-bound
  // timeline filter matches against, so a keystroke never re-serializes the event.
  __search?: SearchIndex;
}

/** A revision's current-assessment rollup. */
export interface CurrentAssessment {
  status?: string;
  assessment?: string;
}

/** A revision's attention rollup (open requests, validation context, etc.). */
export interface OverviewAttention {
  openInputRequestCount?: number;
  // Requests with a resolved response only — the `is:answered` source.
  // Ambiguous requests count in neither this nor the open count.
  respondedInputRequestCount?: number;
  unassessed?: boolean;
  failedValidationCount?: number;
  erroredValidationCount?: number;
  acceptedWithFollowUp?: boolean;
  staleFactCount?: number;
}

/** A revision's fact/diff counts. */
export interface OverviewCounts {
  files?: number;
  rows?: number;
  observations?: number;
  inputRequests?: number;
  assessments?: number;
  validationChecks?: number;
}

/** The most recent activity recorded against a revision. */
export interface LatestActivity {
  title?: string;
  kind?: string;
  at?: string;
}

/** The server-computed review overview for one revision. */
export interface Overview {
  currentAssessment?: CurrentAssessment;
  attention?: OverviewAttention;
  counts?: OverviewCounts;
  latestActivity?: LatestActivity;
  // The per-revision fact-meta aggregation (track ids, writer actor ids,
  // observation tags across the four fact families) the revision search
  // index folds into its token sets.
  tracks?: string[];
  actors?: string[];
  tags?: string[];
}

/**
 * A once-per-load search record: a lowercased haystack plus a small structured
 * projection the query grammar matches by field. `text`/`type` are read by name;
 * the remaining grammar fields (track/revision/object/status/attention) are read
 * dynamically by the query matcher.
 */
export interface SearchIndex {
  text: string;
  type: string;
  [field: string]: string | undefined;
}
