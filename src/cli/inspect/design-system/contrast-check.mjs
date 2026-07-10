// WCAG 2.x text-contrast audit for Pointbreak Review's live token source.
// Reads ../assets/tokens.css directly; no palette values are duplicated here.
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const tokenPath = path.resolve(scriptDirectory, "../assets/tokens.css");
const variantDirectory = path.resolve(scriptDirectory, "variants");

const allowedVariantTokens = new Set([
  "--bg",
  "--bg-elev",
  "--bg-row",
  "--bg-row-sel",
  "--bg-topbar",
  "--sel-bg",
  "--bg-code",
  "--border",
  "--fg",
  "--fg-dim",
  "--accent",
  "--accent-strong",
  "--on-accent",
]);

const requiredTokens = [
  "--bg",
  "--bg-elev",
  "--bg-row",
  "--bg-row-sel",
  "--bg-topbar",
  "--sel-bg",
  "--bg-code",
  "--fg",
  "--fg-dim",
  "--accent",
  "--accent-strong",
  "--on-accent",
  "--success",
  "--warning",
  "--warning-soft",
  "--warning-strong",
  "--danger",
  "--assess",
  "--validation",
  "--info",
  "--teal",
  "--evt-init",
  "--evt-capture",
  "--evt-observation",
  "--evt-assessment",
  "--evt-request",
  "--evt-response",
  "--evt-note",
  "--evt-validation",
  "--tok-keyword",
  "--tok-string",
  "--tok-comment",
  "--tok-number",
  "--tok-type",
  "--tok-function",
  "--tok-constant",
  "--tok-operator",
  "--tok-punctuation",
  "--tok-variable",
  "--diff-add-bg",
  "--diff-add-fg",
  "--diff-del-bg",
  "--diff-del-fg",
  "--emph-add-bg",
  "--emph-del-bg",
  "--hunk-bg",
  "--error-bg",
  "--error-border",
];

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function declarations(block, label) {
  const values = new Map();
  for (const match of block.matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/gi)) {
    assert(!values.has(match[1]), `${label}: duplicate token ${match[1]}`);
    values.set(match[1], match[2].trim());
  }
  return values;
}

function themeBlocks(css) {
  const withoutComments = css.replace(/\/\*[\s\S]*?\*\//g, "");
  const dark = withoutComments.match(
    /:root\s*,\s*\[data-theme\s*=\s*["']dark["']\]\s*\{([\s\S]*?)\}/i,
  );
  const light = withoutComments.match(
    /\[data-theme\s*=\s*["']light["']\]\s*\{([\s\S]*?)\}/i,
  );
  assert(dark, `${tokenPath}: missing :root/[data-theme="dark"] block`);
  assert(light, `${tokenPath}: missing [data-theme="light"] block`);

  const darkTokens = declarations(dark[1], "dark theme");
  const lightTokens = new Map([
    ...darkTokens,
    ...declarations(light[1], "light theme"),
  ]);
  return new Map([
    ["dark", darkTokens],
    ["light", lightTokens],
  ]);
}

function parseVariantPath(argumentsList) {
  if (argumentsList.length === 0) return null;
  assert(
    argumentsList.length === 2 && argumentsList[0] === "--variant",
    "usage: node contrast-check.mjs [--variant variants/instrument-neutral.css]",
  );
  const variantPath = path.resolve(scriptDirectory, argumentsList[1]);
  assert(
    variantPath.startsWith(`${variantDirectory}${path.sep}`),
    `variant path must stay inside ${variantDirectory}`,
  );
  return variantPath;
}

function variantThemeBlocks(css, variantPath) {
  const withoutComments = css.replace(/\/\*[\s\S]*?\*\//g, "");
  for (const match of withoutComments.matchAll(/(--[a-z0-9-]+)\s*:/gi)) {
    assert(
      allowedVariantTokens.has(match[1]),
      `${variantPath}: candidate must not override ${match[1]}`,
    );
  }

  const dark = withoutComments.match(
    /\[data-visual-variant\s*=\s*["']instrument-neutral["']\]\s*,\s*\[data-visual-variant\s*=\s*["']instrument-neutral["']\]\[data-theme\s*=\s*["']dark["']\]\s*\{([\s\S]*?)\}/i,
  );
  const light = withoutComments.match(
    /\[data-visual-variant\s*=\s*["']instrument-neutral["']\]\[data-theme\s*=\s*["']light["']\]\s*\{([\s\S]*?)\}/i,
  );
  assert(dark, `${variantPath}: missing instrument-neutral dark block`);
  assert(light, `${variantPath}: missing instrument-neutral light block`);

  const darkOverrides = declarations(dark[1], "instrument-neutral dark variant");
  const lightOverrides = new Map([
    ...darkOverrides,
    ...declarations(light[1], "instrument-neutral light variant"),
  ]);
  return new Map([
    ["dark", darkOverrides],
    ["light", lightOverrides],
  ]);
}

function composeThemes(baseThemes, variantThemes) {
  return new Map(
    [...baseThemes].map(([theme, tokens]) => [
      theme,
      new Map([...tokens, ...(variantThemes.get(theme) ?? [])]),
    ]),
  );
}

function resolvedValue(tokens, name, stack = []) {
  assert(tokens.has(name), `missing required token ${name}`);
  assert(!stack.includes(name), `token alias cycle: ${[...stack, name].join(" -> ")}`);
  const value = tokens.get(name);
  const alias = value.match(/^var\(\s*(--[a-z0-9-]+)\s*\)$/i);
  if (alias) return resolvedValue(tokens, alias[1], [...stack, name]);
  assert(!value.includes("var("), `${name}: unsupported unresolved value ${value}`);
  return value;
}

function color(value, label) {
  const hex = value.match(/^#([0-9a-f]{6})$/i);
  if (hex) {
    const channels = [0, 2, 4].map((offset) =>
      Number.parseInt(hex[1].slice(offset, offset + 2), 16),
    );
    return [...channels, 1];
  }

  const rgba = value.match(
    /^rgba?\(\s*(\d+(?:\.\d+)?)\s*,\s*(\d+(?:\.\d+)?)\s*,\s*(\d+(?:\.\d+)?)(?:\s*,\s*(\d*\.?\d+))?\s*\)$/i,
  );
  assert(rgba, `${label}: unsupported color ${value}`);
  const channels = rgba.slice(1, 4).map(Number);
  const alpha = rgba[4] === undefined ? 1 : Number(rgba[4]);
  assert(
    channels.every((channel) => channel >= 0 && channel <= 255),
    `${label}: invalid RGB`,
  );
  assert(alpha >= 0 && alpha <= 1, `${label}: invalid alpha`);
  return [...channels, alpha];
}

function tokenColor(tokens, name) {
  return color(resolvedValue(tokens, name), name);
}

function composite(foreground, background) {
  assert(background[3] === 1, "composite background must be opaque");
  const alpha = foreground[3];
  return [
    ...foreground.slice(0, 3).map((channel, index) =>
      Math.round(channel * alpha + background[index] * (1 - alpha)),
    ),
    1,
  ];
}

function luminance([red, green, blue]) {
  const linear = [red, green, blue].map((channel) => {
    const normalized = channel / 255;
    return normalized <= 0.04045
      ? normalized / 12.92
      : ((normalized + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
}

function contrast(foreground, background) {
  const first = luminance(foreground);
  const second = luminance(background);
  const [lighter, darker] = first > second ? [first, second] : [second, first];
  return (lighter + 0.05) / (darker + 0.05);
}

function auditTheme(theme, tokens) {
  for (const name of requiredTokens) resolvedValue(tokens, name);

  const checks = [];
  const add = (label, foreground, background, minimum = 4.5, diagnostic = false) => {
    const ratio = contrast(foreground, background);
    checks.push({ diagnostic, label, minimum, ratio, passed: ratio >= minimum });
  };
  const addTokens = (label, foreground, background) =>
    add(label, tokenColor(tokens, foreground), tokenColor(tokens, background));

  for (const background of [
    "--bg",
    "--bg-elev",
    "--bg-row",
    "--bg-row-sel",
    "--bg-topbar",
    "--sel-bg",
    "--bg-code",
  ]) {
    addTokens(`fg on ${background}`, "--fg", background);
  }
  for (const background of [
    "--bg",
    "--bg-row",
    "--bg-row-sel",
    "--bg-topbar",
    "--sel-bg",
    "--bg-code",
  ]) {
    addTokens(`fg-dim on ${background}`, "--fg-dim", background);
  }
  addTokens("info on hunk", "--info", "--hunk-bg");
  addTokens("on-accent on accent fill", "--on-accent", "--accent-strong");
  addTokens("danger on error surface", "--danger", "--error-bg");

  const diagnosticBackground = color("#3a2a12", "diagnostic surface");
  add(
    "warning-soft on diagnostic surface",
    tokenColor(tokens, "--warning-soft"),
    diagnosticBackground,
  );
  add(
    "warning-strong on diagnostic surface",
    tokenColor(tokens, "--warning-strong"),
    diagnosticBackground,
  );

  for (const foreground of [
    "--success",
    "--warning",
    "--danger",
    "--assess",
    "--validation",
    "--info",
    "--teal",
  ]) {
    for (const background of ["--bg", "--bg-row", "--sel-bg"]) {
      addTokens(`${foreground} on ${background}`, foreground, background);
    }
  }

  for (const foreground of [
    "--evt-init",
    "--evt-capture",
    "--evt-observation",
    "--evt-assessment",
    "--evt-request",
    "--evt-response",
    "--evt-note",
    "--evt-validation",
  ]) {
    for (const background of ["--bg-row", "--sel-bg"]) {
      addTokens(`${foreground} on ${background}`, foreground, background);
    }
  }

  const page = tokenColor(tokens, "--bg");
  const addRow = composite(tokenColor(tokens, "--diff-add-bg"), page);
  const deleteRow = composite(tokenColor(tokens, "--diff-del-bg"), page);
  const emphasizedAdd = composite(tokenColor(tokens, "--emph-add-bg"), addRow);
  const emphasizedDelete = composite(tokenColor(tokens, "--emph-del-bg"), deleteRow);
  add("diff-add-fg on add row", tokenColor(tokens, "--diff-add-fg"), addRow);
  add("diff-del-fg on delete row", tokenColor(tokens, "--diff-del-fg"), deleteRow);
  add("diff-add-fg on emphasized add", tokenColor(tokens, "--diff-add-fg"), emphasizedAdd);
  add("diff-del-fg on emphasized delete", tokenColor(tokens, "--diff-del-fg"), emphasizedDelete);

  const syntaxTokens = [
    "--tok-keyword",
    "--tok-string",
    "--tok-comment",
    "--tok-number",
    "--tok-type",
    "--tok-function",
    "--tok-constant",
    "--tok-operator",
    "--tok-punctuation",
    "--tok-variable",
  ];
  for (const foreground of syntaxTokens) {
    add(`${foreground} on diff body`, tokenColor(tokens, foreground), page);
  }

  // Diagnostic-only decision input. These combinations exist in the live
  // syntax layer but were outside the Phase 1 gating matrix. Keep them visible
  // without making a gallery-only comparison task silently change live values.
  if (theme === "light") {
    for (const foreground of syntaxTokens) {
      for (const [surface, background] of [
        ["add row", addRow],
        ["delete row", deleteRow],
        ["emphasized add", emphasizedAdd],
        ["emphasized delete", emphasizedDelete],
      ]) {
        add(
          `${foreground} on ${surface}`,
          tokenColor(tokens, foreground),
          background,
          4.5,
          true,
        );
      }
    }
  }

  return checks.map((check) => ({ theme, ...check }));
}

async function main() {
  const variantPath = parseVariantPath(process.argv.slice(2));
  const baseThemes = themeBlocks(await readFile(tokenPath, "utf8"));
  const themes = variantPath
    ? composeThemes(
        baseThemes,
        variantThemeBlocks(await readFile(variantPath, "utf8"), variantPath),
      )
    : baseThemes;
  const treatment = variantPath ? "instrument-neutral" : "baseline";
  const results = [...themes].flatMap(([theme, tokens]) => auditTheme(theme, tokens));
  const gatingResults = results.filter((result) => !result.diagnostic);
  const diagnosticResults = results.filter((result) => result.diagnostic);
  const failures = gatingResults.filter((result) => !result.passed);
  const diagnosticFailures = diagnosticResults.filter((result) => !result.passed);

  console.log(`treatment: ${treatment}`);
  console.log("kind       | theme | ratio | check");
  console.log("-----------|-------|-------|------");
  for (const result of results) {
    console.log(
      `${(result.diagnostic ? "diagnostic" : "gate").padEnd(10)} | ${result.theme.padEnd(5)} | ${result.ratio.toFixed(2).padStart(5)} | ${result.label}`,
    );
  }
  console.log(
    `\n${gatingResults.length} gating checks; ${failures.length} failures. ` +
      `${diagnosticResults.length} syntax/tinted-row diagnostics; ` +
      `${diagnosticFailures.length} below AA.`,
  );
  if (failures.length > 0) process.exitCode = 1;
}

main().catch((error) => {
  console.error(`Review contrast audit failed: ${error.message}`);
  process.exitCode = 1;
});
