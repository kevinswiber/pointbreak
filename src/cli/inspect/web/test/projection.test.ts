import { describe, expect, it } from "vitest";
import {
  assessmentCue,
  assessmentDisplayLabel,
  assessmentLabel,
  attentionCues,
  attentionTokens,
  endorsementRow,
  endorsementsBlock,
  endorserDisplay,
  entryAnchor,
  entryRevisionId,
  entryTags,
  entryTitle,
  entryTrack,
  latestActivityLine,
  overviewStats,
  plural,
  principalLabel,
  type Revision,
  renderRevisionOverview,
  revisionSearchIndex,
  verificationChip,
} from "../src/projection";
import type { HistoryEntry } from "../src/types";
import historyJson from "./fixtures/history.json";
import revisionsJson from "./fixtures/revisions.json";

function parse(html: string): Document {
  return new DOMParser().parseFromString(html, "text/html");
}

const history = historyJson as unknown as { entries: HistoryEntry[] };
const revisions = revisionsJson as unknown as { entries: Revision[] };

function entryOfType(type: string): HistoryEntry {
  const found = history.entries.find((e) => e.eventType === type);
  if (!found) throw new Error(`no ${type} entry in fixture`);
  return found;
}

const revision = revisions.entries[0];

describe("entryTrack", () => {
  it("reads the explicit trackId when present", () => {
    expect(entryTrack(entryOfType("review_observation_recorded"))).toBe(
      "agent:codex",
    );
  });

  it("falls back to the writer actor id when there is no trackId", () => {
    expect(entryTrack(entryOfType("work_object_proposed"))).toBe(
      "actor:git-email:shore-tests@example.com",
    );
  });

  it("never reads the resolved principal — the lane fallback stays raw", () => {
    const entry: HistoryEntry = {
      eventType: "review_observation_recorded",
      principal: { status: "resolved", actorId: "actor:git-email:p@b.com" },
    };
    expect(entryTrack(entry)).toBe("");
  });
});

describe("entryRevisionId", () => {
  it("reads the revision id off the subject", () => {
    const capture = entryOfType("work_object_proposed");
    expect(entryRevisionId(capture)).toBe(capture.subject?.revisionId);
    expect(entryRevisionId(capture)).toMatch(/^rev:sha256:/);
  });

  it("returns empty when the subject carries no revision", () => {
    expect(entryRevisionId({ eventType: "x", subject: {} })).toBe("");
    expect(entryRevisionId({ eventType: "x" })).toBe("");
  });
});

describe("principalLabel", () => {
  it("renders `<agent> (for <principal>)` for a resolved principal", () => {
    const entry: HistoryEntry = {
      eventType: "review_observation_recorded",
      writer: { actorId: "actor:agent:codex" },
      principal: {
        status: "resolved",
        actorId: "actor:git-email:kevin@swiber.dev",
      },
    };
    expect(principalLabel(entry)).toBe("codex (for kevin@swiber.dev)");
  });

  it("returns null when the principal is unresolved, absent, or has no actor", () => {
    expect(principalLabel({ eventType: "x" })).toBeNull();
    expect(
      principalLabel({ eventType: "x", principal: { status: "ambiguous" } }),
    ).toBeNull();
    expect(
      principalLabel({
        eventType: "x",
        principal: { status: "resolved" },
      }),
    ).toBeNull();
  });

  it("does not read the principal off a fixture entry that has none", () => {
    expect(
      principalLabel(entryOfType("review_assessment_recorded")),
    ).toBeNull();
  });
});

describe("verificationChip", () => {
  it("returns empty for a missing status", () => {
    expect(verificationChip("")).toBe("");
  });

  it("renders an advisory, reader-relative chip that never claims to gate", () => {
    const span = parse(verificationChip("unsigned")).querySelector(
      "span.verify",
    );
    expect(span?.classList.contains("verify-unsigned")).toBe(true);
    expect(span?.textContent).toBe("unsigned");
    const title = span?.getAttribute("title") ?? "";
    expect(title).toContain("reader-relative");
    expect(title).toContain("never gates a write");
  });

  it("maps a known status to its readback label", () => {
    const span = parse(verificationChip("valid")).querySelector("span.verify");
    expect(span?.classList.contains("verify-valid")).toBe(true);
    expect(span?.textContent).toBe("signature valid");
  });
});

describe("endorserDisplay", () => {
  it("strips the git-email/git-name actor namespace", () => {
    expect(endorserDisplay("actor:git-email:a@b.com")).toBe("a@b.com");
    expect(endorserDisplay("actor:git-name:Kevin Swiber")).toBe("Kevin Swiber");
  });

  it("leaves other actor ids untouched", () => {
    expect(endorserDisplay("agent:codex")).toBe("agent:codex");
  });
});

describe("endorsementRow", () => {
  it("renders the label, endorser, and attributes for a full row", () => {
    const li = parse(
      endorsementRow({
        classification: "endorsement-trusted",
        endorser: "actor:git-email:a@b.com",
        endorserAttributes: {
          kind: "maintainer",
          roles: ["reviewer", "owner"],
        },
      }),
    ).querySelector("li.endorse");
    expect(li?.classList.contains("endorse-endorsement-trusted")).toBe(true);
    expect(li?.querySelector(".endorse-label")?.textContent).toBe(
      "trusted endorsement",
    );
    expect(li?.querySelector(".endorse-who")?.textContent).toBe("a@b.com");
    expect(li?.querySelector(".endorse-attrs")?.textContent).toBe(
      "maintainer · reviewer, owner",
    );
  });

  it("omits the who/attrs spans when only a classification is present", () => {
    const li = parse(
      endorsementRow({ classification: "unknown_endorser" }),
    ).querySelector("li.endorse");
    expect(li?.querySelector(".endorse-label")?.textContent).toBe(
      "unknown endorser",
    );
    expect(li?.querySelector(".endorse-who")).toBeNull();
    expect(li?.querySelector(".endorse-attrs")).toBeNull();
  });
});

describe("endorsementsBlock", () => {
  it("returns empty for no endorsements", () => {
    expect(endorsementsBlock([])).toBe("");
  });

  it("renders an advisory readback block with one row per endorsement", () => {
    const doc = parse(
      endorsementsBlock([
        {
          classification: "endorsement-trusted",
          endorser: "actor:git-name:Kev",
        },
        { classification: "unknown_endorser" },
      ]),
    );
    const block = doc.querySelector("div.endorsements");
    expect(block?.querySelector(".endorsements-label")?.textContent).toBe(
      "endorsements",
    );
    const title = block?.getAttribute("title") ?? "";
    expect(title).toContain("reader-relative");
    expect(title).toContain("never gates a write");
    expect(doc.querySelectorAll("ul.endorse-list li.endorse")).toHaveLength(2);
  });
});

describe("assessmentDisplayLabel", () => {
  it("maps the known assessment values to their hyphenated labels", () => {
    expect(assessmentDisplayLabel("accepted_with_follow_up")).toBe(
      "accepted-with-follow-up",
    );
    expect(assessmentDisplayLabel("needs_changes")).toBe("needs-changes");
    expect(assessmentDisplayLabel("accepted")).toBe("accepted");
  });

  it("passes through unknown or empty values", () => {
    expect(assessmentDisplayLabel("weird")).toBe("weird");
    expect(assessmentDisplayLabel("")).toBe("");
  });
});

describe("entryTitle", () => {
  it("renders a capture with its short base commit, or bare capture", () => {
    expect(entryTitle(entryOfType("work_object_proposed"))).toBe(
      "capture · base ffc93defe117",
    );
    expect(entryTitle({ eventType: "work_object_proposed" })).toBe("capture");
  });

  it("renders a validation check as `name · status`", () => {
    expect(entryTitle(entryOfType("validation_check_recorded"))).toBe(
      "cargo test · passed",
    );
    expect(
      entryTitle({ eventType: "validation_check_recorded", summary: {} }),
    ).toBe("validation");
  });

  it("prefers the assessment display label for an assessment", () => {
    expect(entryTitle(entryOfType("review_assessment_recorded"))).toBe(
      "needs-changes",
    );
  });

  it("uses the summary title when present", () => {
    expect(entryTitle(entryOfType("review_observation_recorded"))).toBe(
      "Observed change",
    );
  });

  it("falls back through outcome, reasonCode, then the type label", () => {
    expect(entryTitle({ eventType: "x", summary: { outcome: "merged" } })).toBe(
      "merged",
    );
    expect(
      entryTitle({ eventType: "x", summary: { reasonCode: "needs_input" } }),
    ).toBe("needs_input");
    expect(entryTitle({ eventType: "review_initialized", summary: {} })).toBe(
      "init",
    );
  });
});

describe("entryTags", () => {
  it("returns the tags array, or empty when missing or not an array", () => {
    expect(
      entryTags({ eventType: "x", summary: { tags: ["a", "b"] } }),
    ).toEqual(["a", "b"]);
    expect(entryTags({ eventType: "x", summary: {} })).toEqual([]);
    expect(entryTags({ eventType: "x" })).toEqual([]);
  });
});

describe("entryAnchor", () => {
  it("renders filePath:start-end, filePath:start-start, or bare filePath", () => {
    expect(
      entryAnchor({
        eventType: "x",
        summary: {
          target: { filePath: "src/lib.rs", startLine: 5, endLine: 10 },
        },
      }),
    ).toBe("src/lib.rs:5-10");
    expect(
      entryAnchor({
        eventType: "x",
        summary: { target: { filePath: "src/lib.rs", startLine: 5 } },
      }),
    ).toBe("src/lib.rs:5-5");
    expect(
      entryAnchor({
        eventType: "x",
        summary: { target: { filePath: "a.rs" } },
      }),
    ).toBe("a.rs");
  });

  it("returns empty when there is no file target", () => {
    expect(entryAnchor({ eventType: "x", summary: {} })).toBe("");
    expect(entryAnchor({ eventType: "x" })).toBe("");
  });
});

describe("assessmentLabel", () => {
  it("spaces underscores and floors empty to empty", () => {
    expect(assessmentLabel("needs_changes")).toBe("needs changes");
    expect(assessmentLabel("accepted")).toBe("accepted");
    expect(assessmentLabel("")).toBe("");
  });
});

describe("assessmentCue", () => {
  it("renders the current assessment from the overview", () => {
    const cue = parse(assessmentCue(revision.overview)).querySelector(
      ".overview-assessment",
    );
    expect(cue?.textContent).toContain("current assessment");
    const status = cue?.querySelector(".fact-status");
    expect(status?.classList.contains("accepted")).toBe(true);
    expect(status?.textContent).toBe("accepted");
  });

  it("floors to unassessed, and labels ambiguous/resolved without an assessment", () => {
    expect(
      parse(assessmentCue(undefined)).querySelector(".fact-status")
        ?.textContent,
    ).toBe("unassessed");
    expect(
      parse(
        assessmentCue({ currentAssessment: { status: "ambiguous" } }),
      ).querySelector(".fact-status")?.textContent,
    ).toBe("ambiguous current assessment");
    expect(
      parse(
        assessmentCue({ currentAssessment: { status: "resolved" } }),
      ).querySelector(".fact-status")?.textContent,
    ).toBe("resolved");
  });
});

describe("plural", () => {
  it("pluralizes by count with an optional explicit plural", () => {
    expect(plural(1, "open request")).toBe("1 open request");
    expect(plural(2, "open request")).toBe("2 open requests");
    expect(plural(2, "validation context", "validation contexts")).toBe(
      "2 validation contexts",
    );
  });
});

describe("attentionTokens", () => {
  it("derives the open-request and validation-context cues from the fixture overview", () => {
    const tokens = attentionTokens(revision.overview);
    expect(tokens.map((t) => t.token)).toEqual([
      "open-request",
      "validation-context",
    ]);
    expect(tokens.map((t) => t.query)).toEqual([
      "attention:open-request",
      "attention:validation-context",
    ]);
    expect(tokens[0].label).toBe("1 open request");
    expect(tokens[1].label).toBe("1 validation context");
  });

  it("serializes every attention cue through an `attention:` query token", () => {
    const tokens = attentionTokens({
      attention: {
        openInputRequestCount: 2,
        unassessed: true,
        failedValidationCount: 1,
        erroredValidationCount: 2,
        acceptedWithFollowUp: true,
      },
    });
    expect(tokens.map((t) => t.query)).toEqual([
      "attention:open-request",
      "attention:unassessed",
      "attention:validation-context",
      "attention:follow-up",
    ]);
    expect(tokens[2].label).toBe("3 validation contexts");
  });

  it("returns no tokens for an overview with no attention", () => {
    expect(attentionTokens(undefined)).toEqual([]);
    expect(attentionTokens({ attention: {} })).toEqual([]);
  });

  it("surfaces a stale-fact cue when the overview reports superseded facts", () => {
    const tokens = attentionTokens({ attention: { staleFactCount: 2 } });
    expect(tokens.map((t) => t.token)).toEqual(["stale-fact"]);
    expect(tokens[0].query).toBe("attention:stale-fact");
    expect(tokens[0].label).toBe("2 stale facts");
  });

  it("omits the stale-fact cue when the count is zero or absent", () => {
    expect(attentionTokens({ attention: { staleFactCount: 0 } })).toEqual([]);
    expect(attentionTokens({ attention: {} })).toEqual([]);
  });
});

describe("attentionCues", () => {
  it("renders a muted placeholder when there are no cues", () => {
    const span = parse(attentionCues({})).querySelector(".overview-muted");
    expect(span?.textContent).toBe("no attention cues");
  });

  it("renders a filter button per cue carrying its attention query", () => {
    const buttons = parse(attentionCues(revision.overview)).querySelectorAll(
      "button.overview-cue",
    );
    expect(buttons).toHaveLength(2);
    expect(buttons[0].getAttribute("data-attention-query")).toBe(
      "attention:open-request",
    );
    expect(buttons[0].textContent).toBe("1 open request");
  });
});

describe("overviewStats", () => {
  it("sums the fact counts and surfaces files and rows", () => {
    const stats = parse(overviewStats(revision.overview)).querySelectorAll(
      ".overview-stat",
    );
    const text = Array.from(stats, (s) => s.textContent?.trim());
    expect(text).toEqual(["1 files", "17 rows", "6 facts"]);
  });

  it("floors missing counts to zero", () => {
    const text = Array.from(
      parse(overviewStats({})).querySelectorAll(".overview-stat b"),
      (b) => b.textContent,
    );
    expect(text).toEqual(["0", "0", "0"]);
  });
});

describe("latestActivityLine", () => {
  it("renders the latest activity title", () => {
    const line = parse(latestActivityLine(revision.overview)).querySelector(
      ".overview-latest",
    );
    expect(line?.textContent).toContain("latest");
    expect(line?.textContent).toContain("cargo clippy");
  });

  it("returns empty when there is no latest activity", () => {
    expect(latestActivityLine({})).toBe("");
    expect(latestActivityLine(undefined)).toBe("");
  });
});

describe("revisionSearchIndex", () => {
  it("projects a searchable record over the revision", () => {
    const idx = revisionSearchIndex(revision);
    expect(idx.type).toBe("revision");
    expect(idx.revision).toBe(revision.revisionId);
    expect(idx.object).toBe(revision.objectId);
    expect(idx.status).toBe("accepted");
    expect(idx.attention).toBe("open-request validation-context");
  });

  it("builds a lowercased haystack of the human-relevant fields", () => {
    const text = revisionSearchIndex(revision).text;
    for (const piece of [
      "accepted",
      "resolved",
      "validation",
      "cargo clippy",
      "ffc93de",
      "1 open request",
      "1 validation context",
      "review cues",
      "attention",
    ]) {
      expect(text).toContain(piece);
    }
    expect(text).toBe(text.toLowerCase());
  });
});

describe("renderRevisionOverview", () => {
  it("composes the assessment, stats, cues, and latest-activity blocks", () => {
    const doc = parse(renderRevisionOverview(revision));
    expect(doc.querySelector(".overview-assessment")).not.toBeNull();
    expect(doc.querySelector(".overview-stats")).not.toBeNull();
    expect(doc.querySelector(".overview-latest")).not.toBeNull();
    expect(
      doc.querySelector(".overview-cues .overview-label")?.textContent,
    ).toBe("review cues");
  });

  it("keeps the overview copy advisory, never gate-like", () => {
    const html = renderRevisionOverview(revision);
    for (const forbidden of ["blocking", "merge status", "required"]) {
      expect(html).not.toContain(forbidden);
    }
  });
});
