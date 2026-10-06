# RWAimport resolver

Runnable Rust HTTP service for contract identification and selected live verification.
It reads the registry distribution directly and queries configured blockchain RPCs.
The web app calls the existing API; the API calls this service. Continuous discovery and historical monitoring belong to the indexer.

```text
Web → rwaimport-api :3000 → rwaimport-resolver :3001
                                  ├─ registry dist/registry.json + ../schemas/
                                  └─ configured blockchain RPC providers
```

## Requirements and setup

Install a current stable Rust toolchain with Cargo. Building the registry requires
Node.js and npm; follow the registry repository's engine and package-manager
requirements. Running the optional API gateway also requires its supported Node.js
version and installed dependencies.

The resolver consumes a built registry distribution, not raw `assets/` directories.
For a standalone resolver checkout, clone and build the registry beside it:

```sh
git clone https://github.com/dicethedev/rwaimport-registry.git ../rwaimport-registry
cd ../rwaimport-registry
npm ci
npm run build
cd ../rwaimport-resolver
```

Skip the clone step when the sibling checkout already exists. The resulting layout
is `rwaimport-registry/dist/registry.json` with registry-owned schemas at
`rwaimport-registry/schemas/`. To consume another checkout or release, set
`REGISTRY_DIST_DIR` to its absolute `dist` directory. The service rejects unsupported
schema versions and invalid records before it starts serving requests.

## Registry updates and version pinning

Use the [pinned registry update workflow](docs/REGISTRY_UPDATES.md) to build a chosen
GitHub revision, validate its distribution, and atomically publish it for both
services. `scripts/sync-registry.py` records the commit/checksum and supports rollback.
Existing refresh polling picks up compatible asset updates without restarting the
services. It does not silently follow GitHub main. Registry schema identifiers use
`https://rwaimport.xyz/schemas/`; validation reads local schema files.

## Run the resolver

From the parent workspace, start the resolver in one terminal:

```sh
cd rwaimport-resolver
RPC_URLS_FILE=./config/public-rpc.json cargo run --locked
```

In another terminal, from the parent workspace:

```sh
cd rwaimport-api
npm ci
npm run build
RESOLVER_BASE_URL=http://127.0.0.1:3001 npm start
```

Resolve BUIDL through the API:

```sh
curl http://127.0.0.1:3000/v1/resolve/1/0x6a9DA2D710BB9B700acde7Cb81F10F1fF8C89041
```

Direct internal calls use the same route and `{ data }` envelope at port 3001.
The API validates upstream responses, bounds their size, applies its normal request
rate limit and error envelope, and returns `Cache-Control: no-store`. Resolution
is documented in the API's `/openapi.json`. With no `RESOLVER_BASE_URL`, the catalog
still runs and resolution requests return `503 RESOLVER_DISABLED`.

## Response contract

Successful responses use `{ "data": { ... } }`. The same response reaches callers
through the API gateway. Requests accept a positive decimal chain ID and a 20-byte
EVM address; addresses are normalized to lowercase. This normalization does not
validate an EIP-55 checksum. Query parameters are rejected.

| Field | Contents |
| --- | --- |
| status | VERIFIED, PARTIAL, MISMATCH or UNKNOWN |
| input | Normalized chain ID and address |
| matched | Product/issuer/underlying IDs and expected deployment claims, or null |
| product / issuer / underlyingAsset | Distinct registry identity records, or null for an unknown deployment |
| deployment / network | Deployment details and its network definition |
| organizations | Organization records paired with product-specific roles |
| standards | Deployment's standard catalog records, including specification status |
| compliance / valuation | Original registry records, availability states and evidence references |
| evidence | Product and underlying sources and field-level claims |
| contract | Live observation with block, metadata, supply, proxy, admin and capabilities, or null when unavailable |
| checks | Selected expected/actual comparisons and their individual outcomes |
| warnings | Source/observation limitations; optional read warnings also appear under contract |
| registryRevision / registryGeneratedAt | SHA-256 revision of consumed registry bytes and registry build timestamp |
| resolvedAt / verificationScope | Resolution timestamp and the claims this verification covers |

An unidentified address returns HTTP 200 with UNKNOWN. Invalid input returns HTTP
400. A provider configured for the wrong chain returns HTTP 503. RPC failures retain
known identity and unavailable checks rather than converting the address to UNKNOWN.
Direct resolver errors include `statusCode` and `error`; the API gateway adds its
standard request ID and error envelope.

## Connected sources

The registry loader validates schema-v2 records against the registry-owned JSON
schemas and checks core references, chain IDs, evidence sources, underlying product
links, related standards and duplicate EVM contracts. When `manifest.json` is present,
its registry byte length and SHA-256 must match before loading or refreshing. This
checks artifact integrity, not publisher authenticity. Exact lookup maps `(chainId, lowercase address)` to deployment, product,
issuer and underlying. Responses include legal/product fields, compliance and
source/claim evidence, valuation, organization roles and standard catalog details. Registry observations remain recorded evidence, separate
from newly read contract state. The distribution is polled and replaced atomically;
failed refreshes retain the last valid revision and report degraded health.

The opt-in [public RPC configuration](config/public-rpc.json) uses PublicNode:

| Network | Chain ID | Provider documentation |
| --- | --- | --- |
| Ethereum | 1 | [Ethereum RPC](https://ethereum.publicnode.com/) |
| Base | 8453 | [Base RPC](https://base.publicnode.com/) |
| Polygon | 137 | [Polygon RPC](https://polygon.publicnode.com/) |
| Arbitrum | 42161 | [Arbitrum RPC](https://arbitrum.publicnode.com/) |
| Avalanche | 43114 | [Avalanche RPC](https://avalanche.publicnode.com/) |
| BNB Smart Chain | 56 | [BNB RPC](https://bsc.publicnode.com/) |
| Robinhood Chain | 4663 | [Robinhood RPC](https://robinhood.publicnode.com/) |

Use your own providers by setting `RPC_URLS` to a JSON object, or `RPC_URLS_FILE` to
its filename. URLs are operator configuration; request parameters cannot choose a
provider. RPC URLs and provider error bodies are never returned in public responses.
Other registry EVM chains can be configured the same way. [Non-EVM adapters](docs/NON_EVM_ADAPTERS.md)
use network locators and the providers in `config/public-ledger.json`.
The service binds to loopback by default; deployments should keep it
on an internal network behind the API.

## Live reads and verification

Each observation verifies `eth_chainId`, captures the latest block number/hash,
and pins `eth_getCode`, `eth_call` and `eth_getStorageAt` to that hash with
`requireCanonical: true`. A final block lookup detects a reorg during observation.
Providers must support EIP-1898 block-hash state queries. Unsupported or failed
reads are unavailable, not a successful verification.

Implemented reads include `name`, `symbol`, `decimals`, full uint256 `totalSupply`,
optional `owner`, and the EIP-1967 proxy admin slot. Dynamic ABI strings and legacy bytes32 strings are decoded.
Proxy detection covers canonical EIP-1167 clones, EIP-1967 implementations/beacons,
legacy OpenZeppelin and EIP-1822 slots. An undetermined proxy does not prove a direct
contract. Reads continue at the submitted address, rather than substituting its
implementation for the token's state.

Capabilities include ERC-20 metadata and ERC-165's valid/invalid interface probes.
For known deployments claiming ERC-4626, ERC-3643 or ERC-1400, the service also probes
vault asset/total assets, identity registry/compliance relationships, or granularity
respectively. These are capability observations, **not full standard-conformance
checks**. Generic role enumeration and custom ABI policies remain future work.

Selected comparisons are existence, deployment-observed name/symbol/decimals,
recorded runtime-code SHA-256, implementation address, and recorded proxy admin where the registry has those values. Product display names/symbols are never assumed to be token metadata:
Ethereum BUIDL's recorded token symbol is `BUIDL-I`, while the product symbol is
`BUIDL`. Explicit deployment decimals/implementation take precedence over recorded
observations. There is no comparison of changing supply against a stale registry.

| Outcome | Meaning |
| --- | --- |
| VERIFIED | Known deployment; existence and at least one other selected claim established; all selected claims match |
| PARTIAL | Known deployment, but a selected check is unavailable or there are insufficient comparable claims |
| MISMATCH | A selected claim conflicts, including absent bytecode at a known deployment |
| UNKNOWN | No registry deployment match; no trust judgment |

`checks` records expected/actual values and `verified`, `unavailable` or `mismatched`
status. Results include registry revision, registry generation timestamp,
`resolvedAt`, and observation block number/hash. `verificationScope` explains that
these checks do not establish legal rights, reserves, safety or full standard
conformance. Missing RPC configuration preserves registry identity with unavailable
checks. A provider chain mismatch fails with HTTP 503.

## Configuration

Environment files are not automatically loaded. See [.env.example](.env.example).

| Variable | Default / bounds |
| --- | --- |
| RESOLVER_BIND | 127.0.0.1:3001 |
| REGISTRY_DIST_DIR | sibling registry dist at local build path; set explicitly when moving the binary |
| RPC_URLS / RPC_URLS_FILE | none; inline JSON takes precedence |
| RPC_TIMEOUT_MS | 2000; 100–30000 per attempt |
| RPC_RETRIES | 1; 0–2 transient retries, no retry of contract revert errors |
| RESOLVE_TIMEOUT_MS | 8000; 1000–120000 including waiting for a coalesced request |
| REGISTRY_REFRESH_INTERVAL_MS | 60000; 1000–3600000 |
| RESOLVER_CACHE_TTL_MS | 15000; 0–60000 |
| RESOLVER_CACHE_MAX_ENTRIES | 256; 0–10000 |
| RESOLVER_CACHE_MAX_BYTES | 8388608; 0–67108864 serialized payload/key budget |
| RESOLVER_MAX_CONCURRENCY | 16; 1–256 |

The bounded TTL cache uses chain/address plus registry content revision. Policy is
fixed for a process; restart after changing it. It coalesces identical concurrent
work and clears on registry updates. Critical unavailable reads/provider failures
are not cached. Set any cache limit to zero to disable storage. Cached results keep
their original observation timestamp/block; they are not relabeled as fresh reads.
RPC response bodies are capped at 2 MiB; API resolver bodies at 4 MiB. RPC redirects
are disabled. Read calls bound execution gas. The API's `RESOLVER_TIMEOUT_MS`
(default 9000) should exceed this service's total deadline.

`/health/live` reports process liveness. `/health/ready` reports loaded registry
revision, generation timestamp, supported EVM chains, configured RPC chain IDs and
refresh health (`ok`/`degraded`). It does not probe providers or impose a registry-age
policy. Source timestamps and configured-chain lists remain visible for operators.
SIGINT/SIGTERM trigger graceful HTTP shutdown.

## Scheduled public RPC checks

A [daily GitHub Actions workflow](.github/workflows/public-rpc.yml) checks all seven
registry EVM networks at 08:23 Africa/Lagos, and can be triggered manually. It records
the checked registry commit, detects missing network coverage, verifies selected
claims and saves JSON reports. It does not deploy or change runtime registry pins.
See [public RPC checks](docs/PUBLIC_RPC_CHECKS.md) for activation requirements, failure
meanings and commands. Solana, Stellar and Aptos have ledger-specific adapters; see
[non-EVM resolution](docs/NON_EVM_ADAPTERS.md) for routes, providers and limits.

## Checks

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
python3 -m unittest discover -s tests -p 'test_*.py'
```

Tests exercise a local HTTP mock RPC, registry/schema validation, input errors,
matching/mismatched metadata, missing code, reverted reads, provider-chain mismatch,
RPC response IDs, retries, reorgs, caching and registry refresh. Fixture schema
copies are snapshots of the registry-owned schemas, used only for isolated tests.

For an **opt-in live test** using the real registry, public RPC configuration and
both compiled services, run from the parent workspace:

```sh
cargo build --locked --manifest-path rwaimport-resolver/Cargo.toml
npm --prefix rwaimport-api run build
python3 rwaimport-resolver/scripts/live-smoke.py
```

This starts temporary services, resolves one registered deployment per configured
network through the API, prints selected check results, and shuts the processes
down. A live mismatch is reported honestly; it is not necessarily a software error.
Public RPC availability is external, so this script is separate from deterministic
tests. Use `--resolver-only` to run without the API checkout, `--report` to save JSON,
and `--strict` to fail on mismatched or unavailable selected claims. Network coverage
is checked against every active EVM network in the consumed registry.

Implementation references: [JSON-RPC](https://ethereum.org/developers/docs/apis/json-rpc/),
[ABI decoding](https://docs.soliditylang.org/en/latest/abi-spec.html),
[EIP-1967](https://eips.ethereum.org/EIPS/eip-1967),
[EIP-1167](https://eips.ethereum.org/EIPS/eip-1167),
[ERC-165](https://eips.ethereum.org/EIPS/eip-165),
[ERC-3643](https://eips.ethereum.org/EIPS/eip-3643).

## Source layout

```text
src/
  input/          EVM address and chain validation
  registry/       Distribution/schema validation and exact deployment matching
  rpc/            Bounded JSON-RPC transport and block-pinned live reads
  contracts/      Observation types and ABI decoding
  proxy/          Common proxy patterns and storage slots
  standards/      Capability interpretation
  verification/   Selected expected/actual comparisons
  resolver/       Injectable domain orchestration
  cache/          Bounded in-memory TTL results
  service.rs      Async resolution, cache coordination and registry refresh
  http.rs         Internal HTTP routes and error mapping
  config.rs       Validated operator configuration
  main.rs         Startup, polling and graceful shutdown
  types/ errors/  Response models and domain failures
tests/            Deterministic core and HTTP/RPC integration tests
fixtures/         Registry/schema test snapshots and attribution
config/           Opt-in public RPC configuration
scripts/          Pinned registry sync and opt-in live test
docs/             Registry compatibility review and remaining priorities
```

## Registry compatibility and next steps

The [registry compatibility review](docs/REGISTRY_COMPATIBILITY.md) records the exact
GitHub revision inspected, consumer requirements already implemented, and remaining
work. Evidence-freshness summaries, Solana/Stellar/Aptos adapters and ordered provider
failover are implemented. Priorities include additional standard-specific policies,
Token-2022 extension checks and deployment-specific permission checks. Inclusion in the registry standards catalog does not mean a live detector
exists for every standard.

Registry-derived test fixtures and schema snapshots retain their upstream
[MIT notice](fixtures/REGISTRY_LICENSE.txt). Provider URLs, attribution and recorded
source evidence are preserved in returned registry context.
