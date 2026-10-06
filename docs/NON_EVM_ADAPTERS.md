# Solana, Stellar and Aptos resolution

The API gateway forwards these routes to the resolver, which looks up the deployment
in the same validated registry snapshot and reads a public ledger provider:

| Ledger | Route | Selected live checks |
| --- | --- | --- |
| Solana | `GET /v1/resolve/network/solana/{mint}` | Mainnet genesis, finalized account slot, initialized mint, SPL program owner, recorded decimals |
| Stellar | `GET /v1/resolve/network/stellar/{issuer}/{assetCode}` | Public-network passphrase, exact issuer/asset-code pair, recorded symbol |
| Aptos | `GET /v1/resolve/network/aptos/{metadataAddress}` | Mainnet chain ID, resources pinned to a ledger version, fungible-asset Metadata resource, recorded name/symbol/decimals |

The numeric `GET /v1/resolve/{chainId}/{address}` route continues to identify EVM
contracts. Non-EVM routes return `input.network`, `input.address`, `input.assetCode`
and an `observation` object; they do not fabricate EVM block hashes or contract fields.
Responses also contain product/deployment context, registry revision, evidence,
`evidenceFreshness`, comparisons, warnings and a verification scope.

Stellar issuer addresses can issue multiple assets. An asset code is mandatory and
is compared to the registry's recorded `verification.observedSymbol`. Missing or
ambiguous registry matches do not become verified identities. Aptos lookups support
fungible-asset metadata objects; legacy Move Coin types need a different locator and
are not implemented. Solana requires a parsed initialized mint owned by the legacy
SPL Token program or Token-2022; an ordinary token account is not a mint.

`VERIFIED` means all selected comparisons succeeded, with more than existence alone.
A conflicting available comparison produces `MISMATCH`; unavailable comparisons
produce `PARTIAL`. A missing registry match produces `UNKNOWN`, even when a ledger
observation is available. These outcomes do not certify legal rights, reserves,
complete token-standard conformance or safety.

## Providers and failover

Default read-only mainnet providers are recorded in
[`config/public-ledger.json`](../config/public-ledger.json):

- Solana: `https://api.mainnet-beta.solana.com`, then `https://solana-rpc.publicnode.com`.
- Stellar Horizon: `https://horizon.stellar.org`.
- Aptos REST: `https://api.mainnet.aptoslabs.com/v1/`.

Override these at startup with `LEDGER_URLS`, a JSON object whose keys are `solana`,
`stellar` and `aptos` and whose values are ordered arrays of HTTP(S) base URLs. For
Aptos, include the `/v1/` path. Omitted networks have no configured live provider.
Queries and fragments are rejected for ledger base URLs. Keep private endpoints in
local environment configuration.

For EVM, `RPC_URLS` remains the primary-provider map. `RPC_BACKUP_URLS` optionally
maps numeric chain IDs to ordered arrays of backup URLs. Both paths share the
request deadline across providers. REST transport failures retry within `RPC_RETRIES`
and `RPC_TIMEOUT_MS`; the overall `RESOLVE_TIMEOUT_MS` still bounds the operation. Unavailable reads can trigger failover; a valid
observation is compared as returned rather than trying another provider to obtain a
matching answer. Wrong-network responses fail immediately. Observations are never
assembled from different providers. Provider failures remain visible in warnings.

Horizon asset reads describe indexed state and are not pinned to a specific ledger;
the response explicitly states this limitation. Aptos resources use `ledger_version`;
Solana records the finalized account slot. Public providers can rate-limit requests.
Aptos HTTP errors, including missing-account errors, are unavailable observations
rather than proof that an asset exists or does not exist.

## Evidence and permissions

Evidence freshness summarizes source review schedules and the age of the registry's
recorded observation separately from live verification. Date-only review schedules
use midnight UTC. A missing schedule is unknown. Source URLs are not fetched during
resolution. Stored account-data hashes are not compared as immutable claims because
supply, balances and other mutable data can change them.

Solana mint/freeze authorities, Stellar asset flags and Aptos object ownership are
observations, not an exhaustive permission audit. Token-2022 extension policies,
Stellar signer/threshold policies, deployment-specific Aptos permissions and legacy
Coin adapters remain future work. Registry catalog inclusion alone does not imply a
live detector exists for every standard.

Protocol references: [Solana RPC](https://solana.com/docs/rpc/http),
[Stellar assets](https://developers.stellar.org/docs/data/apis/horizon/api-reference/resources/assets/object),
and [Aptos resources](https://aptos.dev/network/blockchain/resources).
