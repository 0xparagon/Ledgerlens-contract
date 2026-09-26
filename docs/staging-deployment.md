# Staging Continuous Deployment

Every merge to `main` that passes the required `CI gate` check is deployed to
the staging instance by `.github/workflows/staging-deploy.yml`, giving
integrators a stable target that tracks `main`.

## Flow

1. `Contract CI` completes successfully for a push to `main`
   (`workflow_run`). PR runs, failed runs and other branches never deploy.
2. The workflow builds and optimizes the WASM at that exact commit.
3. `scripts/staging-deploy.sh` installs the WASM, then upgrades through the
   standard `propose_upgrade` → `execute_upgrade` flow.
4. It runs `scripts/health_check.sh` and a canary check that the on-chain WASM
   hash equals the built hash.
5. On any failure it re-proposes and executes the previous WASM hash and
   re-runs the same checks against it, so the rollback state is verified.
6. The status file is published to `deploy/staging/status.json` on the
   `staging-status` branch, and on failure an issue labelled
   `staging-deploy-failure` is opened with links to logs and evidence.

Deployments share the `staging-deploy` concurrency group with
`cancel-in-progress: false`: they queue and are never interrupted mid-upgrade.

## Status file

`https://raw.githubusercontent.com/<org>/Ledgerlens-contract/staging-status/deploy/staging/status.json`

```json
{
  "network": "testnet",
  "contracts": { "ledgerlens-score": "C..." },
  "version": "0.x.y",
  "commit": "<sha>",
  "wasm_sha256": "<hash now on chain>",
  "previous_wasm_sha256": "<hash before this run>",
  "last_deployment": { "status": "deployed|unchanged|rolled_back|rollback_failed", "at": "...", "run_url": "..." }
}
```

## Staging key: scope and rotation

- `STAGING_ADMIN_SECRET_KEY` is stored only as a secret of the `staging`
  GitHub environment. Restrict that environment to the `main` branch.
- The key is the admin of the staging instance only. It must never be a
  signer on any mainnet or other shared contract, and must hold only enough
  testnet XLM for fees.
- `STAGING_CONTRACT_ID` is an environment variable of `staging`.
- The staging instance is initialized with a zero upgrade timelock and a
  single-signer upgrade policy; production policies are unaffected.
- Rotate every 90 days and immediately after any suspected exposure or
  maintainer offboarding: generate a new key, transfer staging admin to it
  (see `docs/key-rotation-runbook.md`), update the secret, then revoke the old
  key. Rotations are recorded in the environment's audit log and reviewed by
  a second maintainer.

## Testing the rollback path

```bash
DRY_RUN_FAIL_VERIFY=1 scripts/staging-deploy.sh --dry-run testnet CID ident target/x.wasm /tmp/s.json
# exit 1, status "rolled_back"
```

To exercise it end to end, push a deliberately broken change (for example
one that makes `get_version` panic) to a test branch and run the workflow
against a disposable staging instance: the health check fails, the previous
WASM is restored and verified, and a failure issue is opened.

If rollback itself fails the script exits 3 with status `rollback_failed`;
follow `docs/incident-response-runbook.md`.
