import { build, context } from "esbuild";
import { rm } from "node:fs/promises";

const watch = process.argv.includes("--watch");
const watchStatus = createWatchStatus(["host", "webview"]);

const hostOptions = {
  entryPoints: ["src/extension.ts"],
  bundle: true,
  outfile: "out/extension.js",
  platform: "node",
  format: "cjs",
  external: ["vscode"],
  sourcemap: true,
  plugins: watch ? [watchStatus.plugin("host")] : [],
};

const webviewOptions = {
  entryPoints: ["src/webview/review.ts"],
  bundle: true,
  outfile: "out/review.js",
  platform: "browser",
  format: "iife",
  sourcemap: false,
  plugins: watch ? [watchStatus.plugin("webview")] : [],
};

await rm("out", { force: true, recursive: true });

if (watch) {
  const [hostContext, webviewContext] = await Promise.all([
    context(hostOptions),
    context(webviewOptions),
  ]);
  await Promise.all([hostContext.watch(), webviewContext.watch()]);
} else {
  await Promise.all([build(hostOptions), build(webviewOptions)]);
}

function createWatchStatus(targets) {
  const current = new Set();
  let cycleActive = false;
  let finishTimer;

  function start(target) {
    if (finishTimer) {
      clearTimeout(finishTimer);
      finishTimer = undefined;
    }
    if (!cycleActive) {
      console.log("[watch] build started");
      cycleActive = true;
    }
    current.delete(target);
  }

  function finish(target, result) {
    if (result.errors.length > 0) {
      return;
    }
    current.add(target);
    if (current.size !== targets.length) {
      return;
    }
    finishTimer = setTimeout(() => {
      if (current.size === targets.length) {
        console.log("[watch] build finished");
        cycleActive = false;
      }
      finishTimer = undefined;
    }, 50);
  }

  return {
    plugin(target) {
      return {
        name: `watch-status-${target}`,
        setup(build) {
          build.onStart(() => start(target));
          build.onEnd((result) => finish(target, result));
        },
      };
    },
  };
}
