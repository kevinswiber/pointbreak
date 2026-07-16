#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(git -C "$script_dir/.." rev-parse --show-toplevel)
targets_file="$repo_root/.github/binary-targets.json"
package_script="$script_dir/package-release-archive.sh"
verify_script="$script_dir/verify-release-archives.sh"
version="0.0.0-local"

temp_dir=$(mktemp -d)
trap 'rm -rf "$temp_dir"' EXIT

package_files=$(cargo +stable package \
  --manifest-path "$repo_root/Cargo.toml" \
  --allow-dirty \
  --list)
for required in Cargo.toml LICENSE NOTICE src/main.rs; do
  grep -Fxq "$required" <<<"$package_files" || {
    echo "cargo package is missing $required" >&2
    exit 1
  }
done

license=$(cargo +stable metadata \
  --manifest-path "$repo_root/Cargo.toml" \
  --no-deps \
  --format-version 1 \
  | jq -r '.packages[] | select(.name == "pointbreak") | .license')
if [ "$license" != "Apache-2.0" ]; then
  echo "unexpected Cargo license metadata: $license" >&2
  exit 1
fi

install_root="$temp_dir/install"
cargo +stable install --path "$repo_root" --root "$install_root" --locked

expected_installed="pointbreak"
if [ "${OS:-}" = "Windows_NT" ]; then
  expected_installed="pointbreak.exe"
fi
installed=()
while IFS= read -r installed_name; do
  installed+=("$installed_name")
done < <(find "$install_root/bin" -mindepth 1 -maxdepth 1 -exec basename {} \; | sort)
if [ "${#installed[@]}" -ne 1 ] || [ "${installed[0]}" != "$expected_installed" ]; then
  printf 'unexpected Cargo-installed payload: %s\n' "${installed[*]:-<empty>}" >&2
  exit 1
fi

cargo +stable build --manifest-path "$repo_root/Cargo.toml" --bin pointbreak
host_binary="$repo_root/target/debug/pointbreak"
if [ ! -f "$host_binary" ]; then
  host_binary="${host_binary}.exe"
fi

bin_dir="$temp_dir/bin"
mkdir -p "$bin_dir"
cp "$host_binary" "$bin_dir/pointbreak"
cp "$host_binary" "$bin_dir/pointbreak.exe"

archives_dir="$temp_dir/archives"
mkdir -p "$archives_dir"

while IFS=$'\t' read -r target archive executable; do
  out=$(cd "$archives_dir" && "$package_script" "$target" "$version" "$bin_dir")
  expected="pointbreak-${version}-${target}.${archive}"
  if [ "$out" != "$expected" ] || [ ! -f "$archives_dir/$expected" ]; then
    echo "failed to package $target as $expected (got $out)" >&2
    exit 1
  fi
  case "$executable" in
    pointbreak | pointbreak.exe) ;;
    *)
      echo "unexpected executable for $target: $executable" >&2
      exit 1
      ;;
  esac
done < <(jq -r '.[] | [.target, .archive, .executable] | @tsv' "$targets_file")

"$verify_script" "$version" "$archives_dir" --write-checksums

prepare_case() {
  local name="$1"
  local case_dir="$temp_dir/cases/$name"
  mkdir -p "$case_dir"
  cp "$archives_dir"/pointbreak-* "$case_dir/"
  printf '%s\n' "$case_dir"
}

expect_rejected() {
  local name="$1"
  local case_dir="$2"
  if "$verify_script" "$version" "$case_dir" >/dev/null 2>&1; then
    echo "archive verifier accepted invalid $name payload" >&2
    exit 1
  fi
}

payload_dir="$temp_dir/payload"
mkdir -p "$payload_dir"
printf 'legal\n' >"$payload_dir/LICENSE"
printf 'notice\n' >"$payload_dir/NOTICE"
printf 'binary\n' >"$payload_dir/pointbreak"
chmod +x "$payload_dir/pointbreak"

case_dir=$(prepare_case legacy-unix)
printf 'legacy\n' >"$payload_dir/shore"
tar -czf "$case_dir/pointbreak-${version}-darwin-x64.tar.gz" \
  -C "$payload_dir" shore LICENSE NOTICE
expect_rejected legacy-unix "$case_dir"

case_dir=$(prepare_case legacy-windows)
printf 'legacy\n' >"$payload_dir/shore.exe"
(cd "$payload_dir" && zip -q "$case_dir/pointbreak-${version}-win32-x64.zip" shore.exe LICENSE NOTICE)
expect_rejected legacy-windows "$case_dir"

case_dir=$(prepare_case alias)
alias_dir="$temp_dir/alias-payload"
mkdir -p "$alias_dir"
cp "$payload_dir/LICENSE" "$payload_dir/NOTICE" "$alias_dir/"
ln -s shore "$alias_dir/pointbreak"
tar -czf "$case_dir/pointbreak-${version}-darwin-x64.tar.gz" \
  -C "$alias_dir" pointbreak LICENSE NOTICE
expect_rejected alias "$case_dir"

case_dir=$(prepare_case duplicate-executable)
tar -czf "$case_dir/pointbreak-${version}-darwin-x64.tar.gz" \
  -C "$payload_dir" pointbreak pointbreak LICENSE NOTICE
expect_rejected duplicate-executable "$case_dir"

case_dir=$(prepare_case missing-legal)
tar -czf "$case_dir/pointbreak-${version}-darwin-x64.tar.gz" \
  -C "$payload_dir" pointbreak LICENSE
expect_rejected missing-legal "$case_dir"

case_dir=$(prepare_case unexpected-root)
root_dir="$temp_dir/root-payload/unexpected"
mkdir -p "$root_dir"
cp "$payload_dir/pointbreak" "$payload_dir/LICENSE" "$payload_dir/NOTICE" "$root_dir/"
tar -czf "$case_dir/pointbreak-${version}-darwin-x64.tar.gz" \
  -C "$temp_dir/root-payload" unexpected
expect_rejected unexpected-root "$case_dir"

case_dir=$(prepare_case extra-payload)
printf 'extra\n' >"$payload_dir/README"
tar -czf "$case_dir/pointbreak-${version}-darwin-x64.tar.gz" \
  -C "$payload_dir" pointbreak LICENSE NOTICE README
expect_rejected extra-payload "$case_dir"

echo "package-release selftest ok"
