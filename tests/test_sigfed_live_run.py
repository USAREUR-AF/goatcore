"""Tests for the signal-federation bring-up checks: the relay data-plane guard,
the relay dispatcher post-check, the federation verify probe outcome, the
served-certificate check and DNS-01 clean-up, the relay inventory generator's
profile and key directory, and the relay module description inputs.

The plays run on localhost against fake docker / curl / systemctl / netbird /
aws-vault / terraform binaries put first on PATH; nothing leaves the machine.
"""
import http.server
import json
import os
import re
import shutil
import socket
import stat
import subprocess
import sys
import textwrap
import threading
import time
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parent.parent
ANSIBLE = REPO / "ops" / "ansible"
HAVE_ANSIBLE = shutil.which("ansible-playbook") is not None
needs_ansible = pytest.mark.skipif(not HAVE_ANSIBLE, reason="ansible-playbook not installed")


def fake(bindir: Path, name: str, body: str):
    p = bindir / name
    p.write_text("#!/usr/bin/env bash\n" + textwrap.dedent(body))
    p.chmod(0o755)


def run_play(tmp: Path, play, inventory: str, extra=(), env=None):
    """Run `play` (playbook text, or the Path of a playbook in the repo) on
    localhost hosts with the fakes in tmp/bin first on PATH."""
    if isinstance(play, Path):
        play_path = play
    else:
        play_path = tmp / "play.yml"
        play_path.write_text(textwrap.dedent(play))
    (tmp / "hosts.ini").write_text(textwrap.dedent(inventory))
    e = dict(os.environ)
    e["PATH"] = f"{tmp / 'bin'}:{e['PATH']}"
    e["ANSIBLE_NOCOLOR"] = "1"
    e["ANSIBLE_HOST_KEY_CHECKING"] = "0"
    e["ANSIBLE_RETRY_FILES_ENABLED"] = "0"
    e["ANSIBLE_PYTHON_INTERPRETER"] = sys.executable
    e.update(env or {})
    argv = ["ansible-playbook", "-i", str(tmp / "hosts.ini"), str(play_path), "-e", "ansible_become=false"]
    for x in extra:
        argv += ["-e", x]
    return subprocess.run(argv, cwd=ANSIBLE, env=e, capture_output=True, text=True, timeout=300)


# --------------------------------------------------------------------------
# goat-relay-rathole.yml F-188 guard: either relay container is a data plane
# --------------------------------------------------------------------------

GUARD_PLAY = f"""\
- hosts: relayhost
  gather_facts: false
  tasks:
    - ansible.builtin.include_tasks: {ANSIBLE}/tasks/relay-data-plane-guard.yml
"""


def guard(tmp_path, ps_lines, extra=()):
    (tmp_path / "bin").mkdir()
    (tmp_path / "ps.txt").write_text("".join(f"{n}\t{i}\n" for n, i in ps_lines))
    fake(tmp_path / "bin", "docker", f'[ "$1" = ps ] && cat {tmp_path / "ps.txt"}\n')
    return run_play(tmp_path, GUARD_PLAY, "relayhost ansible_connection=local\n", extra)


@needs_ansible
@pytest.mark.parametrize("ps", [
    [("goat-relay", "ghcr.io/zagros/goat-relay@sha256:00")],
    [("netbird-relay", "netbirdio/relay:0.82.0@sha256:00")],
    [("relay", "docker.io/netbirdio/relay:0.82.0")],
    [("rathole-server", "rapiz1/rathole"), ("netbird-relay", "netbirdio/relay:0.82.0")],
])
def test_guard_accepts_either_relay_data_plane(tmp_path, ps):
    r = guard(tmp_path, ps)
    assert r.returncode == 0, r.stdout + r.stderr
    assert "rathole colocation valid" in r.stdout


@needs_ansible
@pytest.mark.parametrize("ps", [
    [],
    [("rathole-server", "rapiz1/rathole")],
    [("goat-relay-metrics", "ghcr.io/zagros/relay-metrics")],
])
def test_guard_refuses_without_a_relay(tmp_path, ps):
    r = guard(tmp_path, ps)
    assert r.returncode != 0
    assert "neither a goat-relay nor a netbird-relay container is running" in " ".join(r.stdout.replace("\\n", " ").split())


@needs_ansible
def test_guard_waiver_still_works(tmp_path):
    r = guard(tmp_path, [("rathole-server", "rapiz1/rathole")], ["allow_rathole_without_relay=true"])
    assert r.returncode == 0, r.stdout


# --------------------------------------------------------------------------
# relay-ingress-add-site.yml post-check: every dispatcher backend UP
# --------------------------------------------------------------------------

def csv_row(px, sv, status):
    return ",".join([px, sv] + [""] * 15 + [status, ""])


BACKENDS = [
    {"name": "sftest-a", "service": "netbird-mgmt", "tunnel_port": 4430, "weight": 1, "sni_pin": ["federation-a.zagros.example"]},
    {"name": "sftest-b", "service": "netbird-mgmt-sftest-b", "tunnel_port": 4431, "weight": 0, "sni_pin": ["federation-b.zagros.example"]},
]

UP_PLAY = f"""\
- hosts: relay-one
  gather_facts: false
  vars:
    join_site: sftest-b
    founder_site: sftest
    rathole_ingress_check_delay: 0
  tasks:
    - ansible.builtin.include_tasks: {ANSIBLE}/tasks/relay-ingress-backends-up.yml
"""

UP_INV = """\
relay-one ansible_connection=local
[sftest_hosts]
sftest-node ansible_connection=local
[sftest_b_hosts]
sftest-b-node ansible_connection=local
"""


def dispatcher(tmp_path, down_until_restart=(), stuck=(), backends=BACKENDS, joiner_present=True):
    """A relay whose haproxy reports the backends in `down_until_restart` DOWN
    until rathole-client@one is restarted, and those in `stuck` DOWN always."""
    (tmp_path / "bin").mkdir()
    st = tmp_path / "state"
    st.mkdir()
    rows = ["# pxname,svname,qcur"]
    for b in backends:
        if b["name"] == "sftest-b" and not joiner_present:
            continue
        for px in ["balanced"] + ([f"pin_{b['name']}"] if b.get("sni_pin") else []):
            rows.append(csv_row(px, b["name"], "@" + b["name"] + "@"))
        rows.append(csv_row(f"pin_{b['name']}", "BACKEND", "UP"))
    (st / "csv").write_text("\n".join(rows) + "\n")
    script = [f'out=$(cat {st / "csv"})']
    for name in [b["name"] for b in backends]:
        if name in stuck:
            status = "DOWN"
        elif name in down_until_restart:
            status = f'$( [ -e {st}/restarted ] && echo UP || echo "DOWN 1/2")'
        else:
            status = "UP"
        script.append(f'out="${{out//@{name}@/{status}}}"')
    script.append('printf "%s\\n" "$out"')
    fake(tmp_path / "bin", "curl", "\n".join(script) + "\n")
    fake(tmp_path / "bin", "systemctl", f'echo "$*" >> {st}/calls; touch {st}/restarted\n')
    return st


def run_up(tmp_path, backends=BACKENDS):
    return run_play(tmp_path, UP_PLAY, UP_INV, [json.dumps({"rathole_ingress_backends": backends})])


@needs_ansible
def test_dispatcher_all_up_restarts_nothing(tmp_path):
    st = dispatcher(tmp_path)
    r = run_up(tmp_path)
    assert r.returncode == 0, r.stdout
    assert "every dispatcher backend UP" in r.stdout
    assert not (st / "calls").exists()


@needs_ansible
def test_dispatcher_down_founder_backend_gets_its_client_restarted(tmp_path):
    st = dispatcher(tmp_path, down_until_restart=["sftest-a"])
    r = run_up(tmp_path)
    assert r.returncode == 0, r.stdout
    assert (st / "calls").read_text().split("\n")[0] == "restart rathole-client@one"
    assert "sftest-a on sftest-node (rathole-client@one)" in r.stdout


@needs_ansible
def test_dispatcher_down_joiner_backend_restarts_on_the_joiner(tmp_path):
    st = dispatcher(tmp_path, down_until_restart=["sftest-b"])
    r = run_up(tmp_path)
    assert r.returncode == 0, r.stdout
    assert "sftest-b on sftest-b-node (rathole-client@one)" in r.stdout
    assert (st / "calls").read_text().strip() == "restart rathole-client@one"


@needs_ansible
def test_dispatcher_still_down_fails_naming_relay_and_backend(tmp_path):
    dispatcher(tmp_path, stuck=["sftest-a"])
    r = run_up(tmp_path)
    assert r.returncode != 0
    flat = " ".join(r.stdout.replace("\\n", " ").split())
    assert "relay-one: dispatcher backend(s) still not UP" in flat
    assert "balanced/sftest-a DOWN" in flat


@needs_ansible
def test_dispatcher_down_backend_without_a_node_is_named(tmp_path):
    backends = BACKENDS + [{"name": "elsewhere", "service": "netbird-mgmt-elsewhere", "tunnel_port": 4432, "weight": 0}]
    st = dispatcher(tmp_path, stuck=["elsewhere"], backends=backends)
    r = run_up(tmp_path, backends)
    assert r.returncode != 0
    flat = " ".join(r.stdout.replace("\\n", " ").split())
    assert "Backend elsewhere names no node in this inventory" in flat
    assert not (st / "calls").exists()


@needs_ansible
def test_dispatcher_missing_joiner_fails(tmp_path):
    dispatcher(tmp_path, joiner_present=False)
    r = run_up(tmp_path)
    assert r.returncode != 0
    assert "no `sftest-b` server in the haproxy `balanced` backend" in " ".join(r.stdout.split())


# --------------------------------------------------------------------------
# signal-federation-verify.yml: the probe's outcome is named, never empty
# --------------------------------------------------------------------------

VERIFY_INV = """\
[sftest_hosts]
sftest-node ansible_connection=local
[sftest_b_hosts]
sftest-b-node ansible_connection=local
[all:vars]
federation_peer_mesh_addr=127.0.0.1:1
federation_peer_public_sni=federation.zagros.example
federation_public_relays=["relay-one.zagros.example"]
"""


def verify(tmp_path, run_body, extra=()):
    (tmp_path / "bin").mkdir()
    st = tmp_path / "state"
    st.mkdir()
    fake(tmp_path / "bin", "netbird", 'echo \'{"publicKey": "key-of-this-node"}\'\n')
    fake(tmp_path / "bin", "openssl", "exit 0\n")
    fake(tmp_path / "bin", "curl", "echo 'signal_server_forward_sent_total{outcome=\"delivered\"} 3'\n")
    fake(tmp_path / "bin", "docker", f"""\
        case "$1" in
          inspect) echo running; echo running ;;
          logs) echo "federation auth enforced" ;;
          run) printf '%s\\n' "$*" >> {st}/run
               {run_body} ;;
        esac
        """)
    r = run_play(tmp_path, ANSIBLE / "signal-federation-verify.yml", VERIFY_INV,
                 ["federation_site=sftest", "federation_peer_site=sftest-b", *extra])
    return r, st


@needs_ansible
def test_verify_names_an_image_pull_failure(tmp_path):
    r, _ = verify(tmp_path, 'echo "docker: Error response from daemon: manifest unknown" >&2; exit 125')
    assert r.returncode != 0
    flat = " ".join(r.stdout.split())
    assert "FAILED: image pull failed for ghcr.io/" in flat
    assert "manifest unknown" in flat
    assert "FAILED: probe outcome" not in flat


@needs_ansible
def test_verify_names_a_probe_that_ran_and_was_not_delivered(tmp_path):
    r, _ = verify(tmp_path, 'echo "offer outcome: no route" ; exit 1')
    assert "FAILED: probe ran and the offer was not delivered (exit 1): offer outcome: no route" in " ".join(r.stdout.split())


@needs_ansible
def test_verify_a_group_vars_override_of_the_probe_image_wins(tmp_path):
    r, st = verify(tmp_path, "exit 0", ["signal_federation_probe_image=registry.zagros.example/probe@sha256:abc"])
    assert "registry.zagros.example/probe@sha256:abc" in (st / "run").read_text()
    assert "UNPROVEN: the probe was delivered without a forward" in " ".join(r.stdout.split())


@needs_ansible
def test_verify_default_probe_image_is_the_canary_pin(tmp_path):
    _, st = verify(tmp_path, "exit 0")
    import yaml
    pin = yaml.safe_load((ANSIBLE / "roles/signal-federation-canary/defaults/main.yml").read_text())["signal_federation_probe_image"]
    assert pin.split("/", 2)[2] in (st / "run").read_text()


# --------------------------------------------------------------------------
# traefik-acme.yml: the served-certificate check
# --------------------------------------------------------------------------

def served_check_script(name, port):
    import jinja2
    import yaml
    plays = yaml.safe_load((ANSIBLE / "traefik-acme.yml").read_text())
    tpl = next(p for p in plays if "_served_cert_check" in (p.get("vars") or {}))["vars"]["_served_cert_check"]
    # Autoescape on: the values rendered here (a DNS name, a port) carry no
    # HTML-special characters, so the bash text is the same either way.
    return jinja2.Environment(autoescape=True).from_string(tpl).render(item=name, traefik_acme_served_check_port=port)


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


@pytest.fixture
def tls_server(tmp_path):
    procs = []

    def serve(subject, san):
        key, crt = tmp_path / f"{len(procs)}.key", tmp_path / f"{len(procs)}.crt"
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                        "-keyout", key, "-out", crt, "-subj", subject, "-addext", f"subjectAltName=DNS:{san}"],
                       check=True, capture_output=True)
        port = free_port()
        p = subprocess.Popen(["openssl", "s_server", "-quiet", "-accept", str(port), "-cert", crt, "-key", key],
                             stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        procs.append(p)
        for _ in range(50):
            with socket.socket() as s:
                if s.connect_ex(("127.0.0.1", port)) == 0:
                    return port
            time.sleep(0.1)
        raise RuntimeError("s_server did not start")
    yield serve
    for p in procs:
        p.kill()


needs_tls_tools = pytest.mark.skipif(not (shutil.which("openssl") and shutil.which("timeout")), reason="openssl/timeout missing")


@needs_tls_tools
def test_served_check_passes_a_certificate_for_the_name(tls_server):
    port = tls_server("/CN=federation.zagros.example", "federation.zagros.example")
    r = subprocess.run(["bash", "-c", served_check_script("federation.zagros.example", port)], capture_output=True, text=True)
    assert r.returncode == 0, r.stdout + r.stderr


@needs_tls_tools
def test_served_check_refuses_the_traefik_default_certificate(tls_server):
    port = tls_server("/CN=TRAEFIK DEFAULT CERT", "0123.traefik.default")
    r = subprocess.run(["bash", "-c", served_check_script("federation.zagros.example", port)], capture_output=True, text=True)
    assert r.returncode == 1 and "TRAEFIK DEFAULT CERT for federation.zagros.example" in r.stdout


@needs_tls_tools
def test_served_check_refuses_a_certificate_for_another_name(tls_server):
    port = tls_server("/CN=netbird.zagros.example", "netbird.zagros.example")
    r = subprocess.run(["bash", "-c", served_check_script("federation.zagros.example", port)], capture_output=True, text=True)
    assert r.returncode == 1 and "does not cover federation.zagros.example" in r.stdout


@needs_tls_tools
def test_served_check_names_a_closed_port():
    r = subprocess.run(["bash", "-c", served_check_script("federation.zagros.example", free_port())], capture_output=True, text=True)
    assert r.returncode == 1 and "no certificate served for federation.zagros.example" in r.stdout


# --------------------------------------------------------------------------
# cf-acme-challenge-clean.py against a fake Cloudflare API
# --------------------------------------------------------------------------

class FakeCf(http.server.BaseHTTPRequestHandler):
    zones = {"zagros.example": "z1"}
    records = {}
    deleted = []
    tokens = []

    def log_message(self, *a):
        pass

    def reply(self, code, body):
        b = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b)

    def do_GET(self):
        from urllib.parse import urlparse, parse_qs
        FakeCf.tokens.append(self.headers.get("Authorization"))
        u = urlparse(self.path)
        q = {k: v[0] for k, v in parse_qs(u.query).items()}
        parts = u.path.rstrip("/").split("/")
        if parts[-1] == "zones":
            z = self.zones.get(q.get("name"))
            self.reply(200, {"success": True, "result": [{"id": z, "name": q["name"]}] if z else []})
        elif parts[-1] == "dns_records" and parts[-3] == "zones":
            res = [r for r in self.records.get(parts[-2], []) if r["type"] == q.get("type") and r["name"] == q.get("name")]
            self.reply(200, {"success": True, "result": res})
        else:
            self.reply(404, {"success": False, "errors": [{"code": 7003, "message": "no route"}]})

    def do_DELETE(self):
        rid = self.path.rstrip("/").split("/")[-1]
        FakeCf.deleted.append(rid)
        self.reply(200, {"success": True, "result": {"id": rid}})


@pytest.fixture
def fake_cf():
    FakeCf.records = {"z1": [
        {"id": "r1", "type": "TXT", "name": "_acme-challenge.federation.zagros.example", "content": "stale-1"},
        {"id": "r2", "type": "TXT", "name": "_acme-challenge.federation.zagros.example", "content": "stale-2"},
        {"id": "r3", "type": "TXT", "name": "_acme-challenge.netbird.zagros.example", "content": "keep"},
        {"id": "r4", "type": "A", "name": "federation.zagros.example", "content": "192.0.2.1"},
    ]}
    FakeCf.deleted, FakeCf.tokens = [], []
    srv = http.server.HTTPServer(("127.0.0.1", 0), FakeCf)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    yield f"http://127.0.0.1:{srv.server_port}/client/v4"
    srv.shutdown()


def clean(api, *names, token="cf-test-token-value"):
    env = dict(os.environ, CF_DNS_API_TOKEN=token)
    return subprocess.run([sys.executable, REPO / "ops/scripts/cf-acme-challenge-clean.py", "--api", api, *names],
                          env=env, capture_output=True, text=True, timeout=60)


def test_cf_clean_deletes_only_the_names_challenge_records(fake_cf):
    r = clean(fake_cf, "federation.zagros.example")
    assert r.returncode == 0, r.stdout + r.stderr
    assert sorted(FakeCf.deleted) == ["r1", "r2"]
    assert "deleted 2 stale _acme-challenge.federation.zagros.example TXT record(s) in zone zagros.example" in r.stdout
    assert "cf-test-token-value" not in r.stdout + r.stderr
    assert "stale-1" not in r.stdout + r.stderr
    assert set(FakeCf.tokens) == {"Bearer cf-test-token-value"}


def test_cf_clean_with_nothing_stale_is_a_pass(fake_cf):
    r = clean(fake_cf, "dashboard.zagros.example")
    assert r.returncode == 0 and "deleted 0" in r.stdout and FakeCf.deleted == []


def test_cf_clean_names_a_zone_it_cannot_find(fake_cf):
    r = clean(fake_cf, "federation.other.example")
    assert r.returncode == 1 and "federation.other.example: FAILED: no Cloudflare zone" in r.stdout


def test_cf_clean_refuses_without_a_token(fake_cf):
    r = clean(fake_cf, "federation.zagros.example", token="")
    assert r.returncode == 2 and FakeCf.tokens == []


# --------------------------------------------------------------------------
# generate-relays.sh: declared profile, aws-vault errors, key directory
# --------------------------------------------------------------------------

RELAYS_JSON = json.dumps({"one": {"public_ip": "192.0.2.10", "region": "us-test-1", "instance_id": "i-0001",
                                  "ssh_key_secret_name": "zagros-relay-one-ssh", "rathole_secret_name": "zagros-relay-one-rathole",
                                  "relay_url": "relay-one.zagros.example"}})


def generator(tmp_path, goatbuilder_yaml=None, vault_fails=False):
    repo = tmp_path / "repo"
    inv = repo / "ops/ansible/inventory"
    inv.mkdir(parents=True)
    for f in ("generate-relays.sh", "relay-tls-posture.sh"):
        shutil.copy(ANSIBLE / "inventory" / f, inv / f)
    (repo / "ops/scripts").mkdir(parents=True)
    shutil.copy(REPO / "ops/scripts/project-setting.sh", repo / "ops/scripts/project-setting.sh")
    if goatbuilder_yaml is not None:
        (repo / "ops/goatbuilder").mkdir(parents=True)
        (repo / "ops/goatbuilder/goatbuilder.yaml").write_text(goatbuilder_yaml)
    (repo / "terraform-sftest").mkdir()
    bindir = tmp_path / "bin"
    bindir.mkdir()
    log = tmp_path / "vault.log"
    if vault_fails:
        fake(bindir, "aws-vault", 'echo "aws-vault: error: exec: profile $2 not found in ~/.aws/config" >&2; exit 1\n')
    else:
        fake(bindir, "aws-vault", f'echo "$2" >> {log}; shift 3; [ "$1" = -- ] && shift; exec "$@"\n')
    (tmp_path / "relays.json").write_text(RELAYS_JSON)
    fake(bindir, "terraform", f"""\
        case "$3" in
          relays) cat {tmp_path / 'relays.json'} ;;
          ansible_ssm_bucket) echo '"zagros-ssm-bucket"' ;;
          *) echo '{{}}' ;;
        esac
        """)
    fake(bindir, "aws", 'echo "-----BEGIN OPENSSH PRIVATE KEY----- fake"\n')
    home = tmp_path / "home"
    home.mkdir()
    return repo, bindir, home, log


def gen(repo, bindir, home, env_name="sftest", extra_env=None):
    env = {k: v for k, v in os.environ.items() if not k.startswith(("AWS_", "GOAT_AWS"))}
    env.update(PATH=f"{bindir}:{os.environ['PATH']}", HOME=str(home), RELAY_TLS_POSTURE_SFTEST="self-signed")
    env.update(extra_env or {})
    return subprocess.run(["bash", repo / "ops/ansible/inventory/generate-relays.sh", env_name, "terraform-sftest"],
                          env=env, capture_output=True, text=True, timeout=120)


def test_generator_uses_the_declared_profile_and_a_private_key_dir(tmp_path):
    repo, bindir, home, log = generator(tmp_path, "deploy:\n  aws_profile: zagros-account\n")
    legacy = Path("/tmp/sftest-ssh-keys")
    made_legacy = False
    if not legacy.exists():
        legacy.mkdir()
        (legacy / "one.pem").write_text("old")
        made_legacy = True
    try:
        r = gen(repo, bindir, home)
        assert r.returncode == 0, r.stdout + r.stderr
        assert set(log.read_text().split()) == {"zagros-account"}
        assert "aws-vault profile: zagros-account (from deploy.aws_profile in ops/goatbuilder/goatbuilder.yaml)" in r.stdout
        kd = home / ".goat/state/relay-ssh-keys/sftest"
        assert stat.S_IMODE(kd.stat().st_mode) == 0o700
        assert stat.S_IMODE((kd / "one.pem").stat().st_mode) == 0o600
        assert f"ansible_ssh_private_key_file={kd}/one.pem" in (repo / "ops/ansible/inventory/hosts-sftest.ini").read_text()
        assert f"SSH keys:   {kd}/" in r.stdout
        if made_legacy:
            assert not legacy.exists()
    finally:
        if made_legacy and legacy.exists():
            shutil.rmtree(legacy)


def test_generator_env_override_beats_the_declared_profile(tmp_path):
    repo, bindir, home, log = generator(tmp_path, "deploy:\n  aws_profile: zagros-account\n")
    r = gen(repo, bindir, home, extra_env={"AWS_VAULT_PROFILE_SFTEST": "per-env"})
    assert r.returncode == 0, r.stderr
    assert set(log.read_text().split()) == {"per-env"}


def test_generator_declared_state_dir_holds_the_keys(tmp_path):
    repo, bindir, home, _ = generator(tmp_path, f"deploy:\n  aws_profile: a\n  state_dir: {tmp_path}/st\n")
    r = gen(repo, bindir, home)
    assert r.returncode == 0, r.stderr
    assert (tmp_path / "st/relay-ssh-keys/sftest/one.pem").is_file()


def test_generator_prints_the_aws_vault_error(tmp_path):
    repo, bindir, home, _ = generator(tmp_path, "deploy:\n  aws_profile: missing-profile\n", vault_fails=True)
    r = gen(repo, bindir, home)
    assert r.returncode == 68, (r.returncode, r.stdout, r.stderr)
    assert "aws-vault: error: exec: profile missing-profile not found" in r.stderr
    assert "this run used 'missing-profile' (from deploy.aws_profile" in r.stderr


# --------------------------------------------------------------------------
# terraform modules: every resource description is an input
# --------------------------------------------------------------------------

@pytest.mark.parametrize("module,keys", [
    ("goat-relay", {"security_group", "ssh", "websocket", "quic", "wg_cp0", "rathole_ingress", "rathole_control",
                    "coturn_stun_udp", "coturn_stun_tcp", "coturn_turn", "ab_websocket", "ab_quic", "icmp", "egress",
                    "ssh_key_secret", "rathole_secret"}),
    ("relay-health-host", {"security_group", "public_443", "public_80"}),
])
def test_module_descriptions_are_inputs(module, keys):
    main = (REPO / "terraform/modules" / module / "main.tf").read_text()
    body = re.sub(r"locals \{\n  descriptions = merge\(\{.*?\}, var\.descriptions\)\n\}", "", main, flags=re.S)
    literal = re.findall(r'^\s*description\s*=\s*".*"', body, flags=re.M)
    assert literal == [], f"literal resource descriptions left in {module}: {literal}"
    defaults = re.search(r"descriptions = merge\(\{(.*?)\}, var\.descriptions\)", main, flags=re.S).group(1)
    assert set(re.findall(r"^\s*(\w+)\s*=", defaults, flags=re.M)) == keys
    variables = (REPO / "terraform/modules" / module / "variables.tf").read_text()
    allowed = re.search(r'variable "descriptions".*?setsubtract\(keys\(var\.descriptions\), \[(.*?)\]\)', variables, flags=re.S).group(1)
    assert set(re.findall(r'"(\w+)"', allowed)) == keys
