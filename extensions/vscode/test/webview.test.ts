import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { isHostToWebview } from "../src/webviewProtocol";

const browserEntry = readFileSync("src/webview/review.ts", "utf8");
const protocol = readFileSync("src/webviewProtocol.ts", "utf8");
const theme = readFileSync("src/webview/review.css", "utf8");

it("keeps browser messages on one pure protocol", () => {
  expect(protocol).toContain('type: "render"');
  expect(protocol).toContain('type: "focus"');
  expect(protocol).toContain('type: "error"');
  expect(protocol).toContain('type: "freshness"');
  expect(protocol).toContain('type: "ready"');
  expect(protocol).toContain('type: "openSource"');
  expect(protocol).toContain('type: "reload"');
  expect(browserEntry).toContain('from "../webviewProtocol"');
});

it("reserves typed attention focus messages without owning their behavior", () => {
  expect(
    isHostToWebview({
      type: "focus",
      focus: { kind: "attention", id: "attention:stale" },
    }),
  ).toBe(true);
  expect(isHostToWebview({ type: "focus" })).toBe(true);
  expect(
    isHostToWebview({
      type: "render",
      data: {},
      focus: { kind: "attention", id: "attention:stale" },
    }),
  ).toBe(true);
  expect(
    isHostToWebview({
      type: "focus",
      focus: { kind: "attention", id: 42 },
    }),
  ).toBe(false);
});

it("keeps the browser entry presentation-only", () => {
  expect(browserEntry).not.toMatch(/\bfetch\s*\(/);
  expect(browserEntry).not.toMatch(/\bXMLHttpRequest\b/);
  expect(browserEntry).not.toMatch(/\bWebSocket\b/);
  expect(browserEntry).not.toContain('from "node:');
});

it("bridges light, dark, and high-contrast themes through VS Code tokens", () => {
  expect(theme).toContain("body.vscode-light");
  expect(theme).toContain("body.vscode-dark");
  expect(theme).toContain("body.vscode-high-contrast");
  expect(theme).toContain("--vscode-editor-background");
  expect(theme).toContain("--vscode-diffEditor-insertedLineBackground");
  expect(theme).toContain("--vscode-diffEditor-removedLineBackground");
  expect(theme).toContain("--vscode-focusBorder");
});
