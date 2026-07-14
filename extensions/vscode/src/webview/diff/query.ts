// Structured query grammar: the tokenizer, the surface-aware field:value +
// free-text parser with diagnostics, and the per-kind field/predicate matchers,
// still used by the revisions lens. Mirrors the Rust query grammar: every match
// kind and key set lands in both arms with mirrored test cases. All pure
// (argument-driven, no DOM, no state).
//
// The timeline history query (its haystack, filter predicates, and facet counts)
// moved to the server; the client no longer indexes or searches history entries.
// The revisions lens keeps matching client-side over the fully-loaded list, so
// this grammar stays. Import direction stays downward.

import { parseMs } from "./format";
import {
  ASSESSMENT_LABELS,
  EVENT_QUERY_FIELDS,
  KNOWN_QUERY_KEYS,
  type QueryDiagnostic,
  type QuerySurface,
  REVISION_ATTENTION_VALUES,
  REVISION_QUERY_FIELDS,
  type SearchIndex,
  TYPES,
} from "./types";

// One import site for the grammar symbols: downstream consumers take the types
// from this module alongside the functions.
export type { QueryDiagnostic, QuerySurface } from "./types";

/** A parsed query clause: a field:value equality or a free-text term, negatable. */
export type QueryClause =
  | { kind: "field"; field: string; value: string; negate: boolean }
  | { kind: "text"; value: string; negate: boolean };

/** The surface-aware parse result: surviving clauses plus any diagnostics. */
export interface ParsedQuery {
  clauses: QueryClause[];
  diagnostics: QueryDiagnostic[];
}

// One canonical key across both runtimes and records: the Rust records store their
// normalized time under "occurred_at" too, and the revision index stores its
// normalized capturedAt under this key.
export const RANGE_ANCHOR_FIELD = "occurred_at";

// Normalize a minted time token to the canonical fixed-width ISO-8601 UTC form
// (YYYY-MM-DDTHH:MM:SS.mmmZ) for lexical range compare — the store mints BOTH
// unix-ms:<millis> and RFC 3339 Z strings, neither comparable lexically raw. A `Z`
// string goes through Date.parse; everything else (unix-ms:/bare millis) through
// parseMs. Date.toISOString() always emits the .mmm fraction, so the width is fixed.
export function normalizeTimeSlot(raw: unknown): string {
  if (typeof raw !== "string" || raw === "") return "";
  const epoch = /Z$/.test(raw) ? Date.parse(raw) : parseMs(raw);
  return epoch == null || Number.isNaN(epoch)
    ? ""
    : new Date(epoch).toISOString();
}

// Split a query into tokens, honoring "quoted phrases" (optionally negated /
// field-prefixed) and bare runs.
/** Tokenize a raw query string into its words and quoted phrases. */
export function tokenizeQuery(q: string): string[] {
  const out: string[] = [];
  const re = /-?(?:[a-z]+:)?"[^"]*"|\S+/gi;
  let m = re.exec(q);
  while (m !== null) {
    out.push(m[0]);
    m = re.exec(q);
  }
  return out;
}

const EVENT_VALUE_SETS: Record<string, readonly string[]> = {
  is: ["open", "answered"],
};
const REVISION_VALUE_SETS: Record<string, readonly string[]> = {
  is: [
    "open",
    "answered",
    "unassessed",
    "stale",
    "follow-up",
    "contested",
    "superseded",
  ],
  attention: REVISION_ATTENTION_VALUES,
};

/**
 * Parse a query for a surface. A supported qualifier becomes a field clause
 * (aliases applied); a known-but-unsupported one, or an out-of-set `is:`/
 * `attention:` value, yields a diagnostic and drops the clause (never a
 * silent-empty match); an unknown key stays free text.
 */
export function parseSearchQueryFor(
  q: string,
  surface: QuerySurface,
): ParsedQuery {
  const fields =
    surface === "revision" ? REVISION_QUERY_FIELDS : EVENT_QUERY_FIELDS;
  const valueSets =
    surface === "revision" ? REVISION_VALUE_SETS : EVENT_VALUE_SETS;
  const clauses: QueryClause[] = [];
  const diagnostics: QueryDiagnostic[] = [];
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
    const value = tok
      .slice(colon + 1)
      .replace(/^"|"$/g, "")
      .toLowerCase();
    const [field, deprecatedFrom] = resolveAlias(key, surface);
    if (fields.includes(field)) {
      const allowed = valueSets[field];
      if (allowed && !allowed.includes(value)) {
        diagnostics.push({
          code: "unsupported-value",
          key: field,
          message: `\`${field}:${value}\` — expected one of: ${allowed.join(", ")}`,
        });
        continue;
      }
      if (deprecatedFrom)
        diagnostics.push({
          code: "deprecated-qualifier",
          key: deprecatedFrom,
          message: `\`${deprecatedFrom}:\` is deprecated; use \`${field}:\``,
        });
      clauses.push({
        kind: "field",
        field,
        value: canonicalizeFieldValue(field, value),
        negate,
      });
    } else if (KNOWN_QUERY_KEYS.includes(key)) {
      diagnostics.push({
        code: "unsupported-qualifier",
        key,
        message: `\`${key}:\` is not a filter on the ${surface === "revision" ? "revisions" : "timeline"} view`,
      });
    } else {
      pushText(clauses, tok, negate);
    }
  }
  return { clauses, diagnostics };
}

/** The legacy event-surface parse — delegates, discarding diagnostics. */
export function parseSearchQuery(q: string): QueryClause[] {
  return parseSearchQueryFor(q, "event").clauses;
}

// Canonicalize a qualifier value after alias resolution. Actor ids come in two
// shapes: the actor: scheme or a self-certifying Ed25519 did:key. The actor:
// qualifier accepts a scheme id with or without its prefix (actor:agent:codex ≡
// actor:actor:agent:codex) and canonicalizes to the prefixed form the record
// stores; a did:key value is already fully qualified and passes through
// untouched. Mirrors search.rs `canonicalize_field_value`.
function canonicalizeFieldValue(field: string, value: string): string {
  if (
    field === "actor" &&
    value &&
    !value.startsWith("actor:") &&
    !value.startsWith("did:key:")
  )
    return `actor:${value}`;
  return value;
}

function pushText(clauses: QueryClause[], tok: string, negate: boolean): void {
  const term = tok.replace(/^"|"$/g, "").toLowerCase();
  if (term) clauses.push({ kind: "text", value: term, negate });
}

// Rewrite a user-typed key to its canonical field. `object:`→`snapshot:` is
// silent; `status:`→`check:`/`assessment:` carries a deprecation hint.
function resolveAlias(
  key: string,
  surface: QuerySurface,
): [string, string | null] {
  if (key === "object") return ["snapshot", null]; // silent alias
  if (key === "status")
    return [surface === "revision" ? "assessment" : "check", "status"];
  return [key, null];
}

/**
 * Match a single field clause against a search record, dispatching per key on
 * the resolved match kind (the value is already lowercased by the parser).
 * Set-membership fields store space-wrapped token lists; the range kinds compare
 * lexically over the normalized time anchor.
 */
export function fieldMatches(
  idx: SearchIndex,
  field: string,
  value: string,
): boolean {
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
      return (idx[field] || "").toLowerCase().includes(value); // substring
  }
}

// A full-seconds RFC 3339 bound, as the (already-lowercased) parser emits it.
// Deliberately as strict as the Rust instant parser so both runtimes normalize
// the same value set; anything else keeps raw prefix-compare semantics.
const RFC3339_BOUND = /^\d{4}-\d{2}-\d{2}t\d{2}:\d{2}:\d{2}(\.\d+)?z$/;

// The lexical bound a before:/after: value compares as. A full RFC 3339 instant
// normalizes to the anchor's fixed-width .mmm form (a whole-second bound would
// otherwise misorder against millisecond anchors — '.' sorts before 'z'); a
// date/datetime prefix passes through and keeps raw prefix-compare semantics.
function rangeBound(value: string): string {
  if (!RFC3339_BOUND.test(value)) return value;
  const upper = value.toUpperCase();
  const iso = normalizeTimeSlot(upper);
  // Round-trip guard: only a bound the normalizer re-emits verbatim (through
  // the seconds) counts as a recognized instant — Date.parse would otherwise
  // ROLL calendar-invalid inputs (2026-02-30, T24) that the strict Rust parser
  // rejects, so anything moved falls back to the raw compare on both runtimes.
  if (!iso || iso.slice(0, 19) !== upper.slice(0, 19)) return value;
  return iso.toLowerCase();
}

function matchKindFor(
  field: string,
): "exact" | "substring" | "set" | "range-before" | "range-after" {
  if (["type", "check", "assessment"].includes(field)) return "exact";
  if (["track", "actor", "is", "tag", "attention"].includes(field))
    return "set";
  if (field === "before") return "range-before";
  if (field === "after") return "range-after";
  return "substring"; // revision/snapshot + fallback
}

// Exact-match arm: `type` and `assessment` resolve label-or-wire (type also
// comma-OR); every other exact field compares the lowercased record value.
function exactMatches(idx: SearchIndex, field: string, value: string): boolean {
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

// Display label or wire value → wire (mirrors the Rust resolve_assessment_value);
// ASSESSMENT_LABELS is wire→label.
function resolveAssessmentValue(value: string): string {
  for (const [wire, label] of Object.entries(ASSESSMENT_LABELS)) {
    if (wire === value || label === value) return wire;
  }
  return value;
}

/** Match a search record against every clause (AND, honoring negation). */
export function matchesQuery(
  idx: SearchIndex,
  clauses: QueryClause[],
): boolean {
  for (const c of clauses) {
    const hit =
      c.kind === "field"
        ? fieldMatches(idx, c.field, c.value)
        : idx.text.includes(c.value);
    if (c.negate ? hit : !hit) return false;
  }
  return true;
}
