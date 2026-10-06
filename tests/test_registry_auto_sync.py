import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('registry_auto_sync', ROOT / 'scripts/registry-auto-sync.py')
auto = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(auto)


class AutoSyncTests(unittest.TestCase):
    def test_unchanged_pin_is_validated_without_building(self):
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            (root / 'current').mkdir()
            (root / 'current/registry-pin.json').write_text(json.dumps({'commit': 'a' * 40, 'registrySha256': 'b' * 64}))
            with patch.object(auto, 'resolve_ref', return_value='a' * 40), patch.object(auto.SYNC, 'validate_distribution', return_value='b' * 64) as validate, patch.object(auto.subprocess, 'run') as run:
                self.assertEqual(auto.sync_once(root, 'main', ROOT / 'validator')['status'], 'unchanged')
                validate.assert_called_once()
                run.assert_not_called()

    def test_changed_ref_builds_the_exact_commit(self):
        with tempfile.TemporaryDirectory() as work:
            with patch.object(auto, 'resolve_ref', return_value='a' * 40), patch.object(auto.subprocess, 'run') as run:
                self.assertEqual(auto.sync_once(Path(work), 'main', ROOT / 'validator')['status'], 'activated')
                self.assertIn('a' * 40, run.call_args.args[0])
                self.assertNotIn('main', run.call_args.args[0])

    def test_ref_validation_and_annotated_tag_resolution(self):
        with self.assertRaises(ValueError):
            auto.resolve_ref('--upload-pack=bad')
        text = 'a' * 40 + '\trefs/tags/v1\n' + 'b' * 40 + '\trefs/tags/v1^{}\n'
        with patch.object(auto.subprocess, 'check_output', return_value=text):
            self.assertEqual(auto.resolve_ref('v1'), 'b' * 40)


if __name__ == '__main__':
    unittest.main()
