#!/usr/bin/env python3
"""Watch a trusted registry ref and publish changed commits through the pinned validator."""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import time
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('registry_sync', ROOT / 'scripts/sync-registry.py')
SYNC = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SYNC)


def resolve_ref(ref):
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._/-]{0,199}', ref):
        raise ValueError('Invalid registry ref')
    if re.fullmatch(r'[0-9a-f]{40}', ref):
        return ref
    output = subprocess.check_output(['git', 'ls-remote', SYNC.REPOSITORY, ref, f'refs/heads/{ref}', f'refs/tags/{ref}', f'refs/tags/{ref}^{{}}'], text=True, timeout=60)
    refs = dict(line.split()[::-1] for line in output.splitlines() if len(line.split()) == 2)
    commit = refs.get(f'refs/heads/{ref}') or refs.get(f'refs/tags/{ref}^{{}}') or refs.get(f'refs/tags/{ref}') or refs.get(ref)
    if not commit or not re.fullmatch(r'[0-9a-f]{40}', commit):
        raise ValueError('Registry ref could not be resolved')
    return commit


def sync_once(root, ref, validator):
    commit = resolve_ref(ref)
    pin_file = root / 'current/registry-pin.json'
    if pin_file.exists():
        pin = json.loads(pin_file.read_text())
        if pin['commit'] == commit:
            if SYNC.validate_distribution(root / 'current', validator) != pin['registrySha256']:
                raise ValueError('Active pin checksum mismatch')
            return {'status': 'unchanged', 'commit': commit}
    subprocess.run(['python3', str(ROOT / 'scripts/sync-registry.py'), '--ref', commit, '--root', str(root), '--validator', str(validator)], check=True)
    return {'status': 'activated', 'commit': commit}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--ref', default='main')
    parser.add_argument('--validator', type=Path, default=ROOT / 'target/debug/rwaimport-resolver')
    parser.add_argument('--interval', type=int, default=300)
    parser.add_argument('--once', action='store_true')
    args = parser.parse_args()
    if args.interval < 60:
        parser.error('Interval must be at least 60 seconds')
    while True:
        try:
            event = sync_once(args.root.resolve(), args.ref, args.validator.resolve())
        except Exception as error:
            event = {'status': 'failed', 'errorType': type(error).__name__}
            if args.once:
                print(json.dumps(event), flush=True)
                raise SystemExit(1) from error
        print(json.dumps({'timestamp': datetime.now(timezone.utc).isoformat(), 'event': 'registry_sync', **event}), flush=True)
        if args.once:
            return
        time.sleep(args.interval)


if __name__ == '__main__':
    main()
