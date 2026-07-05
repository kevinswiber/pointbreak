import { describe, expect, it } from "vitest";
import { fmtDate, fmtDateTime, fmtTime, parseMs } from "../src/format";

describe("parseMs", () => {
  it("extracts the trailing millisecond count from a unix-ms token", () => {
    expect(parseMs("unix-ms:1782698716556")).toBe(1782698716556);
  });

  it("reads trailing digits regardless of prefix or trailing whitespace", () => {
    expect(parseMs("1782698716556")).toBe(1782698716556);
    expect(parseMs("abc123")).toBe(123);
    expect(parseMs("123  ")).toBe(123);
  });

  it("returns null when there are no trailing digits", () => {
    expect(parseMs("abc")).toBeNull();
    expect(parseMs("")).toBeNull();
  });

  it("returns null for non-string input", () => {
    expect(parseMs(123)).toBeNull();
    expect(parseMs(null)).toBeNull();
    expect(parseMs(undefined)).toBeNull();
  });
});

describe("fmtTime", () => {
  it("renders HH:MM:SS with a zero-padded millisecond suffix", () => {
    expect(fmtTime("unix-ms:1782698716556")).toMatch(
      /^\d{1,2}:\d{2}:\d{2}\.556$/,
    );
  });

  it("zero-pads the millisecond suffix to three digits", () => {
    expect(fmtTime("unix-ms:1000007")).toMatch(/\.007$/);
    expect(fmtTime("unix-ms:1000000")).toMatch(/\.000$/);
  });

  it("returns the original string (or empty) when it carries no timestamp", () => {
    expect(fmtTime("no-digits-here")).toBe("no-digits-here");
    expect(fmtTime("")).toBe("");
  });
});

describe("fmtDateTime", () => {
  it("renders a locale date-time string for a timestamp", () => {
    const formatted = fmtDateTime("unix-ms:1782698716556");
    expect(formatted).not.toBe("unix-ms:1782698716556");
    expect(formatted).toMatch(/\d/);
    expect(formatted).toContain(":");
  });

  it("returns the original string (or empty) when it carries no timestamp", () => {
    expect(fmtDateTime("not-a-time")).toBe("not-a-time");
    expect(fmtDateTime("")).toBe("");
  });
});

describe("fmtDate", () => {
  it("renders a locale date (no clock) for a timestamp", () => {
    const formatted = fmtDate("unix-ms:1782698716556");
    expect(formatted).not.toBe("unix-ms:1782698716556");
    expect(formatted).toMatch(/\d/);
    // date-only: no clock separator
    expect(formatted).not.toContain(":");
  });

  it("returns the original string (or empty) when it carries no timestamp", () => {
    expect(fmtDate("not-a-time")).toBe("not-a-time");
    expect(fmtDate("")).toBe("");
  });
});
