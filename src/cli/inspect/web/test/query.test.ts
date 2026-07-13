import { describe, expect, it } from "vitest";
import { entryRevisionId, entryTrack } from "../src/projection";
import {
  fieldMatches,
  matchesQuery,
  normalizeTimeSlot,
  parseSearchQuery,
  parseSearchQueryFor,
  tokenizeQuery,
} from "../src/query";
import type { HistoryEntry, SearchIndex } from "../src/types";
import historyJson from "./fixtures/history.json";

const history = historyJson as unknown as { entries: HistoryEntry[] };

function entryOfType(type: string): HistoryEntry {
  const found = history.entries.find((e) => e.eventType === type);
  if (!found) throw new Error(`no ${type} entry in fixture`);
  return found;
}

describe("tokenizeQuery", () => {
  it("splits on whitespace", () => {
    expect(tokenizeQuery("foo bar")).toEqual(["foo", "bar"]);
    expect(tokenizeQuery("")).toEqual([]);
  });

  it("keeps quoted phrases — bare, negated, and field-prefixed — intact", () => {
    expect(tokenizeQuery('type:observation "quoted phrase"')).toEqual([
      "type:observation",
      '"quoted phrase"',
    ]);
    expect(tokenizeQuery('-"neg phrase"')).toEqual(['-"neg phrase"']);
    expect(tokenizeQuery('track:"agent codex"')).toEqual([
      'track:"agent codex"',
    ]);
    expect(tokenizeQuery("-status:failed")).toEqual(["-status:failed"]);
  });
});

describe("parseSearchQuery", () => {
  it("parses a known field:value clause, lowercasing field and value", () => {
    expect(parseSearchQuery("TYPE:Observation")).toEqual([
      { kind: "field", field: "type", value: "observation", negate: false },
    ]);
  });

  it("marks a leading - as negation", () => {
    // `status:` aliases to `check:` on the event surface; negation rides along.
    expect(parseSearchQuery("-status:failed")).toEqual([
      { kind: "field", field: "check", value: "failed", negate: true },
    ]);
    expect(parseSearchQuery("-observed")).toEqual([
      { kind: "text", value: "observed", negate: true },
    ]);
  });

  it("strips quotes from field values and free-text phrases", () => {
    expect(parseSearchQuery('track:"agent:codex"')).toEqual([
      { kind: "field", field: "track", value: "agent:codex", negate: false },
    ]);
    expect(parseSearchQuery('"quoted phrase"')).toEqual([
      { kind: "text", value: "quoted phrase", negate: false },
    ]);
  });

  it("treats an unrecognized field as free text", () => {
    expect(parseSearchQuery("nope:value")).toEqual([
      { kind: "text", value: "nope:value", negate: false },
    ]);
  });

  it("splits bare terms into free-text clauses", () => {
    expect(parseSearchQuery("free text")).toEqual([
      { kind: "text", value: "free", negate: false },
      { kind: "text", value: "text", negate: false },
    ]);
    expect(parseSearchQuery("")).toEqual([]);
  });
});

describe("fieldMatches", () => {
  const idx: SearchIndex = {
    text: "haystack",
    type: "review_observation_recorded",
    track: " agent:codex ",
    revision: "rev:sha256:abcdef",
  };

  it("matches the type field by human label or raw id, exactly", () => {
    expect(fieldMatches(idx, "type", "observation")).toBe(true);
    expect(fieldMatches(idx, "type", "review_observation_recorded")).toBe(true);
    expect(fieldMatches(idx, "type", "assessment")).toBe(false);
  });

  it("matches non-type fields as a substring of the lowercased index", () => {
    // The index field is lowercased; the value is matched verbatim (the parser
    // lowercases values before they reach here), so short ids work.
    expect(fieldMatches(idx, "revision", "abcd")).toBe(true);
    // track is whole-token membership over the wrapped field, not substring.
    expect(fieldMatches(idx, "track", "agent:codex")).toBe(true);
    expect(fieldMatches(idx, "track", "codex")).toBe(false);
    expect(fieldMatches(idx, "revision", "zzz")).toBe(false);
  });

  it("treats a missing field as no match", () => {
    expect(fieldMatches(idx, "snapshot", "anything")).toBe(false);
  });
});

describe("fieldMatches match kinds (mirror of the Rust field_matches corpus)", () => {
  const idx: SearchIndex = {
    text: "",
    type: "review_observation_recorded",
    track: " agent:codex ",
    actor: " actor:agent:codex-loop ",
    is: " open answered ",
    tag: " issue issue:191 ",
    assessment: "needs_changes",
    occurred_at: "2026-05-13T10:00:01Z", // the shared anchor key
  };

  it("matches track and actor by whole token, not substring", () => {
    expect(fieldMatches(idx, "track", "agent:codex")).toBe(true);
    expect(fieldMatches(idx, "track", "codex")).toBe(false);
    expect(fieldMatches(idx, "actor", "actor:agent:codex-loop")).toBe(true);
    expect(fieldMatches(idx, "actor", "codex-loop")).toBe(false);
  });

  it("matches is and tag as set membership, not substring", () => {
    expect(fieldMatches(idx, "is", "open")).toBe(true);
    expect(fieldMatches(idx, "is", "pen")).toBe(false);
    expect(fieldMatches(idx, "tag", "issue:191")).toBe(true);
    expect(fieldMatches(idx, "tag", "issue")).toBe(true);
    expect(fieldMatches(idx, "tag", "issues")).toBe(false);
  });

  it("matches before/after lexically over the range anchor", () => {
    expect(fieldMatches(idx, "after", "2026-05-13t10:00:00z")).toBe(true);
    expect(fieldMatches(idx, "after", "2026-05-13t10:00:02z")).toBe(false);
    expect(fieldMatches(idx, "before", "2026-05-13t10:00:02z")).toBe(true);
    expect(fieldMatches(idx, "before", "2026-05-13t10:00:00z")).toBe(false);
  });

  it("normalizes full RFC 3339 range bounds before comparing", () => {
    // A whole-second RFC 3339 bound must compare against the fixed-width
    // millisecond anchor by instant, not raw bytes ('.' sorts before 'z').
    const later: SearchIndex = {
      text: "",
      type: "review_observation_recorded",
      occurred_at: "2026-05-13T10:00:01.500Z",
    };
    expect(fieldMatches(later, "before", "2026-05-13t10:00:01z")).toBe(false);
    expect(fieldMatches(later, "after", "2026-05-13t10:00:01z")).toBe(true);
    // An anchor equal to the bound matches neither strict range.
    const equal: SearchIndex = {
      text: "",
      type: "review_observation_recorded",
      occurred_at: "2026-05-13T10:00:01.000Z",
    };
    expect(fieldMatches(equal, "before", "2026-05-13t10:00:01z")).toBe(false);
    expect(fieldMatches(equal, "after", "2026-05-13t10:00:01z")).toBe(false);
    // A date/datetime prefix keeps raw lexical prefix-compare semantics.
    expect(fieldMatches(later, "before", "2026-05-14")).toBe(true);
    expect(fieldMatches(later, "after", "2026-05-13")).toBe(true);
    // A calendar-invalid instant never normalizes — it compares raw, matching
    // the strict Rust parser (Date.parse would roll 2026-02-30 into March).
    const march: SearchIndex = {
      text: "",
      type: "review_observation_recorded",
      occurred_at: "2026-03-01T00:00:00.000Z",
    };
    expect(fieldMatches(march, "before", "2026-02-30t00:00:00z")).toBe(false);
    // T24 rolls to the next day under Date.parse; it must fall back raw too
    // (raw "…t24…" sorts after any real clock time that day, same as Rust).
    const midMay: SearchIndex = {
      text: "",
      type: "review_observation_recorded",
      occurred_at: "2026-05-14T00:00:00.000Z",
    };
    expect(fieldMatches(midMay, "after", "2026-05-13t24:00:00z")).toBe(true);
  });

  it("matches type as a comma-list OR", () => {
    expect(fieldMatches(idx, "type", "observation,assessment")).toBe(true);
    expect(fieldMatches(idx, "type", "assessment,response")).toBe(false);
  });

  it("matches assessment by display label or wire value", () => {
    expect(fieldMatches(idx, "assessment", "needs-changes")).toBe(true);
    expect(fieldMatches(idx, "assessment", "needs_changes")).toBe(true);
    expect(fieldMatches(idx, "assessment", "accepted")).toBe(false);
  });
});

describe("fieldMatches range-anchor normalization", () => {
  it("normalizes both unix-ms and RFC 3339 inputs to the fixed-width .mmm form", () => {
    const iso = normalizeTimeSlot("unix-ms:1747130401250"); // ~ 2025-05-13, .250 frac
    expect(iso.startsWith("2025-") && iso.endsWith(".250Z")).toBe(true);
    // An RFC 3339 input round-trips to the canonical form.
    expect(normalizeTimeSlot("2026-05-13T10:00:01.500Z")).toBe(
      "2026-05-13T10:00:01.500Z",
    );
    // Same-second, different-millis order preserved (fractional boundary).
    expect(
      normalizeTimeSlot("unix-ms:1747130401000") <
        normalizeTimeSlot("unix-ms:1747130401250"),
    ).toBe(true);
    const dated: SearchIndex = {
      text: "",
      type: "review_observation_recorded",
      occurred_at: iso,
    };
    expect(fieldMatches(dated, "after", "2025-01-01")).toBe(true);
    expect(fieldMatches(dated, "before", "2027-01-01")).toBe(true);
  });
});

describe("parseSearchQueryFor (mirror of the Rust parse_search_query_for corpus)", () => {
  it("flags an unsupported qualifier, drops the clause, keeps free text", () => {
    const parsed = parseSearchQueryFor("attention:open free", "event");
    expect(parsed.clauses).toEqual([
      { kind: "text", value: "free", negate: false },
    ]);
    expect(parsed.diagnostics[0].code).toBe("unsupported-qualifier");
    expect(parsed.diagnostics[0].key).toBe("attention");
  });

  it("actor qualifier accepts the id with or without its prefix", () => {
    const expected = [
      {
        kind: "field",
        field: "actor",
        value: "actor:agent:codex",
        negate: false,
      },
    ];
    expect(parseSearchQueryFor("actor:agent:codex", "event").clauses).toEqual(
      expected,
    );
    expect(
      parseSearchQueryFor("actor:actor:agent:codex", "event").clauses,
    ).toEqual(expected);
  });

  it("actor qualifier preserves self-certifying did:key values", () => {
    const did = "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd";
    expect(parseSearchQueryFor(`actor:${did}`, "event").clauses).toEqual([
      {
        kind: "field",
        field: "actor",
        value: did.toLowerCase(),
        negate: false,
      },
    ]);
  });

  it("aliases status: per surface with a deprecation hint", () => {
    expect(parseSearchQueryFor("status:passed", "event").clauses).toEqual([
      { kind: "field", field: "check", value: "passed", negate: false },
    ]);
    expect(parseSearchQueryFor("status:accepted", "revision").clauses).toEqual([
      { kind: "field", field: "assessment", value: "accepted", negate: false },
    ]);
    expect(
      parseSearchQueryFor("status:passed", "event").diagnostics[0].code,
    ).toBe("deprecated-qualifier");
  });

  it("validates is: values per surface", () => {
    expect(
      parseSearchQueryFor("is:contested", "event").diagnostics[0].code,
    ).toBe("unsupported-value");
    // is: is deferred from the revision surface until its index slot exists —
    // a diagnostic, not a silently dropped match.
    expect(
      parseSearchQueryFor("is:contested", "revision").diagnostics[0].code,
    ).toBe("unsupported-qualifier");
    // The revision surface still validates enumerated values via attention:.
    expect(
      parseSearchQueryFor("attention:bogus", "revision").diagnostics[0].code,
    ).toBe("unsupported-value");
    expect(
      parseSearchQueryFor("attention:open-request", "revision").diagnostics,
    ).toEqual([]);
  });

  it("defers unindexed revision qualifiers with a diagnostic", () => {
    // track/actor/tag/is are deferred from the revision surface until the
    // revision index carries their slots — diagnosed, never silent-empty.
    for (const q of [
      "track:agent:codex",
      "actor:actor:local",
      "tag:issue",
      "is:open",
    ]) {
      const parsed = parseSearchQueryFor(q, "revision");
      expect(parsed.clauses).toEqual([]);
      expect(parsed.diagnostics[0].code).toBe("unsupported-qualifier");
    }
    // before:/after: stay supported — the revision index carries the anchor.
    const ranged = parseSearchQueryFor("after:2026-01-01", "revision");
    expect(ranged.clauses.length).toBe(1);
    expect(ranged.diagnostics).toEqual([]);
  });

  it("aliases object: to snapshot: silently", () => {
    const parsed = parseSearchQueryFor("object:obj-1", "event");
    expect(parsed.clauses).toEqual([
      { kind: "field", field: "snapshot", value: "obj-1", negate: false },
    ]);
    expect(parsed.diagnostics).toEqual([]); // silent alias, unchanged
  });

  it("keeps an unknown key as free text (regression)", () => {
    expect(parseSearchQueryFor("nope:value", "event").clauses).toEqual([
      { kind: "text", value: "nope:value", negate: false },
    ]);
  });

  it("parseSearchQuery delegates to the event surface", () => {
    expect(parseSearchQuery("status:passed")).toEqual([
      { kind: "field", field: "check", value: "passed", negate: false },
    ]);
  });
});

describe("matchesQuery", () => {
  const idx: SearchIndex = {
    text: "observed change in src/lib.rs",
    type: "review_observation_recorded",
    track: "agent:codex",
  };

  it("matches when there are no clauses", () => {
    expect(matchesQuery(idx, [])).toBe(true);
  });

  it("ANDs field and free-text clauses", () => {
    expect(
      matchesQuery(idx, parseSearchQuery("type:observation observed")),
    ).toBe(true);
    expect(
      matchesQuery(idx, parseSearchQuery("type:observation missing")),
    ).toBe(false);
  });

  it("honors negation", () => {
    expect(matchesQuery(idx, parseSearchQuery("-observed"))).toBe(false);
    expect(matchesQuery(idx, parseSearchQuery("-missing"))).toBe(true);
  });
});

describe("query grammar over a real entry's projected fields", () => {
  it("filters a fixture observation by its projected fields and free text", () => {
    // The haystack is now built server-side; the client grammar still matches the
    // projected fields (type/track/revision) and a lowercased free-text haystack.
    const obs = entryOfType("review_observation_recorded");
    // Set-membership fields (track) store the space-wrapped token encoding.
    const idx: SearchIndex = {
      text: "observed change in src/lib.rs",
      type: obs.eventType,
      track: ` ${entryTrack(obs)} `,
      revision: entryRevisionId(obs),
    };
    expect(matchesQuery(idx, parseSearchQuery("type:observation"))).toBe(true);
    expect(matchesQuery(idx, parseSearchQuery("track:agent:codex"))).toBe(true);
    expect(matchesQuery(idx, parseSearchQuery("track:codex"))).toBe(false);
    expect(matchesQuery(idx, parseSearchQuery("type:assessment"))).toBe(false);
    expect(matchesQuery(idx, parseSearchQuery("observed"))).toBe(true);
  });
});
