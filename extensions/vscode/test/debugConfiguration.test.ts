import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, it } from "vitest";

const repositoryRoot = resolve(import.meta.dirname, "../../..");
const workspaceFolder = "$" + "{workspaceFolder}";

function readJson(path: string): unknown {
  return JSON.parse(readFileSync(resolve(repositoryRoot, path), "utf8"));
}

it("launches the extension from the repository workspace", () => {
  expect(readJson(".vscode/launch.json")).toMatchObject({
    configurations: [
      {
        name: "Run Pointbreak Extension",
        type: "extensionHost",
        request: "launch",
        args: [
          `--extensionDevelopmentPath=${workspaceFolder}/extensions/vscode`,
        ],
        outFiles: [`${workspaceFolder}/extensions/vscode/out/**/*.js`],
        preLaunchTask: "build VS Code extension",
      },
    ],
  });
});

it("builds the extension before launching it", () => {
  expect(readJson(".vscode/tasks.json")).toMatchObject({
    tasks: [
      {
        label: "build VS Code extension",
        type: "npm",
        script: "watch",
        path: "extensions/vscode",
        isBackground: true,
        problemMatcher: {
          background: {
            activeOnStart: true,
            beginsPattern: "^\\[watch\\] build started$",
            endsPattern: "^\\[watch\\] build finished$",
          },
        },
      },
    ],
  });
});

it("provides an esbuild watch command", () => {
  expect(readJson("extensions/vscode/package.json")).toMatchObject({
    scripts: {
      watch: "node build.mjs --watch",
    },
  });
});
