#!/usr/bin/env python3
"""Own a disposable loopback-only MinIO server; never connect to a live endpoint."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import time
import uuid


def private_file(path, text):
    """Create a fixture credential file readable only by its owner from the first byte."""
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as stream:
        stream.write(text)
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-image", required=True)
    parser.add_argument("--mc-image", required=True)
    parser.add_argument("--engine", type=Path, default=Path(__file__).resolve().parents[1] /
                        "ops/ansible/roles/audit-fifo-evict/files/evict.py")
    parser.add_argument("--policy", type=Path, default=Path(__file__).resolve().parents[1] /
                        "ops/minio/policies/audit-evict.json")
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location("engine", args.engine)
    engine = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(engine)
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    name = "goat-fifo-test-" + uuid.uuid4().hex[:12]
    admin_password, evict_password = secrets.token_hex(24), secrets.token_hex(24)
    mib = 2**20
    with tempfile.TemporaryDirectory(prefix="goat-fifo-test-") as tmp:
        directory = Path(tmp)
        server_env = private_file(directory / "server.env",
                                  "MINIO_ROOT_USER=fixture-admin\nMINIO_ROOT_PASSWORD=" + admin_password +
                                  "\nMINIO_BROWSER=off\n")
        root_env = private_file(directory / "root.env",
                                f"MC_HOST_fixture=http://fixture-admin:{admin_password}@127.0.0.1:{port}\n")
        evict_env = private_file(directory / "evict.env",
                                 f"MC_HOST_fixture=http://fixture-evictor:{evict_password}@127.0.0.1:{port}\n")
        config = dict(bucket="audit-integrity", alias="fixture", quota_gib=1,
                      high_pct=2, low_pct=1, batch_hours=6, max_pass_pct=25,
                      trail_prefix="ops/eviction", state_dir=tmp, textfile_dir=tmp,
                      env_file=str(evict_env), mc_image=args.mc_image)
        root = engine.MC({**config, "env_file": str(root_env)})
        backend = engine.MC(config)
        command = ["docker", "run", "-d", "--name", name, "--network", "host",
                   "--label", "goat.test=audit-fifo", "--env-file", str(server_env),
                   args.server_image, "server", "--address", f"127.0.0.1:{port}", "/data"]
        try:
            subprocess.run(command, check=True, capture_output=True, text=True)
            for attempt in range(30):
                try:
                    root.command("ls", "fixture")
                    break
                except engine.Failure:
                    if attempt == 29:
                        raise
                    time.sleep(1)
            target = "fixture/audit-integrity"
            root.command("mb", "--with-lock", target)
            root.command("retention", "set", "--default", "GOVERNANCE", "30d", target)
            policy = directory / "policy.json"
            policy.write_bytes(args.policy.read_bytes())
            root.command("admin", "user", "add", "fixture", "fixture-evictor", evict_password)
            root.command("admin", "policy", "create", "fixture", "fixture-evict", "/tmp/trail.json", source=policy)
            root.command("admin", "policy", "attach", "fixture", "fixture-evict", "--user", "fixture-evictor")
            data = directory / "data.bin"
            data.write_bytes(b"a" * (5 * mib))
            oldest = "z-site/2026/01/01/00/event.ndjson"
            root.command("cp", "/tmp/trail.json", target + "/" + oldest, source=data)
            data.write_bytes(b"b" * (5 * mib))
            root.command("cp", "/tmp/trail.json", target + "/" + oldest, source=data)
            data.write_bytes(b"c" * (30 * mib))
            newest = "a-site/2026/01/01/01/event.ndjson"
            root.command("cp", "/tmp/trail.json", target + "/" + newest, source=data)
            before_rows = backend.versions(target)
            old_ids = {row["versionId"] for row in before_rows if row["key"] == oldest}
            assert len(old_ids) == 2, before_rows
            used_before = backend.used(target)
            assert used_before == 40 * mib, used_before
            assert engine.run(config, backend) == 0
            assert backend.used(target) == used_before
            assert engine.run(config, backend, evict=True) == 2
            after_rows = backend.versions(target)
            assert not old_ids.intersection(row["versionId"] for row in after_rows)
            assert any(row["key"] == newest for row in after_rows)
            used_after = backend.used(target)
            assert used_after < used_before
            assert used_before - used_after <= used_before * config["max_pass_pct"] // 100
            trails = [row for row in after_rows if row["key"].startswith("ops/eviction/")]
            assert trails
            try:
                backend.remove(target + "/" + trails[0]["key"], trails[0]["versionId"])
            except engine.Failure:
                pass
            else:
                raise AssertionError("evictor policy allowed deletion of the trail")
            remaining = {row["versionId"] for row in backend.versions(target)}
            assert engine.run(config, backend, evict=True) == 2
            assert remaining == {row["versionId"] for row in backend.versions(target)}
            cold = "fixture/audit-cold-fixture"
            root.command("mb", "--with-lock", cold)
            root.command("retention", "set", "--default", "COMPLIANCE", "30d", cold)
            root.command("cp", "/tmp/trail.json", cold + "/evidence.bin", source=data)
            cold_before = root.used(cold)
            cold_version = root.versions(cold)[0]["versionId"]
            for client in (backend, root):
                try:
                    client.remove(cold + "/evidence.bin", cold_version)
                except engine.Failure:
                    pass
                else:
                    raise AssertionError("COMPLIANCE cold evidence was removable")
            assert root.used(cold) == cold_before
            print(json.dumps({"engine_sha256": hashlib.sha256(args.engine.read_bytes()).hexdigest(),
                              "used_before": used_before, "used_after": used_after,
                              "old_versions_removed": len(old_ids), "trail_delete_denied": True,
                              "oversized_next_hour_preserved": True,
                              "measure_only_preserved_usage": True, "cold_compliance_preserved": True}, sort_keys=True))
        except Exception:
            logs = subprocess.run(["docker", "logs", "--tail", "20", name],
                                  capture_output=True, text=True)
            print((logs.stdout + logs.stderr).replace(admin_password, "<fixture-secret>"))
            diagnostic = subprocess.run(["docker", "run", "--rm", "--network", "host",
                                         "--env-file", str(root_env), args.mc_image,
                                         "--json", "ls", "fixture"], capture_output=True, text=True)
            print((diagnostic.stdout + diagnostic.stderr).replace(admin_password, "<fixture-secret>"))
            raise
        finally:
            subprocess.run(["docker", "rm", "-f", name], capture_output=True, text=True)


if __name__ == "__main__":
    main()
