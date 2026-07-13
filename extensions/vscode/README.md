# Pointbreak

Pointbreak Review companion for Visual Studio Code. In development.

## Local dogfood

From the repository root, build a host-specific VSIX with a bundled release CLI:

```sh
just extension-package
code --install-extension extensions/vscode/pointbreak-*.vsix
```

Open a Git worktree with a Pointbreak store, then open the Pointbreak activity-bar view. The
extension checks the bundled CLI handshake before it reads review data.

To use a development build without repackaging, set `pointbreak.binaryPath` to the absolute path
of that `shore` binary and reload the extension host.
