#!/bin/sh
# Install the latest (or a requested) Pointbreak Review release on macOS or Linux.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/withpointbreak/pointbreak/main/scripts/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/withpointbreak/pointbreak/main/scripts/install.sh \
#     | sh -s -- --version=v0.6.0 --prefix="$HOME/.local/bin"

set -eu

REPOSITORY="withpointbreak/pointbreak"
API_ROOT="https://api.github.com/repos"
DOWNLOAD_ROOT="https://github.com/${REPOSITORY}/releases/download"
RELEASES_URL="https://github.com/${REPOSITORY}/releases"
VERSION="latest"
INSTALL_DIR="${HOME:?HOME must be set}/.local/bin"
VERIFY_CHECKSUM=1

usage() {
    cat <<'EOF'
Pointbreak Review installer

Usage: install.sh [options]

Options:
  --version VERSION   Install a release tag or version (for example v0.6.0)
  --prefix PATH       Install directory (default: ~/.local/bin)
  --no-verify         Skip SHA-256 verification (not recommended)
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
        --no-verify)
            VERIFY_CHECKSUM=0
            shift
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
        die "sha256sum or shasum is required for verification; use --no-verify to bypass it"
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
    printf 'Then run: shore --help\n'
}

release_tag=$(resolve_release_tag)
release_version=${release_tag#v}
target=$(detect_target)
archive="pointbreak-${release_version}-${target}.tar.gz"

temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/pointbreak-install.XXXXXX")
staged_binary=""
cleanup() {
    rm -rf "$temp_dir"
    [ -z "$staged_binary" ] || rm -f "$staged_binary"
}
trap cleanup 0
trap 'exit 1' 1 2 3 15

archive_path="${temp_dir}/${archive}"
printf 'Downloading Pointbreak Review %s for %s...\n' "$release_tag" "$target"
download_asset "$release_tag" "$archive" "$archive_path" \
    || die "could not download $archive for $release_tag; check $RELEASES_URL"

if [ "$VERIFY_CHECKSUM" -eq 1 ]; then
    checksums_path="${temp_dir}/checksums.txt"
    download_asset "$release_tag" checksums.txt "$checksums_path" \
        || die "could not download checksums.txt; use --no-verify only if you accept the risk"

    expected_checksum=$(awk -v name="$archive" \
        '$2 == name || $2 == "*" name { print $1 }' "$checksums_path" | head -n 1)
    [ -n "$expected_checksum" ] \
        || die "checksums.txt has no entry for $archive"
    if ! printf '%s\n' "$expected_checksum" | grep -Eq '^[0-9A-Fa-f]{64}$'; then
        die "checksums.txt contains an invalid SHA-256 value for $archive"
    fi

    actual_checksum=$(sha256_file "$archive_path")
    expected_checksum=$(printf '%s' "$expected_checksum" | tr '[:upper:]' '[:lower:]')
    actual_checksum=$(printf '%s' "$actual_checksum" | tr '[:upper:]' '[:lower:]')
    [ "$actual_checksum" = "$expected_checksum" ] \
        || die "checksum mismatch for $archive"
    printf 'Verified SHA-256 checksum.\n'
else
    printf 'Skipping checksum verification because --no-verify was supplied.\n' >&2
fi

extract_dir="${temp_dir}/extract"
mkdir -p "$extract_dir"
tar -xzf "$archive_path" -C "$extract_dir"
binary_path="${extract_dir}/shore"
[ -f "$binary_path" ] || die "$archive does not contain shore"
chmod +x "$binary_path"

version_output=$("$binary_path" --version 2>&1) \
    || die "downloaded shore binary failed its version check"

mkdir -p "$INSTALL_DIR"
staged_binary="${INSTALL_DIR}/.shore-install.$$"
cp "$binary_path" "$staged_binary"
chmod +x "$staged_binary"
mv -f "$staged_binary" "${INSTALL_DIR}/shore"
staged_binary=""

printf 'Installed %s to %s/shore\n' "$version_output" "$INSTALL_DIR"
case ":${PATH:-}:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        print_path_guidance
        exit 0
        ;;
esac
printf 'Run: shore --help\n'
