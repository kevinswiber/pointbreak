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

while [ $# -gt 0 ]; do
  case "$1" in
    --url) BASE_URL="$2"; shift 2 ;;
    --revision) REVISION="$2"; shift 2 ;;
    --track) TRACK="$2"; shift 2 ;;
    --assessment) ASSESSMENT="$2"; shift 2 ;;
    --out-dir) OUT_DIR="$2"; shift 2 ;;
    -h|--help) show_help; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

command -v curl >/dev/null 2>&1 || die "curl not found"
command -v node >/dev/null 2>&1 || die "node not found"

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

run_pw() {
  (cd "$TMP_DIR" && "${PWCLI[@]}" -s="$SESSION" "$@")
}

cleanup() {
  run_pw close >/dev/null 2>&1 || true
  rm -rf "$TMP_DIR"
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
const [baseUrl, revision, track, assessment, darkPath, lightPath] = process.argv.slice(1);
process.stdout.write(JSON.stringify({ baseUrl, revision, track, assessment, darkPath, lightPath }));
' "$BASE_URL" "$REVISION" "$TRACK" "$ASSESSMENT" \
  "$TMP_DIR/shore-inspector-dark.png" "$TMP_DIR/shore-inspector-light.png")"

cat > "$TMP_DIR/capture.mjs" <<'EOF'
((config) => async page => {
const { baseUrl, revision, track, assessment, darkPath, lightPath } = config;
const consoleErrors = [];

page.on("console", (message) => {
  if (message.type() === "error") consoleErrors.push(message.text());
});

const query = `revision:${revision} track:${track}`;
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

async function prepareFrame() {
  const search = page.getByRole("searchbox", { name: /search/i });
  await search.waitFor({ state: "visible" });
  await search.fill(query);

  const hideValidation = page.getByRole("button", { name: /Hide validation events/i });
  if (await hideValidation.count()) await hideValidation.click();

  const row = page
    .getByRole("listitem")
    .filter({ hasText: assessmentPattern })
    .filter({ hasText: revision })
    .filter({ hasText: track })
    .first();
  await row.waitFor({ state: "visible" });
  await row.click();

  await page.getByRole("heading", { name: assessmentPattern }).waitFor({ state: "visible" });
  await page.waitForTimeout(100);

  const rowText = await row.innerText();
  const searchValue = await search.inputValue();
  if (!rowText.includes(revision) || !rowText.includes(track)) {
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

echo "Capturing Pointbreak Review README screenshots"
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

cp "$DARK_CAPTURE" "$OUT_DIR/shore-inspector-dark.png"
cp "$LIGHT_CAPTURE" "$OUT_DIR/shore-inspector-light.png"

note "dark      : $OUT_DIR/shore-inspector-dark.png"
note "light     : $OUT_DIR/shore-inspector-light.png"
note "result    : 1800x1012 PNG pair"
