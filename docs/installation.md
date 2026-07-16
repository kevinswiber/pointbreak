# Installation

Pointbreak Review publishes prebuilt `pointbreak` binaries for macOS, Linux, and Windows. The install
scripts select the archive for the current platform and verify it against the release's
`checksums.txt` before writing the binary.

## macOS and Linux

Install the latest release into `~/.local/bin`:

```sh
curl -fsSL https://raw.githubusercontent.com/withpointbreak/pointbreak/main/scripts/install.sh | sh
```

If `~/.local/bin` is not already in `PATH`, the installer prints a command for your current shell and
names the appropriate zsh or bash configuration file for a permanent change. Run the command, then
verify the result:

```sh
pointbreak --version
```

The script also supports `wget` when run from a local checkout.

### Pin a version or change the install directory

Pass installer options after `sh -s --`:

```sh
curl -fsSL https://raw.githubusercontent.com/withpointbreak/pointbreak/main/scripts/install.sh \
  | sh -s -- --version=v0.7.0 --prefix="$HOME/bin"
```

`--version` accepts a release with or without its leading `v`. The default is the latest published,
non-prerelease GitHub release. `--prefix` names the directory that will contain `pointbreak`.

## Windows PowerShell

Install the latest release into `%LOCALAPPDATA%\Pointbreak\bin`:

```powershell
irm https://raw.githubusercontent.com/withpointbreak/pointbreak/main/scripts/install.ps1 | iex
```

The installer adds that directory to your user `PATH` when necessary. Restart the terminal before
running:

```powershell
pointbreak --version
```

To pin a version, choose another install directory, or leave `PATH` unchanged, invoke the downloaded
script as a script block:

```powershell
$Install = [scriptblock]::Create((irm https://raw.githubusercontent.com/withpointbreak/pointbreak/main/scripts/install.ps1))
& $Install -Version v0.7.0 -InstallDir "$HOME\bin" -NoModifyPath
```

## Checksum verification

Verification is on by default and fails closed. The installer stops without replacing `pointbreak` if:

- `checksums.txt` cannot be downloaded;
- the selected archive has no valid entry;
- SHA-256 tooling is unavailable on macOS or Linux; or
- the downloaded archive's checksum does not match.

You can bypass verification explicitly with `--no-verify` on macOS or Linux, or `-NoVerify` on
Windows. This is intended for exceptional situations where you have verified the archive another
way.

To inspect a script before running it:

```sh
curl -fsSL -o install.sh https://raw.githubusercontent.com/withpointbreak/pointbreak/main/scripts/install.sh
less install.sh
sh install.sh
```

On Windows:

```powershell
irm https://raw.githubusercontent.com/withpointbreak/pointbreak/main/scripts/install.ps1 -OutFile install.ps1
Get-Content .\install.ps1
.\install.ps1
```

## Install with Cargo

The published crate is named `pointbreak` and installs the `pointbreak` command:

```sh
cargo install pointbreak
pointbreak --version
```

This path builds from source and requires a current Rust toolchain. The release installers do not.

## Upgrading to 0.7.0

Release `0.7.0` is a one-release hard cutover. Before its first use:

1. Stop every process that can write Review state.
2. Move owner-controlled state and config offline, preserving the directory contents:

   | pre-`0.7.0` operational location | `0.7.0` location |
   | --- | --- |
   | `<repo>/.shore/` | `<repo>/.pointbreak/` |
   | `<git-common-dir>/shore/` | `<git-common-dir>/pointbreak/` |
   | `<git-common-dir>/shore.link.json` | `<git-common-dir>/pointbreak.link.json` |
   | `$XDG_DATA_HOME/shore` | `$XDG_DATA_HOME/pointbreak` |
   | `$HOME/.shore` | `$HOME/.pointbreak` |
   | `%APPDATA%\shore` | `%APPDATA%\pointbreak` |

   Move a linked clone's shared common-directory store once, not once per worktree. If you set an
   explicit user home, move that directory to the location now selected by `POINTBREAK_HOME`.
3. Update environment and configuration references to the canonical `POINTBREAK_*` names and
   Pointbreak paths.
4. Run `pointbreak store paths --repo <path> --format json` to confirm the canonical repository,
   common-directory, binding, home, and key locations, then verify readback with commands such as
   `pointbreak revision list` and `pointbreak history`.

Rollback is the inverse filesystem move performed while writers remain stopped. Pointbreak provides
no runtime fallback, compatibility alias, automatic migration, migration CLI, or dual read/write
window. The existing `pointbreak store migrate` command serves an older Pointbreak store-topology
change; it does not perform this `0.7.0` namespace cutover.

## Supported platforms

| Target | Operating system | Architecture | Archive |
| --- | --- | --- | --- |
| `darwin-x64` | macOS | Intel 64-bit | `.tar.gz` |
| `darwin-arm64` | macOS | Apple silicon | `.tar.gz` |
| `linux-x64` | Linux (glibc) | x86-64 | `.tar.gz` |
| `linux-arm64` | Linux (glibc) | ARM64 | `.tar.gz` |
| `alpine-x64` | Linux (musl/Alpine) | x86-64 | `.tar.gz` |
| `alpine-arm64` | Linux (musl/Alpine) | ARM64 | `.tar.gz` |
| `win32-x64` | Windows | x86-64 | `.zip` |
| `win32-arm64` | Windows | ARM64 | `.zip` |

The macOS/Linux installer requires `tar`, plus either `curl` or `wget`, and either `sha256sum` or
the macOS-provided `shasum`. The Windows installer uses built-in PowerShell archive and hashing
commands.

## Manual download

Download the archive for your target and `checksums.txt` from the
[GitHub releases page](https://github.com/withpointbreak/pointbreak/releases). For example, on Apple
silicon macOS:

```sh
VERSION=0.7.0
TARGET=darwin-arm64
ARCHIVE="pointbreak-${VERSION}-${TARGET}.tar.gz"
BASE="https://github.com/withpointbreak/pointbreak/releases/download/v${VERSION}"

curl -fsSLO "${BASE}/${ARCHIVE}"
curl -fsSLO "${BASE}/checksums.txt"
grep "  ${ARCHIVE}$" checksums.txt | shasum -a 256 -c -
tar -xzf "$ARCHIVE"
install -m 0755 pointbreak "$HOME/.local/bin/pointbreak"
```

On Linux, replace `shasum -a 256` with `sha256sum`. Windows archives contain `pointbreak.exe` and can be
verified with `Get-FileHash -Algorithm SHA256` before using `Expand-Archive`.

Archives downloaded with `curl` on macOS normally need no quarantine adjustment. If a browser adds
the quarantine attribute, remove it from the extracted binary with:

```sh
xattr -d com.apple.quarantine ./pointbreak
```
