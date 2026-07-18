#!/usr/bin/env bash

set -euo pipefail

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

normalize_for_shell_comparison() {
  local native_path="${1//\\//}"
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -u "$native_path"
  else
    printf '%s\n' "$native_path"
  fi
}

command -v git >/dev/null 2>&1 || die "git is required"
command -v jq >/dev/null 2>&1 || die "jq is required"

[ "$#" -eq 1 ] || die "usage: $0 <empty-destination>"

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
destination="$1"
pointbreak_binary="${POINTBREAK_BINARY:-$repo_root/target/debug/pointbreak}"

[ -x "$pointbreak_binary" ] \
  || die "POINTBREAK_BINARY is not executable; provide an absolute installed binary or run 'just build'"
case "$pointbreak_binary" in
  /* | [A-Za-z]:/* | [A-Za-z]:\\* | \\\\*) ;;
  *) die "POINTBREAK_BINARY must resolve to an absolute path" ;;
esac

if [ -e "$destination" ]; then
  [ -d "$destination" ] || die "destination exists and is not a directory: $destination"
  [ -z "$(find "$destination" -mindepth 1 -maxdepth 1 -print -quit)" ] \
    || die "destination is not empty: $destination"
else
  mkdir -p "$destination"
fi

destination="$(cd "$destination" && pwd -P)"
case "$destination" in
  "$repo_root"|"$repo_root"/*)
    die "destination must be outside the Pointbreak source worktree"
    ;;
esac

pointbreak_home="${POINTBREAK_HOME:-$destination/.git/pointbreak-home}"
mkdir -p "$pointbreak_home"
pointbreak_home="$(cd "$pointbreak_home" && pwd -P)"
if [ -n "${POINTBREAK_HOME:-}" ]; then
  destination_parent="$(dirname "$destination")"
  case "$pointbreak_home" in
    "$destination_parent"/*) ;;
    *) die "POINTBREAK_HOME must remain beneath the destination's temporary parent" ;;
  esac
fi

pointbreak_json() {
  POINTBREAK_HOME="$pointbreak_home" \
    "$pointbreak_binary" "$@" --format json
}

pointbreak_actor_json() {
  local actor="$1"
  shift
  POINTBREAK_HOME="$pointbreak_home" \
    POINTBREAK_ACTOR_ID="$actor" "$pointbreak_binary" "$@" --format json
}

capture_revision() {
  local actor="$1"
  shift
  pointbreak_actor_json "$actor" capture --repo "$destination" "$@" \
    | jq -er '.revision.id'
}

record_validation() {
  local revision="$1"
  local check_name="$2"
  local status="$3"
  local completed_at="$4"
  pointbreak_actor_json "actor:agent:pointbreak-matrix-validation-writer" \
    validation add --repo "$destination" --exact-revision "$revision" \
    --track "agent:matrix-validation" --check-name "$check_name" \
    --status "$status" --completed-at "$completed_at" >/dev/null
}

git -C "$destination" init --quiet
git -C "$destination" symbolic-ref HEAD refs/heads/main
git -C "$destination" config user.name "Pointbreak Matrix"
git -C "$destination" config user.email "pointbreak-matrix@example.com"
git -C "$destination" config commit.gpgsign false

mkdir -p "$destination/src"
printf 'pub fn matrix_value() -> u32 { 1 }\n' > "$destination/src/lib.rs"
git -C "$destination" add --all
git -C "$destination" commit --quiet -m "matrix base"
base_commit="$(git -C "$destination" rev-parse HEAD)"

git -C "$destination" switch --quiet -c feat/decision-matrix
printf 'pub fn matrix_value() -> u32 { 2 }\n' > "$destination/src/lib.rs"
primary_revision="$(capture_revision \
  "actor:agent:pointbreak-matrix-capture-writer" \
  --summary "Decision continuity matrix")"

pointbreak_actor_json "actor:agent:pointbreak-matrix-fact-writer" \
  observation add --repo "$destination" --exact-revision "$primary_revision" \
  --track "agent:matrix-facts" --title "Matrix fact" \
  --body "The matrix keeps evidence classes distinct." >/dev/null

pointbreak_actor_json \
  "actor:agent:pointbreak-matrix-participant-opener" \
  input-request open --repo "$destination" --revision "$primary_revision" \
  --track "agent:matrix-requests" --title "Open decision" \
  --reason insufficient-evidence --body "More evidence is required." >/dev/null

responded_request="$(pointbreak_actor_json \
  "actor:agent:pointbreak-matrix-participant-opener" \
  input-request open --repo "$destination" --revision "$primary_revision" \
  --track "agent:matrix-requests" --title "Responded decision" \
  --reason manual-decision-required --body "Is the evidence sufficient?" \
  | jq -er '.inputRequestId')"
pointbreak_actor_json \
  "actor:agent:pointbreak-matrix-participant-responder" \
  input-request respond "$responded_request" --repo "$destination" \
  --outcome approved --reason "the evidence is sufficient" >/dev/null

ambiguous_request="$(pointbreak_actor_json \
  "actor:agent:pointbreak-matrix-request-opener" \
  input-request open --repo "$destination" --revision "$primary_revision" \
  --track "agent:matrix-requests" --title "Ambiguous decision" \
  --reason conflicting-event --body "The responses may conflict." \
  | jq -er '.inputRequestId')"
pointbreak_actor_json "actor:agent:pointbreak-matrix-response-one" \
  input-request respond "$ambiguous_request" --repo "$destination" \
  --outcome approved --reason "first response approves" >/dev/null
pointbreak_actor_json "actor:agent:pointbreak-matrix-response-two" \
  input-request respond "$ambiguous_request" --repo "$destination" \
  --outcome rejected --reason "second response rejects" >/dev/null

replaced_assessment="$(pointbreak_actor_json \
  "actor:agent:pointbreak-matrix-assessment-writer-one" \
  assessment add --repo "$destination" --exact-revision "$primary_revision" \
  --track "agent:matrix-assessment" --assessment needs-changes \
  --summary "The matrix is incomplete." | jq -er '.assessmentId')"
pointbreak_actor_json "actor:agent:pointbreak-matrix-assessment-writer-two" \
  assessment add --repo "$destination" --exact-revision "$primary_revision" \
  --track "agent:matrix-assessment" --assessment accepted-with-follow-up \
  --summary "The matrix is complete with bounded follow-up." \
  --replaces "$replaced_assessment" >/dev/null

record_validation "$primary_revision" "passed current" passed "2026-07-17T10:00:00Z"
record_validation "$primary_revision" "failed current" failed "2026-07-17T10:00:00Z"
record_validation "$primary_revision" "errored current" errored "2026-07-17T10:00:00Z"
record_validation "$primary_revision" "skipped only" skipped "2026-07-17T10:00:00Z"
record_validation "$primary_revision" "failed then passed" failed "2026-07-17T10:00:00Z"
record_validation "$primary_revision" "failed then passed" passed "2026-07-17T10:01:00Z"
record_validation "$primary_revision" "errored then passed" errored "2026-07-17T10:00:00Z"
record_validation "$primary_revision" "errored then passed" passed "2026-07-17T10:01:00Z"
record_validation "$primary_revision" "equal time" failed "2026-07-17T10:00:00Z"
record_validation "$primary_revision" "equal time" passed "2026-07-17T10:00:00Z"
record_validation "$primary_revision" "regression" passed "2026-07-17T10:00:00Z"
record_validation "$primary_revision" "regression" failed "2026-07-17T10:01:00Z"
record_validation "$primary_revision" "failure followed by skip" failed "2026-07-17T10:00:00Z"
record_validation "$primary_revision" "failure followed by skip" skipped "2026-07-17T10:01:00Z"

git -C "$destination" add --all
git -C "$destination" commit --quiet -m "first matrix landing"
first_landing="$(git -C "$destination" rev-parse HEAD)"
first_commit_association="$(pointbreak_actor_json \
  "actor:agent:pointbreak-matrix-association-writer" \
  association record --repo "$destination" --revision "$primary_revision" \
  --track "agent:matrix-associations" --commit "$first_landing" \
  | jq -er '.commitAssociationId')"

printf 'pub fn matrix_value() -> u32 { 3 }\n' > "$destination/src/lib.rs"
git -C "$destination" add --all
git -C "$destination" commit --quiet -m "second matrix landing"
second_landing="$(git -C "$destination" rev-parse HEAD)"
pointbreak_actor_json "actor:agent:pointbreak-matrix-association-writer" \
  association record --repo "$destination" --revision "$primary_revision" \
  --track "agent:matrix-associations" --commit "$second_landing" >/dev/null
pointbreak_actor_json "actor:agent:pointbreak-matrix-association-writer" \
  association withdraw "$first_commit_association" --repo "$destination" \
  --revision "$primary_revision" --track "agent:matrix-associations" >/dev/null

git -C "$destination" branch withdrawn-matrix "$first_landing"
withdrawn_ref_association="$(pointbreak_actor_json \
  "actor:agent:pointbreak-matrix-association-writer" \
  association record --repo "$destination" --revision "$primary_revision" \
  --track "agent:matrix-associations" --ref withdrawn-matrix \
  --head "$first_landing" | jq -er '.refAssociationId')"
pointbreak_actor_json "actor:agent:pointbreak-matrix-association-writer" \
  association withdraw "$withdrawn_ref_association" --repo "$destination" \
  --revision "$primary_revision" --track "agent:matrix-associations" >/dev/null
git -C "$destination" branch live-matrix "$second_landing"
pointbreak_actor_json "actor:agent:pointbreak-matrix-association-writer" \
  association record --repo "$destination" --revision "$primary_revision" \
  --track "agent:matrix-associations" --ref live-matrix \
  --head "$second_landing" >/dev/null

git -C "$destination" switch --quiet main
git -C "$destination" merge --quiet --ff-only feat/decision-matrix

git -C "$destination" switch --quiet -c feat/live-matrix
printf 'pub fn matrix_value() -> u32 { 4 }\n' > "$destination/src/lib.rs"
live_revision="$(capture_revision \
  "actor:agent:pointbreak-matrix-capture-writer" \
  --summary "Live landing matrix")"
git -C "$destination" add --all
git -C "$destination" commit --quiet -m "live matrix landing"
live_landing="$(git -C "$destination" rev-parse HEAD)"
pointbreak_actor_json "actor:agent:pointbreak-matrix-association-writer" \
  association record --repo "$destination" --revision "$live_revision" \
  --track "agent:matrix-associations" --commit "$live_landing" >/dev/null

git -C "$destination" switch --quiet --detach main
printf 'pub fn matrix_value() -> u32 { 5 }\n' > "$destination/src/lib.rs"
unassessed_revision="$(capture_revision \
  "actor:agent:pointbreak-matrix-capture-writer" \
  --summary "Unassessed matrix")"
git -C "$destination" reset --quiet --hard main

git -C "$destination" switch --quiet -c feat/competing-heads
printf 'pub fn matrix_value() -> u32 { 6 }\n' > "$destination/src/lib.rs"
superseded_revision="$(capture_revision \
  "actor:agent:pointbreak-matrix-capture-writer" \
  --summary "Supersession root")"
pointbreak_actor_json "actor:agent:pointbreak-matrix-fact-writer" \
  observation add --repo "$destination" --exact-revision "$superseded_revision" \
  --track "agent:matrix-facts" --title "Stale predecessor fact" \
  --body "This fact remains on the addressed predecessor." >/dev/null

printf 'pub fn matrix_value() -> u32 { 7 }\n' > "$destination/src/lib.rs"
ambiguous_assessment_revision="$(capture_revision \
  "actor:agent:pointbreak-matrix-capture-writer" \
  --summary "Competing head A" --supersedes "$superseded_revision")"
pointbreak_actor_json "actor:agent:pointbreak-matrix-assessment-writer-one" \
  assessment add --repo "$destination" \
  --exact-revision "$ambiguous_assessment_revision" \
  --track "agent:matrix-assessment-a" --assessment accepted \
  --summary "Candidate A accepts." >/dev/null
pointbreak_actor_json "actor:agent:pointbreak-matrix-assessment-writer-two" \
  assessment add --repo "$destination" \
  --exact-revision "$ambiguous_assessment_revision" \
  --track "agent:matrix-assessment-b" --assessment needs-changes \
  --summary "Candidate B requests changes." >/dev/null

printf 'pub fn matrix_value() -> u32 { 8 }\n' > "$destination/src/lib.rs"
competing_revision="$(capture_revision \
  "actor:agent:pointbreak-matrix-capture-writer" \
  --summary "Competing head B" --supersedes "$superseded_revision")"
git -C "$destination" reset --quiet --hard main

git -C "$destination" switch --quiet -c feat/source-matrix
printf 'pub fn matrix_value() -> u32 { 9 }\n' > "$destination/src/lib.rs"
git -C "$destination" add --all
git -C "$destination" commit --quiet -m "range matrix target"
range_revision="$(capture_revision \
  "actor:agent:pointbreak-matrix-capture-writer" \
  --base HEAD~1 --target HEAD --summary "Range matrix")"
root_revision="$(capture_revision \
  "actor:agent:pointbreak-matrix-capture-writer" \
  --root --target HEAD --summary "Root matrix")"

printf 'pub fn staged_value() -> u32 { 10 }\n' > "$destination/src/staged.rs"
git -C "$destination" add src/staged.rs
staged_revision="$(capture_revision \
  "actor:agent:pointbreak-matrix-capture-writer" \
  --staged --summary "Staged matrix")"
git -C "$destination" branch second-current-ref HEAD
pointbreak_actor_json "actor:agent:pointbreak-matrix-association-writer" \
  association record --repo "$destination" --revision "$staged_revision" \
  --track "agent:matrix-associations" --ref second-current-ref \
  --head "$(git -C "$destination" rev-parse HEAD)" >/dev/null

git -C "$destination" reset --quiet HEAD -- src/staged.rs
rm "$destination/src/staged.rs"
printf 'pub fn matrix_value() -> u32 { 10 }\n' > "$destination/src/lib.rs"
unstaged_revision="$(capture_revision \
  "actor:agent:pointbreak-matrix-capture-writer" \
  --unstaged --summary "Unstaged matrix")"
git -C "$destination" reset --quiet --hard HEAD

git -C "$destination" switch --quiet --detach HEAD
printf 'pub fn matrix_value() -> u32 { 11 }\n' > "$destination/src/lib.rs"
detached_revision="$(capture_revision \
  "actor:agent:pointbreak-matrix-capture-writer" \
  --summary "Detached worktree matrix")"
git -C "$destination" reset --quiet --hard HEAD

git -C "$destination" switch --quiet -c feat/missing-object main
printf 'pub fn matrix_value() -> u32 { 12 }\n' > "$destination/src/lib.rs"
git -C "$destination" add --all
git -C "$destination" commit --quiet -m "missing object matrix target"
missing_commit="$(git -C "$destination" rev-parse HEAD)"
missing_revision="$(capture_revision \
  "actor:agent:pointbreak-matrix-capture-writer" \
  --base HEAD~1 --target HEAD --summary "Missing object matrix")"
git -C "$destination" switch --quiet main
git -C "$destination" branch --delete --force feat/missing-object >/dev/null
git -C "$destination" reflog expire --expire=now --all
git_object_dir="$(git -C "$destination" rev-parse --path-format=absolute --git-path objects)"
missing_object_path="$git_object_dir/${missing_commit:0:2}/${missing_commit:2}"
[ -f "$missing_object_path" ] || die "expected a loose synthetic commit object"
rm "$missing_object_path"

store_paths="$(pointbreak_json store paths --repo "$destination")"
common_store="$(printf '%s\n' "$store_paths" | jq -er '.commonStore')"
common_store_for_comparison="$(normalize_for_shell_comparison "$common_store")"
case "$common_store_for_comparison" in
  "$destination"/*) ;;
  *) die "generated store escaped the isolated repository: $common_store" ;;
esac

jq -n \
  --arg primary_revision "$primary_revision" \
  --arg live_revision "$live_revision" \
  --arg unassessed_revision "$unassessed_revision" \
  --arg superseded_revision "$superseded_revision" \
  --arg ambiguous_assessment_revision "$ambiguous_assessment_revision" \
  --arg competing_revision "$competing_revision" \
  --arg range_revision "$range_revision" \
  --arg root_revision "$root_revision" \
  --arg staged_revision "$staged_revision" \
  --arg unstaged_revision "$unstaged_revision" \
  --arg detached_revision "$detached_revision" \
  --arg missing_revision "$missing_revision" \
  --arg base_commit "$base_commit" \
  --arg first_landing "$first_landing" \
  --arg second_landing "$second_landing" \
  --arg live_landing "$live_landing" \
  '{
    primary_revision: $primary_revision,
    live_revision: $live_revision,
    unassessed_revision: $unassessed_revision,
    superseded_revision: $superseded_revision,
    ambiguous_assessment_revision: $ambiguous_assessment_revision,
    competing_revision: $competing_revision,
    range_revision: $range_revision,
    root_revision: $root_revision,
    staged_revision: $staged_revision,
    unstaged_revision: $unstaged_revision,
    detached_revision: $detached_revision,
    missing_revision: $missing_revision,
    base_commit: $base_commit,
    first_landing: $first_landing,
    second_landing: $second_landing,
    live_landing: $live_landing
  }'
