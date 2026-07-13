import { build, context } from "esbuild";

const watch = process.argv.includes("--watch");
const watchStatusPlugin = {
  name: "watch-status",
  setup(build) {
    build.onStart(() => {
      console.log("[watch] build started");
    });
    build.onEnd((result) => {
      if (result.errors.length === 0) {
        console.log("[watch] build finished");
      }
    });
  },
};

const options = {
  entryPoints: ["src/extension.ts"],
  bundle: true,
  outfile: "out/extension.js",
  platform: "node",
  format: "cjs",
  external: ["vscode"],
  sourcemap: true,
  plugins: watch ? [watchStatusPlugin] : [],
};

if (watch) {
  const buildContext = await context(options);
  await buildContext.watch();
} else {
  await build(options);
}
