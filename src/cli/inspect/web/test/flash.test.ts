import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "vitest";

// The served stylesheet, resolved from the web package root (vitest's working
// directory is `src/cli/inspect/web`). Reads the committed source CSS, mirroring
// css-coverage.test.ts.
const APP_CSS = readFileSync(
  resolve(process.cwd(), "../assets/app.css"),
  "utf8",
);

/** The `@keyframes anno-flash { … }` block text. */
function annoFlashKeyframes(css: string): string {
  const match = css.match(/@keyframes anno-flash\s*\{[\s\S]*?\n\}/);
  if (!match) throw new Error("anno-flash keyframes not found");
  return match[0];
}

// The reveal-in-diff navigation flash is a neutral "here's the item you jumped
// to" cue, not a warning state — so it must not animate the semantic `--warning`
// amber, and must not settle on a theme-broken hardcoded white.
test("the navigation flash uses the neutral --accent, not the semantic --warning", () => {
  const block = annoFlashKeyframes(APP_CSS);
  expect(block).toContain("var(--accent)");
  expect(block).not.toContain("var(--warning)");
});

test("the navigation flash fades to transparent, not a hardcoded white", () => {
  const block = annoFlashKeyframes(APP_CSS);
  expect(block).not.toMatch(/rgba\(\s*255\s*,\s*255\s*,\s*255/);
});
