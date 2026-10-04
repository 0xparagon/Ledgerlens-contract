# OSS-Fuzz onboarding evaluation

Spike for issue #1232. Harnesses live in [`fuzz/`](../fuzz) and are committed regardless of
whether the OSS-Fuzz application goes ahead.

## Targets

| Target | Entry point | Property | Seeds |
|---|---|---|---|
| `verkle_proof` | `verify_membership` → `verkle::decode_proof` | arbitrary proof bytes return `bool`, never panic/trap | `fuzz/corpus/verkle_proof` |
| `range_proof` | `verify_score_range_proof` → `Bulletproof::from_bytes` | arbitrary proof bytes return `bool`, never panic/trap | `fuzz/corpus/range_proof` |
| `invocation_campaign` | `invocation_fuzzer::{validate,execute}_campaign` | any JSON campaign is rejected or executed via `Result` | copy of `tools/invocation-fuzzer/corpus` |

The decoders are private modules, so harnesses reach them through the public contract functions.
That exercises the same bytes-to-struct path an attacker controls and needs no visibility changes.
Not covered yet: the gate-expression evaluator, credential decoders and replay input parsers. They
have no public pure entry point today. Each needs a thin `pub` (or `#[cfg(fuzzing)]`) wrapper
before it can get a harness. Plan about 0.5 day per target.

## Running locally

```bash ignore
cargo +nightly install cargo-fuzz
cd fuzz && cargo +nightly fuzz run verkle_proof corpus/verkle_proof -- -max_total_time=60
```

OSS-Fuzz helper validation, using the files in [`fuzz/oss-fuzz/`](../fuzz/oss-fuzz):

```bash ignore
cp -r fuzz/oss-fuzz <oss-fuzz>/projects/ledgerlens-contract
cd <oss-fuzz>
python3 infra/helper.py build_image ledgerlens-contract
python3 infra/helper.py build_fuzzers ledgerlens-contract
python3 infra/helper.py check_build ledgerlens-contract
python3 infra/helper.py run_fuzzer ledgerlens-contract verkle_proof -- -max_total_time=60
```

`build.sh` deletes `rust-toolchain.toml` inside the builder because the repo pins 1.81.0 for
reproducible WASM, and cargo-fuzz needs the image's nightly. This affects only the fuzz build.

## OSS-Fuzz acceptance and obligations

- **Acceptance.** OSS-Fuzz accepts projects with significant user impact or critical-infrastructure
  status. A young smart-contract repo may be declined. [ClusterFuzzLite](https://google.github.io/clusterfuzzlite/)
  has no such bar.
- **Contacts.** `primary_contact` must be a Google-account email. It is set to the SECURITY.md
  address, `security@ledgerlens.io`. At least one maintainer should be in `auto_ccs`.
- **Response times.** Bugs are auto-filed in the OSS-Fuzz tracker and disclosed 90 days after
  filing, or 30 days after a fix, whichever comes first. The SECURITY.md process should name an
  owner who triages the tracker weekly.
- **Build health.** A broken build emails the contacts, and projects that stay broken are
  deprecated. `fuzz/` must keep compiling on nightly whenever `soroban-sdk` is bumped.
- **Disclosure interaction.** The OSS-Fuzz 90-day deadline applies to fuzz findings in place of
  the SECURITY.md private-report timeline. Before submitting, SECURITY.md needs a line stating that
  OSS-Fuzz findings follow Google's disclosure policy.

## Recommendation

**Start with ClusterFuzzLite and defer the OSS-Fuzz application.** Reasons:

1. The harnesses are the same. ClusterFuzzLite reuses `fuzz/oss-fuzz/{Dockerfile,build.sh,project.yaml}`
   unchanged (moved to `.clusterfuzzlite/`), so no work is lost.
2. There is no acceptance risk and no external disclosure clock. Crashes stay private in GitHub
   Actions artifacts and go through the existing SECURITY.md process.
3. It can run on PRs (short, diff-targeted) and on a nightly batch job. That covers most of the
   value for a codebase this size.

Revisit OSS-Fuzz once the contract has mainnet integrators. At that point the free compute and
long-running corpus are worth the disclosure obligations.

### Effort estimate

| Work | Effort |
|---|---|
| Harnesses + seeds for the three targets above (this PR) | done |
| First local `helper.py build_fuzzers` / `check_build` run and fixes | 0.5 day |
| ClusterFuzzLite PR + nightly workflows | 0.5–1 day |
| Wrappers + harnesses for the gate evaluator, credential decoder, replay parsers | 1.5 days |
| OSS-Fuzz application, SECURITY.md update, contact setup (if pursued later) | 0.5 day + review wait |
| Ongoing: triage and nightly-build upkeep | about 1 hour/week |
