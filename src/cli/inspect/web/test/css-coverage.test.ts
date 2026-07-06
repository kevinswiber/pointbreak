import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "vitest";
import { ALL_EMITTABLE_CLASSES } from "../src/classNames";

// The served stylesheet, resolved from the web package root (vitest's working
// directory is `src/cli/inspect/web`, where this suite always runs). This reads
// the committed source CSS, not the bundle.
const APP_CSS_PATH = resolve(process.cwd(), "../assets/app.css");

// Classes the inspector can emit that have no `app.css` rule and fall back to
// their base class, each with a one-line reason. Whether any is a real styling
// gap is being evaluated in kevinswiber/shoreline#296; this list keeps the drift
// test green while that decision is owned there. An emitted class with no rule
// and no entry here fails the test — that is the JS-vs-CSS drift catch.
const REF_BASE_STYLED =
  "clickable ref chip; styled via `.ref[data-ref-kind]` (accent), the per-kind class is only a hook — intentional, the `.ref-commit`/`.ref-hash` rules exist to dim the non-clickable kinds (#296)";
const REF_NONCLICKABLE_STYLED =
  "non-clickable content-id chip (no resolveRef route); base `.ref` styling without the `.ref[data-ref-kind]` accent — display-only membership (#344)";
const CSS_LESS_ALLOWLIST: Record<string, string> = {
  // anno-validation and s-modified were #296 gaps and now have app.css rules, so
  // they are NOT allowlisted here (the guard test below would flag them if they were).
  resolved:
    "`fact-status resolved` cue; inherits base `.fact-status` (intentional — only emits for a resolved assessment with no value) — see #296",
  "ref-input-request-response": REF_BASE_STYLED,
  "ref-input-request": REF_BASE_STYLED,
  "ref-obs": REF_BASE_STYLED,
  "ref-assess": REF_BASE_STYLED,
  "ref-rev": REF_BASE_STYLED,
  "ref-evt": REF_BASE_STYLED,
  "ref-validation": REF_BASE_STYLED,
  "ref-track": REF_BASE_STYLED,
  // #344 promoted content ids: linkified as non-clickable chips.
  "ref-obj": REF_NONCLICKABLE_STYLED,
  "ref-engagement": REF_NONCLICKABLE_STYLED,
  "ref-checkpoint": REF_NONCLICKABLE_STYLED,
  "ref-task-attempt": REF_NONCLICKABLE_STYLED,
  "ref-assoc-commit": REF_NONCLICKABLE_STYLED,
  "ref-assoc-ref": REF_NONCLICKABLE_STYLED,
  "ref-withdraw-commit": REF_NONCLICKABLE_STYLED,
  "ref-withdraw-ref": REF_NONCLICKABLE_STYLED,
};

// Every `.class` token in the stylesheet, INCLUDING those inside compound /
// descendant / pseudo selectors (`.dag-node.head rect`, `.fact-status.passed`,
// `.cmd-item:hover`), so a class counts as present if it appears in any selector.
function cssClassSelectors(css: string): Set<string> {
  return new Set(
    [...css.matchAll(/\.([a-z][a-z0-9_-]*)/g)].map((match) => match[1]),
  );
}

test("every emittable class has an app.css selector (or is an allowlisted CSS-less class)", () => {
  const css = readFileSync(APP_CSS_PATH, "utf8");
  const selectors = cssClassSelectors(css);
  const missing = ALL_EMITTABLE_CLASSES.filter(
    (cls) => !selectors.has(cls),
  ).filter((cls) => !(cls in CSS_LESS_ALLOWLIST));
  expect(missing).toEqual([]);
});

test("the CSS-less allowlist stays honest (every entry is still emittable and still rule-less)", () => {
  const css = readFileSync(APP_CSS_PATH, "utf8");
  const selectors = cssClassSelectors(css);
  const emittable = new Set(ALL_EMITTABLE_CLASSES);
  // An allowlist entry the JS can no longer emit, or one that now HAS an app.css
  // rule (e.g. a #296 gap was closed), is dead weight — surface it for removal.
  const emittableButCovered = Object.keys(CSS_LESS_ALLOWLIST).filter((cls) =>
    selectors.has(cls),
  );
  const notEmittable = Object.keys(CSS_LESS_ALLOWLIST).filter(
    (cls) => !emittable.has(cls),
  );
  expect({ emittableButCovered, notEmittable }).toEqual({
    emittableButCovered: [],
    notEmittable: [],
  });
});
