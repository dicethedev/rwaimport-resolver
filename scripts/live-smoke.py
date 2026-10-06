#!/usr/bin/env python3
"""Live public RPC check. Defaults to API gateway; --resolver-only checks the service.
Build services first. Temporary processes are always terminated. No deployments.
"""
import argparse
import concurrent.futures
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
API = ROOT.parent / 'rwaimport-api'


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def get(url, timeout=35):
    with urllib.request.urlopen(url, timeout=timeout) as response:
        return json.load(response)


def wait_ready(process, url):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError('Service exited before readiness')
        try:
            get(url, 1)
            return
        except (OSError, urllib.error.URLError):
            time.sleep(0.1)
    raise RuntimeError('Service readiness deadline exceeded')


def select_samples(registry, endpoints):
    evm = {str(chain['chainId']): chain for chain in registry['chains'] if chain['type'] == 'evm' and chain['status'] == 'active'}
    missing = set(evm) - set(endpoints)
    extra = set(endpoints) - set(evm)
    if missing or extra:
        raise ValueError(f'Public RPC coverage differs from active registry EVM networks: missing={sorted(missing)}, extra={sorted(extra)}')
    samples = {}
    for product in registry['assets']:
        for deployment in product['deployments']:
            chain_id = str(deployment.get('chainId'))
            if chain_id in evm and deployment['status'] == 'active':
                if chain_id not in samples or product['asset']['id'] == 'buidl':
                    samples[chain_id] = (product['asset']['id'], deployment['address'])
    if set(samples) != set(evm):
        raise ValueError(f'No active deployment available for RPC observation: {sorted(set(evm) - set(samples))}')
    return evm, samples


def save_report(path, report):
    if path:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(report, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resolver-only', action='store_true', help='Run without a sibling API checkout')
    parser.add_argument('--registry-dir', type=Path, default=Path(os.environ.get('REGISTRY_DIST_DIR', ROOT.parent / 'rwaimport-registry/dist')))
    parser.add_argument('--report', type=Path, help='Write JSON results, including failures')
    parser.add_argument('--strict', action='store_true', help='Fail when selected claims mismatch or are unavailable')
    args = parser.parse_args()
    registry_dir = args.registry_dir.resolve()
    report = {'checkedAt': datetime.now(timezone.utc).isoformat(), 'registryCommit': os.environ.get('REGISTRY_COMMIT'), 'registryRevision': None, 'networks': [], 'unsupportedNetworks': [], 'errors': []}
    endpoints = json.loads((ROOT / 'config/public-rpc.json').read_text())
    processes = []
    with tempfile.TemporaryFile(mode='w+') as logs:
        try:
            registry = json.loads((registry_dir / 'registry.json').read_text())
            report['unsupportedNetworks'] = [{'id': c['id'], 'type': c['type'], 'reason': 'Non-EVM adapter not implemented'} for c in registry['chains'] if c['type'] not in ['evm', 'solana', 'stellar', 'aptos']]
            evm, samples = select_samples(registry, endpoints)
            resolver_port = free_port()
            resolver_url = f'http://127.0.0.1:{resolver_port}'
            env = dict(os.environ, REGISTRY_DIST_DIR=str(registry_dir), RESOLVER_BIND=f'127.0.0.1:{resolver_port}', RPC_URLS=json.dumps(endpoints), RPC_TIMEOUT_MS='8000', RESOLVE_TIMEOUT_MS='30000', PROVIDER_PROBE_INTERVAL_SECONDS='0')
            resolver = subprocess.Popen([str(ROOT / 'target/debug/rwaimport-resolver')], cwd=ROOT, env=env, stdout=logs, stderr=logs)
            processes.append(resolver)
            wait_ready(resolver, f'{resolver_url}/health/ready')
            report['registryRevision'] = get(f'{resolver_url}/health/ready')['registryRevision']
            request_url = resolver_url
            if not args.resolver_only:
                api_port = free_port()
                request_url = f'http://127.0.0.1:{api_port}'
                env = dict(os.environ, REGISTRY_DIST_DIR=str(registry_dir), PORT=str(api_port), HOST='127.0.0.1', RESOLVER_BASE_URL=resolver_url, RESOLVER_TIMEOUT_MS='31000', REQUEST_TIMEOUT_MS='32000', REGISTRY_REFRESH_INTERVAL_MS='0', API_DOCS_ENABLED='false')
                api = subprocess.Popen(['node', 'dist/main.js'], cwd=API, env=env, stdout=logs, stderr=logs)
                processes.append(api)
                wait_ready(api, f'{request_url}/health/live')

            def resolve_sample(item):
                chain_id, (product_id, address) = item
                result = {'chainId': int(chain_id), 'network': evm[chain_id]['id'], 'productId': product_id}
                try:
                    data = get(f'{request_url}/v1/resolve/{chain_id}/{address}')['data']
                    if data['matched']['identity']['productId'] != product_id:
                        raise RuntimeError('Resolver returned a different product identity')
                    result.update({'status': data['status'], 'blockNumber': (data['contract'] or {}).get('blockNumber'), 'blockHash': (data['contract'] or {}).get('blockHash'), 'checks': data['checks'], 'warnings': data['warnings'], 'observationWarnings': (data['contract'] or {}).get('warnings', [])})
                except urllib.error.HTTPError as error:
                    result.update({'status': 'ERROR', 'error': f'HTTP {error.code}', 'blockNumber': None})
                except Exception as error:
                    result.update({'status': 'ERROR', 'error': str(error), 'blockNumber': None})
                return result

            with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
                report['networks'] = list(pool.map(resolve_sample, sorted(samples.items(), key=lambda item: int(item[0]))))
            for network in ['solana', 'stellar', 'aptos']:
                candidates = [(b, d) for b in registry['assets'] for d in b['deployments'] if d['chain'] == network and d['status'] == 'active']
                if not candidates:
                    report['errors'].append(f'No active {network} sample')
                    continue
                bundle, deployment = candidates[0]
                code = deployment.get('verification', {}).get('observedSymbol') if network == 'stellar' else None
                result = {'network': network, 'productId': bundle['asset']['id']}
                try:
                    path = f"/v1/resolve/network/{network}/{deployment['address']}" + (f'/{code}' if code else '')
                    data = get(request_url + path)['data']
                    if data['product']['id'] != bundle['asset']['id']:
                        raise RuntimeError('Different product identity')
                    result.update({'status': data['status'], 'observation': data['observation'], 'checks': data['checks'], 'warnings': data['warnings']})
                    if data['observation'] is None:
                        report['errors'].append(f'No live {network} observation')
                except Exception as error:
                    result.update({'status': 'ERROR', 'error': str(error)})
                    report['errors'].append(f'{network} observation failed')
                report['networks'].append(result)
            if any(r.get('blockNumber') is None for r in report['networks'] if 'chainId' in r):
                report['errors'].append('At least one RPC did not provide a live observation')
            if args.strict and any(r['status'] != 'VERIFIED' for r in report['networks']):
                report['errors'].append('Selected deployment claims mismatched or could not all be established')
        except Exception as error:
            report['errors'].append(str(error))
            logs.seek(0)
            print(logs.read()[-5000:])
        finally:
            for process in reversed(processes):
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
            save_report(args.report, report)
    print(json.dumps(report, indent=2))
    if report['errors']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
