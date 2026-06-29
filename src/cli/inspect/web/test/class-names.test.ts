import { describe, expect, test } from "vitest";
import {
  ALL_EMITTABLE_CLASSES,
  ANNO_KINDS,
  annoContainerClass,
  annoKindClass,
  bodyClass,
  CLASS,
  cmdItemClass,
  DIFF_FILE_STATUSES,
  DIFF_ROW_KINDS,
  dagNodeClass,
  dfileClass,
  diffStatusClass,
  drowClass,
  ENDORSE_CLASSES,
  endorseClass,
  FACT_STATUSES,
  factStatusClass,
  REF_KINDS,
  refClass,
  VERDICT_ASSESSMENTS,
  VERIFY_STATUSES,
  verdictClass,
  verifyClass,
} from "../src/classNames";

describe("CLASS static tokens", () => {
  test("a representative spread resolves to its exact legacy literal", () => {
    expect(CLASS.dfileHead).toBe("dfile-head");
    expect(CLASS.dpath).toBe("dpath");
    expect(CLASS.badge).toBe("badge");
    expect(CLASS.kv).toBe("kv");
    expect(CLASS.empty).toBe("empty");
    expect(CLASS.ghost).toBe("ghost");
    expect(CLASS.diffBtn).toBe("diff-btn");
    expect(CLASS.unitCard).toBe("unit-card");
    expect(CLASS.unitPage).toBe("unit-page");
    expect(CLASS.factStatus).toBe("fact-status");
    expect(CLASS.verdictSummary).toBe("verdict-summary");
    expect(CLASS.dagEdge).toBe("dag-edge");
    expect(CLASS.revisionDag).toBe("revision-dag");
    expect(CLASS.dagArrowHead).toBe("dag-arrow-head");
    expect(CLASS.dagArrowHeadTraced).toBe("dag-arrow-head-traced");
    expect(CLASS.drow).toBe("drow");
    expect(CLASS.drowMeta).toBe("drow-meta");
    expect(CLASS.competing).toBe("competing");
  });

  test("every CLASS value is a single, well-formed kebab-case class token", () => {
    for (const value of Object.values(CLASS)) {
      expect(value).toMatch(/^[a-z][a-z0-9-]*$/);
    }
  });

  test("CLASS keys and values are unique", () => {
    const values = Object.values(CLASS);
    expect(new Set(values).size).toBe(values.length);
  });
});

describe("dynamic helpers reproduce the exact legacy composition", () => {
  test("annotation kind/container helpers", () => {
    expect(annoContainerClass("observation")).toBe("anno anno-observation");
    expect(annoContainerClass("validation")).toBe("anno anno-validation");
    expect(annoKindClass("input-request")).toBe(
      "anno-kind anno-kind-input-request",
    );
    expect(annoKindClass("assessment")).toBe("anno-kind anno-kind-assessment");
  });

  test("diff row + file status helpers", () => {
    expect(drowClass("added", false)).toBe("drow drow-added");
    expect(drowClass("context", true)).toBe("drow drow-context drow-noted");
    expect(drowClass("removed", false)).toBe("drow drow-removed");
    expect(diffStatusClass("modified")).toBe("dstatus s-modified");
    expect(diffStatusClass("renamed")).toBe("dstatus s-renamed");
  });

  test("verify / endorse / verdict / fact-status helpers", () => {
    expect(verifyClass("valid")).toBe("verify verify-valid");
    expect(verifyClass("untrusted_key")).toBe("verify verify-untrusted_key");
    expect(endorseClass("endorsement-trusted")).toBe(
      "endorse endorse-endorsement-trusted",
    );
    expect(endorseClass("ambiguous_endorser")).toBe(
      "endorse endorse-ambiguous_endorser",
    );
    expect(verdictClass("needs_changes")).toBe("verdict verdict-needs_changes");
    expect(verdictClass("unassessed")).toBe("verdict verdict-unassessed");
    expect(factStatusClass("superseded")).toBe("fact-status superseded");
    expect(factStatusClass("passed")).toBe("fact-status passed");
  });

  test("ref helper uses the short-prefix kinds (rev, not revision)", () => {
    expect(refClass("rev")).toBe("ref ref-rev");
    expect(refClass("review-unit")).toBe("ref ref-review-unit");
    expect(refClass("commit")).toBe("ref ref-commit");
  });

  test("conditional/boolean helpers", () => {
    expect(dfileClass(true)).toBe("dfile dfile-lowsignal");
    expect(dfileClass(false)).toBe("dfile");
    expect(dagNodeClass({ isHead: true, isSuperseded: false })).toBe(
      "dag-node head",
    );
    expect(dagNodeClass({ isHead: false, isSuperseded: true })).toBe(
      "dag-node superseded",
    );
    expect(dagNodeClass({ isHead: false, isSuperseded: false })).toBe(
      "dag-node",
    );
    expect(bodyClass("anno-body", true)).toBe("anno-body markdown-body");
    expect(bodyClass("anno-body", false)).toBe("anno-body");
    expect(bodyClass("verdict-summary", true)).toBe(
      "verdict-summary markdown-body",
    );
    expect(cmdItemClass(true)).toBe("cmd-item active");
    expect(cmdItemClass(false)).toBe("cmd-item");
  });
});

describe("dynamic-family vocabulary arrays (derived from their producers)", () => {
  test("ANNO_KINDS", () => {
    expect([...ANNO_KINDS]).toEqual([
      "observation",
      "assessment",
      "input-request",
      "validation",
    ]);
  });

  test("DIFF_ROW_KINDS", () => {
    expect([...DIFF_ROW_KINDS]).toEqual(["added", "removed", "context"]);
  });

  test("DIFF_FILE_STATUSES", () => {
    expect([...DIFF_FILE_STATUSES]).toEqual([
      "added",
      "deleted",
      "modified",
      "renamed",
      "copied",
    ]);
  });

  test("VERIFY_STATUSES match VERIFICATION_LABELS keys", () => {
    expect([...VERIFY_STATUSES]).toEqual([
      "valid",
      "invalid",
      "unsigned",
      "untrusted_key",
    ]);
  });

  test("ENDORSE_CLASSES match ENDORSEMENT_LABELS keys (endorsement-trusted, not endorsement)", () => {
    expect([...ENDORSE_CLASSES]).toEqual([
      "endorsement-trusted",
      "ambiguous_endorser",
      "unknown_endorser",
    ]);
  });

  test("VERDICT_ASSESSMENTS (no verdict-needs; ambiguous + unassessed are real)", () => {
    expect([...VERDICT_ASSESSMENTS]).toEqual([
      "accepted",
      "accepted_with_follow_up",
      "ambiguous",
      "needs_changes",
      "needs_clarification",
      "unassessed",
    ]);
  });

  test("REF_KINDS are the refInfo/REF_RE short prefixes", () => {
    expect([...REF_KINDS]).toEqual([
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
    ]);
  });

  test("FACT_STATUSES includes replaced and resolved", () => {
    expect(FACT_STATUSES).toContain("replaced");
    expect(FACT_STATUSES).toContain("resolved");
    expect(FACT_STATUSES).toContain("passed");
    expect(FACT_STATUSES).toContain("failed");
    expect(FACT_STATUSES).toContain("unassessed");
  });
});

describe("ALL_EMITTABLE_CLASSES", () => {
  test("covers every CLASS value, split on space", () => {
    for (const value of Object.values(CLASS)) {
      for (const token of value.split(" ")) {
        expect(ALL_EMITTABLE_CLASSES).toContain(token);
      }
    }
  });

  test("covers every vocabulary member through its helper", () => {
    const families: [readonly string[], (member: string) => string][] = [
      [ANNO_KINDS, (k) => annoContainerClass(k)],
      [ANNO_KINDS, (k) => annoKindClass(k)],
      [DIFF_ROW_KINDS, (k) => drowClass(k, false)],
      [DIFF_FILE_STATUSES, (s) => diffStatusClass(s)],
      [VERIFY_STATUSES, (s) => verifyClass(s)],
      [ENDORSE_CLASSES, (c) => endorseClass(c)],
      [VERDICT_ASSESSMENTS, (a) => verdictClass(a)],
      [FACT_STATUSES, (s) => factStatusClass(s)],
      [REF_KINDS, (k) => refClass(k)],
    ];
    for (const [members, toClass] of families) {
      for (const member of members) {
        for (const token of toClass(member).split(" ")) {
          expect(ALL_EMITTABLE_CLASSES).toContain(token);
        }
      }
    }
  });

  test("covers the conditional/boolean family extras", () => {
    for (const token of [
      "drow-noted",
      "dfile-lowsignal",
      "head",
      "superseded",
      "markdown-body",
      "active",
    ]) {
      expect(ALL_EMITTABLE_CLASSES).toContain(token);
    }
  });

  test("is deduplicated", () => {
    expect(new Set(ALL_EMITTABLE_CLASSES).size).toBe(
      ALL_EMITTABLE_CLASSES.length,
    );
  });
});
