import { describe, expect, it } from "vitest";
import { revisionDiscoveryDisplay, shortReferenceId } from "../src/idDisplay";

describe("shortReferenceId", () => {
  it("uses the final typed-ID component and preserves short values", () => {
    expect(shortReferenceId("rev:sha256:1234567890abcdef")).toBe(
      "1234567890ab",
    );
    expect(shortReferenceId("assessment:short")).toBe("short");
  });
});

describe("revisionDiscoveryDisplay", () => {
  it("prefers a summary and retains identity plus landing in the description", () => {
    expect(
      revisionDiscoveryDisplay({
        revisionId: "rev:sha256:1234567890abcdef",
        summary: "Make revision discovery readable",
        mergeStatus: "open",
      }),
    ).toEqual({
      label: "Make revision discovery readable",
      description: "1234567890ab · open",
    });
  });

  it("falls back to the short revision id for legacy captures", () => {
    expect(
      revisionDiscoveryDisplay({
        revisionId: "rev:sha256:1234567890abcdef",
        mergeStatus: "merged",
      }),
    ).toEqual({ label: "1234567890ab", description: "merged" });
  });
});
