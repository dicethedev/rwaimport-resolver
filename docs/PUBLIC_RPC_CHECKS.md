# Public RPC coverage and scheduled checks

The public registry at reviewed commit `a1a9a26f0b1e43a1ee4e26089f27a4fbbe66abda`
defines ten networks. All seven EVM networks are configured in
[`config/public-rpc.json`](../config/public-rpc.json):

| Network | Chain ID | Endpoint |
| --- | --- | --- |
| Ethereum | 1 | https://ethereum-rpc.publicnode.com |
| BNB Smart Chain | 56 | https://bsc-rpc.publicnode.com |
| Polygon | 137 | https://polygon-bor-rpc.publicnode.com |
| Robinhood Chain | 4663 | https://robinhood-rpc.publicnode.com |
| Base | 8453 | https://base-rpc.publicnode.com |
| Arbitrum One | 42161 | https://arbitrum-one-rpc.publicnode.com |
| Avalanche C-Chain | 43114 | https://avalanche-c-chain-rpc.publicnode.com |

BNB and Robinhood endpoints are published by PublicNode's [BNB gateway](https://bsc.publicnode.com/)
and [Robinhood gateway](https://robinhood.publicnode.com/). Robinhood's chain ID and
alternative first-party public endpoint appear in its [connection documentation](https://docs.robinhood.com/chain/connecting/).
The first-party endpoint did not produce a usable observation during initial testing;
the default therefore uses PublicNode. Operators can override providers through normal
resolver configuration.

Solana, Stellar and Aptos are checked using [ledger-specific adapters](NON_EVM_ADAPTERS.md)
and their public mainnet providers. They use network locators, not EVM chain IDs.
Reports contain an `observation` for these ledgers; only networks without an adapter
remain in `unsupportedNetworks`.

## Daily workflow

[`.github/workflows/public-rpc.yml`](../.github/workflows/public-rpc.yml) runs daily
at **08:23 Africa/Lagos (07:23 UTC)**, with a manual Actions trigger. It is intended
for the standalone resolver repository; GitHub must receive the workflow in that
repository's `.github/workflows/` directory on its default branch for scheduled runs.
GitHub schedules can be delayed; this is a periodic verification check, not a precise
time guarantee. See [GitHub's schedule documentation](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#schedule).

Each run checks out the resolver, builds the current registry main branch, records
its exact commit, builds the resolver and queries every configured EVM chain plus Solana, Stellar and Aptos. Set
repository variables `REGISTRY_REPOSITORY` or `REGISTRY_LIVE_REF` to override the
registry source/ref. This live-check ref is separate from production activation pins.

Checks cover registry network/provider configuration agreement, one active known
contract per chain, provider chain ID, block-pinned observations and all selected
registry claims. A new EVM network without configuration, missing deployment sample,
provider failure, mismatched claim or unavailable critical check fails the job.
Optional capability-read warnings are retained without changing selected claim rules.
All networks get a result when individual provider calls fail. The report includes
registry commit/revision, per-network results and any unsupported ledgers.

JSON reports are uploaded as the `public-rpc-report` Actions artifact for 30 days.
GitHub displays failed runs; account notification settings determine notifications.
The workflow does not deploy services, activate production registry releases, write
registry evidence, commit files or send external messages.

## Run locally

```sh
cargo build --locked
python3 scripts/live-smoke.py --resolver-only --strict --report reports/public-rpc.json
```

Without `--resolver-only`, the check starts the sibling compiled API too and resolves
through the API gateway. `--registry-dir` selects another built distribution. Without
`--strict`, live mismatches are reported but do not fail solely because they conflict;
missing live observations and network coverage gaps still fail.

The deterministic [resolver CI workflow](../.github/workflows/ci.yml) runs formatting,
Clippy, Rust tests and Python publication/coverage tests on pushes and pull requests.
It does not query public blockchains. Live checks use current external state, so a
failure requires checking the report to distinguish provider outage, source drift,
configuration gaps and software errors.
