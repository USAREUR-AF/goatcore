#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Exercise fixture build failures without Docker or network access."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class BuildFailures(unittest.TestCase):
    def test_build_diagnostics_and_failure_survive(self):
        for name in ("test-peering-forwarder-e2e.sh",
                     "test-peering-forwarder-role.sh", "test-host-firewall-role.sh"):
            with self.subTest(script=name), tempfile.TemporaryDirectory() as directory:
                fixture = Path(directory)
                docker = fixture / "docker"
                docker.write_text('''#!/bin/sh
if [ "$1" = build ]; then
  printf '%s\\n' "$@" > "$FIXTURE_ARGS"
  cat > "$FIXTURE_DOCKERFILE"
  echo 'synthetic apt diagnostic: mirror unavailable'
  exit 100
fi
exit 0
''')
                docker.chmod(0o755)
                ansible = fixture / "ansible-playbook"
                ansible.write_text("#!/bin/sh\nexit 0\n")
                ansible.chmod(0o755)
                env = dict(os.environ, PATH=directory + os.pathsep + os.environ["PATH"],
                           FIXTURE_ARGS=str(fixture / "args"),
                           FIXTURE_DOCKERFILE=str(fixture / "Dockerfile"))
                result = subprocess.run(["bash", str(ROOT / "ops/ci" / name)],
                                        env=env, capture_output=True, text=True, timeout=15)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("synthetic apt diagnostic: mirror unavailable",
                              result.stdout + result.stderr)
                self.assertIn("--progress=plain", (fixture / "args").read_text())
                recipe = (fixture / "Dockerfile").read_text()
                self.assertIn("APT::Update::Error-Mode=any", recipe)
                self.assertIn('Acquire::Retries "3";', recipe)


if __name__ == "__main__":
    unittest.main()
