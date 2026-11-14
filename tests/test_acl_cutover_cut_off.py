"""The access-rule cutover's cut-off mode (F-591).

A site cut off from the others may only update the access rules it already has:
`ops/ansible/netbird-acl-cutover.yml -e cut_off=true` PUTs existing policy ids
and never creates or deletes one. NetBird assigns a created policy's id, so two
cut-off sites that each create the same rule leave two policies after the heal,
and a delete at one site is undone or doubled by the other (island-mode write
inventory, N15; partition harness `netbird-c5-policy-regenerate-no-duplicates`).
Where the run would create or delete a policy, it refuses with GB-295 before it
writes anything, naming each policy it would have created and each id it would
have deleted; the hub makes those changes after the heal.

The play runs on localhost against a stand-in NetBird management API that
records every request; nothing leaves the machine.
"""
import http.server
import json
import os
import shutil
import subprocess
import threading
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parent.parent
PLAY = REPO / "ops" / "ansible" / "netbird-acl-cutover.yml"
needs_ansible = pytest.mark.skipif(shutil.which("ansible-playbook") is None, reason="ansible-playbook not installed")

TAG = "[L4-cutover via netbird-acl-cutover.yml]"
GROUPS = [{"id": "g-admin", "name": "ZAGROS-ADMIN"}, {"id": "g-infra", "name": "ZAGROS-INFRA"}, {"id": "g-all", "name": "All"}]

TAXONOMY = {
    "netbird_acl_taxonomy": {"zagros": {"site_prefix": "ZAGROS", "groups": [{"suffix": "ADMIN"}, {"suffix": "INFRA"}]}},
    "netbird_exercises": {},
}


def matrix(action="disable"):
    return {
        "netbird_acl_matrix": {
            "zagros": {
                "cutover": {"default_policy_name_pattern": "^Default$", "default_policy_action": action},
                "rules": [
                    {"name": "admin to infra", "description": "operators reach the substrate", "sources": ["ADMIN"],
                     "destinations": ["INFRA"], "protocol": "all", "action": "accept", "bidirectional": True},
                    {"name": "infra to admin", "description": "the substrate answers", "sources": ["INFRA"],
                     "destinations": ["ADMIN"], "protocol": "all", "action": "accept", "bidirectional": True},
                ],
            }
        }
    }


def group(gid):
    """A group as NetBird's GET returns it inside a rule: its id and its name."""
    return next(g for g in GROUPS if g["id"] == gid)


def rule(src, dst, name="r"):
    return {"name": name, "enabled": True, "action": "accept", "protocol": "all", "bidirectional": True,
            "sources": [group(src)], "destinations": [group(dst)], "ports": []}


def policy(pid, name, src, dst, tagged=True):
    return {"id": pid, "name": name, "enabled": True,
            "description": f"generated {TAG}" if tagged else "hand-made",
            "rules": [rule(src, dst, name)]}


DEFAULT = {"id": "p-default", "name": "Default", "enabled": True, "description": "",
           "rules": [{"name": "Default", "enabled": True, "action": "accept", "protocol": "all", "bidirectional": True,
                      "sources": [{"id": "g-all", "name": "All"}], "destinations": [{"id": "g-all", "name": "All"}]}]}


class NetBird:
    """A stand-in management API: GET groups, peers and policies; every write recorded."""

    def __init__(self, policies):
        self.policies = policies
        self.writes = []
        api = self

        class H(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def _send(self, code, body=None):
                data = json.dumps(body if body is not None else {}).encode()
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                self._send(200, {"/api/groups": GROUPS, "/api/peers": [], "/api/policies": api.policies}.get(self.path, []))

            def _write(self, method):
                n = int(self.headers.get("Content-Length") or 0)
                api.writes.append((method, self.path, json.loads(self.rfile.read(n) or b"{}")))
                self._send(200, {"id": "p-new"})

            def do_POST(self):
                self._write("POST")

            def do_PUT(self):
                self._write("PUT")

            def do_DELETE(self):
                self._write("DELETE")

        self.server = http.server.HTTPServer(("127.0.0.1", 0), H)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.server.shutdown()


def run(tmp, nb, *extra, action="disable"):
    (tmp / "taxonomy.yml").write_text(json.dumps(TAXONOMY))
    (tmp / "matrix.yml").write_text(json.dumps(matrix(action)))
    env = dict(os.environ, ZAGROS_NB_API_TOKEN="t", ANSIBLE_NOCOLOR="1", ANSIBLE_LOCAL_TEMP=str(tmp / "ansible"))
    argv = ["ansible-playbook", "-i", "localhost,", str(PLAY), "-e", "target_site=zagros",
            "-e", f"taxonomy_path={tmp / 'taxonomy.yml'}", "-e", f"matrix_path={tmp / 'matrix.yml'}",
            "-e", f"netbird_mgmt_url={nb.url}", "-e", "netbird_api_ca=public"]
    for e in extra:
        argv += ["-e", e]
    return subprocess.run(argv, cwd=REPO / "ops" / "ansible", env=env, capture_output=True, text=True, timeout=300)


@pytest.fixture
def drifted_and_missing():
    # "admin to infra" exists with a drifted destination; "infra to admin" is missing.
    nb = NetBird([DEFAULT, policy("p-1", "admin to infra", "g-admin", "g-admin")])
    yield nb
    nb.close()


@needs_ansible
def test_cut_off_refuses_a_create_naming_it_and_writes_nothing(tmp_path, drifted_and_missing):
    nb = drifted_and_missing
    out = run(tmp_path, nb, "mode=apply", "cut_off=true")
    text = out.stdout + out.stderr
    assert out.returncode != 0, text
    assert "GB-295" in text, text
    assert "infra to admin" in text, "names the policy it would have created"
    assert nb.writes == [], f"a refusal writes nothing: {nb.writes}"


@needs_ansible
def test_cut_off_updates_existing_ids_only(tmp_path):
    nb = NetBird([DEFAULT, policy("p-1", "admin to infra", "g-admin", "g-admin"),
                  policy("p-2", "infra to admin", "g-infra", "g-admin")])
    try:
        out = run(tmp_path, nb, "mode=apply", "cut_off=true")
        assert out.returncode == 0, out.stdout + out.stderr
        assert [(m, p) for m, p, _ in nb.writes] == [("PUT", "/api/policies/p-1")], nb.writes
    finally:
        nb.close()


@needs_ansible
def test_cut_off_refuses_a_reap_naming_the_id(tmp_path):
    nb = NetBird([DEFAULT, policy("p-1", "admin to infra", "g-admin", "g-infra"),
                  policy("p-2", "infra to admin", "g-infra", "g-admin"),
                  policy("p-old", "retired rule", "g-admin", "g-infra")])
    try:
        out = run(tmp_path, nb, "mode=apply", "cut_off=true", "reap_orphans=true")
        text = out.stdout + out.stderr
        assert out.returncode != 0 and "GB-295" in text, text
        assert "p-old" in text, "names the id it would have deleted"
        assert nb.writes == [], nb.writes
    finally:
        nb.close()


@needs_ansible
def test_cut_off_refuses_deleting_the_default_policy(tmp_path):
    nb = NetBird([DEFAULT, policy("p-1", "admin to infra", "g-admin", "g-infra"),
                  policy("p-2", "infra to admin", "g-infra", "g-admin")])
    try:
        out = run(tmp_path, nb, "mode=enforce", "confirm_cutover=YES", "cut_off=true", action="delete")
        text = out.stdout + out.stderr
        assert out.returncode != 0 and "GB-295" in text, text
        assert "p-default" in text, text
        assert nb.writes == [], nb.writes
    finally:
        nb.close()


@needs_ansible
def test_cut_off_still_disables_the_default_policy(tmp_path):
    # Disabling the Default policy is a PUT of an existing id: allowed while cut off.
    nb = NetBird([DEFAULT, policy("p-1", "admin to infra", "g-admin", "g-infra"),
                  policy("p-2", "infra to admin", "g-infra", "g-admin")])
    try:
        out = run(tmp_path, nb, "mode=enforce", "confirm_cutover=YES", "cut_off=true")
        assert out.returncode == 0, out.stdout + out.stderr
        assert [(m, p) for m, p, _ in nb.writes] == [("PUT", "/api/policies/p-default")], nb.writes
    finally:
        nb.close()


@needs_ansible
def test_without_cut_off_the_cutover_creates_as_before(tmp_path, drifted_and_missing):
    nb = drifted_and_missing
    out = run(tmp_path, nb, "mode=apply")
    assert out.returncode == 0, out.stdout + out.stderr
    assert sorted((m, p) for m, p, _ in nb.writes) == [("POST", "/api/policies"), ("PUT", "/api/policies/p-1")], nb.writes
