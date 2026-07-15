#!/bin/sh
# Hermetic smoke tests for scripts/install.sh.

set -eu

repo_root=$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)
temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/pointbreak-installer-test.XXXXXX")
cleanup() {
    rm -rf "$temp_dir"
}
trap cleanup 0
trap 'exit 1' 1 2 3 15

case "$(uname -s)" in
    Darwin) os=darwin ;;
    Linux)
        if [ -f /etc/alpine-release ] \
            || { command -v ldd >/dev/null 2>&1 && ldd --version 2>&1 | grep -qi musl; }; then
            os=alpine
        else
            os=linux
        fi
        ;;
    *)
        printf 'installer self-test only supports macOS and Linux\n' >&2
        exit 1
        ;;
esac

case "$(uname -m)" in
    x86_64|amd64) arch=x64 ;;
    arm64|aarch64) arch=arm64 ;;
    *)
        printf 'unsupported self-test architecture: %s\n' "$(uname -m)" >&2
        exit 1
        ;;
esac

tag=v9.8.7-test
version=${tag#v}
target="${os}-${arch}"
archive="pointbreak-${version}-${target}.tar.gz"
release_dir="${temp_dir}/releases/${tag}"
payload_dir="${temp_dir}/payload"
install_dir="${temp_dir}/bin"
mkdir -p "$release_dir" "$payload_dir"

cat > "${payload_dir}/shore" <<'EOF'
#!/bin/sh
printf 'shore 9.8.7-test\n'
EOF
chmod +x "${payload_dir}/shore"
cp "${repo_root}/LICENSE" "${repo_root}/NOTICE" "$payload_dir/"
tar -czf "${release_dir}/${archive}" -C "$payload_dir" shore LICENSE NOTICE

if command -v sha256sum >/dev/null 2>&1; then
    checksum=$(sha256sum "${release_dir}/${archive}" | awk '{print $1}')
else
    checksum=$(shasum -a 256 "${release_dir}/${archive}" | awk '{print $1}')
fi
printf '%s  %s\n' "$checksum" "$archive" > "${release_dir}/checksums.txt"

install_output=$(SHELL=/bin/zsh POINTBREAK_INSTALLER_FIXTURE_ROOT="${temp_dir}/releases" \
    "${repo_root}/scripts/install.sh" --version="$tag" --prefix="$install_dir")
printf '%s\n' "$install_output"
test -x "${install_dir}/shore"
test "$("${install_dir}/shore" --version)" = "shore 9.8.7-test"
printf '%s\n' "$install_output" \
    | grep -F "export PATH=\"${install_dir}:\$PATH\"" >/dev/null
printf '%s\n' "$install_output" | grep -F "add the same line to ~/.zshrc" >/dev/null

missing_tag=v9.8.6-missing
if POINTBREAK_INSTALLER_FIXTURE_ROOT="${temp_dir}/releases" \
    "${repo_root}/scripts/install.sh" --version="$missing_tag" --prefix="$install_dir" \
    > "${temp_dir}/missing-release.log" 2>&1; then
    printf 'installer accepted a missing release\n' >&2
    exit 1
fi
grep -F "check https://github.com/withpointbreak/pointbreak/releases" \
    "${temp_dir}/missing-release.log" >/dev/null

rm -f "${install_dir}/shore"
printf '%064d  %s\n' 0 "$archive" > "${release_dir}/checksums.txt"
if POINTBREAK_INSTALLER_FIXTURE_ROOT="${temp_dir}/releases" \
    "${repo_root}/scripts/install.sh" --version="$tag" --prefix="$install_dir" \
    > "${temp_dir}/checksum-failure.log" 2>&1; then
    printf 'installer accepted an invalid checksum\n' >&2
    exit 1
fi
grep -q "checksum mismatch" "${temp_dir}/checksum-failure.log"
test ! -e "${install_dir}/shore"

no_verify_output=$(SHELL=/usr/bin/fish POINTBREAK_INSTALLER_FIXTURE_ROOT="${temp_dir}/releases" \
    "${repo_root}/scripts/install.sh" --version="$tag" --prefix="$install_dir" --no-verify 2>&1)
printf '%s\n' "$no_verify_output"
test -x "${install_dir}/shore"
printf '%s\n' "$no_verify_output" \
    | grep -F "fish_add_path \"${install_dir}\"" >/dev/null

printf 'install.sh self-test ok\n'
