import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

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
