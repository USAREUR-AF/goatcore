#!/usr/bin/env python3
"""Run the actual identity/delegation tasks against loopback API fixtures.

Only Cloudflare's fixed endpoint is redirected in the temporary task copy.
No real provider, credentials, inventory, or deployment is used.
"""
import copy
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import yaml

ROOT = Path(__file__).resolve().parents[3]
clients = {}
records = {}
requests = []
SENTINEL = "fixture-only-do-not-display-" + "x" * 32

class API(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def respond(self, code, value=None):
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        if value is not None:
            self.wfile.write(json.dumps(value).encode())

    def do_GET(self):
        if self.path == "/health/ready":
            return self.respond(200, {})
        if self.path.startswith("/admin/clients/"):
            key = self.path.rsplit("/", 1)[-1]
            return self.respond(200 if key in clients else 404, clients.get(key, {}))
        if self.path.startswith("/zones?"):
            return self.respond(200, {"result": [{"id": "fixture-zone"}]})
        if self.path.startswith("/zones/fixture-zone/dns_records?"):
            return self.respond(200, {"result": list(records.values())})
        return self.respond(404, {})

    def write(self):
        requests.append((self.command, self.path))
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path.startswith("/admin/clients"):
            if self.path.endswith("/lifespans"):
                return self.respond(200, {})
            clients[body["client_id"]] = body
        else:
            body["id"] = "fixture-record"
            records[body["id"]] = body
        return self.respond(201 if self.command == "POST" and self.path == "/admin/clients" else 200, body)

    do_POST = write
    do_PUT = write

    def do_DELETE(self):
        key = self.path.rsplit("/", 1)[-1]
        if self.path.startswith("/admin/clients/"):
            existed = clients.pop(key, None)
            return self.respond(204 if existed else 404)
        records.pop(key, None)
        return self.respond(200, {"success": True})

server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), API)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = "http://127.0.0.1:%d" % server.server_port
source = yaml.safe_load((ROOT / "ops/ansible/external-service.yml").read_text())[0]["tasks"]
first = next(i for i, task in enumerate(source) if task["name"] == "The client secret is in the env")
last = next(i for i, task in enumerate(source) if task["name"] == "What the service owner configures on their side")
tasks = copy.deepcopy(source[first:last])
for task in tasks:
    uri = task.get("ansible.builtin.uri")
    if uri:
        uri["url"] = uri["url"].replace("https://api.cloudflare.com/client/v4", endpoint)
ext = {"name": "example", "hostnames": ["example"], "tls": "partner",
       "acme_delegate": "acme.partner.invalid", "client": {"id": "fixture-client",
       "secret_env": "ARCHIVE_FIXTURE_SECRET", "token_lifespan": "8h"}}
try:
    with tempfile.TemporaryDirectory(prefix="external-lifecycle-") as tmp:
        path = Path(tmp) / "play.yml"
        for state in ("present", "present", "absent", "absent", "present"):
            if state == "present" and clients:
                ext["acme_delegate"] = "updated.partner.invalid"
            variables = {"ansible_python_interpreter": "/usr/bin/python3", "ext": ext,
                         "external_service_state": state, "hydra_admin": endpoint,
                         "site_id": "fixture", "site_dns_zone": "mesh.example.invalid",
                         "service": "example", "cloudflare_api_token": SENTINEL,
                         "site_env_prefix": "FIXTURE", "target": "localhost"}
            path.write_text(yaml.safe_dump([{"hosts": "localhost", "gather_facts": False,
                                             "vars": variables, "tasks": tasks}], sort_keys=False))
            env = dict(os.environ, ARCHIVE_FIXTURE_SECRET=SENTINEL)
            run = subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local", "-vvv", str(path)],
                                 capture_output=True, text=True, env=env)
            output = run.stdout + run.stderr
            assert SENTINEL not in output, "fixture credential appeared in Ansible output"
            if run.returncode:
                raise AssertionError(output)
            assert bool(clients) == (state == "present"), state
            assert bool(records) == (state == "present"), state
            if state == "present":
                assert clients["fixture-client"]["client_secret"] == SENTINEL
                assert records["fixture-record"]["content"] == ext["acme_delegate"]
            print("PASS:", state, "client/delegation convergence; verbose output redacted")
        assert ("PUT", "/admin/clients/fixture-client") in requests
        assert ("PUT", "/zones/fixture-zone/dns_records/fixture-record") in requests
finally:
    server.shutdown()
    server.server_close()
