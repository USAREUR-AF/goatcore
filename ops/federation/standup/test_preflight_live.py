"""Phase-1 live checks, each with fakes: no test reads a host, a registry or a
real container runtime (docs/design/federation-preflight.md)."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import unittest.mock as mock

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
sys.path.insert(0, str(HERE))
import preflight_live as live  # noqa: E402


def load_preflight():
    spec = importlib.util.spec_from_file_location("preflight_under_test", HERE / "preflight.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def host(ss="", wg="", containers="", pids="", ss_ok=True, name="edge-1"):
    return {"host": name, "ss": ss, "wg": wg, "containers": containers, "pids": pids, "ss_ok": ss_ok}


NETBIRD_WG = "udp   UNCONN 0      0            0.0.0.0:51820      0.0.0.0:*\n"
EDGE_TCP = 'tcp   LISTEN 0      4096   192.0.2.10:7448   0.0.0.0:*    users:(("zenohd",pid=4242,fd=9))\n'


class EdgePorts(unittest.TestCase):
    def check(self, facts, wg_port=51820, edge_port=7448, bind=None):
        return live.check_edge_ports({"edge-1": facts}, ["edge-1"], wg_port, "wg-fed0", edge_port,
                                     "goat-zenoh-edge-xab", "xab_edge", bind)

    def test_free_ports_pass(self):
        r = self.check(host(ss='tcp LISTEN 0 128 0.0.0.0:22 0.0.0.0:* users:(("sshd",pid=1,fd=3))\n'))
        self.assertEqual(r.outcome, live.PASS, r)

    def test_netbird_holding_the_tunnel_port_is_a_collision(self):
        # The 9 Nov run: NetBird's own WireGuard interface held UDP 51820.
        r = self.check(host(ss=NETBIRD_WG, wg="wt0\t51820\n"))
        self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-131"), r)
        self.assertIn("WireGuard interface wt0", r.detail)
        self.assertIn("NetBird holds UDP 51820", r.fix)

    def test_a_userspace_process_holding_the_port_is_named(self):
        r = self.check(host(ss='udp UNCONN 0 0 [::]:51820 [::]:* users:(("netbird",pid=77,fd=5))\n'))
        self.assertEqual(r.outcome, live.FAIL)
        self.assertIn("process netbird (pid 77)", r.detail)

    def test_this_peerings_own_interface_on_a_rerun_is_not_a_collision(self):
        r = self.check(host(ss=NETBIRD_WG, wg="wg-fed0\t51820\n"))
        self.assertEqual(r.outcome, live.PASS, r)
        self.assertIn("own service", r.detail)

    def test_this_peerings_own_edge_container_is_not_a_collision(self):
        r = self.check(host(ss=EDGE_TCP, pids="4242 /goat-zenoh-edge-xab\n"))
        self.assertEqual(r.outcome, live.PASS, r)

    def test_another_peerings_edge_router_on_our_address_is_a_collision(self):
        # Same address, same port: the two routers cannot both listen.
        r = self.check(host(ss=EDGE_TCP, pids="4242 /goat-zenoh-edge-xac\n"), bind="192.0.2.10")
        self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-131"), r)
        self.assertIn("container goat-zenoh-edge-xac", r.detail)

    def test_another_peerings_edge_router_on_its_own_link_address_does_not_collide(self):
        # The shared edge: the first peering's router holds 7448 on its link
        # address; ours binds a different link address.
        first = 'tcp LISTEN 0 4096 169.254.77.0:7448 0.0.0.0:* users:(("zenohd",pid=4242,fd=9))\n'
        r = self.check(host(ss=first, pids="4242 /goat-zenoh-edge-xab2\n"), bind="169.254.77.2")
        self.assertEqual(r.outcome, live.PASS, r)
        self.assertIn("on another address", r.detail)
        # Before phase 9 the link address is not known yet: a holder on a
        # specific address still does not collide.
        self.assertEqual(self.check(host(ss=first, pids="4242 /goat-zenoh-edge-xab2\n"), bind="pending").outcome,
                         live.PASS)

    def test_a_wildcard_on_either_side_is_a_collision(self):
        wild = 'tcp LISTEN 0 4096 0.0.0.0:7448 0.0.0.0:* users:(("zenohd",pid=4242,fd=9))\n'
        wild6 = 'tcp LISTEN 0 4096 [::]:7448 [::]:* users:(("zenohd",pid=4242,fd=9))\n'
        pids = "4242 /goat-zenoh-edge-xac\n"
        for ss in (wild, wild6):
            for bind in ("169.254.77.2", "pending"):
                self.assertEqual(self.check(host(ss=ss, pids=pids), bind=bind).outcome, live.FAIL, (ss, bind))
        specific = 'tcp LISTEN 0 4096 169.254.77.0:7448 0.0.0.0:* users:(("zenohd",pid=4242,fd=9))\n'
        self.assertEqual(self.check(host(ss=specific, pids=pids), bind="0.0.0.0").outcome, live.FAIL)
        # A container publishing the port on every address collides too.
        self.assertEqual(self.check(host(containers="goat-zenoh-edge-xac\t0.0.0.0:7448->7448/tcp\n"),
                                    bind="169.254.77.2").outcome, live.FAIL)
        self.assertEqual(self.check(host(containers="goat-zenoh-edge-xac\t169.254.77.0:7448->7448/tcp\n"),
                                    bind="169.254.77.2").outcome, live.PASS)

    def test_the_bind_address_comes_from_apply_then_the_deployment_else_it_is_pending(self):
        with tempfile.TemporaryDirectory() as t:
            self.assertEqual(live.edge_bind_address(t, t, "xab_edge", {})[0], "pending")
            gv = Path(t) / "ops/ansible/group_vars/xab_edge"
            gv.mkdir(parents=True)
            (gv / "edge.yml").write_text('goat_zenoh_edge_bind_address: "0.0.0.0"\n')
            self.assertEqual(live.edge_bind_address(t, t, "xab_edge", {})[0], "0.0.0.0")
            ap = Path(t) / "applied/ops/ansible/group_vars/xab_edge"
            ap.mkdir(parents=True)
            (ap / "federation-xab-transport.yml").write_text('goat_zenoh_edge_bind_address: "169.254.77.2"\n')
            self.assertEqual(live.edge_bind_address(t, t, "xab_edge", {})[0], "169.254.77.2")
            self.assertEqual(live.edge_bind_address(t, t, "xab_edge", {"FED_EDGE_BIND_ADDRESS": "x"})[0], "x")

    def test_docker_proxy_for_our_published_port_is_ours(self):
        ss = 'tcp LISTEN 0 4096 192.0.2.10:7448 0.0.0.0:* users:(("docker-proxy",pid=900,fd=4))\n'
        cont = "goat-zenoh-edge-xab\t192.0.2.10:7448->7448/tcp\n"
        self.assertEqual(self.check(host(ss=ss, containers=cont)).outcome, live.PASS)
        self.assertEqual(self.check(host(ss=ss, containers="other\t192.0.2.10:7448->7448/tcp\n")).outcome, live.FAIL)

    def test_a_port_published_without_a_socket_is_a_collision(self):
        # Docker's userland proxy off: the published port has no listening socket.
        cont = "goat-zenoh-edge-xac\t0.0.0.0:7448->7448/tcp\n"
        r = self.check(host(containers=cont))
        self.assertEqual(r.outcome, live.FAIL, r)
        self.assertIn("published by container goat-zenoh-edge-xac", r.detail)
        self.assertEqual(self.check(host(containers="goat-zenoh-edge-xab\t0.0.0.0:7448->7448/tcp\n")).outcome,
                         live.PASS)

    def test_no_probe_answer_is_a_failure_not_a_pass(self):
        r = live.check_edge_ports({}, ["edge-1"], 51820, "wg-fed0", 7448, "x", "xab_edge")
        self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-133"))

    def test_ss_missing_is_a_failure(self):
        self.assertEqual(self.check(host(ss_ok=False)).code, "GB-133")

    def test_an_empty_group_is_blocked(self):
        r = live.check_edge_ports({}, [], 51820, "wg-fed0", 7448, "x", "xab_edge")
        self.assertEqual(r.outcome, live.BLOCKED)


class CorePorts(unittest.TestCase):
    def test_metrics_port_held_by_another_service(self):
        ss = 'tcp LISTEN 0 4096 127.0.0.1:9310 0.0.0.0:* users:(("prometheus",pid=55,fd=7))\n'
        r = live.check_core_ports({"core-1": host(ss=ss, name="core-1")}, ["core-1"], 9310,
                                  "goat-fed-republisher-xab", "xab_core")
        self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-132"))

    def test_metrics_port_held_by_our_republisher(self):
        ss = 'tcp LISTEN 0 4096 127.0.0.1:9310 0.0.0.0:* users:(("republisher",pid=56,fd=7))\n'
        r = live.check_core_ports({"core-1": host(ss=ss, pids="56 /goat-fed-republisher-xab\n", name="core-1")},
                                  ["core-1"], 9310, "goat-fed-republisher-xab", "xab_core")
        self.assertEqual(r.outcome, live.PASS, r)


class PortInputs(unittest.TestCase):
    def test_env_then_group_vars_then_role_default(self):
        with tempfile.TemporaryDirectory() as t:
            gv = Path(t) / "ops/ansible/group_vars/xab_edge"
            gv.mkdir(parents=True)
            (gv / "ports.yml").write_text("peering_wg_listen_port: 51830\ngoat_zenoh_edge_listen_port: '{{ x }}'\n")
            rd = Path(t) / "ops/ansible/roles/goat-zenoh-edge/defaults"
            rd.mkdir(parents=True)
            (rd / "main.yml").write_text("goat_zenoh_edge_listen_port: 7448\n")
            got = live.resolve_port_inputs(t, "xab_edge", "xab_core", {"FED_WG_INTERFACE": "wg-fed3"})
        self.assertEqual(got["FED_WG_INTERFACE"][0], "wg-fed3")
        self.assertEqual(got["FED_WG_LISTEN_PORT"], ("51830", "group_vars/xab_edge: peering_wg_listen_port"))
        # A templated group value is not resolved here, and the role default
        # does not stand in for it.
        self.assertIsNone(got["FED_EDGE_LISTEN_PORT"][0])
        self.assertIn("templated", got["FED_EDGE_LISTEN_PORT"][1])
        self.assertIsNone(got["FED_REPUBLISHER_METRICS_PORT"][0])

    def test_a_templated_declaration_blocks_instead_of_falling_back(self):
        with tempfile.TemporaryDirectory() as t:
            for g, body in (("xab_edge", "peering_wg_listen_port: '{{ alloc_port }}'\n"),
                            ("peering_cores", "goat_fed_republisher_metrics_port: 9411\n"),
                            ("all", "goat_zenoh_edge_listen_port: 7450\n")):
                d = Path(t) / "ops/ansible/group_vars" / g
                d.mkdir(parents=True)
                (d / "vars.yml").write_text(body)
            rd = Path(t) / "ops/ansible/roles/goat-peering-wg/defaults"
            rd.mkdir(parents=True)
            (rd / "main.yml").write_text("peering_wg_listen_port: 51820\n")
            got = live.resolve_port_inputs(t, "xab_edge", "xab_core", {})
        self.assertIsNone(got["FED_WG_LISTEN_PORT"][0])
        self.assertIn("templated", got["FED_WG_LISTEN_PORT"][1])
        self.assertEqual(got["FED_REPUBLISHER_METRICS_PORT"], ("9411", "group_vars/peering_cores: goat_fed_republisher_metrics_port"))
        self.assertEqual(got["FED_EDGE_LISTEN_PORT"], ("7450", "group_vars/all: goat_zenoh_edge_listen_port"))

    def test_the_repository_roles_supply_port_defaults(self):
        got = live.resolve_port_inputs(REPO, "no_such_edge", "no_such_core", {})
        for n in ("FED_WG_INTERFACE", "FED_WG_LISTEN_PORT", "FED_EDGE_LISTEN_PORT", "FED_REPUBLISHER_METRICS_PORT"):
            self.assertIsNotNone(got[n][0], n)

    def test_missing_inputs_are_listed_with_where_to_declare(self):
        r = live.check_inputs([("FED_SIGNING_KEY", "goat_federation_signing_key")])
        self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-130"))
        self.assertIn("goat_federation_signing_key", r.fix)
        self.assertEqual(live.check_inputs([]).outcome, live.PASS)


class SlotAnchors(unittest.TestCase):
    def setUp(self):
        self.t = tempfile.TemporaryDirectory()
        f = Path(self.t.name) / "ops/ansible/group_vars/zagros_hosts/zenoh-lazy-storage.yml"
        f.parent.mkdir(parents=True)
        f.write_text("zenoh_active_slot_anchors: []\n")

    def tearDown(self):
        self.t.cleanup()

    def runner(self, rc, out=b"", err=b""):
        calls = []

        def fake(argv, **kw):
            calls.append(argv)
            return rc, out, err
        return fake, calls

    def test_drift_is_a_failure_with_the_regenerate_command(self):
        fake, calls = self.runner(1, "zagros: live slots 462 · committed 461 · +1 / -0\n  + s-0462\n".encode(),
                                  b"DRIFT: the committed anchor list does not match the live registry.\n")
        r = live.check_slot_anchors(self.t.name, "zagros", {"db": "/srv/registry.db"}, fake)
        self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-134"))
        self.assertIn("462", r.detail)
        self.assertIn("--check", calls[0])
        self.assertIn("sync-slot-anchors.py --env zagros --db /srv/registry.db\n", r.fix)

    def test_in_sync_passes(self):
        fake, _ = self.runner(0, b"zagros: live slots 462 \xc2\xb7 committed 462\nin sync\n")
        self.assertEqual(live.check_slot_anchors(self.t.name, "zagros", {"db": "/x"}, fake).outcome, live.PASS)

    def test_unreadable_registry_is_not_a_drift_verdict(self):
        fake, _ = self.runner(2, b"", b"UNREADABLE: /x: unable to open database file\n")
        r = live.check_slot_anchors(self.t.name, "zagros", {"db": "/x"}, fake)
        self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-135"))

    def test_postgres_registry_flags(self):
        argv = live.slot_check_command(self.t.name, "zagros", {"store": "postgres", "pg_ssh": "mgmt-1",
                                                                "pg_container": "portal-db"})
        self.assertEqual(argv[argv.index("--registry-store") + 1], "postgres")
        self.assertEqual(argv[argv.index("--pg-ssh") + 1], "mgmt-1")
        self.assertNotIn("--db", argv)

    def test_no_lazy_storage_is_not_applicable(self):
        fake, calls = self.runner(0)
        r = live.check_slot_anchors(self.t.name, "other-net", {"db": "/x"}, fake)
        self.assertEqual(r.outcome, live.NA)
        self.assertEqual(calls, [])

    def test_no_registry_named_is_blocked(self):
        fake, calls = self.runner(0)
        r = live.check_slot_anchors(self.t.name, "zagros", {}, fake)
        self.assertEqual((r.outcome, r.code), (live.BLOCKED, "GB-130"))
        self.assertEqual(calls, [])


def fake_runtime(cases):
    """call(tool, args) from {(tool, request): (ok, payload)}."""
    def call(tool, args):
        req = "ps" if args[0] == "ps" else ("exec" if args[0] == "exec" else f"inspect:{args[-1]}")
        return cases.get((tool, req), (False, f"{tool}: no reply for {req}"))
    return call


class CoreRouter(unittest.TestCase):
    def test_the_shared_discovery_cases_hold(self):
        # The same file crates/goat-federation/src/derive.rs reads.
        rows = (HERE / "fixtures/router-discovery.tsv").read_text().splitlines()
        cases, unesc = [], lambda s: s.replace("\\t", "\t").replace("\\n", "\n").replace("\\\\", "\\")
        for line in rows:
            if not line or line.startswith("#"):
                continue
            f = line.split("\t")
            if f[0] == "case":
                cases.append({"name": f[1], "sel": "" if f[2] == "-" else f[2], "tools": f[3].split(","),
                              "expect": f[4], "id": f[5] if len(f) > 5 else "", "replies": {}})
            else:
                cases[-1]["replies"][(f[1], f[2])] = (f[3] == "ok", unesc(f[4] if len(f) > 4 else ""))
        self.assertGreaterEqual(len(cases), 10)
        for c in cases:
            with self.subTest(c["name"]):
                verdict, what, _ = live.select_router(c["sel"], c["tools"], fake_runtime(c["replies"]))
                self.assertEqual(verdict, c["expect"], what)
                if verdict == "ok":
                    self.assertEqual(what, c["id"])

    def test_missing_router_names_the_fix(self):
        call = fake_runtime({("docker", "ps"): (True, "e1\tgoat-zenoh-edge-xab\t\n")})
        r = live.check_core_router("ssh://goat@192.0.2.20", "", call)
        self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-138"))
        self.assertIn("FED_ROUTER_DOCKER_HOST", r.fix)
        self.assertIn("DOCKER_HOST=ssh://goat@192.0.2.20", r.detail)

    def test_an_underived_router_host_names_where_it_was_looked_for(self):
        call = fake_runtime({("docker", "ps"): (True, "e1\tgoat-zenoh-edge-xab\t\n")})
        old = os.environ.get("FED_ROUTER_DOCKER_HOST_SOURCES")
        os.environ["FED_ROUTER_DOCKER_HOST_SOURCES"] = "peerings[].hosts.core in goatbuilder.yaml; [xab_core]"
        try:
            r = live.check_core_router("", "", call)
        finally:
            if old is None:
                os.environ.pop("FED_ROUTER_DOCKER_HOST_SOURCES", None)
            else:
                os.environ["FED_ROUTER_DOCKER_HOST_SOURCES"] = old
        self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-138"))
        self.assertIn("was not derived: goatbuilder looked in peerings[].hosts.core", r.fix)
        # With a daemon given, the hint is not added: the router is simply not there.
        os.environ["FED_ROUTER_DOCKER_HOST_SOURCES"] = "x"
        try:
            r = live.check_core_router("ssh://goat@192.0.2.20", "", call)
        finally:
            os.environ.pop("FED_ROUTER_DOCKER_HOST_SOURCES", None)
        self.assertNotIn("was not derived", r.fix)

    def test_several_routers_name_the_selector(self):
        call = fake_runtime({("docker", "ps"): (True, "a\tone-zenoh-router\t\nb\ttwo-zenoh-router\t\n"),
                             ("docker", "inspect:a"): (True, "A\ttrue\timg"),
                             ("docker", "inspect:b"): (True, "B\ttrue\timg")})
        r = live.check_core_router("ssh://goat@192.0.2.20", "", call)
        self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-139"))

    def test_an_explicit_daemon_is_docker_only(self):
        self.assertEqual(live.router_tools("ssh://goat@192.0.2.20"), ["docker"])
        self.assertEqual(live.router_tools(""), ["docker", "podman"])

    def test_the_daemon_is_the_one_phase_3_reads(self):
        # As derive.rs: an ambient DOCKER_HOST selects Docker on that daemon too.
        self.assertEqual(live.effective_docker_host("", {"DOCKER_HOST": "ssh://goat@192.0.2.21"}), "ssh://goat@192.0.2.21")
        self.assertEqual(live.effective_docker_host("ssh://goat@192.0.2.20", {"DOCKER_HOST": "x"}), "ssh://goat@192.0.2.20")
        self.assertEqual(live.effective_docker_host("", {}), "")

    def test_a_name_with_a_trailing_newline_is_not_a_name(self):
        self.assertFalse(live.container_name("core\n"))
        self.assertTrue(live.container_name("zagros-zenoh-router"))


ADMITS_XAB = ' access_control: { subjects: [ { id: "sovereign-xab", cert_common_names: ["fed.xab.zagros.01"] } ] },'
CORE_WIDE = ('{' + ADMITS_XAB +
             ' plugins: { storage_manager: { storages: { landing: { key_expr: "from/**", volume: "memory" } } } } }')
CORE_SCOPED = ('{' + ADMITS_XAB + ' plugins: { storage_manager: { storages: { landing: { key_expr: "from/**" },'
               ' xab: { key_expr: "from/xab/**" } } } } }')
CORE_NOT_ADMITTED = '{ plugins: { storage_manager: { storages: { landing: { key_expr: "from/**" } } } } }'


class IngressStores(unittest.TestCase):
    def check(self, config, ok=True, receives=True, router=("docker", "full-a")):
        call = fake_runtime({("docker", "exec"): (ok, config)})
        return live.check_ingress_stores(REPO, receives, ["from/xab/**"], "test", router, call,
                                         "/etc/zenoh/config.json5", "xab")

    def test_a_store_wider_than_the_grant_is_refused(self):
        # The 9 Nov run: a core store on from/** while the edge grants from/xab/**.
        r = self.check(CORE_WIDE)
        self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-136"))
        self.assertIn("from/xab/**", r.detail)
        self.assertIn("the wider from/**", r.detail)

    def test_a_peering_not_yet_admitted_is_not_expected_to_have_its_store(self):
        # A new peering on a role-deployed core: the role renders the store
        # only once the peering is declared and admitted, at phase 10.
        r = self.check(CORE_NOT_ADMITTED)
        self.assertEqual(r.outcome, live.NA, r)
        self.assertIn("not yet expected", r.detail)
        self.assertIn("phase 10", r.detail)

    def test_an_admitted_peering_through_apply_fragment_is_checked(self):
        frag = CORE_WIDE.replace('"sovereign-xab"', '"fed-xab"')
        self.assertEqual(self.check(frag).code, "GB-136")
        self.assertFalse(live.admitted(CORE_WIDE.replace('"sovereign-xab"', '"sovereign-xabc"'), "xab"))

    def test_a_matching_scoped_store_passes_beside_history(self):
        self.assertEqual(self.check(CORE_SCOPED).outcome, live.PASS)

    def test_an_unreadable_config_is_skipped_not_passed(self):
        r = self.check("permission denied", ok=False)
        self.assertEqual((r.outcome, r.code), (live.SKIP, "GB-137"))
        self.assertNotIn("permission denied", r.detail)

    def test_no_router_blocks(self):
        self.assertEqual(self.check(CORE_WIDE, router=None).outcome, live.BLOCKED)

    def test_send_only_is_not_applicable(self):
        self.assertEqual(self.check(CORE_WIDE, receives=False).outcome, live.NA)

    def test_required_scopes_come_from_rendered_vars_else_the_spec_rule(self):
        with tempfile.TemporaryDirectory() as t:
            self.assertEqual(live.required_ingress_scopes(t, "xab_edge", "xab", t)[0], ["from/xab/**"])
            f = Path(t) / "applied/ops/ansible/group_vars/xab_edge/federation-xab.yml"
            f.parent.mkdir(parents=True)
            f.write_text('goat_zenoh_edge_ingress_key_exprs:\n  - "from/xab/weather/**"\n')
            self.assertEqual(live.required_ingress_scopes(t, "xab_edge", "xab", t)[0], ["from/xab/weather/**"])


def openssl(*args, input_bytes=None):
    return subprocess.run(["openssl", *args], capture_output=True, input=input_bytes, check=True).stdout


@unittest.skipUnless(subprocess.run(["which", "openssl"], capture_output=True).returncode == 0, "no openssl")
class SigningCA(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.t = tempfile.TemporaryDirectory()
        d = Path(cls.t.name)
        cls.key, cls.cert, cls.other = d / "ca-key.pem", d / "ca.pem", d / "other-key.pem"
        openssl("req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
                "-keyout", str(cls.key), "-out", str(cls.cert), "-days", "2", "-subj", "/CN=zagros test CA",
                "-addext", "basicConstraints=critical,CA:TRUE")
        openssl("genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256", "-out", str(cls.other))
        cls.leaf = d / "leaf.pem"
        openssl("req", "-x509", "-key", str(cls.other), "-out", str(cls.leaf), "-days", "2", "-subj", "/CN=leaf",
                "-addext", "basicConstraints=critical,CA:FALSE")

    @classmethod
    def tearDownClass(cls):
        cls.t.cleanup()

    def test_matching_plaintext_key_passes(self):
        self.assertEqual(live.check_signing_ca(str(self.cert), str(self.key), "", "").outcome, live.PASS)

    def test_missing_key_is_refused(self):
        r = live.check_signing_ca(str(self.cert), "/nonexistent/ca-key.pem", "", "")
        self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-141"))

    def test_mismatched_key_is_refused(self):
        r = live.check_signing_ca(str(self.cert), str(self.other), "", "")
        self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-141"))
        self.assertIn("does not match", r.detail)

    def test_a_leaf_is_not_a_ca(self):
        r = live.check_signing_ca(str(self.leaf), str(self.other), "", "")
        self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-140"))

    def test_no_ca_named_is_blocked(self):
        self.assertEqual(live.check_signing_ca("", "", "", "").outcome, live.BLOCKED)
        self.assertEqual(live.check_signing_ca(str(self.cert), "", "", "").outcome, live.BLOCKED)

    def test_age_key_is_decrypted_in_memory_and_compared(self):
        # A fake `age` hands back the plaintext key; nothing is written.
        real = live.run

        def runner(argv, **kw):
            if argv[0] == "age":
                return 0, self.key.read_bytes(), b""
            return real(argv, **kw)
        with tempfile.NamedTemporaryFile() as enc, tempfile.NamedTemporaryFile() as ident:
            r = live.check_signing_ca(str(self.cert), "", enc.name, ident.name, runner)
            self.assertEqual(r.outcome, live.PASS, r)
            r = live.check_signing_ca(str(self.cert), "", enc.name, "/nonexistent/keys.txt", runner)
            self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-141"))

        def failing(argv, **kw):
            return (1, b"", b"no identity matched") if argv[0] == "age" else real(argv, **kw)
        with tempfile.NamedTemporaryFile() as enc, tempfile.NamedTemporaryFile() as ident:
            r = live.check_signing_ca(str(self.cert), "", enc.name, ident.name, failing)
            self.assertEqual((r.outcome, r.code), (live.FAIL, "GB-141"))


class WholeRun(unittest.TestCase):
    """preflight.py end to end against a fixture repository and fakes."""

    def setUp(self):
        self.t = tempfile.TemporaryDirectory()
        root = Path(self.t.name)
        self.repo, self.work, self.profile = root / "repo", root / "work", root / "profile"
        ctx = self.profile / "contexts/default"
        ctx.mkdir(parents=True)
        (ctx / "federation.toml").write_text('sovereign_id = "xaa"\n[data]\nrelease = ["release/weather/**"]\n'
                                             'accept_hz = 10\n')
        (ctx / "mtls.cert.pem").write_text("-----BEGIN CERTIFICATE-----\nx\n")
        (ctx / "ca-roots.pem").write_text("x")
        inv = self.repo / "ops/ansible/inventory"
        inv.mkdir(parents=True)
        (inv / "peering-edges.ini").write_text("[xab_edge]\nedge-1\n[xab_core]\ncore-1\n"
                                               "[peering_edges:children]\nxab_edge\n"
                                               "[peering_cores:children]\nxab_core\n")
        (self.repo / "ops/goatbuilder").mkdir(parents=True)
        (self.repo / "ops/goatbuilder/goatbuilder.yaml").write_text("peerings:\n  - with: xab\n    environment: zagros\n")
        for role, var, val in (("goat-peering-wg", "peering_wg_listen_port", 51820),
                               ("goat-peering-wg", "peering_wg_interface", "wg-fed0"),
                               ("goat-zenoh-edge", "goat_zenoh_edge_listen_port", 7448),
                               ("goat-fed-republisher", "goat_fed_republisher_metrics_port", 9310)):
            d = self.repo / "ops/ansible/roles" / role / "defaults"
            d.mkdir(parents=True, exist_ok=True)
            with open(d / "main.yml", "a") as f:
                f.write(f"{var}: {val}\n")
        filt = self.repo / "ops/ansible/filter_plugins"
        filt.mkdir(parents=True)
        (filt / "zenoh_ingress.py").symlink_to(REPO / "ops/ansible/filter_plugins/zenoh_ingress.py")
        (self.repo / "ops/goatbuilder/endstate").mkdir(parents=True)
        (self.repo / "ops/goatbuilder/endstate/edges.py").symlink_to(REPO / "ops/goatbuilder/endstate/edges.py")
        key = root / "signing.key"
        key.write_text("0" * 64)
        self.env = {"PATH": os.environ.get("PATH", ""), "HOME": str(root), "FED_SIGNING_KEY": str(key),
                    "FED_NO_DIRECTORY": "1", "FED_SKIP_LOCAL_DOCKER_CHECK": "1", "FED_CA_CERT": "",
                    "FED_ROUTER_DOCKER_HOST": "ssh://goat@192.0.2.20"}

    def tearDown(self):
        self.t.cleanup()

    def run_main(self, probe_hosts, extra_env=None, args=(), core_config=CORE_SCOPED):
        pf = load_preflight()

        def probe(argv, cwd, env):
            out = Path(argv[argv.index("-e") + 1].split("=", 1)[1])
            for name, facts in probe_hosts.items():
                (out / f"{name}.json").write_text(json.dumps(dict(facts, host=name)))
            return 0, b"", b""
        pf.PROBE = probe
        runtime = fake_runtime({("docker", "ps"): (True, "a1\tzagros-zenoh-router\t\n"),
                                ("docker", "inspect:a1"): (True, "full-a\ttrue\timg"),
                                ("docker", "exec"): (True, core_config)})
        out = io.StringIO()
        env = dict(self.env, **(extra_env or {}))
        with mock.patch.dict(os.environ, env, clear=True), \
                mock.patch.object(pf.live, "docker_caller", lambda h: runtime), \
                contextlib.redirect_stdout(out):
            rc = pf.main(["xab", "--work", str(self.work), "--profile-dir", str(self.profile),
                          "--repo", str(self.repo), *args])
        result = json.loads((self.work / "preflight/result.json").read_text())
        return rc, out.getvalue(), {r["check"]: r for r in result["results"]}

    def test_the_netbird_collision_stops_phase_one(self):
        rc, out, by = self.run_main({"edge-1": host(ss=NETBIRD_WG, wg="wt0\t51820\n"), "core-1": host(name="core-1")})
        self.assertEqual(rc, 1, out)
        self.assertEqual(by["edge-ports"]["outcome"], "FAIL")
        self.assertEqual(by["edge-ports"]["code"], "GB-131")
        self.assertEqual(by["core-ports"]["outcome"], "PASS")
        self.assertEqual(by["core-router"]["outcome"], "PASS")
        self.assertEqual(by["ingress-stores"]["outcome"], "PASS")
        self.assertIn("NOT READY", out)

    def test_missing_ca_is_listed_once_and_blocks_its_check(self):
        rc, out, by = self.run_main({"edge-1": host(), "core-1": host(name="core-1")})
        self.assertEqual(by["inputs"]["outcome"], "FAIL")
        self.assertIn("FED_CA_CERT", by["inputs"]["detail"])
        self.assertEqual(by["signing-ca"]["outcome"], "BLOCKED")
        self.assertEqual(rc, 1)

    def test_a_wide_store_stops_phase_one(self):
        rc, out, by = self.run_main({"edge-1": host(), "core-1": host(name="core-1")}, core_config=CORE_WIDE)
        self.assertEqual(by["ingress-stores"]["code"], "GB-136")

    def test_no_live_reports_skips_not_passes(self):
        rc, out, by = self.run_main({}, {"FED_PREFLIGHT_NO_LIVE": "1"})
        for cid in ("edge-ports", "core-ports", "core-router", "ingress-stores"):
            self.assertEqual(by[cid]["outcome"], "SKIP", cid)

    def test_only_slot_anchors_runs_just_that_check(self):
        f = self.repo / "ops/ansible/group_vars/zagros_hosts/zenoh-lazy-storage.yml"
        f.parent.mkdir(parents=True)
        f.write_text("zenoh_active_slot_anchors: []\n")
        calls = []

        def runner(argv, **kw):
            calls.append(argv)
            return 1, b"zagros: live slots 462 \xc2\xb7 committed 461\n", b"DRIFT\n"
        with mock.patch.object(live, "run", runner):
            rc, out, by = self.run_main({}, {"FED_SLOT_DB": "/srv/registry.db"}, ["--only", "slot-anchors"])
        self.assertEqual(list(by), ["slot-anchors"])
        self.assertEqual((rc, by["slot-anchors"]["code"]), (1, "GB-134"))
        self.assertIn("--env", calls[0])
        self.assertEqual(calls[0][calls[0].index("--env") + 1], "zagros")

    def test_an_unusable_port_value_blocks_and_never_passes(self):
        for bad in ("70000", "51820x"):
            with self.subTest(bad):
                rc, out, by = self.run_main({"edge-1": host(), "core-1": host(name="core-1")},
                                            {"FED_WG_LISTEN_PORT": bad}, ["--only", "edge-ports"])
                self.assertEqual(by["edge-ports"]["outcome"], "BLOCKED", out)
                self.assertEqual(rc, 1)

    def test_the_restart_recheck_writes_its_own_result(self):
        rc, out, _ = self.run_main({}, {"FED_PREFLIGHT_NO_LIVE": "1"})
        full = (self.work / "preflight/result.json").read_text()
        pf = load_preflight()
        with mock.patch.dict(os.environ, self.env, clear=True), contextlib.redirect_stdout(io.StringIO()):
            pf.main(["xab", "--work", str(self.work), "--profile-dir", str(self.profile), "--repo", str(self.repo),
                     "--only", "slot-anchors", "--result", str(self.work / "preflight/slot-recheck.json")])
        self.assertEqual((self.work / "preflight/result.json").read_text(), full)
        recheck = json.loads((self.work / "preflight/slot-recheck.json").read_text())
        self.assertEqual([r["check"] for r in recheck["results"]], ["slot-anchors"])

    def test_strict_refuses_on_a_skip(self):
        self.env.update({"FED_CA_CERT": "/x", "FED_CA_KEY": "/y"})
        rc, out, by = self.run_main({}, {"FED_PREFLIGHT_NO_LIVE": "1", "FED_PREFLIGHT_STRICT": "1"},
                                    ["--only", "core-router"])
        self.assertEqual(by["core-router"]["outcome"], "SKIP")
        self.assertEqual(rc, 1)
        rc, out, _ = self.run_main({}, {"FED_PREFLIGHT_NO_LIVE": "1"}, ["--only", "core-router"])
        self.assertEqual(rc, 0)
        self.assertIn("SKIPPED and not verified", out)


class Sides(unittest.TestCase):
    """Phase 1 by side (docs/design/federation-two-drivers.md, gate row B10): the
    edge driver reaches the edge and not the core, and holds no CA key; the
    controller reaches the core and not the edge. Each runs the checks of its
    side, and asks only for the inputs those checks need."""

    setUp = WholeRun.setUp
    tearDown = WholeRun.tearDown

    def run_side(self, side, probe_hosts, extra_env=None):
        seen = {"limits": [], "docker": 0}
        pf = load_preflight()

        def probe(argv, cwd, env):
            seen["limits"].append(argv[argv.index("--limit") + 1] if "--limit" in argv else "")
            out = Path(argv[argv.index("-e") + 1].split("=", 1)[1])
            for name, facts in probe_hosts.items():
                (out / f"{name}.json").write_text(json.dumps(dict(facts, host=name)))
            return 0, b"", b""
        pf.PROBE = probe
        runtime = fake_runtime({("docker", "ps"): (True, "a1\tzagros-zenoh-router\t\n"),
                                ("docker", "inspect:a1"): (True, "full-a\ttrue\timg"),
                                ("docker", "exec"): (True, CORE_SCOPED)})

        def caller(h):
            seen["docker"] += 1
            return runtime
        out = io.StringIO()
        with mock.patch.dict(os.environ, dict(self.env, **(extra_env or {})), clear=True), \
                mock.patch.object(pf.live, "docker_caller", caller), \
                contextlib.redirect_stdout(out):
            rc = pf.main(["xab", "--work", str(self.work), "--profile-dir", str(self.profile),
                          "--repo", str(self.repo), "--side", side])
        result = json.loads((self.work / "preflight/result.json").read_text())
        live_ids = [r["check"] for r in result["results"] if r["check"]]
        return rc, out.getvalue(), live_ids, {r["check"]: r for r in result["results"]}, seen, result

    def test_the_edge_side_checks_the_edge_and_asks_nothing_of_the_core(self):
        # No CA, no core certificate names, no signing key: the edge driver holds none of them.
        env = {"FED_CA_CERT": "", "FED_SIGNING_KEY": "", "FED_CORE_SAN": "", "FED_ROUTER_DOCKER_HOST": ""}
        rc, out, ids, by, seen, result = self.run_side("edge", {"edge-1": host()}, env)
        self.assertEqual(ids, ["inputs", "edge-ports"], out)
        self.assertEqual(by["inputs"]["outcome"], "PASS", out)
        self.assertEqual(by["edge-ports"]["outcome"], "PASS", out)
        self.assertEqual(seen["limits"], ["xab_edge"], "the probe reaches only the edge")
        self.assertEqual(seen["docker"], 0, "nothing asks the core's container runtime")
        self.assertEqual(result["side"], "edge")
        self.assertEqual(rc, 0, out)

    def test_the_core_side_checks_the_core_and_asks_nothing_of_the_edge(self):
        # An edge input that would block the edge's ports check is not the core's to need.
        env = {"FED_WG_LISTEN_PORT": "70000", "FED_CORE_SAN": "core-1"}
        rc, out, ids, by, seen, result = self.run_side("core", {"core-1": host(name="core-1")}, env)
        self.assertEqual(ids, ["inputs", "core-ports", "slot-anchors", "core-router", "ingress-stores", "signing-ca"], out)
        self.assertNotIn("FED_WG_LISTEN_PORT", by["inputs"]["detail"], out)
        self.assertEqual(by["core-ports"]["outcome"], "PASS", out)
        self.assertEqual(by["core-router"]["outcome"], "PASS", out)
        self.assertEqual(seen["limits"], ["xab_core"], "the probe reaches only the core")
        self.assertEqual(result["side"], "core")

    def test_a_check_of_the_other_side_is_refused(self):
        pf = load_preflight()
        err = io.StringIO()
        with mock.patch.dict(os.environ, self.env, clear=True), contextlib.redirect_stderr(err), \
                contextlib.redirect_stdout(io.StringIO()):
            rc = pf.main(["xab", "--work", str(self.work), "--profile-dir", str(self.profile),
                          "--repo", str(self.repo), "--side", "edge", "--only", "signing-ca"])
        self.assertEqual(rc, 2)
        self.assertIn("signing-ca", err.getvalue())
        self.assertIn("core", err.getvalue())

    def test_every_check_has_a_side(self):
        self.assertEqual({c for c, _, _ in live.CHECKS}, set(live.SIDES))
        self.assertTrue(set(live.SIDES.values()) <= {"edge", "core", "both"})


class CheckList(unittest.TestCase):
    def test_goatbuilder_lists_the_same_checks(self):
        rs = (REPO / "crates/goatbuilder/src/federate/preflight.rs").read_text()
        for cid, _, code in live.CHECKS:
            self.assertIn(f'"{cid}",', rs, cid)
            self.assertIn(f'"{code}"', rs, code)

    def test_every_code_is_in_the_catalog(self):
        cat = (REPO / "ops/errors/catalog.yaml").read_text()
        src = (HERE / "preflight_live.py").read_text() + (HERE / "preflight.py").read_text()
        import re
        for code in sorted(set(re.findall(r'"(GB-1[34]\d)"', src))):
            self.assertIn(f"code: {code}", cat, code)


if __name__ == "__main__":
    unittest.main()
