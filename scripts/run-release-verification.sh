#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 <tag> --expected-source <full-sha> [--output <directory>]" >&2
  exit 2
}

[ "$#" -ge 3 ] || usage
TAG="$1"
shift
[ "${1:-}" = "--expected-source" ] || usage
EXPECTED_SOURCE_COMMIT="${2:-}"
shift 2
[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || usage
[[ "$EXPECTED_SOURCE_COMMIT" =~ ^[0-9a-f]{40}$ ]] || usage

OUTPUT=""
if [ "$#" -gt 0 ]; then
  if [ "$1" != "--output" ] || [ "$#" -ne 2 ]; then
    usage
  fi
  OUTPUT="$2"
fi

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(git -C "$SCRIPT_DIR/.." rev-parse --show-toplevel)
remote_url=$(git -C "$REPO_ROOT" remote get-url origin)
remote_url=${remote_url#git@github.com:}
remote_url=${remote_url#https://github.com/}
REPO=${remote_url%.git}
[[ "$REPO" =~ ^[^/]+/[^/]+$ ]] || { echo "invalid GitHub origin" >&2; exit 1; }

gh workflow run verify-release.yml --repo "$REPO" \
  -f "tag=${TAG}" \
  -f "expected_source_commit=${EXPECTED_SOURCE_COMMIT}"
sleep 3
RUN_ID=$(gh run list --repo "$REPO" --workflow verify-release.yml --limit 1 \
  --json databaseId --jq '.[0].databaseId')
[ -n "$RUN_ID" ] || { echo "could not find verification run" >&2; exit 1; }
gh run watch "$RUN_ID" --repo "$REPO" --exit-status

if [ -n "$OUTPUT" ]; then
  mkdir -p "$OUTPUT"
  gh run download "$RUN_ID" --repo "$REPO" --pattern 'release-verification-*' --dir "$OUTPUT"
  echo "Downloaded release verification to $OUTPUT"
else
  echo "Verification run: https://github.com/${REPO}/actions/runs/${RUN_ID}"
fi
