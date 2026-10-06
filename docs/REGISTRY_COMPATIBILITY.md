# Registry compatibility review

Reviewed [dicethedev/rwaimport-registry](https://github.com/dicethedev/rwaimport-registry)
at commit [`a1a9a26`](https://github.com/dicethedev/rwaimport-registry/tree/a1a9a26f0b1e43a1ee4e26089f27a4fbbe66abda)
on October 6, 2026. The registry package reports version 0.3.0; its generated bundle
and product records use schema version 2. Supporting records and the distribution
manifest use schema version 1. The reviewed schema shapes are compatible with this workspace. The local schema
identifier namespace has since migrated to `https://rwaimport.xyz/schemas/`.

## Implemented consumer requirements

| Registry feature | Resolver behavior |
| --- | --- |
| Product / underlying / deployment separation | Exact EVM deployment lookup returns distinct product and underlying records |
| Issuers and product-specific organization roles | Returns issuer plus organization records with their assigned roles |
| Valuation and compliance | Returns original records with availability states and source references intact |
| Standards and standard evidence | Returns catalog records, specification status and deployment evidence; capability probes remain separate |
| Recorded deployment verification | Compares live metadata, code hash, implementation and proxy admin when recorded |
| Field-level sources and claims | Returns product and underlying evidence, including provenance and review dates |
| Distribution manifest | Verifies registry byte length and SHA-256 when manifest.json exists, both on load and refresh |
| References and duplicate records | Rejects broken consumed references, source links, EVM chain IDs and exact deployment duplicates |
| Unknown / not-applicable metadata | Preserves registry states; missing values do not become verified facts |
| Undisclosed deployments | Does not infer a contract address from a product record |

The manifest comparison detects inconsistent artifacts. It does not authenticate a
publisher: registry files and their manifest must come from the same trusted release.
A missing manifest permits the existing raw-distribution integration; a present
invalid manifest causes load/refresh failure. Publish complete distributions
atomically to avoid temporary mismatches during deployment.

## Extended consumer capabilities

- [Deployment policies](DEPLOYMENT_POLICIES.md) compare explicit owners, role holders,
  pause state, implementation/admin targets and configured transfer scenarios.
- Additional standard reads cover ERC-4626 conversions/previews, ERC-3643 pause and
  relationships, and configured ERC-1404/CMTAT getters. Unsupported variants remain
  unavailable; no full standard-conformance claim is inferred from the catalog.
- [Non-EVM policies](NON_EVM_ADAPTERS.md) cover reported Token-2022 extension controls,
  Stellar signer thresholds, Aptos resource fields and simple legacy Coin locators.
- [Operations](OPERATIONS.md) include provider circuits, probes, metrics, structured
  logs, stale-registry readiness, shared envelopes, ledger caching and bounded batches.
- [Registry publication](REGISTRY_UPDATES.md) includes a watcher and a pinned-artifact
  workflow with an explicitly configured runtime activation job.

Remaining extensions include generic Aptos Coin types, raw Token-2022 TLV decoding,
additional deployment ABI variants, product completeness scoring and indexed history.

Continuous monitoring, change history from new observations and indexed state remain
indexer work. No resolver query writes reviewed registry evidence or source records.

## Review sources

- [Registry model and build outputs](https://github.com/dicethedev/rwaimport-registry/blob/a1a9a26f0b1e43a1ee4e26089f27a4fbbe66abda/README.md)
- [Distribution and reliability migrations](https://github.com/dicethedev/rwaimport-registry/blob/a1a9a26f0b1e43a1ee4e26089f27a4fbbe66abda/MIGRATIONS.md)
- [Deployment verification schema](https://github.com/dicethedev/rwaimport-registry/blob/a1a9a26f0b1e43a1ee4e26089f27a4fbbe66abda/schemas/deployments.schema.json)
- [Manifest generation](https://github.com/dicethedev/rwaimport-registry/blob/a1a9a26f0b1e43a1ee4e26089f27a4fbbe66abda/scripts/build-index.ts)
- [Standards catalog](https://github.com/dicethedev/rwaimport-registry/blob/a1a9a26f0b1e43a1ee4e26089f27a4fbbe66abda/standards/README.md)
- [Provider attribution](https://github.com/dicethedev/rwaimport-registry/blob/a1a9a26f0b1e43a1ee4e26089f27a4fbbe66abda/ATTRIBUTION.md)
