import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const packageScript = readFileSync("scripts/package-local.mjs", "utf8");

it("excludes the package-local Git ignore file from the VSIX", () => {
  const ignored = readFileSync(".vscodeignore", "utf8").split("\n");

  expect(ignored).toContain(".gitignore");
});

it("excludes debug source maps from the VSIX", () => {
  const ignored = readFileSync(".vscodeignore", "utf8").split("\n");

  expect(ignored).toContain("out/**/*.map");
});

it("excludes development-only packaging scripts from the VSIX", () => {
  const ignored = readFileSync(".vscodeignore", "utf8").split("\n");

  expect(ignored).toContain("scripts/**");
});

it("includes the webview runtime in both exact package allowlists", () => {
  expect(packageScript.match(/"out\/review\.js"/g)).toHaveLength(1);
  expect(packageScript.match(/"out\/review\.css"/g)).toHaveLength(1);
  expect(packageScript.match(/\.\.\.runtimeFiles/g)).toHaveLength(2);
});

it("excludes source and local-only build inputs from the VSIX", () => {
  const ignored = readFileSync(".vscodeignore", "utf8").split("\n");

  expect(ignored).toEqual(
    expect.arrayContaining([
      ".gitignore",
      "build.mjs",
      "src/**",
      "test/**",
      "scripts/**",
      "out/**/*.map",
    ]),
  );
});
