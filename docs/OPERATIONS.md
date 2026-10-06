# Resolver operations and batches

## Provider health

Transport requests are tracked by configured provider, with opaque provider IDs in
metrics and logs. Asset paths, provider URLs and credentials are not metric labels.
Contract reverts are valid RPC responses; they do not count as transport outages.
Full observation failures are also tracked, so a provider that returns its network
ID but fails subsequent critical reads can enter cooldown.

`PROVIDER_FAILURE_THRESHOLD` defaults to 3 (range 1–100).
`PROVIDER_COOLDOWN_SECONDS` defaults to 30 (range 1–3,600). Open circuits are skipped
without restarting cooldown on every skipped request. After cooldown, one recovery
request is allowed; concurrent recovery requests are excluded. A wrong-network
response immediately opens that provider's circuit and fails the current request.

Primary and backup providers share the resolution deadline. An available observation
is compared as received; a mismatch does not trigger provider shopping. Optional
unsupported contract getters do not imply a provider outage.

The process probes configured provider network identities every 60 seconds.
`PROVIDER_PROBE_INTERVAL_SECONDS` accepts zero to disable probes, or 10–3,600 seconds.
Health reports distinguish unknown, closed, open and half-open states and include
network-verification state, failure counters, accumulated latency and cooldown.

## Readiness and visibility

- `GET /health/live` checks that the HTTP service is running.
- `GET /health/ready` returns 503 when the registry refresh has failed, or an enabled
  readiness policy fails; the last valid snapshot remains usable for resolution.
- `REGISTRY_MAX_AGE_SECONDS=0` disables registry-age readiness enforcement. A positive
  value, up to 31,536,000, rejects stale build timestamps. This is independent of the
  freshness of individual evidence records.
- `READINESS_REQUIRE_RPC=true` requires an available network-verified provider for
  every active supported registry network. The default is false, allowing registry
  identification when live RPC is deliberately not configured.
- `GET /metrics` exports Prometheus counters for resolutions, errors, cache hits,
  provider requests/failures, latency sums/counts and circuit state. Keep the service
  on its internal network and scrape this endpoint there.

Resolution outcomes, registry activation, provider probes and observation-induced
circuit openings use structured JSON logs. Provider errors and credentials are not
included. HTTP resolution responses use `Cache-Control: no-store`; the in-process
cache remains private to the resolver.

## Shared responses and caching

`ResolutionEnvelope` is the shared Rust response type. The gateway exports
`SharedResolutionResult`. Both EVM and ledger responses contain input, status, checks,
registry context, evidence freshness, policy metadata and an `observation` object.
EVM responses retain `contract`; ledger responses set it to null. Inputs preserve
chain IDs or network/asset locators rather than inventing numeric IDs for other ledgers.
Comparison values are strings across adapters; boolean and numeric observations
retain their native types inside `observation`.

Non-EVM requests now use bounded TTL caching and lock-based request coalescing, as
EVM requests already do. Both caches clear on registry activation. Policies are
immutable for the process lifetime. Failed observations and unavailable required
comparisons are not cached. `resolvedAt` and freshness assessment timestamps describe
when the cached result was produced. They are not rewritten to suggest a new read.

## Mixed-ledger batches

```http
POST /v1/resolve/batch
Content-Type: application/json
```

```json
{
  "requests": [
    {"chainId": 1, "address": "0x6a9da2d710bb9b700acde7cb81f10f1ff8c89041"},
    {"network": "solana", "address": "GyWgeqpy5GueU2YbkE8xqUeVEokCMMCEeUrfbtMw6phr"},
    {"network": "stellar", "address": "GBHNGLLIE3KWGKCHIKMHJ5HVZHYIK7WTBE4QF5PLAKL4CJGSEU7HZIW5", "assetCode": "BENJI"}
  ]
}
```

The response is `{ "data": [ { "index", "input", "data", "error" }, ... ] }`.
Each item has either a resolution or a sanitized error, and results preserve request
order. Invalid locators, unsupported networks and provider failures are isolated to
the relevant item. Structurally invalid JSON fails the whole request.

The resolver defaults to 64 items (`RESOLVER_BATCH_MAX`, range 1–256). The API gateway
accepts up to 64. Requests are limited to 256 KiB at the resolver. Work runs with at
most eight concurrent items, further limited by the resolver's configured concurrency.
Each item has its own resolution deadline; batches with many items can take multiple
waves. Configure the gateway's resolver timeout accordingly. A client disconnect
cancels remaining batch tasks at the resolver once the batch future is dropped.
The gateway limits upstream responses to 4 MiB; use smaller batches when registry
context is large to keep the complete response within that budget.
