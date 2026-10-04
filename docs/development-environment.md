# Reproducible Development Environment

The development container and Nix shell read pinned tool versions from
[`toolchain-versions.json`](../toolchain-versions.json). CI loads the same
manifest through `.github/actions/tool-versions`; it also fails if the native
`rust-toolchain.toml` or Stellar CLI deployment manifest pins have drifted.
The Nix inputs still require a committed `flake.lock` to freeze nixpkgs and
rust-overlay revisions; generate and review that lockfile when Nix is available
before treating shell package versions as immutable.

## Start

- VS Code Dev Containers: open the repository in WSL2 or Docker Desktop and run
  **Dev Containers: Reopen in Container**.
- Nix: run `nix develop` from the repository root.
- Full local verification: run `scripts/verify-local.sh` in the container or a
  POSIX shell after installing the pinned tools.

The verification script runs formatting, Clippy, workspace tests, contract
build lints, the release WASM build and size budget, and the TLA+ simulation. It
expects `TLA_TOOLS_JAR` to point to the pinned `tla2tools.jar` when the default
`.cache/tla2tools.jar` path is not used.

## Windows and macOS

Windows contributors are supported through WSL2 with Docker Desktop's WSL
integration. Keep the checkout inside the WSL2 filesystem (for example,
`~/src/Ledgerlens-contract`) for reliable file watching and build performance.
Native Windows is not supported for the shell-based verification, deployment,
and operational scripts; they depend on POSIX shell behavior and Unix command
line tools. Run those scripts inside WSL2 or the devcontainer. macOS users can
use the Nix shell or the Linux devcontainer through Docker Desktop.