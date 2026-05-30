#!/usr/bin/env bash
#
# Snapshot a git worktree plus its `.shore` review data into a self-contained
# Shoreline test fixture. Worktrees come and go; this preserves the review
# context (the exact tree that was reviewed, the base it was reviewed against,
# and the captured `.shore` store) so it survives the worktree's deletion.
#
# The fixture is a standalone git repo (origin removed) so git-based Shoreline
# behavior keeps working without the source repo present. Build artifacts
# (`target/`) are excluded by default. `.shore` is copied verbatim.
#
# Fixtures default to a location OUTSIDE this repo, since captured review data
# may be private. Do not commit fixtures into the source tree.
#
# Usage:
#   scripts/worktree-to-fixture.sh <worktree-path> [options]
#
# Options:
#   --out <dir>          Fixtures root. Default: $SHORELINE_FIXTURES_DIR or
#                        ~/src/shoreline-fixtures
#   --name <name>        Fixture dir name. Default: basename of the worktree.
#   --base <ref>         Review base. Default: merge-base of the worktree HEAD
#                        with the source repo's default branch.
#   --tool-repo <dir>    Shoreline repo used for the review. Default: this
#                        script's repo root.
#   --tool-commit <sha>  Shoreline tool commit. Default: HEAD of --tool-repo.
#   --pr <num>           Associated Shoreline PR number (recorded; looked up
#                        via `gh` if available).
#   --include-target     Include target/ build artifacts (default: excluded).
#   --force              Overwrite an existing fixture directory.
#   -h, --help           Show this help.
#
# Examples:
#   scripts/worktree-to-fixture.sh ~/worktrees/boardwalk/plan-0006-job-runner-api-simplification
#   scripts/worktree-to-fixture.sh ../wt/feat-x --name review-feat-x --pr 109
#
set -euo pipefail

die() { printf 'error: %s\n' "$*" >&2; exit 1; }
note() { printf '  %s\n' "$*"; }

show_help() { sed -n '2,/^set -euo pipefail/p' "$0" | sed 's/^# \{0,1\}//; s/^#$//' | sed '$d'; }

# ---- defaults -------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

OUT_DIR="${SHORELINE_FIXTURES_DIR:-$HOME/src/shoreline-fixtures}"
NAME=""
BASE_REF=""
TOOL_REPO="$REPO_ROOT"
TOOL_COMMIT=""
PR_NUM=""
INCLUDE_TARGET=0
FORCE=0
WT=""

# ---- args -----------------------------------------------------------------
while [ $# -gt 0 ]; do
  case "$1" in
    -h|--help) show_help; exit 0 ;;
    --out) OUT_DIR="$2"; shift 2 ;;
    --name) NAME="$2"; shift 2 ;;
    --base) BASE_REF="$2"; shift 2 ;;
    --tool-repo) TOOL_REPO="$2"; shift 2 ;;
    --tool-commit) TOOL_COMMIT="$2"; shift 2 ;;
    --pr) PR_NUM="$2"; shift 2 ;;
    --include-target) INCLUDE_TARGET=1; shift ;;
    --force) FORCE=1; shift ;;
    --) shift ;;
    -*) die "unknown option: $1" ;;
    *) [ -z "$WT" ] || die "unexpected argument: $1"; WT="$1"; shift ;;
  esac
done

[ -n "$WT" ] || { show_help; exit 2; }
command -v git >/dev/null 2>&1 || die "git not found"
command -v rsync >/dev/null 2>&1 || die "rsync not found"
[ -d "$WT" ] || die "worktree path not found: $WT"
WT="$(cd "$WT" && pwd)"
git -C "$WT" rev-parse --is-inside-work-tree >/dev/null 2>&1 || die "not a git worktree: $WT"

[ -n "$NAME" ] || NAME="$(basename "$WT")"
DEST="$OUT_DIR/$NAME"

# ---- gather facts from the source worktree --------------------------------
MAIN_WT="$(git -C "$WT" worktree list --porcelain | awk '/^worktree /{print $2; exit}')"
COMMON_DIR="$(git -C "$WT" rev-parse --path-format=absolute --git-common-dir)"
HEAD_COMMIT="$(git -C "$WT" rev-parse HEAD)"
HEAD_SUBJECT="$(git -C "$WT" log -1 --format=%s "$HEAD_COMMIT")"
BRANCH="$(git -C "$WT" rev-parse --abbrev-ref HEAD)"   # "HEAD" if detached

# Detect the source repo's default branch (for base merge-base detection).
DEFAULT_BRANCH=""
if dref="$(git -C "$WT" symbolic-ref --quiet refs/remotes/origin/HEAD 2>/dev/null)"; then
  DEFAULT_BRANCH="${dref#refs/remotes/origin/}"
fi
if [ -z "$DEFAULT_BRANCH" ]; then
  for b in main master trunk; do
    if git -C "$WT" rev-parse --verify --quiet "$b" >/dev/null 2>&1 \
       || git -C "$WT" rev-parse --verify --quiet "origin/$b" >/dev/null 2>&1; then
      DEFAULT_BRANCH="$b"; break
    fi
  done
fi
[ -n "$DEFAULT_BRANCH" ] || DEFAULT_BRANCH="$(git -C "$MAIN_WT" rev-parse --abbrev-ref HEAD 2>/dev/null || echo main)"

if [ -z "$BASE_REF" ]; then
  BASE_COMMIT="$(git -C "$WT" merge-base "$HEAD_COMMIT" "$DEFAULT_BRANCH" 2>/dev/null || true)"
  [ -n "$BASE_COMMIT" ] || BASE_COMMIT="$(git -C "$WT" rev-parse "origin/$DEFAULT_BRANCH" 2>/dev/null || true)"
else
  BASE_COMMIT="$(git -C "$WT" rev-parse "$BASE_REF")"
fi
[ -n "$BASE_COMMIT" ] || die "could not determine review base; pass --base <ref>"
BASE_SUBJECT="$(git -C "$WT" log -1 --format=%s "$BASE_COMMIT" 2>/dev/null || echo '(unknown)')"

DIRTY="clean"
[ -n "$(git -C "$WT" status --porcelain=v1 2>/dev/null)" ] && DIRTY="dirty (uncommitted changes preserved)"

# Shoreline tool provenance
if [ -z "$TOOL_COMMIT" ] && { [ -d "$TOOL_REPO/.git" ] || [ -f "$TOOL_REPO/.git" ]; }; then
  TOOL_COMMIT="$(git -C "$TOOL_REPO" rev-parse HEAD 2>/dev/null || true)"
fi
TOOL_SUBJECT=""
[ -n "$TOOL_COMMIT" ] && TOOL_SUBJECT="$(git -C "$TOOL_REPO" log -1 --format=%s "$TOOL_COMMIT" 2>/dev/null || true)"

PR_LINE=""
if [ -n "$PR_NUM" ]; then
  if command -v gh >/dev/null 2>&1; then
    PR_JSON="$( (cd "$TOOL_REPO" && gh pr view "$PR_NUM" --json number,title,headRefName,baseRefName,state,url 2>/dev/null) || true)"
    if [ -n "$PR_JSON" ] && command -v jq >/dev/null 2>&1; then
      PR_LINE="$(printf '%s' "$PR_JSON" | jq -r '"#\(.number) \"\(.title)\" (branch \(.headRefName), base \(.baseRefName), \(.state)) \(.url)"')"
    fi
  fi
  [ -n "$PR_LINE" ] || PR_LINE="#$PR_NUM"
fi

# ---- pre-flight -----------------------------------------------------------
RESOLVED_OUT="$(cd "$OUT_DIR" 2>/dev/null && pwd || echo "$OUT_DIR")"
case "$RESOLVED_OUT" in
  "$REPO_ROOT"|"$REPO_ROOT"/*)
    printf 'warning: output is inside the source repo (%s).\n' "$REPO_ROOT" >&2
    printf 'warning: fixtures may be private; do not commit them. Continuing.\n' >&2 ;;
esac

if [ -e "$DEST" ]; then
  [ "$FORCE" -eq 1 ] || die "fixture already exists: $DEST (use --force to overwrite)"
  rm -rf "$DEST"
fi
mkdir -p "$OUT_DIR"

echo "Migrating worktree -> fixture"
note "source worktree : $WT ($DIRTY)"
note "source repo      : $MAIN_WT (default branch: $DEFAULT_BRANCH)"
note "head (reviewed)  : ${HEAD_COMMIT:0:12}  $HEAD_SUBJECT"
note "base (review)    : ${BASE_COMMIT:0:12}  $BASE_SUBJECT"
note "branch           : $BRANCH"
note "tool             : ${TOOL_COMMIT:0:12}  $TOOL_SUBJECT"
[ -n "$PR_LINE" ] && note "pr               : $PR_LINE"
note "destination      : $DEST"

# ---- build the standalone repo --------------------------------------------
# Clone history independently (no hardlinks/local shortcuts) without a checkout;
# we populate the working tree from the worktree directly so dirty state and
# untracked files are preserved faithfully.
git clone --quiet --no-checkout --no-local --no-hardlinks "$MAIN_WT" "$DEST"

# Make sure the reviewed and base commits are present (handles detached/odd HEADs).
git -C "$DEST" cat-file -e "$HEAD_COMMIT" 2>/dev/null || git -C "$DEST" fetch --quiet --no-tags "$MAIN_WT" "$HEAD_COMMIT" 2>/dev/null || true
git -C "$DEST" cat-file -e "$BASE_COMMIT" 2>/dev/null || git -C "$DEST" fetch --quiet --no-tags "$MAIN_WT" "$BASE_COMMIT" 2>/dev/null || true
git -C "$DEST" cat-file -e "$HEAD_COMMIT" >/dev/null 2>&1 || die "reviewed commit $HEAD_COMMIT not found in clone"

# Point HEAD at the reviewed commit (named branch when possible). Detach first
# so we can force the target branch even when the clone has it checked out.
git -C "$DEST" update-ref --no-deref HEAD "$HEAD_COMMIT"
if [ "$BRANCH" != "HEAD" ]; then
  git -C "$DEST" branch -f "$BRANCH" "$HEAD_COMMIT"
  git -C "$DEST" symbolic-ref HEAD "refs/heads/$BRANCH"
fi
git -C "$DEST" reset -q   # index -> HEAD; working tree untouched (still empty)

# Populate the working tree exactly as the worktree has it (incl. .shore).
RSYNC_EXCLUDES=(--exclude='/.git')
[ "$INCLUDE_TARGET" -eq 1 ] || RSYNC_EXCLUDES+=(--exclude='/target/')
rsync -a "${RSYNC_EXCLUDES[@]}" "$WT/" "$DEST/"

# Detach from the source repo so the fixture stands alone.
git -C "$DEST" remote remove origin 2>/dev/null || true
[ "$BRANCH" != "HEAD" ] && git -C "$DEST" branch --unset-upstream 2>/dev/null || true

# Replicate the source repo's local excludes so `.shore` (and friends) stay
# ignored as in the original, then exclude this fixture's metadata file.
DEST_EXCLUDE="$DEST/.git/info/exclude"
: > "$DEST_EXCLUDE"
if [ -f "$COMMON_DIR/info/exclude" ]; then
  cat "$COMMON_DIR/info/exclude" >> "$DEST_EXCLUDE"
fi
grep -qxF '.shore' "$DEST_EXCLUDE" 2>/dev/null || printf '%s\n' '.shore' >> "$DEST_EXCLUDE"
grep -qxF 'FIXTURE.md' "$DEST_EXCLUDE" 2>/dev/null || printf '%s\n' 'FIXTURE.md' >> "$DEST_EXCLUDE"

# ---- .shore summary -------------------------------------------------------
SHORE_DIR="$DEST/.shore"
SHORE_SUMMARY="(no .shore directory found in worktree)"
if [ -d "$SHORE_DIR" ]; then
  SJ="$SHORE_DIR/state.json"
  EVENTS_N="$(ls "$SHORE_DIR/events" 2>/dev/null | wc -l | tr -d ' ')"
  if [ -f "$SJ" ] && command -v jq >/dev/null 2>&1; then
    SHORE_SUMMARY="$(jq -r '"\(.eventCount) events, \(.reviewUnitCount) review units, \(.observationCount) observations, \(.assessmentCount) assessments, \(.inputRequestCount) input requests" + (if (.diagnostics // []) | length > 0 then "; diagnostics: " + ((.diagnostics | map(.code)) | join(", ")) else "" end) + "\n  eventSetHash: \(.eventSetHash)"' "$SJ")"
  elif [ -f "$SJ" ] && command -v python3 >/dev/null 2>&1; then
    SHORE_SUMMARY="$(python3 - "$SJ" <<'PY'
import json,sys
d=json.load(open(sys.argv[1]))
diag=", ".join(x.get("code","") for x in d.get("diagnostics",[]))
s="%s events, %s review units, %s observations, %s assessments, %s input requests"%(
  d.get("eventCount"),d.get("reviewUnitCount"),d.get("observationCount"),d.get("assessmentCount"),d.get("inputRequestCount"))
if diag: s+="; diagnostics: "+diag
s+="\n  eventSetHash: %s"%d.get("eventSetHash")
print(s)
PY
)"
  else
    SHORE_SUMMARY="$EVENTS_N events (install jq or python3 for full stats)"
  fi
fi

# ---- FIXTURE.md -----------------------------------------------------------
TODAY="$(date +%Y-%m-%d)"
TOOL_ORIGIN="$(git -C "$TOOL_REPO" remote get-url origin 2>/dev/null || echo '(unknown)')"
{
  printf '# Fixture: %s\n\n' "$NAME"
  printf 'A snapshot of a git worktree plus its `.shore` review data, captured for\n'
  printf 'testing Shoreline. The source worktree was *copied*, so this fixture stays\n'
  printf 'valid after the original worktree is deleted.\n\n'
  printf '> Excluded from the git working tree (see `.git/info/exclude`) so `git status`\n'
  printf '> stays clean for tests that re-run git.\n\n'
  printf '## Commit map\n\n'
  printf '| Role | Commit | Subject |\n| --- | --- | --- |\n'
  printf '| Reviewed head (HEAD, branch `%s`) | `%s` | %s |\n' "$BRANCH" "$HEAD_COMMIT" "$HEAD_SUBJECT"
  printf '| Review base (`%s`) | `%s` | %s |\n' "$DEFAULT_BRANCH" "$BASE_COMMIT" "$BASE_SUBJECT"
  if [ -n "$TOOL_COMMIT" ]; then
    printf '| Shoreline tool build | `%s` | %s |\n' "$TOOL_COMMIT" "$TOOL_SUBJECT"
  fi
  printf '\nReview diff: `%s..%s`.\n\n' "${BASE_COMMIT:0:12}" "${HEAD_COMMIT:0:12}"
  printf '## Provenance\n\n'
  printf -- '- Source repo: `%s` (default branch `%s`)\n' "$MAIN_WT" "$DEFAULT_BRANCH"
  printf -- '- Source worktree (copied from): `%s`\n' "$WT"
  printf -- '- Worktree state at capture: %s\n' "$DIRTY"
  [ -n "$TOOL_COMMIT" ] && printf -- '- Shoreline tool repo: `%s` (%s)\n' "$TOOL_REPO" "$TOOL_ORIGIN"
  [ -n "$PR_LINE" ] && printf -- '- Associated PR: %s\n' "$PR_LINE"
  printf -- '- Captured into fixture: %s\n' "$TODAY"
  printf -- '- target/ build artifacts: %s\n\n' "$([ "$INCLUDE_TARGET" -eq 1 ] && echo included || echo excluded)"
  printf '## .shore review data\n\n'
  printf -- '- %s\n\n' "$SHORE_SUMMARY"
  printf '## Notes\n\n'
  printf -- '- Standalone git repo (origin removed): `git log`/`diff`/`rev-parse`/`status` work without the source repo present.\n'
  printf -- '- Built by `scripts/worktree-to-fixture.sh`.\n'
} > "$DEST/FIXTURE.md"

# ---- top-level README index ----------------------------------------------
INDEX="$OUT_DIR/README.md"
if [ ! -f "$INDEX" ]; then
  {
    printf '# shoreline-fixtures\n\n'
    printf 'Local snapshots of review sessions for testing Shoreline. Each fixture is a\n'
    printf 'standalone copy of a source worktree plus its `.shore` data; see each\n'
    printf "fixture's \`FIXTURE.md\` for provenance. These may contain private review data\n"
    printf -- '— do not commit them into the Shoreline source repo.\n\n'
    printf '## Fixtures\n\n'
  } > "$INDEX"
fi
if ! grep -qF "($NAME/" "$INDEX" 2>/dev/null; then
  printf -- '- [`%s/`](%s/FIXTURE.md) — head `%s` on `%s`, base `%s`\n' \
    "$NAME" "$NAME" "${HEAD_COMMIT:0:12}" "$BRANCH" "${BASE_COMMIT:0:12}" >> "$INDEX"
fi

# ---- verify ---------------------------------------------------------------
echo "Verifying fixture"
git -C "$DEST" fsck --connectivity-only >/dev/null 2>&1 && note "fsck: clean (independent of source repo)" || note "fsck: WARNING (check $DEST)"
note "status: $(git -C "$DEST" status -sb | head -1)"
note "resolves base: $(git -C "$DEST" cat-file -e "$BASE_COMMIT" 2>/dev/null && echo yes || echo NO)"
note "size: $(du -sh "$DEST" 2>/dev/null | cut -f1)"

echo "Done -> $DEST"
echo "  FIXTURE.md and the index at $INDEX were updated."
