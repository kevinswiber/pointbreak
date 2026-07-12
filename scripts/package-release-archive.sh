#!/usr/bin/env bash
# Package a built release binary into a distributable archive.
# Usage: package-release-archive.sh <target-label> <version> <archive-ext> <bin-dir>
# Emits pointbreak-<version>-<target-label>.<archive-ext> in the working directory,
# containing shore (shore.exe for zip), LICENSE, and NOTICE at the archive root.
# Prints the archive filename on stdout.
set -euo pipefail

if [ "$#" -ne 4 ]; then
  echo "usage: $0 <target-label> <version> <archive-ext> <bin-dir>" >&2
  exit 2
fi

target="$1"; version="$2"; ext="$3"; bin_dir="$4"

case "$ext" in
  tar.gz) bin="shore" ;;
  zip)    bin="shore.exe" ;;
  *) echo "unsupported archive ext: $ext" >&2; exit 1 ;;
esac

src="${bin_dir}/${bin}"
if [ ! -f "$src" ]; then
  echo "missing binary: $src" >&2
  exit 1
fi

staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
cp "$src" LICENSE NOTICE "$staging/"

out="pointbreak-${version}-${target}.${ext}"
case "$ext" in
  tar.gz) tar -czf "$out" -C "$staging" "$bin" LICENSE NOTICE ;;
  zip)    (cd "$staging" && 7z a -tzip "${OLDPWD}/${out}" "$bin" LICENSE NOTICE >/dev/null) ;;
esac

echo "$out"
