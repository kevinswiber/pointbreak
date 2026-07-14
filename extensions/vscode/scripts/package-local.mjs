import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptRoot = path.dirname(fileURLToPath(import.meta.url));
const extensionRoot = path.resolve(scriptRoot, "..");
const repoRoot = path.resolve(extensionRoot, "../..");
const targetManifest = JSON.parse(
  readFileSync(path.join(repoRoot, ".github/binary-targets.json"), "utf8"),
);
const hostLabel = `${process.platform}-${process.arch}`;
const knownLabels = new Set(targetManifest.map(({ target }) => target));
if (!knownLabels.has(hostLabel)) {
  throw new Error(
    `Unsupported packaging host ${hostLabel}; expected one of ${[...knownLabels].join(", ")}`,
  );
}

const executable = process.platform === "win32" ? "shore.exe" : "shore";
const sourceBinary = path.join(repoRoot, "target", "release", executable);
const bundledRelative = `bin/${hostLabel}/${executable}`;
const bundledBinary = path.join(extensionRoot, bundledRelative);
const runtimeFiles = ["out/extension.js", "out/review.js", "out/review.css"];

run("cargo", ["build", "--release", "--bin", "shore"], repoRoot);
mkdirSync(path.dirname(bundledBinary), { recursive: true });
copyFileSync(sourceBinary, bundledBinary);

for (const entry of readdirSync(extensionRoot)) {
  if (entry.startsWith("pointbreak-") && entry.endsWith(".vsix")) {
    rmSync(path.join(extensionRoot, entry));
  }
}

run("npm", ["run", "build"], extensionRoot);
assertListedFiles(bundledRelative);
run("npx", ["--no-install", "vsce", "package"], extensionRoot);

const artifacts = readdirSync(extensionRoot).filter(
  (entry) => entry.startsWith("pointbreak-") && entry.endsWith(".vsix"),
);
if (artifacts.length !== 1) {
  throw new Error(
    `Expected one packaged VSIX, found ${artifacts.length}: ${artifacts.join(", ")}`,
  );
}
const artifact = path.join(extensionRoot, artifacts[0]);
assertArchiveFiles(artifact, bundledRelative);
console.log(artifact);

function assertListedFiles(binary) {
  const result = run(
    "npx",
    ["--no-install", "vsce", "ls"],
    extensionRoot,
    true,
  );
  assertExactFiles(
    result.stdout.split(/\r?\n/).filter(Boolean),
    ["package.json", "README.md", "LICENSE", "NOTICE", ...runtimeFiles, binary],
    "vsce ls",
  );
}

function assertArchiveFiles(artifact, binary) {
  const extensionFiles = archiveEntries(artifact)
    .split(/\r?\n/)
    .filter((entry) => entry.startsWith("extension/") && !entry.endsWith("/"))
    .map((entry) => entry.slice("extension/".length));
  assertExactFiles(
    extensionFiles,
    [
      "package.json",
      "readme.md",
      "LICENSE.txt",
      "NOTICE",
      ...runtimeFiles,
      binary,
    ],
    "VSIX archive",
  );
}

function archiveEntries(artifact) {
  if (process.platform !== "win32") {
    return run("unzip", ["-Z1", artifact], extensionRoot, true).stdout;
  }
  const listScript = [
    "Add-Type -AssemblyName System.IO.Compression.FileSystem",
    "$archive = [System.IO.Compression.ZipFile]::OpenRead($args[0])",
    "try { $archive.Entries | ForEach-Object FullName } finally { $archive.Dispose() }",
  ].join("; ");
  return run(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-Command", listScript, artifact],
    extensionRoot,
    true,
  ).stdout;
}

function assertExactFiles(actual, expected, source) {
  const sortedActual = [...actual].sort();
  const sortedExpected = [...expected].sort();
  if (JSON.stringify(sortedActual) !== JSON.stringify(sortedExpected)) {
    throw new Error(
      `${source} contained unexpected files.\nExpected: ${sortedExpected.join(", ")}\nActual: ${sortedActual.join(", ")}`,
    );
  }
}

function run(command, args, cwd, capture = false) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    stdio: capture ? ["ignore", "pipe", "pipe"] : "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed with exit code ${result.status}${result.stderr ? `: ${result.stderr.trim()}` : ""}`,
    );
  }
  return result;
}
