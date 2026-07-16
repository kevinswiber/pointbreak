#!/usr/bin/env bash
#
# Refresh the dark/light Pointbreak Review screenshots embedded in README.md.
# The inspector must already be running with access to the selected revision.
#
# Defaults reproduce the checked-in framing. Override the source record when a
# clearer review story becomes available:
#
#   just capture-inspector-screenshots \
#     --revision 93326e73 \
#     --track agent:codex-450
#
# To bind a capture to the canonical public example pack and write its provenance
# manifest, add both of these options:
#
#   --example-manifest examples/review/checkout-refactor/manifest.json
#   --manifest assets/marketing/review-interface-capture.json
#
set -euo pipefail

die() { printf 'error: %s\n' "$*" >&2; exit 1; }
note() { printf '  %s\n' "$*"; }

show_help() {
  sed -n '2,/^set -euo pipefail/p' "$0" | sed 's/^# \{0,1\}//; s/^#$//' | sed '$d'
  cat <<'EOF'

Options:
  --url <url>           Running inspector URL (default: http://127.0.0.1:7878)
  --revision <id>       Revision filter, full or abbreviated (default: 93326e73)
  --track <id>          Track filter (default: agent:codex-450)
  --assessment <value> Assessment row to select (default: accepted)
  --out-dir <dir>       Asset destination (default: <repo>/assets)
  --example-manifest <path>
                        Verified Review example pack manifest
  --manifest <path>     Capture manifest to replace last (requires --example-manifest)
  --hide-observations   Hide observation rows to keep assessment transitions together
  -h, --help            Show this help

Environment:
  PLAYWRIGHT_CLI        Optional path to playwright-cli or a compatible wrapper
EOF
}

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BASE_URL="http://127.0.0.1:7878"
REVISION="93326e73"
TRACK="agent:codex-450"
ASSESSMENT="accepted"
OUT_DIR="$REPO_ROOT/assets"
HIDE_OBSERVATIONS="false"
EXAMPLE_MANIFEST=""
MANIFEST=""
REVISION_SET="false"
TRACK_SET="false"
ASSESSMENT_SET="false"

while [ $# -gt 0 ]; do
  case "$1" in
    --url) BASE_URL="$2"; shift 2 ;;
    --revision) REVISION="$2"; REVISION_SET="true"; shift 2 ;;
    --track) TRACK="$2"; TRACK_SET="true"; shift 2 ;;
    --assessment) ASSESSMENT="$2"; ASSESSMENT_SET="true"; shift 2 ;;
    --out-dir) OUT_DIR="$2"; shift 2 ;;
    --example-manifest) EXAMPLE_MANIFEST="$2"; shift 2 ;;
    --manifest) MANIFEST="$2"; shift 2 ;;
    --hide-observations) HIDE_OBSERVATIONS="true"; shift ;;
    -h|--help) show_help; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

command -v curl >/dev/null 2>&1 || die "curl not found"
command -v node >/dev/null 2>&1 || die "node not found"

if { [ -n "$EXAMPLE_MANIFEST" ] && [ -z "$MANIFEST" ]; } || \
   { [ -z "$EXAMPLE_MANIFEST" ] && [ -n "$MANIFEST" ]; }; then
  die "--example-manifest and --manifest must be supplied together"
fi

if [ -n "$EXAMPLE_MANIFEST" ]; then
  EXAMPLE_MANIFEST="$(node -e 'process.stdout.write(require("node:path").resolve(process.argv[1]))' "$EXAMPLE_MANIFEST")"
  MANIFEST="$(node -e 'process.stdout.write(require("node:path").resolve(process.argv[1]))' "$MANIFEST")"
  [ -f "$EXAMPLE_MANIFEST" ] || die "example manifest not found: $EXAMPLE_MANIFEST"

  cargo run --quiet --example review_example_pack -- verify \
    --pack "$(dirname "$EXAMPLE_MANIFEST")" \
    || die "example pack verification failed"

  IFS=$'\t' read -r PACK_REVISION PACK_TRACK PACK_ASSESSMENT \
    < <(node -e '
const fs = require("node:fs");
const manifest = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
process.stdout.write([
  manifest.record.revision,
  manifest.record.track,
  manifest.record.selectedAssessment,
].join("\t") + "\n");
' "$EXAMPLE_MANIFEST")

  [ "$REVISION_SET" = "false" ] || [ "$REVISION" = "$PACK_REVISION" ] \
    || die "--revision does not match the example manifest"
  [ "$TRACK_SET" = "false" ] || [ "$TRACK" = "$PACK_TRACK" ] \
    || die "--track does not match the example manifest"
  [ "$ASSESSMENT_SET" = "false" ] || [ "$ASSESSMENT" = "$PACK_ASSESSMENT" ] \
    || die "--assessment does not match the example manifest"
  REVISION="$PACK_REVISION"
  TRACK="$PACK_TRACK"
  ASSESSMENT="$PACK_ASSESSMENT"
fi

BASE_URL="${BASE_URL%/}"
curl -fsS "$BASE_URL/" >/dev/null \
  || die "inspector is not reachable at $BASE_URL"

if [ -n "${PLAYWRIGHT_CLI:-}" ]; then
  PWCLI=("$PLAYWRIGHT_CLI")
elif command -v playwright-cli >/dev/null 2>&1; then
  PWCLI=(playwright-cli)
else
  command -v npx >/dev/null 2>&1 || die "playwright-cli and npx are both unavailable"
  PWCLI=(npx --yes --package @playwright/cli@0.1.17 playwright-cli)
fi

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

TMP_DIR="$(mktemp -d)"
SESSION="pointbreak-readme-$$"
STAGED_DARK=""
STAGED_LIGHT=""
STAGED_MANIFEST=""

run_pw() {
  (cd "$TMP_DIR" && "${PWCLI[@]}" -s="$SESSION" "$@")
}

cleanup() {
  run_pw close >/dev/null 2>&1 || true
  [ -z "$STAGED_DARK" ] || rm -f "$STAGED_DARK"
  [ -z "$STAGED_LIGHT" ] || rm -f "$STAGED_LIGHT"
  [ -z "$STAGED_MANIFEST" ] || rm -f "$STAGED_MANIFEST"
  # rm -rf "$TMP_DIR"
}
trap cleanup EXIT

cat > "$TMP_DIR/cli.config.json" <<'EOF'
{
  "browser": {
    "contextOptions": {
      "viewport": { "width": 900, "height": 506 },
      "deviceScaleFactor": 2,
      "colorScheme": "dark"
    }
  }
}
EOF

CAPTURE_CONFIG="$(node -e '
const [baseUrl, revision, track, assessment, hideObservations, darkPath, lightPath] = process.argv.slice(1);
process.stdout.write(JSON.stringify({
  baseUrl,
  revision,
  track,
  assessment,
  hideObservations: hideObservations === "true",
  darkPath,
  lightPath,
}));
' "$BASE_URL" "$REVISION" "$TRACK" "$ASSESSMENT" "$HIDE_OBSERVATIONS" \
  "$TMP_DIR/shore-inspector-dark.png" "$TMP_DIR/shore-inspector-light.png")"

cat > "$TMP_DIR/capture.mjs" <<'EOF'
((config) => async page => {
const { baseUrl, revision, track, assessment, hideObservations, darkPath, lightPath } = config;
const consoleErrors = [];

page.on("console", (message) => {
  if (message.type() === "error") consoleErrors.push(message.text());
});

const query = `revision:${revision} track:${track}`;
const revisionDisplay = revision.startsWith("rev:sha256:")
  ? revision.slice("rev:sha256:".length, "rev:sha256:".length + 8)
  : revision;
const escapedAssessment = assessment.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const assessmentPattern = new RegExp(`\\b${escapedAssessment}\\b`, "i");

async function setDisplay(theme) {
  await page.evaluate(({ selectedTheme }) => {
    localStorage.setItem("shore-inspect-theme", selectedTheme);
    localStorage.setItem("shore-inspect-density", "comfortable");
    localStorage.setItem("shore-inspect-split", "55");
  }, { selectedTheme: theme });
  await page.reload({ waitUntil: "domcontentloaded" });
}

async function clickVisible(locator) {
  for (const candidate of await locator.all()) {
    if (await candidate.isVisible()) {
      await candidate.click();
      return true;
    }
  }
  return false;
}

async function prepareFrame() {
  const search = page.getByRole("searchbox", { name: /search/i });
  await search.waitFor({ state: "visible" });
  await search.fill(query);

  const filters = page.getByRole("button", { name: /Filters/i });
  await clickVisible(filters);
  const hideValidation = page.getByRole("button", { name: /Hide validation events/i });
  await clickVisible(hideValidation);
  const hideCommitAssociated = page.getByRole("button", { name: /Hide revision_commit_associated events/i });
  await clickVisible(hideCommitAssociated);

  if (hideObservations) {
    const hideObservation = page.getByRole("button", { name: /Hide observation events/i });
    await clickVisible(hideObservation);
  }

  await clickVisible(filters);

  const row = page
    .getByRole("listitem")
    .filter({ hasText: assessmentPattern })
    .filter({ hasText: revisionDisplay })
    .filter({ hasText: track })
    .first();
  await row.waitFor({ state: "visible" });
  await row.click();

  await page.getByRole("heading", { name: assessmentPattern }).waitFor({ state: "visible" });
  await page.waitForTimeout(100);

  const rowText = await row.innerText();
  const searchValue = await search.inputValue();
  if (!rowText.includes(revisionDisplay) || !rowText.includes(track)) {
    throw new Error(`selected row does not match revision ${revision} and track ${track}`);
  }
  if (searchValue !== query) {
    throw new Error(`search query changed during capture: ${searchValue}`);
  }

  const metrics = await page.evaluate(() => ({
    width: innerWidth,
    height: innerHeight,
    scrollWidth: document.documentElement.scrollWidth,
    theme: document.documentElement.dataset.theme,
  }));
  if (metrics.width !== 900 || metrics.height !== 506) {
    throw new Error(`unexpected viewport ${metrics.width}x${metrics.height}`);
  }
  if (metrics.scrollWidth !== metrics.width) {
    throw new Error(`horizontal overflow: ${metrics.scrollWidth}px in ${metrics.width}px viewport`);
  }
  return metrics;
}

await page.goto(baseUrl, { waitUntil: "domcontentloaded" });

await setDisplay("dark");
const darkMetrics = await prepareFrame();
if (darkMetrics.theme !== "dark") throw new Error(`expected dark theme, got ${darkMetrics.theme}`);
await page.screenshot({ path: darkPath, scale: "device", type: "png" });

await setDisplay("light");
const lightMetrics = await prepareFrame();
if (lightMetrics.theme !== "light") throw new Error(`expected light theme, got ${lightMetrics.theme}`);
await page.screenshot({ path: lightPath, scale: "device", type: "png" });

if (consoleErrors.length) {
  throw new Error(`browser console errors:\n${consoleErrors.join("\n")}`);
}
EOF
printf '})(%s)\n' "$CAPTURE_CONFIG" >> "$TMP_DIR/capture.mjs"

cat > "$TMP_DIR/verify.mjs" <<'EOF'
import fs from "node:fs";

const expectedSignature = "89504e470d0a1a0a";
for (const file of process.argv.slice(2)) {
  const bytes = fs.readFileSync(file);
  if (bytes.subarray(0, 8).toString("hex") !== expectedSignature) {
    throw new Error(`${file} is not a PNG`);
  }
  const width = bytes.readUInt32BE(16);
  const height = bytes.readUInt32BE(20);
  if (width !== 1800 || height !== 1012) {
    throw new Error(`${file} is ${width}x${height}; expected 1800x1012`);
  }
}
EOF

cat > "$TMP_DIR/manifest.mjs" <<'EOF'
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const [
  repoRoot,
  exampleManifestPath,
  darkPath,
  lightPath,
  finalDarkPath,
  finalLightPath,
  outputPath,
  producerVersion,
  producerCommit,
  hideObservations,
] = process.argv.slice(2);

const digest = bytes => crypto.createHash("sha256").update(bytes).digest("hex");
const relative = file => {
  const value = path.relative(repoRoot, file).split(path.sep).join("/");
  if (!value || value === ".." || value.startsWith("../")) {
    throw new Error(`${file} is outside ${repoRoot}`);
  }
  return value;
};
const png = file => {
  const bytes = fs.readFileSync(file);
  if (bytes.subarray(0, 8).toString("hex") !== "89504e470d0a1a0a") {
    throw new Error(`${file} is not a PNG`);
  }
  return {
    width: bytes.readUInt32BE(16),
    height: bytes.readUInt32BE(20),
    sha256: digest(bytes),
  };
};

const exampleBytes = fs.readFileSync(exampleManifestPath);
const example = JSON.parse(exampleBytes);
if (example.schema !== "pointbreak.review-example-pack" || example.version !== 1) {
  throw new Error("unsupported Review example pack manifest");
}
if (example.classification !== "reproducible_sample_record") {
  throw new Error(`unsupported example classification: ${example.classification}`);
}
if (example.record.verificationStatus !== "unsigned") {
  throw new Error(`expected unsigned example, got ${example.record.verificationStatus}`);
}
if (!Array.isArray(example.record.writerActors) || example.record.writerActors.length === 0) {
  throw new Error("example writerActors must be a non-empty array");
}
if (!/^[0-9a-f]{40}$/.test(producerCommit)) {
  throw new Error(`producer commit is not a full Git OID: ${producerCommit}`);
}

const hiddenEventTypes = ["validation"];
if (hideObservations === "true") hiddenEventTypes.unshift("observation");
const manifest = {
  schema: "com.withpointbreak.review-interface-capture/v2",
  producer: {
    name: "pointbreak",
    version: producerVersion,
    commit: producerCommit,
  },
  example: {
    name: example.name,
    classification: example.classification,
    manifestPath: relative(exampleManifestPath),
    manifestSha256: digest(exampleBytes),
  },
  record: {
    revision: example.record.revision,
    track: example.record.track,
    selectedAssessment: example.record.selectedAssessment,
    eventSetHash: example.record.eventSetHash,
    writerActors: example.record.writerActors,
    verificationStatus: example.record.verificationStatus,
    reproducibleFromPublicPack: true,
    publiclyInspectable: false,
    redactions: [],
  },
  capture: {
    viewport: { width: 900, height: 506 },
    deviceScaleFactor: 2,
    density: "comfortable",
    sort: "newest_first",
    hiddenEventTypes,
  },
  assets: {
    dark: { path: relative(finalDarkPath), ...png(darkPath) },
    light: { path: relative(finalLightPath), ...png(lightPath) },
  },
};

fs.writeFileSync(outputPath, `${JSON.stringify(manifest, null, 2)}\n`);
EOF

echo "Capturing Pointbreak Review screenshots"
note "inspector : $BASE_URL"
note "query     : revision:$REVISION track:$TRACK"
note "selected  : $ASSESSMENT"
note "viewport  : 900x506 @ 2x"

run_pw open "$BASE_URL" --config "$TMP_DIR/cli.config.json" >/dev/null
if ! run_pw run-code --filename="$TMP_DIR/capture.mjs" >"$TMP_DIR/run-code.log" 2>&1; then
  cat "$TMP_DIR/run-code.log" >&2
  die "Playwright command failed"
fi
if grep -q '^### Error' "$TMP_DIR/run-code.log"; then
  cat "$TMP_DIR/run-code.log" >&2
  die "Playwright capture failed"
fi

DARK_CAPTURE="$TMP_DIR/shore-inspector-dark.png"
LIGHT_CAPTURE="$TMP_DIR/shore-inspector-light.png"
node "$TMP_DIR/verify.mjs" "$DARK_CAPTURE" "$LIGHT_CAPTURE"

if [ -z "$EXAMPLE_MANIFEST" ]; then
  cp "$DARK_CAPTURE" "$OUT_DIR/shore-inspector-dark.png"
  cp "$LIGHT_CAPTURE" "$OUT_DIR/shore-inspector-light.png"
else
  MANIFEST_DIR="$(dirname "$MANIFEST")"
  mkdir -p "$MANIFEST_DIR"
  OUT_DIR="$(cd "$OUT_DIR" && pwd)"
  MANIFEST_DIR="$(cd "$MANIFEST_DIR" && pwd)"
  MANIFEST="$MANIFEST_DIR/$(basename "$MANIFEST")"

  PRODUCER_COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD)"
  PRODUCER_VERSION="$(cargo metadata --no-deps --format-version=1 \
    | node -e 'let value=""; process.stdin.on("data", chunk => value += chunk); process.stdin.on("end", () => { const data = JSON.parse(value); const root = data.packages.find(pkg => pkg.manifest_path === process.argv[1]); if (!root) throw new Error("Pointbreak package not found"); process.stdout.write(root.version); });' "$REPO_ROOT/Cargo.toml")"

  STAGED_DARK="$(mktemp "$OUT_DIR/.shore-inspector-dark.XXXXXX.png")"
  STAGED_LIGHT="$(mktemp "$OUT_DIR/.shore-inspector-light.XXXXXX.png")"
  STAGED_MANIFEST="$(mktemp "$MANIFEST_DIR/.review-interface-capture.XXXXXX.json")"
  cp "$DARK_CAPTURE" "$STAGED_DARK"
  cp "$LIGHT_CAPTURE" "$STAGED_LIGHT"
  node "$TMP_DIR/manifest.mjs" \
    "$REPO_ROOT" "$EXAMPLE_MANIFEST" "$STAGED_DARK" "$STAGED_LIGHT" \
    "$OUT_DIR/shore-inspector-dark.png" "$OUT_DIR/shore-inspector-light.png" \
    "$STAGED_MANIFEST" "$PRODUCER_VERSION" "$PRODUCER_COMMIT" "$HIDE_OBSERVATIONS"

  # The lock moves last. An interruption during these three operations is
  # detected by the integrity test rather than blessing a mixed capture.
  mv "$STAGED_DARK" "$OUT_DIR/shore-inspector-dark.png"
  mv "$STAGED_LIGHT" "$OUT_DIR/shore-inspector-light.png"
  mv "$STAGED_MANIFEST" "$MANIFEST"
fi

note "tmp dir   : $TMP_DIR"
note "dark      : $OUT_DIR/shore-inspector-dark.png"
note "light     : $OUT_DIR/shore-inspector-light.png"
note "result    : 1800x1012 PNG pair"
[ -z "$MANIFEST" ] || note "manifest  : $MANIFEST"
