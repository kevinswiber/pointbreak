#!/usr/bin/env bash
#
# Trigger the Release Plan workflow, wait for it to complete, download
# the report, and print it to stdout.
#
# Usage:
#   ./scripts/run-release-plan.sh                            # plan mode (default)
#   ./scripts/run-release-plan.sh plan 0.1.0                 # plan exact version
#   ./scripts/run-release-plan.sh release                    # release mode
#   ./scripts/run-release-plan.sh release 0.1.0              # release exact version
#   ./scripts/run-release-plan.sh plan -- glow               # custom viewer
#   ./scripts/run-release-plan.sh plan -- bat --paging=always # custom bat flags
#   RELEASE_PLAN_DIR=. ./scripts/run-release-plan.sh          # keep release-plan.md in cwd
#   RELEASE_PLAN_DIR=. ./scripts/run-release-plan.sh -- open  # open in default app
#
set -euo pipefail

MODE="plan"
VERSION=""
VIEWER=()

parsing_opts=true
pos=0
for arg in "$@"; do
  if [ "$arg" = "--" ]; then
    parsing_opts=false
    continue
  fi
  if $parsing_opts; then
    case $pos in
      0) MODE="$arg" ;;
      1) VERSION="$arg" ;;
    esac
    pos=$((pos + 1))
  else
    VIEWER+=("$arg")
  fi
done

if [ ${#VIEWER[@]} -eq 0 ]; then
  if command -v bat &>/dev/null; then
    VIEWER=(bat --paging=never)
  else
    VIEWER=(cat)
  fi
fi

REPO="kevinswiber/pointbreak"
WORKFLOW="release-plan.yml"

if [ -n "${RELEASE_PLAN_DIR:-}" ]; then
  OUTDIR="$RELEASE_PLAN_DIR"
  mkdir -p "$OUTDIR"
else
  OUTDIR=$(mktemp -d)
  trap 'rm -rf "$OUTDIR"' EXIT
fi

flags=(-f "mode=${MODE}")
if [ -n "$VERSION" ]; then
  flags+=(-f "version=${VERSION}")
fi

echo "Dispatching Release Plan workflow (mode=${MODE}, version=${VERSION:-auto})..."
gh workflow run "$WORKFLOW" --repo "$REPO" "${flags[@]}"

sleep 3

RUN_ID=$(gh run list --repo "$REPO" --workflow "$WORKFLOW" --limit 1 \
  --json databaseId --jq '.[0].databaseId')

if [ -z "$RUN_ID" ]; then
  echo "error: could not find workflow run" >&2
  exit 1
fi

echo "Waiting for run ${RUN_ID}..."
gh run watch "$RUN_ID" --repo "$REPO" --exit-status || {
  echo "error: workflow run failed" >&2
  gh run view "$RUN_ID" --repo "$REPO"
  exit 1
}

echo ""

gh run download "$RUN_ID" --repo "$REPO" --name release-plan --dir "$OUTDIR"

if [ -f "$OUTDIR/release-plan.md" ]; then
  echo "To re-download: gh run download ${RUN_ID} -n release-plan"
  echo ""
  "${VIEWER[@]}" "$OUTDIR/release-plan.md"
else
  echo "error: release-plan.md not found in artifacts" >&2
  exit 1
fi
