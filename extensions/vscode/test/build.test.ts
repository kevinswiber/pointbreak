import { type ChildProcessByStdio, spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import {
  existsSync,
  readdirSync,
  readFileSync,
  rmSync,
  utimesSync,
} from "node:fs";
import type { Readable } from "node:stream";
import { describe, expect, it } from "vitest";

const expectedOutputs = [
  "extension.js",
  "extension.js.map",
  "review.css",
  "review.js",
];

describe.sequential("extension builds", () => {
  it("emits deterministic host and browser bundles", () => {
    rmSync("out", { force: true, recursive: true });

    const result = spawnSync(process.execPath, ["build.mjs"], {
      cwd: process.cwd(),
      encoding: "utf8",
    });

    expect(result.status, result.stderr).toBe(0);
    expect(readdirSync("out").sort()).toEqual(expectedOutputs);

    const browserBundle = readFileSync("out/review.js", "utf8");
    expect(browserBundle).toContain("acquireVsCodeApi");
    expect(browserBundle).not.toContain("module.exports");
    expect(browserBundle).not.toContain('require("vscode")');
  });

  it("reports watch completion only when both entries are current", async () => {
    rmSync("out", { force: true, recursive: true });
    const watcher = spawn(process.execPath, ["build.mjs", "--watch"], {
      cwd: process.cwd(),
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const output = captureOutput(watcher);

    try {
      await output.waitForFinished(1);
      expect(readdirSync("out").sort()).toEqual(expectedOutputs);

      touch("src/extension.ts");
      await output.waitForFinished(2);
      expect(existsSync("out/extension.js")).toBe(true);
      expect(existsSync("out/review.js")).toBe(true);
      expect(existsSync("out/review.css")).toBe(true);

      touch("src/webview/review.ts");
      await output.waitForFinished(3);
      expect(existsSync("out/review.js")).toBe(true);
      expect(existsSync("out/review.css")).toBe(true);
      expect(existsSync("out/extension.js")).toBe(true);
    } finally {
      watcher.kill();
      if (watcher.exitCode === null) {
        await once(watcher, "exit");
      }
    }
  }, 30_000);
});

function captureOutput(child: ChildProcessByStdio<null, Readable, Readable>): {
  waitForFinished(count: number): Promise<void>;
} {
  let text = "";
  child.stdout.on("data", (chunk: Buffer) => {
    text += chunk.toString();
  });
  child.stderr.on("data", (chunk: Buffer) => {
    text += chunk.toString();
  });

  return {
    async waitForFinished(count: number): Promise<void> {
      const deadline = Date.now() + 10_000;
      while (finishedCount(text) < count) {
        if (child.exitCode !== null) {
          throw new Error(`watch exited before completion:\n${text}`);
        }
        if (Date.now() >= deadline) {
          throw new Error(`watch did not finish both entries:\n${text}`);
        }
        await new Promise((resolve) => setTimeout(resolve, 25));
      }
    },
  };
}

function finishedCount(output: string): number {
  return output.match(/^\[watch\] build finished$/gm)?.length ?? 0;
}

function touch(path: string): void {
  const future = new Date(Date.now() + 1_000);
  utimesSync(path, future, future);
}
