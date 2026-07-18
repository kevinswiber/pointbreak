#!/bin/sh
# Hermetic contract tests for the withpointbreak/pointbreak Unix installer.
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)
installer="${repo_root}/scripts/install.sh"
temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/pointbreak-installer-test.XXXXXX")
trap 'rm -rf "$temp_dir"' 0
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
    *) printf 'installer self-test only supports macOS and Linux\n' >&2; exit 1 ;;
esac
case "$(uname -m)" in
    x86_64|amd64) arch=x64 ;;
    arm64|aarch64) arch=arm64 ;;
    *) printf 'unsupported self-test architecture: %s\n' "$(uname -m)" >&2; exit 1 ;;
esac

tag=v9.8.7-test
version=${tag#v}
target="${os}-${arch}"
archive="pointbreak-${version}-${target}.tar.gz"
release_dir="${temp_dir}/releases/${tag}"
payload_dir="${temp_dir}/payload"
install_dir="${temp_dir}/bin"
destination="${install_dir}/pointbreak"
neighbor="${install_dir}/shore"
mkdir -p "$release_dir" "$install_dir"

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

write_candidate() {
    candidate_version=$1
    installed_version=$2
    candidate_identity=$3
    installed_identity=$4
    output=$5
    cat > "$output" <<EOF
#!/bin/sh
if [ "\$#" -ne 3 ] || [ "\$1" != version ] || [ "\$2" != --format ] || [ "\$3" != json ]; then
    exit 64
fi
document_version='$candidate_version'
identity='$candidate_identity'
case "\$0" in
    */pointbreak)
        document_version='$installed_version'
        identity='$installed_identity'
        ;;
esac
commit=0123456789abcdef0123456789abcdef01234567
case "\$identity" in
    exact)
        printf '{"schema":"pointbreak.version","version":1,"cliVersion":"%s","build":{"source":"git","commit":"%s","describe":"v%s","dirty":false},"documents":{"pointbreak.version":1},"diagnostics":[]}\\n' "\$document_version" "\$commit" "\$document_version"
        ;;
    additive-fields-and-order)
        printf '{"extra":{"ignored":true},"diagnostics":[],"build":{"dirty":false,"describe":"v%s","extraBuild":"ignored","commit":"%s","source":"git"},"documents":{"unrelated.document":7,"pointbreak.version":1},"cliVersion":"%s","version":1,"schema":"pointbreak.version"}\\n' "\$document_version" "\$commit" "\$document_version"
        ;;
    wrong-tag)
        printf '{"schema":"pointbreak.version","version":1,"cliVersion":"%s","build":{"source":"git","commit":"%s","describe":"v0.0.0","dirty":false},"documents":{"pointbreak.version":1},"diagnostics":[]}\\n' "\$document_version" "\$commit"
        ;;
    dirty-build)
        printf '{"schema":"pointbreak.version","version":1,"cliVersion":"%s","build":{"source":"git","commit":"%s","describe":"v%s-dirty","dirty":true},"documents":{"pointbreak.version":1},"diagnostics":[]}\\n' "\$document_version" "\$commit" "\$document_version"
        ;;
    package-build)
        printf '{"schema":"pointbreak.version","version":1,"cliVersion":"%s","build":{"source":"package","commit":null,"describe":"package:%s","dirty":false},"documents":{"pointbreak.version":1},"diagnostics":[]}\\n' "\$document_version" "\$document_version"
        ;;
    short-commit)
        printf '{"schema":"pointbreak.version","version":1,"cliVersion":"%s","build":{"source":"git","commit":"0123456","describe":"v%s","dirty":false},"documents":{"pointbreak.version":1},"diagnostics":[]}\\n' "\$document_version" "\$document_version"
        ;;
    missing-build)
        printf '{"schema":"pointbreak.version","version":1,"cliVersion":"%s","documents":{"pointbreak.version":1},"diagnostics":[]}\\n' "\$document_version"
        ;;
    null-build)
        printf '{"schema":"pointbreak.version","version":1,"cliVersion":"%s","build":null,"documents":{"pointbreak.version":1},"diagnostics":[]}\\n' "\$document_version"
        ;;
    malformed-document)
        printf '{not-json\\n'
        ;;
    *) exit 65 ;;
esac
EOF
    chmod +x "$output"
}

make_archive() {
    candidate_version=$1
    installed_version=$2
    candidate_identity=$3
    installed_identity=$4
    layout=$5
    rm -rf "$payload_dir"
    mkdir -p "$payload_dir" "$release_dir"
    write_candidate "$candidate_version" "$installed_version" \
        "$candidate_identity" "$installed_identity" "${payload_dir}/pointbreak"
    cp "${repo_root}/LICENSE" "${repo_root}/NOTICE" "$payload_dir/"
    case "$layout" in
        exact) tar -czf "${release_dir}/${archive}" -C "$payload_dir" pointbreak LICENSE NOTICE ;;
        extra)
            printf 'unexpected payload\n' > "${payload_dir}/unexpected.txt"
            tar -czf "${release_dir}/${archive}" -C "$payload_dir" \
                pointbreak LICENSE NOTICE unexpected.txt
            ;;
        *) printf 'unknown fixture layout: %s\n' "$layout" >&2; exit 1 ;;
    esac
}

write_checksum() {
    checksum=$(sha256_file "${release_dir}/${archive}")
    printf '%s  %s\n' "$checksum" "$archive" > "${release_dir}/checksums.txt"
}
write_invalid_checksum() {
    printf '%064d  %s\n' 0 "$archive" > "${release_dir}/checksums.txt"
}
write_previous_install() {
    cat > "$destination" <<'EOF'
#!/bin/sh
printf 'previous pointbreak\n'
EOF
    chmod +x "$destination"
}
write_neighbor() { printf 'arbitrary neighboring bytes\nnot an executable\n' > "$neighbor"; }
prepare_upgrade() {
    mkdir -p "$install_dir"
    write_previous_install
    write_neighbor
    previous_hash=$(sha256_file "$destination")
    neighbor_hash=$(sha256_file "$neighbor")
}
assert_neighbor_unchanged() {
    test -f "$neighbor"
    test "$(sha256_file "$neighbor")" = "$neighbor_hash"
}
assert_previous_restored() {
    test -x "$destination"
    test "$(sha256_file "$destination")" = "$previous_hash"
    assert_neighbor_unchanged
    if find "$install_dir" -maxdepth 1 \
        \( -name '.pointbreak-install.*' -o -name '.pointbreak-backup.*' \
        -o -name '.pointbreak-transaction.*' \) | grep -q .; then
        printf 'installer left transaction files behind\n' >&2
        exit 1
    fi
}
run_installer() {
    POINTBREAK_INSTALLER_FIXTURE_ROOT="${temp_dir}/releases" \
        "$installer" --version="$tag" --prefix="$install_dir"
}
expect_failure() {
    scenario=$1
    shift
    if "$@" > "${temp_dir}/${scenario}.log" 2>&1; then
        printf 'installer accepted %s\n' "$scenario" >&2
        exit 1
    fi
    assert_previous_restored
}

help_output=$($installer --help)
printf '%s\n' "$help_output" | grep -F 'Pointbreak Review installer' >/dev/null
if printf '%s\n' "$help_output" | grep -i 'shore' >/dev/null; then
    printf 'installer help teaches a second executable\n' >&2
    exit 1
fi

# Fresh exact-tag install.
make_archive "$version" "$version" exact exact exact
write_checksum
fresh_output=$(SHELL=/bin/zsh run_installer)
printf '%s\n' "$fresh_output"
test -x "$destination"
test ! -L "$destination"
test ! -e "$neighbor"
printf '%s\n' "$fresh_output" | grep -F "Installed Pointbreak Review $version to $destination" >/dev/null
printf '%s\n' "$fresh_output" | grep -F "export PATH=\"${install_dir}:\$PATH\"" >/dev/null
printf '%s\n' "$fresh_output" | grep -F 'Then run: pointbreak --help' >/dev/null
if printf '%s\n' "$fresh_output" | grep -i 'shore' >/dev/null; then
    printf 'installer success output teaches a second executable\n' >&2
    exit 1
fi

# additive-fields-and-order: unrelated fields and order do not weaken required checks.
prepare_upgrade
make_archive "$version" "$version" additive-fields-and-order additive-fields-and-order exact
write_checksum
run_installer >/dev/null
assert_neighbor_unchanged

# Upgrade replaces only Pointbreak.
prepare_upgrade
make_archive "$version" "$version" exact exact exact
write_checksum
run_installer >/dev/null
test "$(sha256_file "$destination")" = "$(sha256_file "${payload_dir}/pointbreak")"
assert_neighbor_unchanged

# A hostile collision at the old predictable name is untouched.
prepare_upgrade
make_archive "$version" "$version" exact exact exact
write_checksum
collision_path_file="${temp_dir}/collision-path"
INSTALLER="$installer" INSTALL_DIR="$install_dir" NEIGHBOR="$neighbor" \
    TAG="$tag" COLLISION_PATH_FILE="$collision_path_file" \
    POINTBREAK_INSTALLER_FIXTURE_ROOT="${temp_dir}/releases" /bin/sh -c '
        collision="${INSTALL_DIR}/.pointbreak-install.$$"
        ln -s "$NEIGHBOR" "$collision"
        printf "%s\n" "$collision" > "$COLLISION_PATH_FILE"
        set -- --version="$TAG" --prefix="$INSTALL_DIR"
        . "$INSTALLER"
    ' >/dev/null
collision_path=$(sed -n '1p' "$collision_path_file")
test -L "$collision_path"
test "$(readlink "$collision_path")" = "$neighbor"
test "$(sha256_file "$neighbor")" = "$neighbor_hash"
test -x "$destination"
test ! -L "$destination"
test "$(sha256_file "$destination")" = "$(sha256_file "${payload_dir}/pointbreak")"
rm -f "$collision_path"

prepare_upgrade
make_archive "$version" "$version" exact exact exact
write_invalid_checksum
expect_failure checksum-failure run_installer
grep -F 'checksum mismatch' "${temp_dir}/checksum-failure.log" >/dev/null

prepare_upgrade
make_archive "$version" "$version" exact exact extra
write_checksum
expect_failure archive-layout-failure run_installer
grep -F 'invalid archive layout' "${temp_dir}/archive-layout-failure.log" >/dev/null

for scenario in wrong-tag dirty-build package-build short-commit malformed-document null-build; do
    prepare_upgrade
    make_archive "$version" "$version" "$scenario" "$scenario" exact
    write_checksum
    expect_failure "$scenario" run_installer
    grep -F 'version document did not match' "${temp_dir}/${scenario}.log" >/dev/null
done

# missing-build-after-v0.7.0 is rejected.
prepare_upgrade
make_archive "$version" "$version" missing-build missing-build exact
write_checksum
expect_failure missing-build-after-v0.7.0 run_installer

prepare_upgrade
make_archive 9.8.6-test 9.8.6-test exact exact exact
write_checksum
expect_failure version-mismatch run_installer

prepare_upgrade
make_archive "$version" "$version" exact exact exact
write_checksum
expect_failure replacement-failure env POINTBREAK_INSTALLER_FIXTURE_ROOT="${temp_dir}/releases" \
    POINTBREAK_INSTALLER_TEST_FAIL_REPLACE=1 "$installer" --version="$tag" --prefix="$install_dir"
grep -F 'could not replace' "${temp_dir}/replacement-failure.log" >/dev/null

prepare_upgrade
make_archive "$version" 9.8.6-test exact exact exact
write_checksum
expect_failure post-replacement-verification-failure run_installer
grep -F 'installed Pointbreak version document did not match' \
    "${temp_dir}/post-replacement-verification-failure.log" >/dev/null

# legacy-v0.7.0 is the sole allowed missing-build document.
original_tag=$tag
original_version=$version
original_archive=$archive
original_release_dir=$release_dir
tag=v0.7.0
version=0.7.0
archive="pointbreak-${version}-${target}.tar.gz"
release_dir="${temp_dir}/releases/${tag}"
prepare_upgrade
make_archive "$version" "$version" missing-build missing-build exact
write_checksum
run_installer >/dev/null
assert_neighbor_unchanged

# A present-but-null build is not the legacy no-build document.
prepare_upgrade
make_archive "$version" "$version" null-build null-build exact
write_checksum
expect_failure legacy-null-build run_installer
tag=$original_tag
version=$original_version
archive=$original_archive
release_dir=$original_release_dir

if grep -i 'shore' "$installer" >/dev/null; then
    printf 'installer implementation references a neighboring executable\n' >&2
    exit 1
fi
printf 'install.sh self-test ok\n'
