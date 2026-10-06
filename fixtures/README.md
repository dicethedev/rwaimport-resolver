# Resolver test fixtures

`registry.json` is a focused BUIDL subset of the schema-v2 registry distribution.
`schemas/` contains test snapshots of the registry-owned schemas. Runtime validation
always uses the actual schemas next to the configured registry distribution.
The local RPC tests replace the fixture runtime hash to match their mocked bytecode.
These fixtures are recorded registry evidence, not live observations. Never put
provider credentials here. Update fixture schema snapshots when registry contracts
change.

Registry-derived files originate from [dicethedev/rwaimport-registry](https://github.com/dicethedev/rwaimport-registry) and retain its [MIT license notice](REGISTRY_LICENSE.txt).
