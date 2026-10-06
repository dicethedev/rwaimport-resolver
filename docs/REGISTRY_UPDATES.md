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
triggering commit SHA. An operator scheduler can instead run `--ref main`. This change
does not install an external workflow or scheduler. API/resolver CI compatibility
pins remain separate from the runtime data pin: schema-version changes must pass
both consumers' compatibility checks before activation.

Schema IDs now use `https://rwaimport.xyz/schemas/`. Validation reads the local schema
files shipped with the release; the domain need not serve them for validation. If
the URLs should be browsable, publish the schemas separately after configuring the
domain. Push the registry's domain changes before selecting its new GitHub commit.
