FROM mcr.microsoft.com/devcontainers/base:ubuntu-24.04

COPY toolchain-versions.json /tmp/toolchain-versions.json
RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
       ca-certificates curl openjdk-17-jre-headless python3 xz-utils \
    && rm -rf /var/lib/apt/lists/* \
    && stellar="$(python3 -c 'import json; print(json.load(open("/tmp/toolchain-versions.json"))["stellar_cli"])')" \
    && tla="$(python3 -c 'import json; print(json.load(open("/tmp/toolchain-versions.json"))["tla_tools"])')" \
    && shellcheck="$(python3 -c 'import json; print(json.load(open("/tmp/toolchain-versions.json"))["shellcheck"])')" \
    && curl --proto '=https' --tlsv1.2 -sSfL "https://github.com/stellar/stellar-cli/releases/download/v${stellar}/stellar-cli-${stellar}-x86_64-unknown-linux-gnu.tar.gz" -o /tmp/stellar-cli.tar.gz \
    && tar -xzf /tmp/stellar-cli.tar.gz -C /usr/local/bin stellar \
    && mkdir -p /opt/tla \
    && curl --proto '=https' --tlsv1.2 -sSfL "https://github.com/tlaplus/tlaplus/releases/download/v${tla}/tla2tools.jar" -o /opt/tla/tla2tools.jar \
    && curl --proto '=https' --tlsv1.2 -sSfL "https://github.com/koalaman/shellcheck/releases/download/v${shellcheck}/shellcheck-v${shellcheck}.linux.x86_64.tar.xz" -o /tmp/shellcheck.tar.xz \
    && tar -xJf /tmp/shellcheck.tar.xz -C /tmp \
    && install "/tmp/shellcheck-v${shellcheck}/shellcheck" /usr/local/bin/shellcheck \
    && rm -rf /tmp/stellar-cli.tar.gz /tmp/shellcheck.tar.xz \
    && echo 'export TLA_TOOLS_JAR=/opt/tla/tla2tools.jar' >> /etc/profile.d/ledgerlens.sh

USER vscode
RUN contract_rust="$(python3 -c 'import json; print(json.load(open("/tmp/toolchain-versions.json"))["contract_rust"])')" \
    && audit_rust="$(python3 -c 'import json; print(json.load(open("/tmp/toolchain-versions.json"))["audit_rust"])')" \
    && mutants="$(python3 -c 'import json; print(json.load(open("/tmp/toolchain-versions.json"))["cargo_mutants"])')" \
    && cargo_audit="$(python3 -c 'import json; print(json.load(open("/tmp/toolchain-versions.json"))["cargo_audit"])')" \
    && cargo_deny="$(python3 -c 'import json; print(json.load(open("/tmp/toolchain-versions.json"))["cargo_deny"])')" \
    && cargo_cyclonedx="$(python3 -c 'import json; print(json.load(open("/tmp/toolchain-versions.json"))["cargo_cyclonedx"])')" \
    && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "$contract_rust" \
    && . "$HOME/.cargo/env" \
    && rustup toolchain install "$audit_rust" --profile minimal \
    && rustup target add wasm32-unknown-unknown --toolchain "$contract_rust" \
    && rustup component add rustfmt clippy rust-src --toolchain "$contract_rust" \
    && cargo +"$audit_rust" install cargo-mutants --version "$mutants" --locked \
    && cargo +"$audit_rust" install cargo-audit --version "$cargo_audit" --locked \
    && cargo +"$audit_rust" install cargo-deny --version "$cargo_deny" --locked \
    && cargo +"$contract_rust" install cargo-cyclonedx --version "$cargo_cyclonedx" --locked

ENV PATH="/home/vscode/.cargo/bin:${PATH}"
ENV TLA_TOOLS_JAR="/opt/tla/tla2tools.jar"