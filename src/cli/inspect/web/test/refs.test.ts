import { describe, expect, it } from "vitest";
import { REF_ID_PREFIXES, REF_KINDS } from "../src/classNames";
import {
  isMarkdownContentType,
  linkify,
  linkifyEscaped,
  REF_RE,
  refInfo,
  safeMarkdownHref,
  shortId,
  shortRef,
  targetDisplayLabel,
  targetHeadBadge,
} from "../src/refs";

function parse(html: string): Document {
  return new DOMParser().parseFromString(html, "text/html");
}

describe("shortId", () => {
  it("returns the last colon segment, truncated to 12 chars", () => {
    expect(shortId("")).toBe("");
    expect(shortId("plainid")).toBe("plainid");
    expect(shortId("a:b:c")).toBe("c");
    expect(shortId("obj:sha256:38a493d2f09d6fde9d1dcac6")).toBe("38a493d2f09d");
    expect(shortId("rev:sha256:1ace028b9f00")).toBe("1ace028b9f00");
  });
});

describe("shortRef", () => {
  it("keeps the kind prefix and shortens the hash to 8", () => {
    expect(shortRef("obs:sha256:1ace028b9f00deadbeef")).toBe("obs:1ace028b");
    expect(shortRef("rev:git:sha256:abcdef012345")).toBe("rev:abcdef01");
    // The worktree-capture revision shape shortens like the git/plain forms.
    expect(shortRef("rev:worktree:sha256:abcdef012345")).toBe("rev:abcdef01");
  });

  it("shortens a bare sha256 hash and a 40-char git oid", () => {
    expect(shortRef("sha256:abcdef0123456789")).toBe("sha256:abcdef01");
    expect(shortRef("0123456789abcdef0123456789abcdef01234567")).toBe(
      "0123456789",
    );
  });

  it("returns non-matching input unchanged", () => {
    expect(shortRef("agent:codex")).toBe("agent:codex");
  });
});

describe("refInfo", () => {
  it("classifies validation ids as non-clickable", () => {
    expect(refInfo("validation:sha256:abcdef")).toEqual({
      kind: "validation",
      clickable: false,
    });
  });

  it("classifies routable prefixed sha256 ids as clickable by their kind", () => {
    expect(refInfo("rev:sha256:abc123")).toEqual({
      kind: "rev",
      clickable: true,
    });
    expect(refInfo("obs:sha256:abc123")).toEqual({
      kind: "obs",
      clickable: true,
    });
  });

  it("classifies promoted content-id prefixes with no route as non-clickable", () => {
    // #344 linkifies these for display, but they have no resolveRef route, so
    // they must render as non-clickable chips (not dead links).
    for (const kind of [
      "obj",
      "engagement",
      "checkpoint",
      "task-attempt",
      "assoc-commit",
      "assoc-ref",
      "withdraw-commit",
      "withdraw-ref",
    ]) {
      expect(refInfo(`${kind}:sha256:abc123`)).toEqual({
        kind,
        clickable: false,
      });
    }
  });

  it("classifies a worktree-capture revision id as a clickable rev", () => {
    expect(refInfo("rev:worktree:sha256:abc123")).toEqual({
      kind: "rev",
      clickable: true,
    });
  });

  it("classifies bare hashes and git commits as non-clickable", () => {
    expect(refInfo("sha256:abcdef")).toEqual({
      kind: "hash",
      clickable: false,
    });
    expect(refInfo("0123456789abcdef0123456789abcdef01234567")).toEqual({
      kind: "commit",
      clickable: false,
    });
  });

  it("classifies tracks as clickable and rejects unknown tokens", () => {
    expect(refInfo("agent:codex")).toEqual({ kind: "track", clickable: true });
    expect(refInfo("human:kevin")).toEqual({ kind: "track", clickable: true });
    expect(refInfo("not-a-ref")).toBeNull();
  });
});

describe("REF_RE derivation", () => {
  it("derives REF_RE byte-identically from REF_ID_PREFIXES", () => {
    // The alternation lock: change REF_ID_PREFIXES and this expected literal
    // together, deliberately — that is the display-membership decision.
    expect(REF_RE.source).toBe(
      "\\b(?:input-request-response|input-request|obs|assess|rev|evt|validation|obj|engagement|checkpoint|task-attempt|assoc-commit|assoc-ref|withdraw-commit|withdraw-ref):(?:git:|worktree:)?sha256:[0-9a-f]{6,}\\b|(?<!:)\\bsha256:[0-9a-f]{16,}\\b|\\b[0-9a-f]{40}\\b|\\b(?:agent|human):[a-z0-9][a-z0-9_-]*\\b",
    );
    expect(REF_RE.flags).toBe("gi");
  });

  it("derives REF_KINDS as the prefix list plus the non-prefix classifier kinds", () => {
    expect([...REF_KINDS]).toEqual([
      ...REF_ID_PREFIXES,
      "hash",
      "commit",
      "track",
    ]);
  });

  it("keeps longer prefixes distinguishable from their own prefixes", () => {
    // input-request-response must never classify as input-request.
    expect(refInfo("input-request-response:sha256:abcdef12")).toEqual({
      kind: "input-request-response",
      clickable: true,
    });
  });
});

describe("linkify / linkifyEscaped", () => {
  it("renders a clickable ref chip with data attributes", () => {
    const span = parse(
      linkify("see rev:sha256:abcdef0123456789"),
    ).querySelector("span.ref");
    expect(span?.getAttribute("data-ref-kind")).toBe("rev");
    expect(span?.getAttribute("role")).toBe("link");
    expect(span?.getAttribute("tabindex")).toBe("0");
    expect(span?.getAttribute("data-ref-id")).toBe(
      "rev:sha256:abcdef0123456789",
    );
    expect(span?.getAttribute("title")).toBe("rev:sha256:abcdef0123456789");
    expect(span?.textContent).toBe("rev:abcdef01");
  });

  it("renders a non-clickable chip for hashes (no role, no data-ref-kind)", () => {
    const span = parse(linkify("sha256:abcdef0123456789")).querySelector(
      "span.ref",
    );
    expect(span?.getAttribute("data-ref-kind")).toBeNull();
    expect(span?.getAttribute("role")).toBeNull();
    expect(span?.classList.contains("ref-hash")).toBe(true);
    expect(span?.textContent).toBe("sha256:abcdef01");
  });

  it("linkifies a worktree-capture revision id as a clickable rev chip (#344 shape gap)", () => {
    const id = "rev:worktree:sha256:38a493d2f09d6fde9d1dcac61a12c4ccc4de42a0";
    const span = parse(linkify(`see ${id}`)).querySelector("span.ref");
    expect(span?.getAttribute("data-ref-kind")).toBe("rev");
    expect(span?.getAttribute("data-ref-id")).toBe(id);
    expect(span?.textContent).toBe("rev:38a493d2");
  });

  it("linkifies a promoted content id (obj) as a non-clickable chip, not a dead link (#344)", () => {
    const id =
      "obj:sha256:38a493d2f09d6fde9d1dcac61a12c4ccc4de42a0b9c6829752d3";
    const span = parse(linkify(`snapshot ${id}`)).querySelector("span.ref");
    expect(span?.classList.contains("ref-obj")).toBe(true);
    // Non-clickable: no link affordance, no click-delegate dataset.
    expect(span?.getAttribute("role")).toBeNull();
    expect(span?.getAttribute("data-ref-kind")).toBeNull();
    expect(span?.getAttribute("title")).toBe(id);
  });

  it("renders validation ids as non-clickable chips", () => {
    const span = parse(linkify("validation:sha256:abcdef")).querySelector(
      "span.ref",
    );
    expect(span?.classList.contains("ref-validation")).toBe(true);
    expect(span?.getAttribute("data-ref-kind")).toBeNull();
  });

  it("renders retired legacy ids as plain text, not a partial hash chip (#344)", () => {
    // Retiring review-unit/snap from REF_ID_PREFIXES must not leave the sha256
    // tail matching the generic hash arm — the whole id renders plain.
    for (const id of [
      "review-unit:sha256:abcdef0123456789",
      "snap:sha256:abcdef0123456789",
      "snap:git:sha256:abcdef0123456789",
    ]) {
      const html = linkify(`see ${id}`);
      expect(parse(html).querySelector("span.ref")).toBeNull();
      expect(html).toContain(id);
    }
  });

  it("still linkifies a genuinely bare sha256 content hash", () => {
    // The `(?<!:)` guard only excludes a `sha256:` preceded by `:` (a prefixed
    // id tail); a standalone content hash still chips.
    const span = parse(linkify("hash sha256:abcdef0123456789")).querySelector(
      "span.ref",
    );
    expect(span?.classList.contains("ref-hash")).toBe(true);
  });

  it("escapes surrounding text and coerces null to empty (no raw markup)", () => {
    const doc = parse(linkify("<script>alert(1)</script>"));
    expect(doc.querySelector("script")).toBeNull();
    expect(linkify(null)).toBe("");
    expect(linkify(undefined)).toBe("");
  });

  it("operates on already-escaped input without re-escaping", () => {
    expect(linkifyEscaped("plain &amp; text")).toBe("plain &amp; text");
  });
});

describe("targetDisplayLabel", () => {
  it("floors to working tree when absent and escapes the label", () => {
    expect(targetDisplayLabel(null)).toBe("working tree");
    expect(targetDisplayLabel({})).toBe("working tree");
    expect(targetDisplayLabel({ label: "feature-x" })).toBe("feature-x");
    expect(targetDisplayLabel({ label: "<x>" })).toBe("&lt;x&gt;");
  });
});

describe("targetHeadBadge", () => {
  it("returns empty when there is no head label", () => {
    expect(targetHeadBadge(null)).toBe("");
    expect(targetHeadBadge({ head: null })).toBe("");
    expect(targetHeadBadge({ head: {} })).toBe("");
  });

  it("renders a badge with the head label, escaped", () => {
    const span = parse(
      targetHeadBadge({ head: { label: "78a5f33" } }),
    ).querySelector("span.badge");
    expect(span?.textContent).toBe("@ 78a5f33");
  });

  it("adds a live-branch current qualifier", () => {
    const span = parse(
      targetHeadBadge({ head: { label: "78a5f33", liveBranch: "main" } }),
    ).querySelector("span.badge");
    expect(span?.textContent).toBe("@ 78a5f33 · main (current)");
  });
});

describe("isMarkdownContentType", () => {
  it("recognizes only text/markdown", () => {
    expect(isMarkdownContentType("text/markdown")).toBe(true);
    expect(isMarkdownContentType("text/plain")).toBe(false);
    expect(isMarkdownContentType("")).toBe(false);
    expect(isMarkdownContentType(undefined)).toBe(false);
  });
});

describe("safeMarkdownHref", () => {
  it("allows http(s), mailto, and fragment hrefs (escaped)", () => {
    expect(safeMarkdownHref("https://example.com")).toBe("https://example.com");
    expect(safeMarkdownHref("http://example.com")).toBe("http://example.com");
    expect(safeMarkdownHref("mailto:a@b.com")).toBe("mailto:a@b.com");
    expect(safeMarkdownHref("#section")).toBe("#section");
    expect(safeMarkdownHref("  https://example.com  ")).toBe(
      "https://example.com",
    );
    expect(safeMarkdownHref("https://x?a=<b>")).toBe("https://x?a=&lt;b&gt;");
  });

  it("rejects unsafe schemes", () => {
    expect(safeMarkdownHref("javascript:alert(1)")).toBe("");
    expect(safeMarkdownHref("ftp://example.com")).toBe("");
    expect(safeMarkdownHref("")).toBe("");
  });
});
