// Offline verification for the Pointbreak brand artifacts vendored by Review.
import { createHash } from "node:crypto";
import { readFile, realpath } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const LOCK_SCHEMA = "com.withpointbreak.brand-lock/v1";
const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "../../../..");
const lockPath = path.join(scriptDirectory, "pointbreak-brand.lock.json");

const fontFiles = [
  "JetBrainsMono-Bold.woff2",
  "JetBrainsMono-BoldItalic.woff2",
  "JetBrainsMono-ExtraBold.woff2",
  "JetBrainsMono-ExtraBoldItalic.woff2",
  "JetBrainsMono-ExtraLight.woff2",
  "JetBrainsMono-ExtraLightItalic.woff2",
  "JetBrainsMono-Italic.woff2",
  "JetBrainsMono-Light.woff2",
  "JetBrainsMono-LightItalic.woff2",
  "JetBrainsMono-Medium.woff2",
  "JetBrainsMono-MediumItalic.woff2",
  "JetBrainsMono-Regular.woff2",
  "JetBrainsMono-SemiBold.woff2",
  "JetBrainsMono-SemiBoldItalic.woff2",
  "JetBrainsMono-Thin.woff2",
  "JetBrainsMono-ThinItalic.woff2",
  "OFL.txt",
];
const expectedSources = new Map([
  ...fontFiles.map((file) => [
    `src/cli/inspect/design-system/fonts/${file}`,
    `assets/fonts/jetbrains-mono/${file}`,
  ]),
  [
    "src/cli/inspect/assets/pointbreak-logo-mono.svg",
    "assets/logo/pointbreak-logo-mono.svg",
  ],
  [
    "src/cli/inspect/design-system/logo/pointbreak-logo.svg",
    "assets/logo/pointbreak-logo.svg",
  ],
]);

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function attributeValue(source, name) {
  const match = source.match(
    new RegExp(`(?:^|\\s)${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)')`, "i"),
  );
  return match ? (match[1] ?? match[2]) : null;
}

function normalizeGeometryValue(value) {
  return value
    .trim()
    .replaceAll(",", " ")
    .replace(/\s+/g, " ")
    .replace(/\s*([A-Za-z])\s*/g, "$1");
}

function normalizedSvgGeometry(svg) {
  const svgTag = svg.match(/<svg\b([^>]*)>/i);
  assert(svgTag, "SVG geometry: missing <svg> root");
  const viewBox = attributeValue(svgTag[1], "viewBox");
  assert(viewBox, "SVG geometry: missing viewBox");

  const geometryAttributes = [
    "d",
    "points",
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "width",
    "height",
    "transform",
    "pathLength",
  ];
  const records = [`svg|viewBox=${normalizeGeometryValue(viewBox)}`];
  const elementPattern =
    /<(path|circle|ellipse|rect|line|polyline|polygon)\b([^>]*)\/?\s*>/gi;

  for (const match of svg.matchAll(elementPattern)) {
    const values = geometryAttributes.flatMap((name) => {
      const value = attributeValue(match[2], name);
      return value === null
        ? []
        : [`${name}=${normalizeGeometryValue(value)}`];
    });
    assert(values.length > 0, `SVG geometry: <${match[1]}> has no geometry`);
    records.push(`${match[1].toLowerCase()}|${values.join("|")}`);
  }

  assert(records.length > 1, "SVG geometry: no geometry elements found");
  return records.join("\n");
}

function validateRelativePath(value, label) {
  assert(typeof value === "string" && value.length > 0, `${label}: missing path`);
  assert(!path.isAbsolute(value), `${label}: path must be relative`);
  assert(!value.includes("\\"), `${label}: path must use / separators`);
  const normalized = path.posix.normalize(value);
  assert(
    normalized === value && !normalized.startsWith("../"),
    `${label}: path escapes or is not normalized: ${value}`,
  );
}

function validateLock(lock) {
  assert(lock?.schema === LOCK_SCHEMA, `lock: expected schema ${LOCK_SCHEMA}`);
  assert(
    lock?.source?.repository === "https://github.com/withpointbreak/brand",
    "lock: unexpected source repository",
  );
  assert(
    lock?.source?.commit === "45f3bc61a00535f5f7b59bf04dc6391a1153f31c",
    "lock: unexpected source commit",
  );
  assert(
    lock?.source?.manifestSha256 ===
      "a6d36770cd2e9db2951c45835c7739fbb6d89ad45e959c50fe2bbe2e7a76eabe",
    "lock: unexpected manifestSha256",
  );
  assert(/^[0-9a-f]{40}$/.test(lock?.source?.commit ?? ""), "lock: invalid source commit");
  assert(
    /^[0-9a-f]{64}$/.test(lock?.source?.manifestSha256 ?? ""),
    "lock: invalid manifestSha256",
  );
  assert(Array.isArray(lock?.artifacts), "lock: artifacts must be an array");

  const ids = new Set();
  const destinations = new Set();
  for (const artifact of lock.artifacts) {
    const label = artifact?.id ?? "artifact";
    assert(/^[a-z0-9]+(?:[.-][a-z0-9]+)*$/.test(label), `${label}: invalid id`);
    assert(!ids.has(label), `${label}: duplicate id`);
    ids.add(label);
    validateRelativePath(artifact.sourcePath, `${label}.sourcePath`);
    validateRelativePath(artifact.destination, `${label}.destination`);
    assert(!destinations.has(artifact.destination), `${label}: duplicate destination`);
    destinations.add(artifact.destination);
    assert(
      artifact.sourcePath === expectedSources.get(artifact.destination),
      `${label}: source path does not match ${artifact.destination}`,
    );
    assert(/^[0-9a-f]{64}$/.test(artifact.sha256 ?? ""), `${label}: invalid sha256`);
    assert(artifact.mode === "vendored", `${label}: mode must be vendored`);

    const isSvg = artifact.destination.endsWith(".svg");
    if (isSvg) {
      assert(
        /^[0-9a-f]{64}$/.test(artifact.geometrySha256 ?? ""),
        `${label}: invalid geometrySha256`,
      );
    } else {
      assert(artifact.geometrySha256 === undefined, `${label}: geometry is SVG-only`);
    }
  }

  const artifactIds = lock.artifacts.map((artifact) => artifact.id);
  assert(
    JSON.stringify(artifactIds) === JSON.stringify([...artifactIds].sort()),
    "lock: artifacts must be sorted by id",
  );
  const actualDestinations = [...destinations].sort();
  const expectedDestinations = [...expectedSources.keys()].sort();
  assert(
    JSON.stringify(actualDestinations) === JSON.stringify(expectedDestinations),
    "lock: artifact destinations do not match Review's complete brand asset set",
  );
}

async function verifyArtifact(artifact) {
  const absolutePath = path.join(repositoryRoot, artifact.destination);
  let resolvedPath;
  let bytes;
  try {
    [resolvedPath, bytes] = await Promise.all([
      realpath(absolutePath),
      readFile(absolutePath),
    ]);
  } catch (error) {
    throw new Error(`${artifact.id}: ${artifact.destination}: ${error.message}`);
  }
  assert(
    resolvedPath.startsWith(`${repositoryRoot}${path.sep}`),
    `${artifact.id}: destination resolves outside the repository`,
  );

  const actualSha256 = sha256(bytes);
  assert(
    actualSha256 === artifact.sha256,
    `${artifact.id}: sha256 ${actualSha256} != ${artifact.sha256}`,
  );
  if (artifact.geometrySha256) {
    const actualGeometry = sha256(normalizedSvgGeometry(bytes.toString("utf8")));
    assert(
      actualGeometry === artifact.geometrySha256,
      `${artifact.id}: geometrySha256 ${actualGeometry} != ${artifact.geometrySha256}`,
    );
  }
}

async function main() {
  let lock;
  try {
    lock = JSON.parse(await readFile(lockPath, "utf8"));
  } catch (error) {
    throw new Error(`${lockPath}: ${error.message}`);
  }
  validateLock(lock);
  await Promise.all(lock.artifacts.map(verifyArtifact));
  console.log(`Verified ${lock.artifacts.length} vendored Pointbreak brand artifacts.`);
}

main().catch((error) => {
  console.error(`Review brand verification failed: ${error.message}`);
  process.exitCode = 1;
});
