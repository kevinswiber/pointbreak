#!/usr/bin/env bash

set -euo pipefail

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

command -v git >/dev/null 2>&1 || die "git is required"
command -v jq >/dev/null 2>&1 || die "jq is required"
command -v node >/dev/null 2>&1 || die "node is required"
command -v rg >/dev/null 2>&1 || die "rg is required"

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
pointbreak_binary="${POINTBREAK_BINARY:-$repo_root/target/debug/pointbreak}"
browser_program_template="$script_dir/verify-inspector-decision-continuity.mjs"

[ -x "$pointbreak_binary" ] \
  || die "POINTBREAK_BINARY is not executable; provide an installed binary or build the worktree-local binary"
case "$pointbreak_binary" in
  /* | [A-Za-z]:/* | [A-Za-z]:\\* | \\\\*) ;;
  *) die "POINTBREAK_BINARY must resolve to an absolute path" ;;
esac
[ -f "$browser_program_template" ] \
  || die "browser program is missing: $browser_program_template"

root=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --root) root="$2"; shift 2 ;;
    -h|--help)
      printf 'usage: %s [--root <empty-directory>]\n' "$0"
      exit 0
      ;;
    *) die "unknown option: $1" ;;
  esac
done

[ -z "$(git -C "$repo_root" status --porcelain --untracked-files=all)" ] \
  || die "Pointbreak source worktree must be clean so the gate names the exact tested commit"

if [ -z "$root" ]; then
  root="$(mktemp -d)"
elif [ -e "$root" ]; then
  [ -d "$root" ] || die "root exists and is not a directory: $root"
  [ -z "$(find "$root" -mindepth 1 -maxdepth 1 -print -quit)" ] \
    || die "root is not empty: $root"
else
  mkdir -p "$root"
fi
root="$(cd "$root" && pwd -P)"
case "$root" in
  "$repo_root"|"$repo_root"/*) die "root must be outside the Pointbreak source worktree" ;;
esac

canonical_destination="$root/canonical"
synthetic_destination="$root/synthetic"
pointbreak_home="$root/pointbreak-home"
artifact_dir="$root/browser-artifacts"
log_dir="$root/logs"
mkdir -p \
  "$canonical_destination" \
  "$synthetic_destination" \
  "$pointbreak_home" \
  "$artifact_dir" \
  "$log_dir"
export POINTBREAK_HOME="$pointbreak_home"

source_commit="$(git -C "$repo_root" rev-parse HEAD)"
binary_sha256="$(shasum -a 256 "$pointbreak_binary" | awk '{print $1}')"
"$pointbreak_binary" version --format json >"$log_dir/pointbreak-version.json"

(
  cd "$repo_root"
  just review-example-materialize "$canonical_destination"
) >"$log_dir/canonical-materialize.log" 2>&1
(
  cd "$repo_root"
  POINTBREAK_BINARY="$pointbreak_binary" \
    just review-decision-matrix-materialize "$synthetic_destination"
) >"$log_dir/synthetic-ids.json" 2>"$log_dir/synthetic-materialize.log"

"$pointbreak_binary" revision list --repo "$canonical_destination" --format json \
  >"$log_dir/canonical-revisions.json"
"$pointbreak_binary" revision list --repo "$synthetic_destination" --format json \
  >"$log_dir/synthetic-revisions.json"
"$pointbreak_binary" store paths --repo "$canonical_destination" --format json \
  >"$log_dir/canonical-store-paths.json"
"$pointbreak_binary" store paths --repo "$synthetic_destination" --format json \
  >"$log_dir/synthetic-store-paths.json"

canonical_revision="$(jq -er '.entries | select(length == 1) | .[0].revisionId' "$log_dir/canonical-revisions.json")"
canonical_object="$(jq -er '.entries | select(length == 1) | .[0].objectId' "$log_dir/canonical-revisions.json")"
primary_revision="$(jq -er '.primary_revision' "$log_dir/synthetic-ids.json")"
primary_object="$(jq -er --arg revision "$primary_revision" '.entries[] | select(.revisionId == $revision) | .objectId' "$log_dir/synthetic-revisions.json")"

for store_file in canonical-store-paths.json synthetic-store-paths.json; do
  common_store="$(jq -er '.commonStore' "$log_dir/$store_file")"
  case "$common_store" in
    "$canonical_destination"/*|"$synthetic_destination"/*) ;;
    *) die "materialized store escaped the disposable repositories: $common_store" ;;
  esac
done
[ ! -e "$synthetic_destination/.git/pointbreak-home" ] \
  || die "synthetic key home ignored the explicit POINTBREAK_HOME"

canonical_pid=""
synthetic_pid=""
session="pointbreak-decision-browser-$$"
pwcli=()
if [ -n "${PLAYWRIGHT_CLI:-}" ]; then
  pwcli=("$PLAYWRIGHT_CLI")
elif command -v playwright-cli >/dev/null 2>&1; then
  pwcli=(playwright-cli)
else
  command -v npx >/dev/null 2>&1 || die "playwright-cli and npx are unavailable"
  pwcli=(npx --yes --package @playwright/cli@0.1.17 playwright-cli)
fi

run_pw() {
  (cd "$artifact_dir" && "${pwcli[@]}" -s="$session" "$@")
}

cleanup() {
  run_pw close >/dev/null 2>&1 || true
  [ -z "$canonical_pid" ] || kill "$canonical_pid" >/dev/null 2>&1 || true
  [ -z "$synthetic_pid" ] || kill "$synthetic_pid" >/dev/null 2>&1 || true
}
trap cleanup EXIT

start_inspector() {
  local name="$1"
  local repository="$2"
  local startup="$log_dir/$name-startup.json"
  local server_log="$log_dir/$name-server.log"
  POINTBREAK_HOME="$pointbreak_home" \
    "$pointbreak_binary" inspect --repo "$repository" --port 0 --format json \
    >"$startup" 2>"$server_log" &
  local pid=$!
  for _ in $(seq 1 100); do
    [ -s "$startup" ] && break
    kill -0 "$pid" >/dev/null 2>&1 || die "$name inspector exited before startup"
    sleep 0.05
  done
  jq -e '.schema == "pointbreak.inspect-startup" and .version == 1 and (.port > 0) and (.token | length > 0)' \
    "$startup" >/dev/null || die "$name inspector did not emit valid JSON startup"
  printf '%s\n' "$pid"
}

canonical_pid="$(start_inspector canonical "$canonical_destination")"
synthetic_pid="$(start_inspector synthetic "$synthetic_destination")"

server_config() {
  local name="$1"
  jq -c '{baseUrl: ("http://" + .host + ":" + (.port | tostring)), token}' \
    "$log_dir/$name-startup.json"
}
canonical_server="$(server_config canonical)"
synthetic_server="$(server_config synthetic)"

export POINTBREAK_BROWSER_GATE_CONFIG
POINTBREAK_BROWSER_GATE_CONFIG="$(
  jq -cn \
    --arg artifactDir "$artifact_dir" \
    --arg canonicalRevision "$canonical_revision" \
    --arg canonicalObject "$canonical_object" \
    --arg primaryObject "$primary_object" \
    --argjson canonical "$canonical_server" \
    --argjson synthetic "$synthetic_server" \
    --slurpfile ids "$log_dir/synthetic-ids.json" \
    '{
      artifactDir: $artifactDir,
      canonical: ($canonical + {revisionId: $canonicalRevision, objectId: $canonicalObject}),
      synthetic: ($synthetic + {ids: $ids[0], primaryObjectId: $primaryObject})
    }'
)"

browser_program="$log_dir/browser-program.mjs"
node -e '
const fs = require("node:fs");
const source = fs.readFileSync(process.argv[1], "utf8");
const config = JSON.parse(process.argv[2]);
const marker = "__POINTBREAK_BROWSER_GATE_CONFIG__";
if (!source.includes(marker)) throw new Error("browser config marker is missing");
fs.writeFileSync(process.argv[3], source.replace(marker, JSON.stringify(config)));
' "$browser_program_template" "$POINTBREAK_BROWSER_GATE_CONFIG" "$browser_program"

canonical_url="$(jq -r '.canonical.baseUrl + "/#/list?token=" + (.canonical.token | @uri)' <<<"$POINTBREAK_BROWSER_GATE_CONFIG")"
run_pw open "$canonical_url" >"$log_dir/browser-open.log" 2>&1
if ! run_pw run-code --filename="$browser_program" >"$log_dir/browser-gate.log" 2>&1; then
  sed -n '1,240p' "$log_dir/browser-gate.log" >&2
  die "real-browser gate failed"
fi
if rg -q '^### Error' "$log_dir/browser-gate.log"; then
  sed -n '1,240p' "$log_dir/browser-gate.log" >&2
  die "real-browser gate reported an error"
fi

screenshot_count="$(find "$artifact_dir" -maxdepth 1 -type f -name '*.png' | wc -l | tr -d ' ')"
[ "$screenshot_count" -eq 12 ] || die "expected 12 browser screenshots, found $screenshot_count"

jq -n \
  --arg sourceCommit "$source_commit" \
  --arg binary "$pointbreak_binary" \
  --arg binarySha256 "$binary_sha256" \
  --arg root "$root" \
  --arg canonicalRevision "$canonical_revision" \
  --arg syntheticRevision "$primary_revision" \
  --argjson screenshotCount "$screenshot_count" \
  '{
    gate: "review-decision-continuity",
    status: "passed",
    sourceCommit: $sourceCommit,
    binary: $binary,
    binarySha256: $binarySha256,
    root: $root,
    canonicalRevision: $canonicalRevision,
    syntheticRevision: $syntheticRevision,
    viewports: [
      {name: "wide", width: 1440, height: 1000, density: "comfortable"},
      {name: "compact", width: 900, height: 506, density: "compact"},
      {name: "narrow", width: 390, height: 844, density: "comfortable"}
    ],
    screenshotCount: $screenshotCount,
    consoleErrors: 0,
    horizontalOverflowFailures: 0
  }' | tee "$log_dir/result.json"
