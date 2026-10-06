# Pinned registry updates

Both services poll registry files; polling does not download GitHub updates. The
sync command connects a chosen registry revision to their shared runtime files.

```text
Registry Git ref → exact commit SHA → isolated checkout → npm ci + npm run check
  → manifest and resolver validation → immutable release → atomic current pointer
  → API/resolver polling → validated in-memory snapshots
```

## Publish a revision

The command requires Python 3 on POSIX systems, Git, the registry's supported Node.js
and npm versions, outbound GitHub/npm access, and the compiled resolver validator.
Run from the resolver checkout:

```sh
cargo build --locked
python3 scripts/sync-registry.py \
  --ref YOUR_40_CHARACTER_COMMIT_SHA \
  --root ./registry-releases
```

Use `--ref main` to adopt the branch's current revision; it still resolves and records
an exact SHA. Another GitHub update requires another sync run. The script builds in
a temporary checkout and never resets the developer's registry. Use `--validator`
to select another compiled binary.

Successful publication creates:

```text
registry-releases/
  current → releases/<commit>-<registry-sha256>/
  releases/<commit>-<registry-sha256>/
    dist/registry.json
    dist/manifest.json
    schemas/
    registry-pin.json
```

The active pin records the repository, exact commit and registry checksum. Configure
both services with the same absolute path:

```sh
export REGISTRY_DIST_DIR=/absolute/path/to/registry-releases/current/dist
```

When `REGISTRY_DIST_DIR` is unset, the resolver prefers its local
`registry-releases/current/dist` if present, and otherwise uses the sibling registry
checkout. An explicit environment setting always takes precedence. Configure the
API's path explicitly when both services should consume the same pinned release.

New assets become resolvable after publication and the next successful poll, normally
within 60 seconds. Each ingestion resolves the pointer once and reads data/schemas
from that immutable release. The services can switch at different moments; resolution
responses identify the consumed registry revision. New networks still require an
appropriate adapter and configured provider.

## Rollback and failures

Read the active pin with `cat registry-releases/current/registry-pin.json`. To
reactivate a retained release, supply its full directory name under `releases/`:

```sh
python3 scripts/sync-registry.py \
  --activate COMMIT_SHA-REGISTRY_SHA256 \
  --root ./registry-releases
```

Activation validates the retained release before switching the pointer. Failed
builds, mismatched manifests or invalid records leave the active release unchanged.
Old releases remain available; retention cleanup is an operator task. Checksums
detect inconsistent bytes but do not authenticate a publisher.

## Automated publication

A deployment job can run this command after a registry push/release, supplying the
triggering commit SHA. The included GitHub workflow and polling watcher below
provide scheduled publication; runtime activation requires operator configuration.
API/resolver CI compatibility
pins remain separate from the runtime data pin: schema-version changes must pass
both consumers' compatibility checks before activation.

Schema IDs now use `https://rwaimport.xyz/schemas/`. Validation reads the local schema
files shipped with the release; the domain need not serve them for validation. If
the URLs should be browsable, publish the schemas separately after configuring the
domain. Push the registry's domain changes before selecting its new GitHub commit.

## Automatic watcher

Run the watcher beside the services, against their shared publication directory:

```sh
python3 scripts/registry-auto-sync.py \
  --root /absolute/path/to/registry-releases \
  --ref main --interval 300 \
  --validator /absolute/path/to/rwaimport-resolver
```

It resolves the watched ref to an exact SHA, validates an unchanged active release,
and builds/activates changed commits through the existing pinned publisher. Failed
runs retain the current release. `--once` performs one check for an external scheduler.
The minimum interval is 60 seconds. Give the process Git, Python, the registry's
supported Node/npm runtime and outbound access. Supervise it with your process manager.

## Publication workflow

`.github/workflows/registry-publication.yml` builds a validated pinned artifact hourly,
from a manual ref, or from a `repository_dispatch` event of type `registry-published`
whose payload includes `commit`. A registry release workflow can send that event
with an appropriately scoped credential; no cross-repository credential is embedded
in this repository.

Runtime activation requires these deployment settings:

- Repository variable `REGISTRY_PUBLICATION_ENABLED=true`.
- A trusted self-hosted runner labelled `rwaimport-registry-sync`, with Python and
  access to the shared runtime volume.
- `REGISTRY_RELEASE_ROOT` and `RESOLVER_VALIDATOR` repository variables specifying
  absolute paths on that runner.
- The `registry-production` GitHub environment, with any desired deployment protections.

Without activation settings, the workflow only builds and uploads the artifact.
The activation job downloads the artifact from that same run, validates it again
with the installed resolver, and invokes `sync-registry.py --source ... --commit ...`
before changing the pointer. Schedules start after the workflow reaches the default
branch. This workspace change does not provision a runner or activate GitHub settings.
