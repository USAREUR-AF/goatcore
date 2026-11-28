# SPDX-License-Identifier: Apache-2.0
"""Tests for ops/decoy/decoy.py. Run: python3 -m pytest -q ops/decoy"""
import json
import os
import re
import sys
import threading
from types import SimpleNamespace
import urllib.error
import urllib.request

import pytest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import decoy  # noqa: E402

SEED = b"synthetic-seed-for-a-local-trial-0000001"
ZONE = "svc.example.internal"
TDIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "templates", "example")


def test_serve_command_returns_success_when_server_stops(monkeypatch):
    env = {"DECOY_SERVICE_ZONE": ZONE}
    calls = []

    def stopped_server(received):
        assert received is env
        calls.append(True)

    monkeypatch.setattr(decoy, "os", SimpleNamespace(environ=env))
    monkeypatch.setattr(decoy, "serve", stopped_server)
    assert decoy.main(["serve"]) == 0
    assert calls == [True]


@pytest.mark.parametrize("error", [decoy.Refused("synthetic configuration refusal"),
                                 OSError("synthetic startup failure"), KeyboardInterrupt()],
                         ids=["configuration-refusal", "startup-failure", "interrupt"])
def test_serve_command_propagates_server_failure(monkeypatch, error):
    env = {"DECOY_SERVICE_ZONE": ZONE}

    def failed_server(received):
        assert received is env
        raise error

    monkeypatch.setattr(decoy, "os", SimpleNamespace(environ=env))
    monkeypatch.setattr(decoy, "serve", failed_server)
    with pytest.raises(type(error)) as raised:
        decoy.main(["serve"])
    assert raised.value is error


def test_hkdf_matches_rfc_5869_test_case_3():
    assert decoy.hkdf(b"\x0b" * 22, b"").hex() == "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d"


def test_tag_and_digest_match_the_goat_cli():
    # The same seed through `goat decoy tag backup-old` and `goat decoy digest KEYID0EXAMPLE0000001`.
    k = decoy.recognition_key(SEED)
    assert decoy.tagged(k, "backup-old") == "backup-old-apy53sy7"
    assert decoy.issued_digest(k, "KEYID0EXAMPLE0000001") == "f23dc2b1555ab917"


def test_the_two_keys_do_not_recognize_each_others_names():
    r, p = decoy.recognition_key(SEED), decoy.respond_key(SEED)
    planted, served = decoy.tagged(r, "backup-old"), decoy.tagged(p, "replica-3f2a")
    assert decoy.is_tagged((r,), planted) and not decoy.is_tagged((p,), planted)
    assert decoy.is_tagged((p,), served) and not decoy.is_tagged((r,), served)
    assert not decoy.is_tagged((r, p), "backup-old-aaaaaaaa")
    assert not decoy.is_tagged((r, p), "implant-" + planted.rsplit("-", 1)[1])


def test_check_finds_planted_served_and_issued_names_and_nothing_else():
    r, p = decoy.recognition_key(SEED), decoy.respond_key(SEED)
    doc = {"deception_issued": [decoy.issued_digest(r, "KEYID0EXAMPLE0000001")], "deception_zenoh_prefix": "legacy/archive"}
    text = (f"ls /srv/{decoy.tagged(r, 'backup-old')}/README.md; curl https://{decoy.tagged(p, 'replica-3f2a')}.{ZONE}/x "
            "id=KEYID0EXAMPLE0000001 id=KEYID0EXAMPLE0000002 sub legacy/archive/a sub legacy/archived/a /srv/backup-old-aaaaaaaa")
    hits = decoy.check(SEED, text, doc)
    assert set(hits) == {decoy.tagged(r, "backup-old"), decoy.tagged(p, "replica-3f2a"), "KEYID0EXAMPLE0000001", "legacy/archive/a"}


DECL = {
    "deception_service_zone": ZONE,
    "deception_entry_points": [
        {"kind": "false-documentation", "hosts": "core", "path": "/srv/backup-old-{tag}/README.md", "template": "handover", "stop_notice": True},
        {"kind": "decoy-credential", "hosts": "core", "path": "/srv/backup-old-{tag}/credentials.env", "credential": "responder-token"},
        {"kind": "false-documentation", "hosts": "edge", "path": "/opt/notes-{tag}/TODO.md", "template": "handover"},
    ],
}


def test_render_is_deterministic_tagged_and_scoped_to_the_host_group():
    t = decoy.Templates(TDIR)
    a, b = decoy.render(DECL, SEED, t, "core"), decoy.render(DECL, SEED, t, "core")
    assert a == b and len(a) == 2
    assert a[0]["path"] == "/srv/backup-old-apy53sy7/README.md"
    assert a[0]["content"].rstrip().endswith(decoy.STOP_NOTICE)
    assert "{{" not in a[0]["content"] + a[1]["content"]
    p = decoy.respond_key(SEED)
    hosts = re.findall(r"([a-z0-9-]+)\." + re.escape(ZONE), a[0]["content"])
    assert hosts and all(decoy.is_tagged((p,), h) for h in hosts)
    token = re.search(r"API_TOKEN=(\S+)", a[1]["content"])[1]
    assert decoy.is_tagged((p,), token)
    assert len(decoy.render(DECL, SEED, t, "edge")) == 1


def test_render_refuses_an_unbuilt_credential_kind_and_a_bad_path():
    t = decoy.Templates(TDIR)
    bad = {"deception_service_zone": ZONE, "deception_entry_points": [{"kind": "decoy-credential", "hosts": "core", "path": "/a-{tag}/k", "credential": "cloud-access-key"}]}
    with pytest.raises(SystemExit, match="not built"):
        decoy.render(bad, SEED, t, "core")
    with pytest.raises(SystemExit, match="must end in"):
        decoy.plant_path(decoy.recognition_key(SEED), "/srv/{tag}-x/README.md")


def test_lint_refuses_a_real_name_with_the_shape_of_a_decoy():
    hosts = f"100.127.0.1 docs.{ZONE} grafana.{ZONE}\n100.127.0.2 build-abcdefgh.{ZONE}\n"
    problems = decoy.lint(DECL, hosts)
    assert len(problems) == 1 and "build-abcdefgh" in problems[0]
    assert decoy.lint(DECL, f"100.127.0.1 docs.{ZONE}\n") == []


@pytest.fixture()
def responder(tmp_path):
    env = {decoy.RESPOND_KEY_ENV: decoy.respond_key(SEED).hex(), "DECOY_SERVICE_ZONE": ZONE, "DECOY_LISTEN_PORT": "0",
           "DECOY_TEMPLATES_DIR": TDIR, "DECOY_REPLY_DELAY_MS": "0", "DECOY_RATE_PER_SECOND": "0", "DECOY_RATE_BURST": "6",
           "DECOY_METRICS_FILE": str(tmp_path / "decoy.prom"), "DECOY_TRUSTED_PROXIES": "127.0.0.0/8"}
    cfg, state = decoy.Config(env), decoy.State()
    written = []
    httpd = decoy.ThreadingHTTPServer(("127.0.0.1", 0), decoy.make_handler(cfg, state, threading.BoundedSemaphore(8), lambda now: written.append(now)))
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    yield cfg, state, httpd.server_address[1]
    httpd.shutdown()


def get(port, host, path="/", headers=None):
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", headers={"Host": host, **(headers or {})})
    try:
        with urllib.request.urlopen(req, timeout=5) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def test_responder_answers_tagged_names_only_and_every_answer_leads_on(responder):
    cfg, state, port = responder
    p = cfg.key
    host = f"{decoy.tagged(p, 'replica-3f2a')}.{ZONE}"
    assert get(port, f"replica-3f2a-aaaaaaaa.{ZONE}")[0] == 404           # the shape without the key
    assert get(port, f"docs.{ZONE}")[0] == 404                             # a real name
    assert get(port, f"{decoy.tagged(decoy.recognition_key(SEED), 'backup-old')}.{ZONE}")[0] == 404  # a planted name is not served
    assert state.counts["responder-http"] == 0 and state.counts["untagged"] == 3
    s1, b1 = get(port, host, "/status")
    s2, b2 = get(port, host, "/status")
    assert (s1, b1) == (s2, b2) and "{{" not in b1                         # the same question, the same answer
    nxt = re.findall(r"([a-z0-9-]+)\." + re.escape(ZONE), b1)
    nxt = [h for h in nxt if f"{h}.{ZONE}" != host]
    assert nxt and all(decoy.is_tagged((p,), h) for h in nxt)
    assert get(port, f"{nxt[0]}.{ZONE}")[0] in (200, 401)
    assert state.max_depth == 2 and state.counts["responder-http"] == 3


def test_a_presented_decoy_credential_is_a_credential_touch_and_the_limit_holds(responder, capsys):
    cfg, state, port = responder
    host = f"{decoy.tagged(cfg.key, 'vault-0001')}.{ZONE}"
    token = decoy.tagged(cfg.key, "gdt-0123456789abcdef0123")
    status, body = get(port, host, "/v1/keys", {"Authorization": f"Bearer {token}", "X-Forwarded-For": "100.127.0.99"})
    assert status == 403 and "role" in body
    assert state.counts["credential"] == 1
    line = json.loads([ln for ln in capsys.readouterr().out.splitlines() if "decoy_touch" in ln][-1])
    assert line["decoy_kind"] == "credential" and line["source"] == "100.127.0.99" and line["decoy_name"] == host + "/v1/keys"
    assert get(port, host, "/", {"Authorization": "Bearer gdt-0123456789abcdef0123-aaaaaaaa"})[0] != 403
    codes = [get(port, host, f"/{i}", {"X-Forwarded-For": "100.127.0.77"})[0] for i in range(9)]
    assert codes.count(429) == 3 and state.counts["limited"] == 3          # burst 6, no refill
    m = state.metrics(1e12, 3600)
    assert 'goat_decoy_touches_total{kind="credential"} 1' in m and "goat_decoy_responder_up 1" in m
