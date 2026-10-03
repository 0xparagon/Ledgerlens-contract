FROM gcr.io/oss-fuzz-base/base-builder-rust
RUN git clone --depth 1 https://github.com/Ledger-Lenz/Ledgerlens-contract ledgerlens-contract
WORKDIR ledgerlens-contract
COPY build.sh $SRC/
