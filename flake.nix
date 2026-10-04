{
  description = "LedgerLens reproducible contract development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      versions = builtins.fromJSON (builtins.readFile ./toolchain-versions.json);
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in {
      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          rust = pkgs.rust-bin.stable.${versions.contract_rust}.default.override {
            extensions = [ "rust-src" "rustfmt" "clippy" ];
            targets = [ "wasm32-unknown-unknown" ];
          };
          rustAudit = pkgs.rust-bin.stable.${versions.audit_rust}.default;
        in {
          default = pkgs.mkShell {
            packages = [
              rust rustAudit pkgs.curl pkgs.git pkgs.gnumake
              pkgs."jdk${versions.java}" pkgs.shellcheck
            ];
            RUST_AUDIT_CARGO = "${rustAudit}/bin/cargo";
            shellHook = ''
              export PATH="${rust}/bin:${rustAudit}/bin:$HOME/.cargo/bin:$PATH"
              export TLA_TOOLS_JAR="$PWD/.cache/tla2tools.jar"
              if [ ! -f "$TLA_TOOLS_JAR" ]; then
                mkdir -p "$PWD/.cache"
                ${pkgs.curl}/bin/curl -fsSL "https://github.com/tlaplus/tlaplus/releases/download/v${versions.tla_tools}/tla2tools.jar" -o "$TLA_TOOLS_JAR"
              fi
              if ! command -v stellar >/dev/null || ! stellar --version 2>/dev/null | grep -q '${versions.stellar_cli}'; then
                RUSTC="${rustAudit}/bin/rustc" "$RUST_AUDIT_CARGO" install stellar-cli --version '${versions.stellar_cli}' --locked
              fi
              if ! cargo-mutants --version 2>/dev/null | grep -q '${versions.cargo_mutants}'; then
                RUSTC="${rustAudit}/bin/rustc" "$RUST_AUDIT_CARGO" install cargo-mutants --version '${versions.cargo_mutants}' --locked
              fi
              if ! cargo-audit --version 2>/dev/null | grep -q '${versions.cargo_audit}'; then
                RUSTC="${rustAudit}/bin/rustc" "$RUST_AUDIT_CARGO" install cargo-audit --version '${versions.cargo_audit}' --locked
              fi
              if ! cargo-deny --version 2>/dev/null | grep -q '${versions.cargo_deny}'; then
                RUSTC="${rustAudit}/bin/rustc" "$RUST_AUDIT_CARGO" install cargo-deny --version '${versions.cargo_deny}' --locked
              fi
              if ! cargo-cyclonedx --version 2>/dev/null | grep -q '${versions.cargo_cyclonedx}'; then
                RUSTC="${rust}/bin/rustc" cargo install cargo-cyclonedx --version '${versions.cargo_cyclonedx}' --locked
              fi
              shellcheck --version | grep -F 'version: ${versions.shellcheck}' >/dev/null || {
                echo "ShellCheck ${versions.shellcheck} is required by toolchain-versions.json." >&2
                return 1
              }
            '';
          };
        });
    };
}