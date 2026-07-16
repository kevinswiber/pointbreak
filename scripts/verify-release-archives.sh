#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
  echo "usage: $0 <version> <archive-dir> [--write-checksums]" >&2
  exit 2
fi

version="$1"
archive_dir="$2"
checksum_mode="${3:-}"
if [ -n "$checksum_mode" ] && [ "$checksum_mode" != "--write-checksums" ]; then
  echo "unsupported option: $checksum_mode" >&2
  exit 2
fi

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(git -C "$script_dir/.." rev-parse --show-toplevel)
targets_file="$repo_root/.github/binary-targets.json"
archive_dir=$(cd -- "$archive_dir" && pwd)

temp_dir=$(mktemp -d)
trap 'rm -rf "$temp_dir"' EXIT
expected_archives="$temp_dir/expected-archives"
actual_archives="$temp_dir/actual-archives"

jq -r --arg version "$version" \
  '.[] | "pointbreak-\($version)-\(.target).\(.archive)"' \
  "$targets_file" | LC_ALL=C sort >"$expected_archives"
find "$archive_dir" -mindepth 1 -maxdepth 1 ! -type d \
  \( -name '*.tar.gz' -o -name '*.zip' \) \
  -exec basename {} \; | LC_ALL=C sort >"$actual_archives"

if ! diff -u "$expected_archives" "$actual_archives"; then
  echo "release archive set does not match the target table" >&2
  exit 1
fi

while IFS=$'\t' read -r target archive executable; do
  archive_name="pointbreak-${version}-${target}.${archive}"
  archive_path="$archive_dir/$archive_name"
  actual_payload="$temp_dir/${target}-actual"
  expected_payload="$temp_dir/${target}-expected"
  extract_dir="$temp_dir/${target}-extract"
  mkdir -p "$extract_dir"

  if [ ! -f "$archive_path" ] || [ -L "$archive_path" ]; then
    echo "$archive_name is not a regular archive file" >&2
    exit 1
  fi

  case "$archive" in
    tar.gz) tar -tzf "$archive_path" >"$actual_payload" ;;
    zip) unzip -Z1 "$archive_path" >"$actual_payload" ;;
    *)
      echo "unsupported archive type for $target: $archive" >&2
      exit 1
      ;;
  esac
  printf '%s\n' "$executable" LICENSE NOTICE | LC_ALL=C sort >"$expected_payload"
  LC_ALL=C sort "$actual_payload" -o "$actual_payload"
  if ! diff -u "$expected_payload" "$actual_payload"; then
    echo "$archive_name has an invalid archive root" >&2
    exit 1
  fi

  case "$archive" in
    tar.gz) tar -xzf "$archive_path" -C "$extract_dir" ;;
    zip) unzip -qq "$archive_path" -d "$extract_dir" ;;
  esac
  for entry in "$executable" LICENSE NOTICE; do
    if [ ! -f "$extract_dir/$entry" ] || [ -L "$extract_dir/$entry" ]; then
      echo "$archive_name contains a non-regular $entry" >&2
      exit 1
    fi
  done
  if [ "$archive" = "tar.gz" ] && [ ! -x "$extract_dir/$executable" ]; then
    echo "$archive_name contains a non-executable $executable" >&2
    exit 1
  fi
done < <(jq -r '.[] | [.target, .archive, .executable] | @tsv' "$targets_file")

checksum_file="$archive_dir/checksums.txt"
expected_names=()
while IFS= read -r archive_name; do
  expected_names+=("$archive_name")
done <"$expected_archives"

if [ "$checksum_mode" = "--write-checksums" ]; then
  if [ -L "$checksum_file" ]; then
    echo "checksums.txt must not be a symlink" >&2
    exit 1
  fi
  (cd "$archive_dir" && sha256sum "${expected_names[@]}") >"$checksum_file"
fi

if [ -f "$checksum_file" ]; then
  checksum_names="$temp_dir/checksum-names"
  awk '{print $2}' "$checksum_file" | LC_ALL=C sort >"$checksum_names"
  if ! diff -u "$expected_archives" "$checksum_names"; then
    echo "checksums.txt does not contain exactly one entry per archive" >&2
    exit 1
  fi
  (cd "$archive_dir" && sha256sum -c checksums.txt)
fi

echo "verified 8 release archives"
