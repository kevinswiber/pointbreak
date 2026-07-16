#!/usr/bin/env bash
# Package a built release binary into a distributable archive.
# Usage: package-release-archive.sh <target-label> <version> <bin-dir>
# Emits pointbreak-<version>-<target-label>.<archive-ext> in the working directory,
# containing pointbreak (pointbreak.exe for zip), LICENSE, and NOTICE at the archive root.
# Prints the archive filename on stdout.
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: $0 <target-label> <version> <bin-dir>" >&2
  exit 2
fi

target="$1"; version="$2"; bin_dir="$3"

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(git -C "$script_dir/.." rev-parse --show-toplevel)
targets_file="$repo_root/.github/binary-targets.json"

row=$(jq -er --arg target "$target" '
  [.[] | select(.target == $target)] as $rows
  | if ($rows | length) == 1
    then $rows[0] | [.archive, .executable] | @tsv
    else error("target table must contain exactly one row for \($target)")
    end
' "$targets_file")
IFS=$'\t' read -r ext bin <<<"$row"

src="${bin_dir}/${bin}"
if [ ! -f "$src" ] || [ -L "$src" ]; then
  echo "missing binary: $src" >&2
  exit 1
fi

staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
cp "$src" "$repo_root/LICENSE" "$repo_root/NOTICE" "$staging/"

out="pointbreak-${version}-${target}.${ext}"
case "$ext" in
  tar.gz) tar -czf "$out" -C "$staging" "$bin" LICENSE NOTICE ;;
  zip)
    if command -v 7z >/dev/null 2>&1; then
      (cd "$staging" && 7z a -tzip "${OLDPWD}/${out}" "$bin" LICENSE NOTICE >/dev/null)
    elif command -v zip >/dev/null 2>&1; then
      (cd "$staging" && zip -q "${OLDPWD}/${out}" "$bin" LICENSE NOTICE)
    else
      echo "zip or 7z is required to create $out" >&2
      exit 1
    fi
    ;;
esac

echo "$out"
