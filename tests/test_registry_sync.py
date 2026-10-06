"""Run with python3 -m unittest discover -s tests -p 'test_*.py'."""
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('registry_sync', ROOT / 'scripts/sync-registry.py')
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)
VALIDATOR = ROOT / 'target/debug/rwaimport-resolver'


class RegistrySyncTests(unittest.TestCase):
    def test_publish_retain_on_failure_and_switch_to_new_pin(self):
        import hashlib
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            source = root / 'source'
            (source / 'dist').mkdir(parents=True)
            shutil.copytree(ROOT / 'fixtures/schemas', source / 'schemas')
            data = json.loads((ROOT / 'fixtures/registry.json').read_text())

            def write_distribution():
                raw = json.dumps(data).encode()
                (source / 'dist/registry.json').write_bytes(raw)
                (source / 'dist/manifest.json').write_text(json.dumps({'schemaVersion': 1, 'files': {'registry.json': {'sha256': hashlib.sha256(raw).hexdigest(), 'bytes': len(raw)}}}))

            write_distribution()
            publication = root / 'published'
            import subprocess
            subprocess.run(['python3', str(ROOT / 'scripts/sync-registry.py'), '--source', str(source), '--commit', 'a' * 40, '--root', str(publication), '--validator', str(VALIDATOR)], check=True, capture_output=True)
            first = json.loads((publication / 'current/registry-pin.json').read_text())
            active = (publication / 'current').resolve()
            self.assertEqual(first['commit'], 'a' * 40)
            self.assertEqual(json.loads((active / 'registry-pin.json').read_text()), first)
            (source / 'dist/manifest.json').write_text('{}')
            with self.assertRaises((KeyError, ValueError)):
                sync.publish_release(source, publication, 'b' * 40, VALIDATOR)
            self.assertEqual((publication / 'current').resolve(), active)
            data['assets'][0]['asset']['name'] = 'Updated display name'
            write_distribution()
            second = sync.publish_release(source, publication, 'b' * 40, VALIDATOR)
            self.assertNotEqual(second['registrySha256'], first['registrySha256'])
            self.assertNotEqual((publication / 'current').resolve(), active)
            self.assertTrue(active.is_dir())
            data['assets'][0]['asset']['issuerId'] = 'missing'
            write_distribution()
            current = (publication / 'current').resolve()
            import subprocess
            with self.assertRaises(subprocess.CalledProcessError):
                sync.publish_release(source, publication, 'c' * 40, VALIDATOR)
            self.assertEqual((publication / 'current').resolve(), current)
            subprocess.run(['python3', str(ROOT / 'scripts/sync-registry.py'), '--activate', f"{first['commit']}-{first['registrySha256']}", '--root', str(publication), '--validator', str(VALIDATOR)], check=True, capture_output=True)
            self.assertEqual((publication / 'current').resolve(), active)


if __name__ == '__main__':
    unittest.main()
