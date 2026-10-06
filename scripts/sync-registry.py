#!/usr/bin/env python3
"""Build a Git ref, validate an immutable distribution, and activate a pinned release."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
from datetime import datetime, timezone

REPOSITORY = 'https://github.com/dicethedev/rwaimport-registry.git'
ROOT = Path(__file__).resolve().parents[1]


def run(args, cwd=None):
    subprocess.run(args, cwd=cwd, check=True)


def validate_distribution(source, validator):
    with (source / 'dist/registry.json').open('rb') as file:
        raw = file.read(64 * 1024 * 1024 + 1)
    if len(raw) > 64 * 1024 * 1024:
        raise ValueError('Registry exceeds 64 MiB')
    manifest = json.loads((source / 'dist/manifest.json').read_text())
    entry = manifest['files']['registry.json']
    digest = hashlib.sha256(raw).hexdigest()
    if manifest.get('schemaVersion') != 1 or entry['sha256'] != digest or entry['bytes'] != len(raw):
        raise ValueError('Registry manifest does not match distribution')
    completed = subprocess.run([str(validator), '--check-registry', str(source / 'dist')], check=True, capture_output=True, text=True)
    if json.loads(completed.stdout)['registryRevision'] != digest:
        raise ValueError('Validator returned a different registry revision')
    return digest


def publish_release(source, root, commit, validator):
    if not re.fullmatch(r'[0-9a-f]{40}', commit):
        raise ValueError('Release requires an exact Git commit SHA')
    root.mkdir(parents=True, exist_ok=True)
    releases = root / 'releases'
    releases.mkdir(exist_ok=True)
    # Clone/build happens before this function. Serialize validation and activation.
    with (root / '.sync.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        with tempfile.TemporaryDirectory(prefix='.candidate-', dir=root) as staging:
            candidate = Path(staging) / 'release'
            candidate.mkdir()
            shutil.copytree(source / 'dist', candidate / 'dist')
            shutil.copytree(source / 'schemas', candidate / 'schemas')
            digest = validate_distribution(candidate, validator)
            release_id = f'{commit}-{digest}'
            target = releases / release_id
            pin = {'repository': REPOSITORY, 'commit': commit, 'registrySha256': digest, 'builtAt': datetime.now(timezone.utc).isoformat()}
            (candidate / 'registry-pin.json').write_text(json.dumps(pin, indent=2) + '\n')
            if target.exists():
                if validate_distribution(target, validator) != digest:
                    raise ValueError('Existing immutable release has inconsistent contents')
            else:
                candidate.rename(target)
            # The pin lives inside the release; one pointer swaps data, schemas and pin.
            pointer = Path(staging) / 'current'
            pointer.symlink_to(target.resolve(), target_is_directory=True)
            os.replace(pointer, root / 'current')
            return json.loads((target / 'registry-pin.json').read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--ref', help='Exact commit SHA or branch/tag to resolve to a SHA')
    mode.add_argument('--source', type=Path, help='Publish an already-built trusted release artifact')
    parser.add_argument('--commit', help='Exact source commit, required with --source')
    mode.add_argument('--activate', help='Reactivate an existing release ID without rebuilding')
    parser.add_argument('--root', type=Path, required=True, help='Shared publication directory used by API and resolver')
    parser.add_argument('--validator', type=Path, default=ROOT / 'target/debug/rwaimport-resolver')
    args = parser.parse_args()
    if args.ref and not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._/-]{0,199}', args.ref):
        parser.error('Invalid Git ref')
    validator = args.validator.resolve()
    if not validator.is_file():
        parser.error('Build the resolver first, or supply --validator')
    if args.source:
        if not args.commit or not re.fullmatch(r'[0-9a-f]{40}', args.commit):
            parser.error('--source requires --commit with an exact SHA')
        print(json.dumps(publish_release(args.source.resolve(), args.root.resolve(), args.commit, validator), indent=2))
        return
    if args.commit:
        parser.error('--commit is only valid with --source')
    if args.activate:
        if not re.fullmatch(r'[0-9a-f]{40}-[0-9a-f]{64}', args.activate):
            parser.error('Invalid release ID')
        source = args.root.resolve() / 'releases' / args.activate
        pin = json.loads((source / 'registry-pin.json').read_text())
        if args.activate != f"{pin['commit']}-{pin['registrySha256']}":
            raise ValueError('Release pin does not match its ID')
        active = publish_release(source, args.root.resolve(), pin['commit'], validator)
        print(json.dumps(active, indent=2))
        return
    # Build in a temporary checkout: never reset or modify the developer registry.
    with tempfile.TemporaryDirectory(prefix='rwaimport-registry-sync-') as work:
        checkout = Path(work) / 'checkout'
        run(['git', 'init', '--quiet', str(checkout)])
        run(['git', 'remote', 'add', 'origin', REPOSITORY], checkout)
        run(['git', 'fetch', '--depth', '1', 'origin', args.ref], checkout)
        run(['git', 'checkout', '--detach', 'FETCH_HEAD'], checkout)
        commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=checkout, text=True).strip()
        run(['npm', 'ci'], checkout)
        run(['npm', 'run', 'check'], checkout)
        pin = publish_release(checkout, args.root.resolve(), commit, validator)
        print(json.dumps(pin, indent=2))
        print(f'Set REGISTRY_DIST_DIR={args.root.resolve() / "current/dist"} for both services.')


if __name__ == '__main__':
    main()
