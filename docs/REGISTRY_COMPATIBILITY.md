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

## Remaining implementation priorities

Evidence-freshness summaries and ordered provider failover are implemented.
[Non-EVM adapters](NON_EVM_ADAPTERS.md) support Solana mints, Stellar issuer/asset pairs
and Aptos fungible-asset objects. Remaining gaps are not inferred from the catalog.

1. **Evidence lifecycle:** source-review and recorded-observation age summaries are
   available. Preserve source hashes, immutable URLs,
   source versions and snapshot paths without treating an external link as live data.
2. **Additional EVM policies:** add specification-specific probes for ERC-1404,
   CMTAT and other catalog standards as applicable to actual deployments. Treat draft
   standards such as ERC-8056 according to their recorded status; stock-split display
   values must remain distinct from raw balances and total supply.
3. **Non-EVM policy depth:** add Token-2022 extension interpretation, Stellar signer
   policies, Aptos permissions and legacy Coin locators. Existing ledger routes
   validate selected identity and recorded metadata claims.
4. **Deployment policy coverage:** expand tests and probes for actual contract ABIs,
   permissions and roles. Zero EIP-1967 slots or unavailable owner reads do not prove
   that a contract lacks upgrade or transfer controls.
5. **Context and operations:** consider completeness scores, product history and
   relationships and registry-age readiness policy. Ordered provider failover is available. Completeness
   describes documentation coverage; it must not become a live-verification score.

Continuous monitoring, change history from new observations and indexed state remain
indexer work. No resolver query writes reviewed registry evidence or source records.

## Review sources

- [Registry model and build outputs](https://github.com/dicethedev/rwaimport-registry/blob/a1a9a26f0b1e43a1ee4e26089f27a4fbbe66abda/README.md)
- [Distribution and reliability migrations](https://github.com/dicethedev/rwaimport-registry/blob/a1a9a26f0b1e43a1ee4e26089f27a4fbbe66abda/MIGRATIONS.md)
- [Deployment verification schema](https://github.com/dicethedev/rwaimport-registry/blob/a1a9a26f0b1e43a1ee4e26089f27a4fbbe66abda/schemas/deployments.schema.json)
- [Manifest generation](https://github.com/dicethedev/rwaimport-registry/blob/a1a9a26f0b1e43a1ee4e26089f27a4fbbe66abda/scripts/build-index.ts)
- [Standards catalog](https://github.com/dicethedev/rwaimport-registry/blob/a1a9a26f0b1e43a1ee4e26089f27a4fbbe66abda/standards/README.md)
- [Provider attribution](https://github.com/dicethedev/rwaimport-registry/blob/a1a9a26f0b1e43a1ee4e26089f27a4fbbe66abda/ATTRIBUTION.md)
