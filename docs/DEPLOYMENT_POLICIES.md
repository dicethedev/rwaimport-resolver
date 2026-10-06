# Deployment permission and standard policies

Set `DEPLOYMENT_POLICIES_FILE` to an operator-reviewed JSON file. It is loaded and
validated at startup; restart the service when changing policies. The SHA-256 of
that file is included in `policyVersion`, alongside the resolver policy format version.
The supplied [example](../config/deployment-policies.example.json) contains illustrative
expectations, including a hypothetical Solana authority expectation. It is not a
review of those deployments and is never enabled automatically.

The registry currently records selected deployment observations, rather than complete
role-holder expectations. An organization being called an issuer or transfer agent
is insufficient evidence that a particular wallet holds an on-chain permission.

```json
{
  "schemaVersion": 1,
  "deployments": [{
    "input": {"chainId": 1, "address": "0x1111111111111111111111111111111111111111"},
    "evmChecks": [
      {"field": "owner", "signature": "owner()", "expected": "0x2222222222222222222222222222222222222222"},
      {"field": "paused", "signature": "paused()", "expected": false}
    ]
  }]
}
```

Each deployment has at most 32 checks. The file is limited to 1 MiB and 4,096
policies. Duplicate deployment keys, duplicate field names, invalid argument types,
unknown fields and unrecognized getters fail startup. Policies cannot submit
transactions. Checks are namespaced as `policy.<field>` in responses.

## EVM checks

All policy reads use the same block hash and provider as the contract observation;
the block is checked again before returning. Reads are restricted to these getters:

| Purpose | Supported signatures |
| --- | --- |
| Ownership | `owner()`, `pendingOwner()` |
| Roles and permissions | `hasRole(bytes32,address)`, `getRoleAdmin(bytes32)`, `isAgent(address)`, `isFrozen(address)`, `paused()` |
| Upgrade checks | `proxiableUUID()` plus explicitly configured role checks |
| ERC-3643 relationships | `identityRegistry()`, `compliance()`, `canTransfer(address,address,uint256)` |
| ERC-4626 | `asset()`, `totalAssets()`, `convertToAssets(uint256)`, `convertToShares(uint256)`, `previewDeposit(uint256)`, `previewRedeem(uint256)`, `maxDeposit(address)`, `maxRedeem(address)` |
| ERC-1404 scenarios | `detectTransferRestriction(address,address,uint256)`, `messageForTransferRestriction(uint8)` |
| CMTAT deployment variants | `ruleEngine()`, `version()`, `VERSION()` and applicable ownership/role/pause/restriction getters above |

Use `args` for encoded arguments. Addresses use `0x` plus 40 hex characters, role
IDs use `0x` plus 64 hex characters, and unsigned integers are decimal strings.
Unsigned integer expectations are also strings, preserving the full uint256 range.
Boolean expectations are JSON booleans. Zero-address ownership can be compared.

`target` defaults to `deployment`. Set it to `implementation` to read the detected
implementation (for example its UUPS UUID), or `proxyAdmin` to read the EIP-1967 admin
contract. An undetected target is unavailable, not assumed absent. Checking an admin
contract's owner requires that its actual ABI exposes `owner()`.

Mint, burn, pause and upgrade role IDs must come from the deployed ABI or reviewed
sources. The resolver does not assume that every contract uses OpenZeppelin role
names, or enumerate all role members. ERC-1404 scenarios apply only to the exact
sender, recipient and amount configured. They do not prove all transfers will succeed.

Without an explicit policy, registry-selected metadata/admin/implementation comparisons
continue to run. Additional automatic observations include ERC-4626 zero-input
conversion/preview reads and ERC-3643/CMTAT pause reads. These remain capability
observations. They do not independently establish complete standard conformance or
change the selected-claim result merely because an optional getter is unavailable.

References: [ERC-4626](https://eips.ethereum.org/EIPS/eip-4626),
[ERC-3643](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-3643.md),
and [CMTAT variants](https://github.com/CMTA/CMTAT).

## Ledger checks

Use the same deployment `input` as the resolution request and `ledgerChecks`:

```json
{
  "field": "mintAuthority",
  "pointer": "/mintAuthority",
  "expected": null
}
```

Pointers address the returned `observation` object. Selected paths include:

- Solana: `/mintAuthority`, `/freezeAuthority`, `/extensions`,
  `/extensionControls/nonTransferable`, and fields within reported extension states.
- Stellar: `/flags/auth_required`, `/flags/auth_clawback_enabled`,
  `/thresholds/high_threshold`, and `/signerWeights/<signer-public-key>`.
- Aptos: `/objectOwner`, `/allowUngatedTransfer`, or explicit fields within
  `/resourcesByType/<resource-type>/...`.

Expected values preserve JSON types. A missing path is unavailable. A null value
is only comparable when the adapter records it in `availableNullFields`; this keeps
missing information distinct from an explicitly reported null mint/freeze authority.
Raw extension/resource interpretation still requires knowing the deployed program.
Aptos capabilities held elsewhere are not enumerated by reading a metadata object.
Stellar thresholds/signers describe issuer-account permissions, not token ABI roles.

## Results

A mismatched selected comparison produces `MISMATCH`. An unavailable required
comparison prevents `VERIFIED`. A missing registry identity remains `UNKNOWN`, even
if operator policy checks succeed. Responses include `policyVersion`, `policyApplied`,
`checksPerformed` and `checksUnavailable`. These lists cover selected comparisons;
optional capability observations remain in the observation object.
