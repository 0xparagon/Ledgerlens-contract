# Model Card Template

> Template for the model card whose digest is anchored in the LedgerLens model
> version registry (see issue #1169). Fill in every section, then compute the
> card digest and register it together with the signed headline metrics.

## 1. Model identity

- **Model name:**
- **Model version:**
- **Registering authority (address / key id):**
- **Registry entry / transaction reference:**
- **Card digest (bytes32):**
- **Card digest algorithm:** `keccak256` over the canonical UTF-8 bytes of this card
- **Card URI / content address (optional):**

## 2. Intended use

- **Primary use case:**
- **Out-of-scope uses:**
- **Intended consumers:**

## 3. Training data description

- **Data sources:**
- **Collection window:**
- **Preprocessing / filtering:**
- **Known gaps or biases in the data:**

## 4. Evaluation metrics

Report the compact metric set that is anchored on-chain alongside the card
 digest. Use the same units and rounding as the registry entry.

| Metric | Value | Dataset / split | Notes |
| --- | --- | --- | --- |
| Precision at fixed recall (recall = ___) | | | |
| Calibration error (e.g. ECE) | | | |
| Additional metric | | | |

- **Evaluation harness / commit:**
- **Metric signing authority:**
- **Signature scheme:**

## 5. Confidence calibration bands

Describe how the model's confidence scores map to the calibration bands used
by the score contract, and how the reported calibration error was measured.

- **Band boundaries:**
- **Calibration method:**
- **Observed calibration error:**

## 6. Known limitations

- **Failure modes:**
- **Populations with degraded performance:**
- **Assumptions that may not hold:**

## 7. Deprecation and lifecycle

- **Deprecation criteria:**
- **Superseded by (model version):**
- **Notes on interplay with deprecation:** once a version is deprecated, its
  anchored card digest remains immutable and readable for provenance; any
  correction requires registering a new model version with a new card digest.

## 8. Provenance and immutability

- The card digest is immutable once the model version is approved.
- Corrections are published as a new model version, never by mutating an
  approved card.
- Provenance queries return the anchored digest so consumers and auditors can
  verify what a version claims to be.

## 9. Changelog

| Date | Version | Change | Author |
| --- | --- | --- | --- |
| | | | |
