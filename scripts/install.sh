#!/bin/sh
# Install the latest (or a requested) Pointbreak Review release on macOS or Linux.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/withpointbreak/pointbreak/main/scripts/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/withpointbreak/pointbreak/main/scripts/install.sh \
#     | sh -s -- --version=v0.7.0 --prefix="$HOME/.local/bin"

set -eu

REPOSITORY="withpointbreak/pointbreak"
API_ROOT="https://api.github.com/repos"
DOWNLOAD_ROOT="https://github.com/${REPOSITORY}/releases/download"
RELEASES_URL="https://github.com/${REPOSITORY}/releases"
VERSION="latest"
INSTALL_DIR="${HOME:?HOME must be set}/.local/bin"

usage() {
    cat <<'EOF'
Pointbreak Review installer

Usage: install.sh [options]

Options:
  --version VERSION   Install a release tag or version (for example v0.7.0)
  --prefix PATH       Install directory (default: ~/.local/bin)
  -h, --help          Show this help
EOF
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version=*)
            VERSION=${1#*=}
            shift
            ;;
        --version)
            [ "$#" -ge 2 ] || die "--version requires a value"
            VERSION=$2
            shift 2
            ;;
        --prefix=*)
            INSTALL_DIR=${1#*=}
            shift
            ;;
        --prefix)
            [ "$#" -ge 2 ] || die "--prefix requires a value"
            INSTALL_DIR=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown option: $1"
            ;;
    esac
done

[ -n "$INSTALL_DIR" ] || die "--prefix cannot be empty"

fetch_text() {
    url=$1
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO- "$url"
    else
        die "curl or wget is required"
    fi
}

download_file() {
    url=$1
    output=$2
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL -o "$output" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$output" "$url"
    else
        die "curl or wget is required"
    fi
}

resolve_release_tag() {
    if [ "$VERSION" = "latest" ]; then
        printf 'Finding the latest Pointbreak Review release...\n' >&2
        release_json=$(fetch_text "${API_ROOT}/${REPOSITORY}/releases/latest") \
            || die "could not fetch the latest release"
        release_tag=$(printf '%s\n' "$release_json" \
            | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
            | head -n 1)
        [ -n "$release_tag" ] || die "latest release response did not contain a tag"
    else
        case "$VERSION" in
            v*) release_tag=$VERSION ;;
            *) release_tag="v${VERSION}" ;;
        esac
    fi

    if ! printf '%s\n' "$release_tag" \
        | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z][0-9A-Za-z.-]*)?$'; then
        die "unsupported release version: $release_tag"
    fi

    printf '%s\n' "$release_tag"
}

is_musl() {
    [ -f /etc/alpine-release ] && return 0
    if command -v ldd >/dev/null 2>&1 && ldd --version 2>&1 | grep -qi musl; then
        return 0
    fi
    for loader in /lib/ld-musl-*.so.1 /lib64/ld-musl-*.so.1; do
        [ -e "$loader" ] && return 0
    done
    return 1
}

detect_target() {
    case "$(uname -s)" in
        Darwin) os=darwin ;;
        Linux)
            if is_musl; then
                os=alpine
            else
                os=linux
            fi
            ;;
        *) die "this installer supports macOS and Linux; use install.ps1 on Windows" ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64) arch=x64 ;;
        arm64|aarch64) arch=arm64 ;;
        *) die "unsupported architecture: $(uname -m)" ;;
    esac

    printf '%s-%s\n' "$os" "$arch"
}

sha256_file() {
    file=$1
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$file" | awk '{print $1}'
    else
        die "sha256sum or shasum is required"
    fi
}

download_asset() {
    tag=$1
    name=$2
    output=$3

    # Used by the repository's hermetic installer self-tests.
    if [ -n "${POINTBREAK_INSTALLER_FIXTURE_ROOT:-}" ]; then
        cp "${POINTBREAK_INSTALLER_FIXTURE_ROOT}/${tag}/${name}" "$output"
    else
        download_file "${DOWNLOAD_ROOT}/${tag}/${name}" "$output"
    fi
}

verify_checksum() {
    archive_name=$1
    archive_file=$2
    checksums_file=$3
    matches_file=$4

    awk -v name="$archive_name" \
        '$2 == name || $2 == "*" name { print $1 }' "$checksums_file" > "$matches_file"
    [ "$(wc -l < "$matches_file" | tr -d ' ')" -eq 1 ] \
        || die "checksums.txt must contain exactly one SHA-256 entry for $archive_name"

    expected_checksum=$(sed -n '1p' "$matches_file")
    printf '%s\n' "$expected_checksum" | grep -Eq '^[0-9A-Fa-f]{64}$' \
        || die "checksums.txt contains an invalid SHA-256 value for $archive_name"
    actual_checksum=$(sha256_file "$archive_file")
    expected_checksum=$(printf '%s' "$expected_checksum" | tr '[:upper:]' '[:lower:]')
    actual_checksum=$(printf '%s' "$actual_checksum" | tr '[:upper:]' '[:lower:]')
    [ "$actual_checksum" = "$expected_checksum" ] \
        || die "checksum mismatch for $archive_name"
    printf 'Verified SHA-256 checksum.\n'
}

verify_archive_layout() {
    archive_file=$1
    actual_entries=$2
    expected_entries=$3

    tar -tzf "$archive_file" > "$actual_entries" \
        || die "could not read release archive"
    printf '%s\n' pointbreak LICENSE NOTICE | LC_ALL=C sort > "$expected_entries"
    LC_ALL=C sort "$actual_entries" -o "$actual_entries"
    diff -u "$expected_entries" "$actual_entries" >/dev/null \
        || die "release archive has an invalid archive layout"
}

read_version_document() {
    binary=$1
    expected_version=$2

    document=$("$binary" version --format json) || return 1
    case "$document" in
        *'
'*) return 1 ;;
    esac

    printf '%s\n' "$document" \
        | grep -Eq '"schema"[[:space:]]*:[[:space:]]*"pointbreak\.version"' \
        || return 1
    printf '%s\n' "$document" \
        | grep -Eq '"version"[[:space:]]*:[[:space:]]*1([,}])' \
        || return 1
    cli_version=$(printf '%s\n' "$document" \
        | sed -n 's/.*"cliVersion"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
    [ "$cli_version" = "$expected_version" ] || return 1
    printf '%s\n' "$document" \
        | grep -Eq '"pointbreak\.version"[[:space:]]*:[[:space:]]*1([,}])' \
        || return 1
    printf '%s\n' "$document" \
        | grep -Eq '"diagnostics"[[:space:]]*:[[:space:]]*\[[[:space:]]*\]' \
        || return 1

    build_object=$(printf '%s\n' "$document" \
        | sed -n 's/.*"build"[[:space:]]*:[[:space:]]*{\([^{}]*\)}.*/\1/p')
    if [ -z "$build_object" ]; then
        printf '%s\n' "$document" | grep -Eq '"build"[[:space:]]*:' && return 1
        # v0.7.0 is the one published transition artifact whose binary predates
        # build provenance. No other release may omit the required tuple.
        [ "$expected_version" = "0.7.0" ] || return 1
        printf '%s\n' "$document"
        return 0
    fi

    printf '%s\n' "$build_object" \
        | grep -Eq '"source"[[:space:]]*:[[:space:]]*"git"' \
        || return 1
    commit=$(printf '%s\n' "$build_object" \
        | sed -n 's/.*"commit"[[:space:]]*:[[:space:]]*"\([0-9A-Fa-f]*\)".*/\1/p')
    printf '%s\n' "$commit" | grep -Eq '^[0-9a-f]{40}$' || return 1
    describe=$(printf '%s\n' "$build_object" \
        | sed -n 's/.*"describe"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
    [ "$describe" = "v${expected_version}" ] || return 1
    printf '%s\n' "$build_object" \
        | grep -Eq '"dirty"[[:space:]]*:[[:space:]]*false([,}]|$)' \
        || return 1

    printf '%s\n' "$document"
}

print_posix_path_command() {
    printf "Run for this shell:\n  export PATH=\"%s:\$PATH\"\n" "$INSTALL_DIR"
}

print_path_guidance() {
    shell_name=${SHELL:-}
    shell_name=${shell_name##*/}
    printf '%s is not in PATH.\n' "$INSTALL_DIR"
    case "$shell_name" in
        fish)
            printf 'Run:\n  fish_add_path "%s"\n' "$INSTALL_DIR"
            ;;
        zsh|bash)
            print_posix_path_command
            printf 'To make it permanent, add the same line to ~/.%src.\n' "$shell_name"
            ;;
        *)
            print_posix_path_command
            ;;
    esac
    printf 'Then run: pointbreak --help\n'
}

temp_dir=""
transaction_dir=""
staged_binary=""
backup_binary=""
destination=""
replacement_done=0
had_previous=0
transaction_complete=0

cleanup() {
    status=$?
    trap - 0

    if [ "$replacement_done" -eq 1 ] && [ "$transaction_complete" -eq 0 ]; then
        if [ "$had_previous" -eq 1 ]; then
            if ! mv -f "$backup_binary" "$destination"; then
                printf 'error: installation failed and the previous Pointbreak could not be restored\n' >&2
                status=1
            fi
        elif ! rm -f "$destination"; then
            printf 'error: installation failed and the unverified Pointbreak could not be removed\n' >&2
            status=1
        fi
    fi

    [ -z "$transaction_dir" ] || rm -rf "$transaction_dir"
    [ -z "$temp_dir" ] || rm -rf "$temp_dir"
    exit "$status"
}
trap cleanup 0
trap 'exit 1' 1 2 3 15

release_tag=$(resolve_release_tag)
release_version=${release_tag#v}
target=$(detect_target)
archive="pointbreak-${release_version}-${target}.tar.gz"

temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/pointbreak-install.XXXXXX")
archive_path="${temp_dir}/${archive}"
printf 'Downloading Pointbreak Review %s for %s...\n' "$release_tag" "$target"
download_asset "$release_tag" "$archive" "$archive_path" \
    || die "could not download $archive for $release_tag; check $RELEASES_URL"

checksums_path="${temp_dir}/checksums.txt"
download_asset "$release_tag" checksums.txt "$checksums_path" \
    || die "could not download checksums.txt"
verify_checksum "$archive" "$archive_path" "$checksums_path" "${temp_dir}/checksum-match"

verify_archive_layout "$archive_path" "${temp_dir}/archive-entries" "${temp_dir}/expected-entries"
printf 'Verified release archive layout.\n'

extract_dir="${temp_dir}/extract"
mkdir -p "$extract_dir"
tar -xzf "$archive_path" -C "$extract_dir" \
    || die "could not extract release archive"
for entry in pointbreak LICENSE NOTICE; do
    if [ ! -f "${extract_dir}/${entry}" ] || [ -L "${extract_dir}/${entry}" ]; then
        die "release archive contains a non-regular $entry"
    fi
done

mkdir -p "$INSTALL_DIR"
destination="${INSTALL_DIR}/pointbreak"
transaction_dir=$(mktemp -d "${INSTALL_DIR}/.pointbreak-transaction.XXXXXX") \
    || die "could not create an adjacent installation transaction"
staged_binary="${transaction_dir}/candidate"
backup_binary="${transaction_dir}/previous"
cp "${extract_dir}/pointbreak" "$staged_binary" \
    || die "could not stage Pointbreak beside $destination"
chmod +x "$staged_binary"
read_version_document "$staged_binary" "$release_version" >/dev/null \
    || die "staged Pointbreak version document did not match $release_version"

if [ -e "$destination" ]; then
    if [ ! -f "$destination" ] || [ -L "$destination" ]; then
        die "Pointbreak destination is not a regular file: $destination"
    fi
    cp -p "$destination" "$backup_binary" \
        || die "could not preserve the previous Pointbreak"
    had_previous=1
fi

if [ -n "${POINTBREAK_INSTALLER_FIXTURE_ROOT:-}" ] \
    && [ "${POINTBREAK_INSTALLER_TEST_FAIL_REPLACE:-}" = 1 ]; then
    die "could not replace $destination"
fi
mv -f "$staged_binary" "$destination" \
    || die "could not replace $destination"
staged_binary=""
replacement_done=1

read_version_document "$destination" "$release_version" >/dev/null \
    || die "installed Pointbreak version document did not match $release_version"
transaction_complete=1
replacement_done=0
backup_binary=""
rm -rf "$transaction_dir" || true
transaction_dir=""

printf 'Installed Pointbreak Review %s to %s\n' "$release_version" "$destination"
case ":${PATH:-}:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        print_path_guidance
        exit 0
        ;;
esac
printf 'Run: pointbreak --help\n'
