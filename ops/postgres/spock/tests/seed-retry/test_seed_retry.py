#!/usr/bin/env python3
"""Real PostgreSQL, real guard SQL, and the seed play's rendered restore shell.

Run with Python, PyYAML, Jinja2 and Docker. All data is synthetic, in a
throwaway container with no network or published ports. This tests the seed
transaction and preflight, not Ansible's SSH/delegation or Spock transport.
"""

import json
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import time
import uuid

import jinja2
import yaml


ROOT = Path(__file__).resolve().parents[5]
ANSIBLE = ROOT / "ops/ansible"
PLAY = yaml.safe_load((ANSIBLE / "site-seed-from-peer.yml").read_text())[0]
BLOCK = next(t["block"] for t in PLAY["tasks"] if "block" in t)
RESTORE = next(t["ansible.builtin.shell"]["cmd"] for t in BLOCK
               if t["name"].startswith("seed | restore"))
DUMP = next(t["ansible.builtin.shell"]["cmd"] for t in BLOCK
            if t["name"].startswith("seed | dump each"))
PROBE = next(t["ansible.builtin.shell"]["cmd"] for t in PLAY["tasks"]
             if t["name"].startswith("seed | with force_reseed"))
IMAGE = "postgres:16@sha256:ca0bd484cb98bf4b24eb1010e73fb3fcbd6714d240fbc1a10eea5b7dbecb641d"
IDENTITY = "00000000-0000-4000-8000-000000000001"
ENV = jinja2.Environment(undefined=jinja2.StrictUndefined)
ENV.filters.update(bool=bool, quote=shlex.quote)


def run(argv, **kwargs):
    return subprocess.run(argv, text=True, capture_output=True,
                          timeout=kwargs.pop("timeout", 60), **kwargs)


def prepare_docker():
    print("Checking Docker (10-second timeout)...", flush=True)
    try:
        result = run(["docker", "info", "--format", "{{.ServerVersion}}"], timeout=10)
    except (subprocess.TimeoutExpired, OSError) as error:
        raise SystemExit("Docker is not responding. Check Docker Desktop and the selected Docker context, then retry. Nothing was signed or pushed.") from error
    if result.returncode:
        raise SystemExit("Docker is not ready: " + result.stderr.strip())
    print("Docker server: " + result.stdout.strip(), flush=True)
    result = run(["docker", "image", "inspect", IMAGE], timeout=10)
    if result.returncode:
        print("Downloading the pinned PostgreSQL test image (progress below)...", flush=True)
        subprocess.run(["docker", "pull", IMAGE], check=True, timeout=600)


def cleanup_container(name):
    try:
        result = run(["docker", "rm", "-f", name], timeout=5)
        if result.returncode and "No such container" not in result.stderr:
            print(f"Cleanup failed; remove the disposable fixture when Docker responds: docker rm -f {name}", file=sys.stderr)
    except (subprocess.TimeoutExpired, OSError):
        print(f"Cleanup timed out; remove the disposable fixture when Docker responds: docker rm -f {name}", file=sys.stderr)


def render(template, **values):
    def lookup(kind, path):
        source = (ANSIBLE / "templates" / path if kind.endswith("template") else Path(path)).read_text()
        return render(source, **values) if kind.endswith("template") else source
    return ENV.from_string(template).render(lookup=lookup, playbook_dir=str(ANSIBLE), **values)


def main():
    name = "seed-retry-" + uuid.uuid4().hex[:12]
    # The production shell reads /tmp/<stage>-<db>.sql on the host.
    stage = name
    dumps = []

    def sql(db, statement, fail=False):
        result = run(["docker", "exec", "-i", name, "psql", "-X", "-U", "postgres",
                      "-d", db, "-v", "ON_ERROR_STOP=1", "-tAXq"], input=statement)
        if fail:
            assert result.returncode != 0, "expected SQL refusal"
        else:
            assert result.returncode == 0, result.stdout + result.stderr
        return result

    def query(db, expression):
        return sql(db, "SELECT " + expression + ";").stdout.strip()

    def rows(db, tables):
        return {t: json.loads(query(db, f"coalesce(jsonb_agg(to_jsonb(r) ORDER BY id), '[]') FROM {t} r"))
                for t in tables}

    def modes(db):
        return query(db, "coalesce(jsonb_agg(jsonb_build_array(c.relname,t.tgname,t.tgenabled) "
                     "ORDER BY c.relname,t.tgname), '[]') FROM pg_trigger t "
                     "JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace "
                     "WHERE n.nspname='public'")

    def restore(db, tables, force=True, fail=False):
        command = render(RESTORE, item={"db": db, "tables": tables}, force_reseed=force,
                         seed_stage_name=stage, postgres_container=name)
        result = run(["bash", "-c", command])
        assert (result.returncode != 0) == fail, result.stdout + result.stderr
        return result

    prepare_docker()
    print("Starting isolated PostgreSQL fixture: " + name, flush=True)
    try:
        result = run(["docker", "run", "-d", "--rm", "--name", name, "--network", "none",
                      "--tmpfs", "/var/lib/postgresql/data", "-e", "POSTGRES_HOST_AUTH_METHOD=trust", IMAGE])
        assert result.returncode == 0, result.stderr
        for _ in range(120):
            if run(["docker", "exec", name, "pg_isready", "-h", "127.0.0.1", "-U", "postgres"]).returncode == 0:
                break
            time.sleep(0.25)
        else:
            raise AssertionError("PostgreSQL did not become ready")

        nb_tables = yaml.safe_load((ANSIBLE / "roles/spock-mesh-peering/defaults/main.yml").read_text())["spock_mesh_netbird_mgmt_tables"]
        table_sets = {d["db"]: d["tables"] for d in PLAY["vars"]["seed_databases"]}
        table_sets["netbird"] = nb_tables
        for kind, tables in table_sets.items():
            print("Testing " + kind + " seed and retry...", flush=True)
            for db in (kind + "_source", kind):
                sql("postgres", f"CREATE DATABASE {db};")
                if kind == "netbird":
                    sql(db, """
                        CREATE TABLE setup_keys(id text PRIMARY KEY, revoked boolean NOT NULL DEFAULT false);
                        CREATE TABLE users(id text PRIMARY KEY, blocked boolean NOT NULL DEFAULT false);
                        CREATE TABLE peers(id text PRIMARY KEY, user_id text, peer_status_login_expired boolean DEFAULT false);
                        CREATE SCHEMA spock;
                        CREATE TABLE spock.local_node(node_id oid);
                        CREATE TABLE spock.node(node_id oid,node_name text);
                    """)
                    for table in tables:
                        if table not in ("setup_keys", "users", "peers", "goat_revocations", "goat_hub_directives"):
                            sql(db, f"CREATE TABLE {table}(id text PRIMARY KEY);")
                    sql(db, "INSERT INTO setup_keys VALUES ('revoked-key',true); INSERT INTO users VALUES ('blocked-user',true);")
                elif kind == "kratos":
                    sql(db, "CREATE TABLE identities(id uuid PRIMARY KEY,state text NOT NULL,state_changed_at timestamptz);"
                        "CREATE TABLE sessions(id text PRIMARY KEY,identity_id uuid);")
                    for table in tables:
                        if table not in ("identities", "goat_revocations"):
                            owner = "identity_credential_id" if table == "identity_credential_identifiers" else "identity_id"
                            sql(db, f"CREATE TABLE {table}(id uuid PRIMARY KEY,{owner} uuid);")
                    sql(db, f"INSERT INTO identities VALUES ('{IDENTITY}','inactive',now());")
                else:
                    for table in tables:
                        sql(db, f"CREATE TABLE {table}(id text PRIMARY KEY); INSERT INTO {table} VALUES ('{db}');")
                if kind != "hydra":
                    guard = ROOT / "ops/postgres/spock/revocation-guard" / (kind + ".sql")
                    sql(db, guard.read_text())
                    # Reinstall/backfill after a manually emptied ledger, as in #219.
                    sql(db, "TRUNCATE goat_revocations;")
                    # Positive control: the old delete succeeds only while the
                    # ledger is empty; reinstall repopulates it from state.
                    sql(db, "BEGIN; SET LOCAL session_replication_role=replica; DELETE FROM goat_revocations; COMMIT;")
                    sql(db, guard.read_text())
                    assert int(query(db, "count(*) FROM goat_revocations")) > 0
                    if kind == "netbird":
                        sql(db, (ROOT / "ops/postgres/spock/hub-authority/netbird.sql").read_text())
                        sql(db, "INSERT INTO goat_hub_directives(kind,subject) VALUES ('sever_site','fixture-site');")
                        old = sql(db, "BEGIN; SET LOCAL session_replication_role=replica; DELETE FROM goat_hub_directives; COMMIT;", fail=True)
                        assert "insert-only" in old.stderr
                    # The old SQL fails. The surrounding transaction rolls back.
                    old = sql(db, "BEGIN; SET LOCAL session_replication_role=replica; DELETE FROM goat_revocations; COMMIT;", fail=True)
                    assert "insert-only" in old.stderr
                sql(db, "CREATE TABLE non_seed(id text PRIMARY KEY); INSERT INTO non_seed VALUES ('keep');")

            # Exercise the actual pg_dump command's arguments. regex_replace is
            # Ansible's filter; this implementation is only for the dump render.
            import re
            ENV.filters["regex_replace"] = lambda value, pattern, replacement: re.sub(pattern, replacement, value)
            dump_item = {"db": kind + "_source", "tables": tables}
            command = render(DUMP, item=dump_item, postgres_container=name, seed_stage_name=stage)
            source_dump = Path(f"/tmp/{stage}-{kind}_source.sql")
            dumps.append(source_dump)
            result = run(["bash", "-c", command])
            assert result.returncode == 0, result.stderr
            dump_path = Path(f"/tmp/{stage}-{kind}.sql")
            dumps.append(dump_path)
            payload = source_dump.read_text()
            dump_path.write_text(payload)
            before = modes(kind)
            old_rows = rows(kind, tables)
            # A failed load must undo deletion AND trigger changes.
            dump_path.write_text(payload + "\nSELECT 1/0;\n")
            restore(kind, tables, fail=True)
            assert rows(kind, tables) == old_rows
            assert modes(kind) == before
            # A missing host dump must not reach COMMIT either.
            dump_path.unlink()
            restore(kind, tables, fail=True)
            assert rows(kind, tables) == old_rows and modes(kind) == before
            dump_path.write_text(payload)
            restore(kind, tables)
            assert rows(kind, tables) == rows(kind + "_source", tables)
            assert modes(kind) == before
            assert query(kind, "id FROM non_seed") == "keep"
            # Repeat proves retry idempotence with populated immutable ledgers.
            restore(kind, tables)
            assert rows(kind, tables) == rows(kind + "_source", tables)
            if kind != "hydra":
                for table in (["goat_revocations", "goat_hub_directives"] if kind == "netbird" else ["goat_revocations"]):
                    for role in ("origin", "replica"):
                        for mutation in (f"DELETE FROM {table}", f"UPDATE {table} SET actor='overwrite'"):
                            refusal = sql(kind, f"SET session_replication_role={role}; {mutation};", fail=True)
                            assert "insert-only" in refusal.stderr
            print(f"PASS {kind}: populated retry, exact rows and trigger modes, failed/missing load rollback, non-seed preservation", flush=True)

        # A normal initial seed with empty tables still works; use Hydra, whose
        # fixture has no guard backfill. Non-default trigger modes survive too.
        sql("hydra", """
            DELETE FROM hydra_client; DELETE FROM hydra_jwk;
            CREATE FUNCTION fixture_trigger() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;
            CREATE TRIGGER fixture_disabled BEFORE INSERT ON hydra_client FOR EACH ROW EXECUTE FUNCTION fixture_trigger();
            ALTER TABLE hydra_client DISABLE TRIGGER fixture_disabled;
            CREATE TRIGGER fixture_replica BEFORE INSERT ON hydra_client FOR EACH ROW EXECUTE FUNCTION fixture_trigger();
            ALTER TABLE hydra_client ENABLE REPLICA TRIGGER fixture_replica;
        """)
        before = modes("hydra")
        restore("hydra", table_sets["hydra"], force=False)
        assert modes("hydra") == before
        assert rows("hydra", table_sets["hydra"]) == rows("hydra_source", table_sets["hydra"])
        print("PASS initial seed and disabled/replica trigger modes", flush=True)

        sql("netbird", "INSERT INTO spock.local_node VALUES (1);")
        before = rows("netbird", nb_tables)
        result = restore("netbird", nb_tables, fail=True)
        assert "registered Spock node" in result.stderr
        assert rows("netbird", nb_tables) == before
        print("PASS registered Spock node refuses before mutation", flush=True)

        # Fault the probe's external command; its output must never say db=0.
        with tempfile.TemporaryDirectory(prefix="seed-probe-") as directory:
            docker = Path(directory) / "docker"
            docker.write_text("#!/bin/sh\nexit 1\n")
            docker.chmod(0o700)
            import os
            command = render(PROBE, seed_databases=[{"db": "netbird"}], postgres_container=name)
            result = run(["bash", "-c", command], env={**os.environ, "PATH": directory + ":" + os.environ["PATH"]})
            assert result.returncode != 0 and "netbird=0" not in result.stdout
        print("PASS failed node probe refuses", flush=True)
    finally:
        for path in dumps:
            path.unlink(missing_ok=True)
        cleanup_container(name)


if __name__ == "__main__":
    main()
