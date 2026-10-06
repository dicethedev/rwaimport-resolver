import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('live_smoke', ROOT / 'scripts/live-smoke.py')
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)


class PublicRpcTests(unittest.TestCase):
    def registry(self):
        endpoints = json.loads((ROOT / 'config/public-rpc.json').read_text())
        registry = {
            'chains': [{'id': f'network-{key}', 'chainId': int(key), 'type': 'evm', 'status': 'active'} for key in endpoints],
            'assets': [{'asset': {'id': 'fixture'}, 'deployments': [{'chainId': int(key), 'address': '0x' + 'a' * 40, 'status': 'active'} for key in endpoints]}],
        }
        registry['chains'].append({'id': 'solana', 'type': 'solana', 'status': 'active'})
        return registry, endpoints

    def test_public_ledger_configuration_covers_the_three_adapters(self):
        providers = json.loads((ROOT / 'config/public-ledger.json').read_text())
        self.assertEqual(set(providers), {'solana', 'stellar', 'aptos'})
        self.assertTrue(all(urls and all(url.startswith('https://') for url in urls) for urls in providers.values()))

    def test_covers_seven_current_evm_networks_and_excludes_non_evm(self):
        registry, endpoints = self.registry()
        self.assertEqual(set(endpoints), {'1', '56', '137', '8453', '42161', '43114', '4663'})
        networks, samples = live.select_samples(registry, endpoints)
        self.assertEqual(set(networks), set(endpoints))
        self.assertEqual(set(samples), set(endpoints))

    def test_new_network_or_unknown_configured_network_fails_coverage(self):
        registry, endpoints = self.registry()
        registry['chains'].append({'id': 'new-network', 'chainId': 999, 'type': 'evm', 'status': 'active'})
        with self.assertRaisesRegex(ValueError, 'missing'):
            live.select_samples(registry, endpoints)
        registry, endpoints = self.registry()
        endpoints['999'] = 'https://unused.example'
        with self.assertRaisesRegex(ValueError, 'extra'):
            live.select_samples(registry, endpoints)

    def test_no_active_deployment_fails_instead_of_silently_skipping(self):
        registry, endpoints = self.registry()
        registry['assets'][0]['deployments'][0]['status'] = 'inactive'
        with self.assertRaisesRegex(ValueError, 'No active deployment'):
            live.select_samples(registry, endpoints)

    def test_prefers_buidl_when_available(self):
        registry, endpoints = self.registry()
        registry['assets'].append({'asset': {'id': 'buidl'}, 'deployments': [{'chainId': 1, 'address': '0x' + 'b' * 40, 'status': 'active'}]})
        _, samples = live.select_samples(registry, endpoints)
        self.assertEqual(samples['1'][0], 'buidl')


if __name__ == '__main__':
    unittest.main()
