#!/usr/bin/env python3
"""Exercise production smoke failure paths and the shared preflight contract."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from jinja2 import Environment, StrictUndefined
import yaml

ROOT = Path(__file__).resolve().parents[1]
MOCKS = r'''set -uo pipefail
STANZA=fixture
header() { :; }
log() { :; }
pass() { printf 'PASS %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; }
uname() { printf 'Darwin\n'; }
sleep() { :; }
docker() {
  if [[ "$1" == inspect ]]; then
    local n
    n=$(cat "$SMOKE_ROOT/inspects")
    printf '%s' "$((n+1))" > "$SMOKE_ROOT/inspects"
    [[ "$INSPECT_FAIL" == 1 ]] && return 1
    if [[ "$n" == 0 || "$SAME_ID" == 1 ]]; then printf 'container-old\n';
    else printf 'container-new\n'; fi
    return 0
  fi
  if [[ "$*" == *pg_isready* && "$READY_FAIL" == 1 ]] && [[ $(cat "$SMOKE_ROOT/inspects") -ge 2 ]]; then
    return 1
  fi
  if [[ "$*" == *'pgbackrest --stanza=fixture check'* ]]; then return "$CHECK_RC"; fi
  return 0
}
compose_base() {
  if [[ "$1" == up ]]; then return "$RECREATE_RC"; fi
  return 0
}
'''


class Smoke(unittest.TestCase):
    def run_smoke(self, **settings):
        source = (ROOT / "ops/ansible/tests/local-smoke/smoke.sh").read_text()
        start = source.index("pgbackrest_smoke() {")
        end = source.index("\n}\n", start) + 3
        function = source[start:end]
        with tempfile.TemporaryDirectory() as tmp:
            folder = Path(tmp)
            (folder / "pgbackrest/etc").mkdir(parents=True)
            (folder / "inspects").write_text("0")
            env = dict(PATH=os.environ["PATH"], SMOKE_ROOT=tmp, RECREATE_RC="0",
                       SAME_ID="0", READY_FAIL="0", INSPECT_FAIL="0", CHECK_RC="0")
            env.update({key: str(value) for key, value in settings.items()})
            return subprocess.run([os.environ.get("BASH_UNDER_TEST", "bash"), "-c",
                                   MOCKS + function + "\npgbackrest_smoke\n"],
                                  env=env, capture_output=True, text=True, timeout=10)

    def test_new_container_and_archive_check_pass(self):
        result = self.run_smoke()
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertIn("PASS pgbackrest_archive_survives_recreate", result.stdout)

    def test_failed_recreation_rejected(self):
        self.assert_failed(RECREATE_RC=42)

    def test_unchanged_container_rejected(self):
        self.assert_failed(SAME_ID=1)

    def test_unknown_container_rejected(self):
        self.assert_failed(INSPECT_FAIL=1)

    def test_unready_replacement_rejected(self):
        self.assert_failed(READY_FAIL=1)

    def test_failed_archive_check_rejected(self):
        self.assert_failed(CHECK_RC=42)

    def assert_failed(self, **settings):
        result = self.run_smoke(**settings)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("FAIL pgbackrest_archive_survives_recreate", result.stdout)
        self.assertNotIn("PASS pgbackrest_archive_survives_recreate", result.stdout)


class DockerStorage(unittest.TestCase):
    def run_preflight(self, **settings):
        source = (ROOT / "ops/ansible/tests/local-smoke/smoke.sh").read_text()
        functions = []
        for name in ("docker_storage_preflight", "bring_up_base"):
            start = source.index(name + "() {")
            end = source.index("\n}\n", start) + 3
            functions.append(source[start:end])
        mocks = r'''set -uo pipefail
MIN_FREE_MB=1024
header() { :; }
log() { :; }
pass() { printf 'PASS %s\n' "$1"; }
sleep() { :; }
die() { printf 'REFUSED %s\n' "$*"; exit 2; }
compose_base() {
  if [[ "$1" == config ]]; then
    (( IMAGE_RC == 0 )) || return "$IMAGE_RC"
    printf 'fixture/postgres:16\n'
  else
    printf '%s\n' "$1" >> "$MUTATIONS"
  fi
}
docker() {
  if [[ "$1" == run ]]; then
    (( PROBE_RC == 0 )) || return "$PROBE_RC"
    printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\n'
    printf '/dev/fixture 4194304 0 %s 0%% /var/lib/pgsql/16/data\n' "$FREE_KB"
  elif [[ "$1" == inspect ]]; then
    printf 'healthy\n'
  fi
}
'''
        with tempfile.TemporaryDirectory() as tmp:
            mutations = Path(tmp) / "mutations"
            env = dict(PATH=os.environ["PATH"], FREE_KB="2097152", PROBE_RC="0",
                       IMAGE_RC="0", MUTATIONS=str(mutations))
            env.update({k: str(v) for k, v in settings.items()})
            result = subprocess.run([os.environ.get("BASH_UNDER_TEST", "bash"), "-c",
                                     mocks + "\n".join(functions) + "\nbring_up_base\n"],
                                    env=env, capture_output=True, text=True, timeout=10)
            return result, mutations.read_text().splitlines() if mutations.exists() else []

    def test_sufficient_docker_space_allows_the_stack_lifecycle(self):
        result, changes = self.run_preflight()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(changes, ["down", "up"])
        self.assertIn("PASS docker_data_space", result.stdout)

    def assert_refused(self, **settings):
        result, changes = self.run_preflight(**settings)
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertEqual(changes, [], "storage refusal must precede volume removal/recreation")
        self.assertNotIn("PASS docker_data_space", result.stdout)

    def test_full_docker_filesystem_refuses_before_mutation(self):
        self.assert_refused(FREE_KB=0)

    def test_insufficient_docker_space_refuses_before_mutation(self):
        self.assert_refused(FREE_KB=524288)

    def test_unparseable_docker_space_refuses_before_mutation(self):
        self.assert_refused(FREE_KB="unknown")

    def test_failed_docker_probe_refuses_before_mutation(self):
        self.assert_refused(PROBE_RC=42)

    def test_unresolved_postgres_image_refuses_before_mutation(self):
        self.assert_refused(IMAGE_RC=42)


class Preflight(unittest.TestCase):
    def test_metadata_acceptance_and_refusals(self):
        tasks = yaml.safe_load((ROOT / "ops/ansible/tasks/pgbackrest-config-preflight.yml").read_text())
        expression = tasks[-1]["ansible.builtin.assert"]["that"][0]
        evaluate = Environment(undefined=StrictUndefined).compile_expression(expression)
        safe = dict(exists=True, isreg=True, islnk=False, uid=26, gid=0, mode="0640")
        cases = [("new host", dict(exists=False), True), ("safe", safe, True)]
        for key, value in (("uid", 0), ("gid", 1000), ("mode", "0644"),
                           ("islnk", True), ("isreg", False)):
            cases.append((key, {**safe, key: value}, False))
        for name, stat, expected in cases:
            with self.subTest(name=name):
                self.assertEqual(bool(evaluate(_pgbackrest_host_conf={"stat": stat})), expected)

    def test_every_base_compose_writer_checks_before_copy(self):
        def walk(tasks):
            for task in tasks:
                for key in ("block", "rescue", "always"):
                    yield from walk(task.get(key, []))
                yield task
        writers = []
        for path in (ROOT / "ops/ansible").glob("*.yml"):
            text = path.read_text()
            if "src: ../compose/mgmt-host/docker-compose.yml" not in text:
                continue
            writers.append(path.name)
            for play in yaml.safe_load(text):
                tasks = list(walk(play.get("pre_tasks", []) + play.get("tasks", [])))
                for index, task in enumerate(tasks):
                    copy = task.get("ansible.builtin.copy", {})
                    if copy.get("src") == "../compose/mgmt-host/docker-compose.yml":
                        self.assertTrue(any(previous.get("ansible.builtin.import_tasks") ==
                                            "tasks/pgbackrest-config-preflight.yml" and
                                            "always" in previous.get("tags", [])
                                            for previous in tasks[:index]), path.name)
        self.assertEqual(set(writers), {"mgmt-host.yml", "traefik-acme.yml",
                                       "site-bootstrap.yml",
                                       "signal-federation-transport.yml"})


if __name__ == "__main__":
    unittest.main()
