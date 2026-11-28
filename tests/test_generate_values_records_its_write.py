"""A stand-up's phase 13 writes values-<env>.yaml into the tree it runs from.
The write is recorded as the stand-up's own, so the source-of-record check does
not refuse the next run of the same tree for it."""
import importlib.util
import json
import os
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("generate_values", ROOT / "ops/validation/generate-values.py")
gv = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gv)


class RecordsItsWrite(unittest.TestCase):
    def test_the_written_file_is_recorded_with_its_bytes(self):
        tree = pathlib.Path(tempfile.mkdtemp())
        (tree / "ops/validation").mkdir(parents=True)
        (tree / "ops/scripts/lib").mkdir(parents=True)
        (tree / "ops/scripts/lib/standup_written.py").write_text((ROOT / "ops/scripts/lib/standup_written.py").read_text())
        state = tree / "state"
        with mock.patch.object(gv, "HERE", tree / "ops/validation"), mock.patch.object(gv, "REPO", tree), \
                mock.patch.object(gv, "generate", return_value={"site_id": "x"}), \
                mock.patch.object(sys, "argv", ["generate-values.py", "x"]), \
                mock.patch.dict(os.environ, {"GOAT_STANDUP_STATE_DIR": str(state)}):
            sys.modules.pop("standup_written", None)
            gv.main()
        out = tree / "ops/validation/values-x.yaml"
        self.assertTrue(out.is_file())
        rec = json.loads((state / "standup-written.json").read_text())
        self.assertIn("ops/validation/values-x.yaml", rec)

    def test_stdout_mode_writes_and_records_nothing(self):
        tree = pathlib.Path(tempfile.mkdtemp())
        (tree / "ops/validation").mkdir(parents=True)
        with mock.patch.object(gv, "HERE", tree / "ops/validation"), mock.patch.object(gv, "REPO", tree), \
                mock.patch.object(gv, "generate", return_value={"site_id": "x"}), \
                mock.patch.object(sys, "argv", ["generate-values.py", "x", "--stdout"]), \
                mock.patch.object(sys, "stdout"):
            gv.main()
        self.assertEqual(list((tree / "ops/validation").iterdir()), [])


if __name__ == "__main__":
    unittest.main()
