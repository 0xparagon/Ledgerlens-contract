#!/bin/bash -eu
# OSS-Fuzz build script (issue #1232). Runs inside base-builder-rust, which
# provides nightly cargo, cargo-fuzz and the sanitizer flags.
cd "$SRC/ledgerlens-contract/fuzz"
# The repo pins 1.81.0 for WASM reproducibility; fuzzing needs the image's nightly.
rm -f ../rust-toolchain.toml
cargo fuzz build -O --debug-assertions
for target in verkle_proof range_proof invocation_campaign; do
  cp "target/x86_64-unknown-linux-gnu/release/$target" "$OUT/"
  (cd "corpus/$target" && zip -q "$OUT/${target}_seed_corpus.zip" ./*)
done
