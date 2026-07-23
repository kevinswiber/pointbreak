{
  description = "Pointbreak development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, fenix, ... }:
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
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rustToolchains.stable;
            rustc = rustToolchains.stable;
          };

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

          cli = rustPlatform.buildRustPackage {
            pname = "pointbreak";
            inherit version;
            src = sourceWithInspector;
            cargoLock.lockFile = ./Cargo.lock;

            # Match `just build-all`: building an artifact does not run the full
            # test suite. Tests remain an explicit `just test` / CI concern.
            doCheck = false;

            # Git supplies compile-time build identity and is the runtime backend.
            nativeBuildInputs = [
              pkgs.git
              pkgs.makeWrapper
            ];
            postFixup = ''
              wrapProgram "$out/bin/pointbreak" --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.git ]}
            '';

            meta.mainProgram = "pointbreak";
          };

          vscode = pkgs.buildNpmPackage {
            pname = "pointbreak-vscode";
            inherit version;
            src = ./.;
            sourceRoot = "source";
            npmRoot = "extensions/vscode";
            npmDepsHash = "sha256-zoXPLpbbDHgiq5lcvaVIjuKujBa6Hfay0sDbhGaKakY=";
            nativeBuildInputs = [
              cli
              pkgs.unzip
            ];
            buildPhase = ''
              runHook preBuild
              cd "$npmRoot"
              POINTBREAK_EXTENSION_CLEAN_VERSION=1 \
                POINTBREAK_EXTENSION_PROFILE=release \
                POINTBREAK_EXTENSION_BINARY=${cli}/bin/pointbreak \
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
          build-all = pkgs.symlinkJoin {
            name = "pointbreak-build-all-${version}";
            paths = [
              cli
              inspector
              vscode
            ];
          };
        }
      );

      # `nix flake check` builds every derivation under `checks`. This one realises
      # the pinned cocogitto (proving the from-source pin still compiles on a clean
      # machine) and asserts the version-critical tools resolve, so a broken pin or
      # version drift fails the flake rather than only surfacing in a live shell.
      checks = forEachSystem (
        pkgs:
        let
          cocogitto = mkCocogitto pkgs;
          rustToolchains = mkRustToolchains pkgs;
        in
        {
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
