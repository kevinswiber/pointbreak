{
  description = "Pointbreak development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # Kept as a parallel packaging experiment until its dependency-artifact
    # cache proves a meaningful win over buildRustPackage for Pointbreak.
    crane.url = "github:ipetkov/crane/v0.23.4";
  };

  outputs =
    {
      self,
      nixpkgs,
      fenix,
      crane,
      ...
    }:
    let
      # Systems the dev shell is built for: Linux and macOS on x86_64 and arm64.
      systems = [
        "aarch64-linux"
        "x86_64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];
      forEachSystem = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # Fenix provides a fixed stable compiler for normal work and only the
      # nightly formatter required by rustfmt.toml's `unstable_features`.
      # Combining components avoids rustup's mutable per-user toolchains.
      mkRustToolchains =
        pkgs:
        let
          fenixPkgs = fenix.packages.${pkgs.stdenv.hostPlatform.system};
        in
        {
          stable = fenixPkgs.stable.toolchain;
          dev = fenixPkgs.combine [
            fenixPkgs.stable.cargo
            fenixPkgs.stable.rustc
            fenixPkgs.stable.clippy
            fenixPkgs.stable.rust-src
            fenixPkgs.latest.rustfmt
          ];
        };

      # cocogitto pinned to 6.5.0 to match CI (.github/workflows/*: cargo binstall
      # cocogitto@6.5.0) and mise.toml. nixpkgs ships 7.0.0, which this repo is NOT
      # ready for: cog 7 changes the release tag lifecycle the signed-tag finalizer
      # depends on (scripts/finalize-cocogitto-release-tag.sh:64) and adds native
      # scope validation the commit-msg hook still shims by hand (cog.toml:63-67).
      # Moving to cog 7 means bumping the flake, the three CI pins, removing that
      # shim, and re-validating `just release-bump-selftest` together in a
      # dedicated PR.
      mkCocogitto =
        pkgs:
        pkgs.cocogitto.overrideAttrs (_: rec {
          version = "6.5.0";
          src = pkgs.fetchFromGitHub {
            owner = "cocogitto";
            repo = "cocogitto";
            tag = version;
            hash = "sha256-aAVoPPeuJN6QPcuc3oBF93dP6U+74bAoSDw93XR01Vo=";
          };
          cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
            inherit src;
            name = "cocogitto-${version}-vendor";
            hash = "sha256-yDpZHkRKsWXXHuSKnzhGrjsFLUFZEpC23tcU3FeUZK8=";
          };
          # 6.5.0's completion subcommand differs from 7.x; completions aren't
          # needed here, so skip the postInstall that generates them.
          postInstall = "";
        });
    in
    {
      # `nix fmt` formats the flake with the canonical RFC-166 formatter.
      formatter = forEachSystem (pkgs: pkgs.nixfmt);

      devShells = forEachSystem (
        pkgs:
        let
          cocogitto = mkCocogitto pkgs;
          rustToolchains = mkRustToolchains pkgs;
          fenixPkgs = fenix.packages.${pkgs.stdenv.hostPlatform.system};
          # Experimental Windows (msvc) cross toolchain: stable host cargo/rustc plus
          # the prebuilt std for both shipped Windows targets. Paired with cargo-xwin
          # (which supplies the MSVC CRT/SDK), this cross-compiles a cargo-nextest
          # archive on Linux/macOS for execution on a real Windows machine.
          windowsCrossToolchain = fenixPkgs.combine [
            fenixPkgs.stable.cargo
            fenixPkgs.stable.rustc
            fenixPkgs.targets."aarch64-pc-windows-msvc".stable.rust-std
            fenixPkgs.targets."x86_64-pc-windows-msvc".stable.rust-std
          ];
        in
        {
          default = pkgs.mkShell {
            # Everything on PATH inside `nix develop`.
            packages = with pkgs; [
              # --- Rust ---
              # Stable cargo/rustc/clippy plus nightly rustfmt. The direct Fenix
              # cargo binary has no rustup `+toolchain` proxy; the shell selects
              # direct commands for the Justfile compatibility variables below.
              rustToolchains.dev

              # --- Dev tooling (mirrors mise.toml [tools]) ---
              just
              cargo-nextest
              cargo-edit
              cocogitto # `cog`, pinned to 6.5.0 for CI parity — see mkCocogitto above
              gh
              jq
              nodejs_22

              # --- Native build deps ---
              # libsqlite3-sys / rusqlite / zstd / lmdb-master3-sys all compile
              # bundled C, so cargo needs a working C toolchain and pkg-config.
              # NixOS has no global cc; mkShell's stdenv provides one, and these make
              # it explicit.
              pkg-config
              git # used by build.rs (identity capture) and by cog hooks
            ];

            shellHook = ''
              # Outside Nix, Justfile recipes keep using rustup's explicit stable
              # and nightly selectors. This shell has a single Fenix toolchain, so
              # both recipe classes invoke its direct cargo binary instead.
              export POINTBREAK_CARGO_STABLE=cargo
              export POINTBREAK_CARGO_NIGHTLY=cargo

              # Replicate mise's `[env] _.path`: prefer freshly-built binaries.
              # Guarded so re-sourcing the hook doesn't stack duplicate entries.
              case ":$PATH:" in
                *":$PWD/target/debug:"*) ;;
                *) export PATH="$PWD/target/release:$PWD/target/debug:$PATH" ;;
              esac

              # Install cocogitto's commit-msg / pre-push hooks once (mise did this via
              # a postinstall step). Idempotent: only runs when the hook is missing.
              if [ -d .git ] && [ ! -f .git/hooks/commit-msg ]; then
                cog install-hook --all >/dev/null 2>&1 \
                  && echo "pointbreak: installed cocogitto git hooks"
              fi

              echo "pointbreak dev shell — $(rustc --version), $(rustfmt --version), just, nextest, cog, node $(node --version)"
            '';
          };

          # Experimental Windows-msvc cross shell. Produces a cargo-nextest archive
          # (prebuilt test binaries) that runs on a real Windows machine needing no
          # Rust toolchain. cargo-xwin fetches the MSVC CRT/SDK on first use, which
          # needs network — so this is an impure `nix develop` workflow, not a
          # sandboxed derivation. See `just windows-cross-archive`.
          windows-cross = pkgs.mkShell {
            packages = [
              windowsCrossToolchain
              pkgs.cargo-nextest
              pkgs.cargo-xwin
              pkgs.llvmPackages.clang-unwrapped # clang-cl for the bundled-C deps
              pkgs.lld # lld-link
              pkgs.llvm # llvm-lib, llvm-rc
              pkgs.just
              pkgs.git
            ];
            env.XWIN_ACCEPT_LICENSE = "1";
          };
        }
      );

      # `nix build .#build-all` is the store-backed counterpart of `just build-all`:
      # it builds the Inspector asset, CLI, and host-targeted VSIX without mutating
      # the checkout or accessing npm during a sandboxed build.
      packages = forEachSystem (
        pkgs:
        let
          version = "0.8.0";
          rustToolchains = mkRustToolchains pkgs;
          craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchains.stable;
          # rustfmt.toml enables unstable options, so the format check needs the
          # combined dev toolchain (stable cargo, nightly rustfmt) rather than the
          # stable-only toolchain the compile/lint derivations build against.
          craneLibDev = (crane.mkLib pkgs).overrideToolchain rustToolchains.dev;

          inspector = pkgs.buildNpmPackage {
            pname = "pointbreak-inspector";
            inherit version;
            src = ./src/cli/inspect/web;
            npmDepsHash = "sha256-5naTmTgI9JsRLe2nezLMGjkhtJmjPv/TLFNxBo0xOXU=";
            buildPhase = ''
              runHook preBuild
              npm run build -- --outfile="$out/app.js"
              runHook postBuild
            '';
            installPhase = "true";
          };

          # The Rust binary embeds the Inspector asset. Substitute the Nix-built
          # bundle into a copied source tree, so its served UI is exactly the
          # companion `inspector` package rather than a stale checked-in artifact.
          sourceWithInspector =
            pkgs.runCommand "pointbreak-source-with-inspector" { nativeBuildInputs = [ pkgs.coreutils ]; }
              ''
                cp -R ${./.} "$out"
                chmod -R u+w "$out"
                cp ${inspector}/app.js "$out/src/cli/inspect/assets/app.js"
              '';

          # Crane's dummy source isolates this derivation from ordinary Rust
          # source edits, so the delivery package and test check reuse dependency
          # and dev-dependency artifacts across Nix builds.
          cargoArtifacts = craneLib.buildDepsOnly {
            pname = "pointbreak";
            inherit version;
            src = craneLib.cleanCargoSource sourceWithInspector;
            cargoLock = ./Cargo.lock;
            env.POINTBREAK_BUILD_CHANNEL = "nix-dev";
          };

          # Build the distributable artifact without coupling ordinary consumers
          # to the complete repository test suite. `cliNextest` below is exposed
          # through `nix flake check` as the full Git-less quality gate.
          cli = craneLib.buildPackage {
            pname = "pointbreak";
            inherit version;
            src = sourceWithInspector;
            cargoLock = ./Cargo.lock;
            inherit cargoArtifacts;
            doCheck = false;
            env.POINTBREAK_BUILD_CHANNEL = "nix-dev";
            nativeBuildInputs = [
              pkgs.git
              pkgs.makeWrapper
            ];
            postFixup = ''
              mkdir -p "$out/libexec"
              mv "$out/bin/pointbreak" "$out/libexec/pointbreak"
              makeWrapper "$out/libexec/pointbreak" "$out/bin/pointbreak" \
                --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.git ]}
            '';
            meta.mainProgram = "pointbreak";
          };

          # This complete Git-less integration suite is a flake check rather
          # than a dependency of the delivery artifact above.
          cliNextest = craneLib.cargoNextest {
            pname = "pointbreak";
            inherit version;
            src = sourceWithInspector;
            cargoLock = ./Cargo.lock;
            inherit cargoArtifacts;
            doInstallCargoArtifacts = false;
            cargoNextestExtraArgs = "--no-tests pass";
            env.POINTBREAK_BUILD_CHANNEL = "nix-dev";
            nativeBuildInputs = [
              pkgs.git
              pkgs.jq
              pkgs.nodejs_22
            ];
          };

          # Hermetic clippy gate reusing the shared dependency artifacts. Mirrors
          # `just lint`'s clippy invocation so the flake check and the Justfile gate
          # stay in lockstep; `-D warnings` makes any lint fail the check.
          cliClippy = craneLib.cargoClippy {
            pname = "pointbreak";
            inherit version;
            src = sourceWithInspector;
            cargoLock = ./Cargo.lock;
            inherit cargoArtifacts;
            env.POINTBREAK_BUILD_CHANNEL = "nix-dev";
            cargoClippyExtraArgs = "--workspace --all-targets --all-features -- -D warnings";
            # Build scripts run under clippy; build.rs shells out to git.
            nativeBuildInputs = [ pkgs.git ];
          };

          # Format check with the pinned nightly rustfmt. Compiles nothing, so it
          # needs neither the dependency artifacts nor git — only the cleaned source.
          cliFmt = craneLibDev.cargoFmt {
            pname = "pointbreak";
            inherit version;
            src = craneLib.cleanCargoSource sourceWithInspector;
          };

          vscode = pkgs.buildNpmPackage {
            pname = "pointbreak-vscode";
            inherit version;
            src = ./.;
            npmRoot = "extensions/vscode";
            # `npmRoot` scopes the build, but the dependency fetcher needs a
            # source whose root contains this nested project's lockfile.
            npmDeps = pkgs.fetchNpmDeps {
              src = ./extensions/vscode;
              hash = "sha256-zoXPLpbbDHgiq5lcvaVIjuKujBa6Hfay0sDbhGaKakY=";
            };
            # Keep the packaging runtime aligned with the pinned developer Node.
            # keytar 7.9.0 does not compile against Nixpkgs' default Node 24.
            nodejs = pkgs.nodejs_22;
            # VSCE packaging does not use keytar's credential API. Avoid rebuilding
            # that optional native addon after the offline npm installation.
            npmRebuildFlags = [ "--ignore-scripts" ];
            nativeBuildInputs = [
              cli
              pkgs.unzip
            ];
            buildPhase = ''
              runHook preBuild
              cd "$npmRoot"
              POINTBREAK_EXTENSION_CLEAN_VERSION=1 \
                POINTBREAK_EXTENSION_PROFILE=release \
                POINTBREAK_EXTENSION_BINARY=${cli}/libexec/pointbreak \
                node scripts/package-local.mjs
              runHook postBuild
            '';
            installPhase = ''
              install -Dm444 ../../target/vsix/*/release/*.vsix "$out/pointbreak.vsix"
            '';
          };
        in
        {
          default = cli;
          inherit cli inspector vscode;
          cli-nextest = cliNextest;
          cli-clippy = cliClippy;
          cli-fmt = cliFmt;
          # Curate the aggregate as a delivery surface. The Inspector bundle is
          # embedded in the CLI and `libexec/pointbreak` is only the VSIX input;
          # both remain available from their owning package outputs.
          build-all = pkgs.runCommand "pointbreak-build-all-${version}" { } ''
            mkdir -p "$out/bin"
            ln -s ${cli}/bin/pointbreak "$out/bin/pointbreak"
            ln -s ${vscode}/pointbreak.vsix "$out/pointbreak.vsix"
          '';
        }
      );

      # `nix flake check` runs the full Rust gate hermetically: the complete
      # Git-less Nextest suite, clippy (`-D warnings`), and the nightly rustfmt
      # format check — clippy and the tests share the crane dependency artifacts.
      # It also realises the pinned cocogitto (proving the from-source pin still
      # compiles on a clean machine). This keeps package consumers on the fast
      # delivery path while making test, lint, format, or tool drift fail the flake.
      checks = forEachSystem (
        pkgs:
        let
          cocogitto = mkCocogitto pkgs;
          rustToolchains = mkRustToolchains pkgs;
        in
        {
          cli-nextest = self.packages.${pkgs.stdenv.hostPlatform.system}.cli-nextest;
          clippy = self.packages.${pkgs.stdenv.hostPlatform.system}.cli-clippy;
          fmt = self.packages.${pkgs.stdenv.hostPlatform.system}.cli-fmt;
          devshell-tools =
            pkgs.runCommand "devshell-tools-check"
              {
                nativeBuildInputs = [
                  rustToolchains.dev
                  cocogitto
                  pkgs.nodejs_22
                  pkgs.just
                  pkgs.cargo-nextest
                ];
              }
              ''
                cargo --version >/dev/null
                rustc --version >/dev/null
                rustfmt --version | grep -q nightly
                cog --version | grep -qw 6.5.0
                node --version | grep -q '^v22\.'
                just --version >/dev/null
                cargo-nextest nextest --version >/dev/null
                touch "$out"
              '';
        }
      );
    };
}
