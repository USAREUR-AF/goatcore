# SPDX-License-Identifier: Apache-2.0
"""Encryption stage 2, IPsec: the role's declaration reader, its templates, and the sender's side.

The declaration reader is the role's filter plugin (filter_plugins/simple_bn_ipsec.py); the templates
are rendered with plain Jinja2 set up as Ansible sets it (trim_blocks), with stand-ins for the two
Ansible-only names they use (the bool filter and lookup). Addresses are documentation ranges.
Needs jinja2 and PyYAML for the template tests; the reader and sender tests are standard library.
"""
import importlib.util
import os
import re
import socket
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
ROLE = os.path.join(REPO, "ops", "ansible", "roles", "simple-bn")
sys.path.insert(0, os.path.dirname(HERE))
import simple_bn_sender as sender  # noqa: E402

_spec = importlib.util.spec_from_file_location("simple_bn_ipsec", os.path.join(ROLE, "filter_plugins", "simple_bn_ipsec.py"))
ipsec = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ipsec)

try:
    import jinja2
    import yaml
except ImportError:   # CI installs both (boundary-and-door.yml)
    jinja2 = yaml = None

PSK = "correct-horse-battery-staple-0123"
ENV = {"gateway": "198.51.100.7", "our_public": "203.0.113.10", "psk": PSK, "remote_ts": "", "dest": "192.0.2.50:30001"}


def declaration(stage=2, **ipsec_fields):
    d = {"node": "bn-a", "receiver": {"label": "lab", "dest_env": "SBN_DEST", "our_source_env": "SBN_OURS"},
         "feeds": ["radar-a"], "encryption": {"stage": stage}}
    if ipsec_fields:
        d["encryption"]["ipsec"] = dict(ipsec_fields)
    return d


PSK_DECL = dict(gateway_env="SBN_GW", auth="psk", psk_env="SBN_PSK")
PUBKEY_DECL = dict(gateway_env="SBN_GW", auth="pubkey", remote_id="gw.example.net",
                   pubkey={"cert": "bn-a.pem", "key": "bn-a.key.pem", "remote_cacert": "lab-ca.pem"})

# The receiver's gateway dials us from behind NAT: no gateway address, its identity required.
RESPONDER_DECL = dict(initiator=False, auth="psk", psk_env="SBN_PSK", remote_id="gw-lab.example.net",
                      remote_ts_env="SBN_TS")
RESPONDER_ENV = dict(ENV, gateway="", remote_ts="192.0.2.64/28")

# Route mode, as a responder: a route-based gateway behind NAT dials us; a /30 tunnel subnet,
# their tunnel interface .1, ours .2 (documentation addresses).
ROUTE_DECL = dict(RESPONDER_DECL, mode="route", tunnel_cidr="198.51.100.0/30", tunnel_local="198.51.100.2",
                  tunnel_remote_env="SBN_TUNNEL_REMOTE")
ROUTE_DECL.pop("remote_ts_env")
ROUTE_ENV = dict(ENV, gateway="", tunnel_remote="198.51.100.1")


class Reader(unittest.TestCase):
    def test_stage_0_1_and_2_without_ipsec_want_nothing(self):
        for stage in (0, 1, 2):
            out = ipsec.simple_bn_ipsec(declaration(stage), ENV)
            self.assertEqual(out, {"wanted": False, "errors": [], "conf": {}}, stage)
        self.assertFalse(ipsec.simple_bn_ipsec({}, {})["wanted"])

    def test_psk_complete(self):
        out = ipsec.simple_bn_ipsec(declaration(**PSK_DECL), ENV)
        self.assertEqual(out["errors"], [])
        c = out["conf"]
        self.assertEqual(c["name"], "simple-bn-lab")
        self.assertEqual(c["local_ts"], "203.0.113.10/32")
        self.assertEqual(c["remote_ts"], "192.0.2.50/32")       # the receiver address (dest_env) by default
        self.assertEqual((c["local_id"], c["remote_id"]), ("203.0.113.10", "198.51.100.7"))
        self.assertEqual((c["ike_proposals"], c["esp_proposals"]), ("aes256gcm16-prfsha384-ecp384", "aes256gcm16-ecp384"))
        self.assertEqual((c["ike_rekey_time"], c["child_rekey_time"]), ("", ""))

    def test_remote_ts_from_its_own_variable(self):
        out = ipsec.simple_bn_ipsec(declaration(remote_ts_env="SBN_TS", **PSK_DECL),
                                    dict(ENV, remote_ts="192.0.2.50, 192.0.2.64/28", dest="rx.example.net:30001"))
        self.assertEqual(out["errors"], [])
        self.assertEqual(out["conf"]["remote_ts"], "192.0.2.50/32,192.0.2.64/28")

    def test_missing_values_are_refused_by_name_never_by_value(self):
        cases = {
            "gateway_env": (dict(auth="psk", psk_env="SBN_PSK"), ENV),
            "SBN_GW": (PSK_DECL, dict(ENV, gateway="")),
            "SBN_PSK": (PSK_DECL, dict(ENV, psk="")),
            "shorter than": (PSK_DECL, dict(ENV, psk="q7Zk2")),
            "SBN_OURS": (PSK_DECL, dict(ENV, our_public="")),
            "remote_ts_env is required": (PSK_DECL, dict(ENV, dest="rx.example.net:30001")),
            "auth must be": (dict(gateway_env="SBN_GW"), ENV),
            "pubkey.key": (dict(PUBKEY_DECL, pubkey={"cert": "a.pem", "remote_cacert": "ca.pem"}), ENV),
            "exactly one of": (dict(PUBKEY_DECL, pubkey={"cert": "a.pem", "key": "k.pem"}), ENV),
            "plain file name": (dict(PUBKEY_DECL, pubkey={"cert": "../a.pem", "key": "k.pem", "remote_cert": "r.pem"}), ENV),
            "proposal list": (dict(PSK_DECL, ike_proposals="aes256 gcm16"), ENV),
            "is not a time": (dict(PSK_DECL, dpd_delay="soon"), ENV),
            "inside the remote traffic selector": (dict(PSK_DECL, remote_ts_env="SBN_TS"), dict(ENV, remote_ts="198.51.100.0/24")),
            "character swanctl cannot take": (dict(PSK_DECL, remote_id='gw"x'), ENV),
        }
        for want, (decl, env) in cases.items():
            out = ipsec.simple_bn_ipsec(declaration(**decl), env)
            self.assertTrue(out["wanted"], want)
            joined = " ".join(out["errors"])
            self.assertIn(want, joined, want)
            for value in (PSK, "q7Zk2", "198.51.100.7", "203.0.113.10", "192.0.2.50"):
                self.assertNotIn(value, joined, (want, value))

    def test_responder(self):
        for decl in (RESPONDER_DECL, dict({k: v for k, v in RESPONDER_DECL.items() if k != "initiator"}, role="responder")):
            out = ipsec.simple_bn_ipsec(declaration(**decl), RESPONDER_ENV)
            self.assertEqual(out["errors"], [])
            c = out["conf"]
            self.assertFalse(c["initiator"])
            self.assertEqual(c["remote_addrs"], "%any")
            self.assertEqual(c["remote_id"], "gw-lab.example.net")
            self.assertEqual(c["remote_ts"], "192.0.2.64/28")
            self.assertEqual((c["ike_rekey_time"], c["child_rekey_time"], c["child_life_time"]), ("0s", "0s", "75m"))

    def test_responder_needs_the_initiators_identity(self):
        no_id = {k: v for k, v in RESPONDER_DECL.items() if k != "remote_id"}
        out = ipsec.simple_bn_ipsec(declaration(**no_id), RESPONDER_ENV)
        self.assertIn("remote_id is required for a responder", " ".join(out["errors"]))
        out = ipsec.simple_bn_ipsec(declaration(**dict(RESPONDER_DECL, remote_id="%any")), RESPONDER_ENV)
        self.assertIn("% wildcard", " ".join(out["errors"]))
        out = ipsec.simple_bn_ipsec(declaration(**dict(RESPONDER_DECL, role="initiator")), RESPONDER_ENV)
        self.assertIn("agree with initiator", " ".join(out["errors"]))
        # an initiator still needs the gateway
        out = ipsec.simple_bn_ipsec(declaration(**dict(PSK_DECL, gateway_env=None)), ENV)
        self.assertIn("gateway_env is required", " ".join(out["errors"]))

    def test_responder_with_a_known_gateway_names_it(self):
        out = ipsec.simple_bn_ipsec(declaration(**dict(RESPONDER_DECL, gateway_env="SBN_GW")), dict(RESPONDER_ENV, gateway="198.51.100.7"))
        self.assertEqual(out["errors"], [])
        self.assertEqual(out["conf"]["remote_addrs"], "198.51.100.7")

    def test_route_mode(self):
        out = ipsec.simple_bn_ipsec(declaration(**ROUTE_DECL), ROUTE_ENV)
        self.assertEqual(out["errors"], [])
        c = out["conf"]
        self.assertEqual((c["mode"], c["interface"], c["if_id"]), ("route", "sbn-xfrm", 42))
        self.assertEqual((c["local_ts"], c["remote_ts"]), ("0.0.0.0/0", "0.0.0.0/0"))
        self.assertEqual((c["tunnel_local"], c["tunnel_remote"], c["src_addr"]), ("198.51.100.2/30", "198.51.100.1", "198.51.100.2"))
        self.assertEqual(c["routes"], ["192.0.2.50/32"])                # the receiver address (dest_env), into the tunnel
        self.assertEqual(c["remote_addrs"], "%any")
        over = ipsec.simple_bn_ipsec(declaration(**dict(ROUTE_DECL, local_ts="198.51.100.2", remote_ts=["192.0.2.0/24"])), ROUTE_ENV)
        self.assertEqual((over["conf"]["local_ts"], over["conf"]["remote_ts"]), ("198.51.100.2/32", "192.0.2.0/24"))

    def test_route_mode_needs_its_tunnel_addresses(self):
        for key, want in (("tunnel_cidr", "tunnel_cidr (or tunnel_cidr_env) is required"),
                          ("tunnel_local", "tunnel_local (or tunnel_local_env) is required"),
                          ("tunnel_remote_env", "tunnel_remote (or tunnel_remote_env) is required")):
            decl = {k: v for k, v in ROUTE_DECL.items() if k != key}
            self.assertIn(want, " ".join(ipsec.simple_bn_ipsec(declaration(**decl), ROUTE_ENV)["errors"]), key)
        cases = {
            "not set": ({}, dict(ROUTE_ENV, tunnel_remote="")),
            "not a host address inside tunnel_cidr": (dict(tunnel_local="198.51.100.3"), ROUTE_ENV),
            "not an IPv4 network": (dict(tunnel_cidr="198.51.100.1/30"), ROUTE_ENV),
            "are the same address": (dict(tunnel_local="198.51.100.1"), ROUTE_ENV),
            "mode must be policy or route": (dict(mode="tunnel"), ROUTE_ENV),
            "IPv4 HOST:PORT": ({}, dict(ROUTE_ENV, dest="rx.example.net:30001")),
        }
        for want, (extra, env) in cases.items():
            errs = " ".join(ipsec.simple_bn_ipsec(declaration(**dict(ROUTE_DECL, **extra)), env)["errors"])
            if want == "not set":
                self.assertIn("SBN_TUNNEL_REMOTE (encryption.ipsec.tunnel_remote_env) is not set", errs)
            else:
                self.assertIn(want, errs, want)

    def test_ipsec_with_another_stage_is_refused(self):
        out = ipsec.simple_bn_ipsec(declaration(stage=1, **PSK_DECL), ENV)
        self.assertIn("encryption.stage is not 2", " ".join(out["errors"]))


@unittest.skipUnless(jinja2, "jinja2 and PyYAML are not installed")
class Templates(unittest.TestCase):
    def render(self, name, **ctx):
        env = jinja2.Environment(loader=jinja2.FileSystemLoader(os.path.join(ROLE, "templates")),
                                 undefined=jinja2.StrictUndefined, trim_blocks=True, keep_trailing_newline=True,
                                 # config files, not HTML: .j2 is outside the autoescape set, as in Ansible
                                 autoescape=jinja2.select_autoescape(["html", "xml"]))
        env.filters["bool"] = lambda v: v if isinstance(v, bool) else str(v).lower() in ("1", "true", "yes", "on")
        with open(os.path.join(ROLE, "defaults", "main.yml")) as f:
            defaults = yaml.safe_load(f)
        defaults["simple_bn_etc"] = "/etc/simple-bn"
        return env.get_template(name).render(dict(defaults, lookup=lambda *_: "", **ctx))

    def swanctl(self, decl, env=ENV):
        out = ipsec.simple_bn_ipsec(declaration(**decl), env)
        self.assertEqual(out["errors"], [])
        text = self.render("swanctl-simple-bn.conf.j2", _sbn_ipsec=out)
        self.assertEqual(text.count("{"), text.count("}"), text)
        return text

    def test_psk_connection(self):
        t = self.swanctl(PSK_DECL)
        for line in ("version = 2", "local_addrs = %any", "remote_addrs = 198.51.100.7",
                     "proposals = aes256gcm16-prfsha384-ecp384", "esp_proposals = aes256gcm16-ecp384",
                     "local_ts = 203.0.113.10/32", "remote_ts = 192.0.2.50/32", 'id = "203.0.113.10"',
                     'id = "198.51.100.7"', "auth = psk", "start_action = start", "dpd_action = restart",
                     "close_action = start", "dpd_delay = 30s", "ike-simple-bn-lab {"):
            self.assertIn(line, t)
        # The key is in the file only base64-encoded (swanctl's 0s prefix), never as typed.
        self.assertNotIn(PSK, t)
        self.assertRegex(t, r"secret = 0s[A-Za-z0-9+/=]+\n")
        self.assertNotIn("rekey_time", t)
        self.assertNotIn("certs", t)

    def test_pubkey_connection_and_timers(self):
        t = self.swanctl(dict(PUBKEY_DECL, ike_rekey_time="4h", child_rekey_time="1h"))
        self.assertIn("auth = pubkey", t)
        self.assertIn("certs = bn-a.pem", t)
        self.assertIn("cacerts = lab-ca.pem", t)
        self.assertIn('id = "gw.example.net"', t)
        self.assertEqual(len(re.findall(r"rekey_time = ", t)), 2)
        self.assertNotIn("secrets", t)

    def test_responder_connection(self):
        out = ipsec.simple_bn_ipsec(declaration(**RESPONDER_DECL), RESPONDER_ENV)
        self.assertEqual(out["errors"], [])
        t = self.render("swanctl-simple-bn.conf.j2", _sbn_ipsec=out)
        self.assertEqual(t.count("{"), t.count("}"), t)
        for line in ("remote_addrs = %any", 'id = "gw-lab.example.net"', "unique = replace", "start_action = none",
                     "dpd_action = clear", "close_action = none", "life_time = 75m", "remote_ts = 192.0.2.64/28",
                     'id-remote = "gw-lab.example.net"'):
            self.assertIn(line, t)
        self.assertEqual(len(re.findall(r"^\s+rekey_time = 0s$", t, re.M)), 2)
        for line in ("start_action = start", "dpd_action = restart", "close_action = start"):
            self.assertNotIn(line, t)
        self.assertNotIn(PSK, t)

    def test_route_connection_and_interface(self):
        out = ipsec.simple_bn_ipsec(declaration(**ROUTE_DECL), ROUTE_ENV)
        t = self.render("swanctl-simple-bn.conf.j2", _sbn_ipsec=out)
        self.assertEqual(t.count("{"), t.count("}"), t)
        for line in ("if_id_in = 42", "if_id_out = 42", "local_ts = 0.0.0.0/0", "remote_ts = 0.0.0.0/0",
                     "remote_addrs = %any", "start_action = none", 'id = "gw-lab.example.net"'):
            self.assertIn(line, t)
        nd = self.render("sbn-ipsec.netdev.j2", _sbn_ipsec=out)
        for line in ("Name=sbn-xfrm", "Kind=xfrm", "InterfaceId=42", "Independent=yes"):
            self.assertIn(line, nd)
        nw = self.render("sbn-xfrm.network.j2", _sbn_ipsec=out)
        self.assertIn("Address=198.51.100.2/30", nw)
        self.assertEqual(nw.count("Destination=192.0.2.50/32\nGateway=198.51.100.1"), 2)   # main table and table 4101
        self.assertIn("Type=unreachable\nDestination=0.0.0.0/0\nTable=4101", nw)
        self.assertIn("Destination=198.51.100.0/30\nScope=link\nTable=4101", nw)
        self.assertIn("[RoutingPolicyRule]\nFrom=198.51.100.2/32\nTable=4101", nw)
        env = self.receiver_env(declaration(**ROUTE_DECL), out)
        self.assertIn("SIMPLE_BN_SRC_ADDR=198.51.100.2\n", env)
        self.assertIn("SIMPLE_BN_ENCRYPTION=ipsec\n", env)

    def test_receiver_on_the_tunnel_subnet_is_reachable_from_the_tunnel_address(self):
        # Sending to the far tunnel address (the gateway forwards to the receiver): no host route, but the
        # tunnel subnet is in the source table, so the send does not hit the unreachable default.
        out = ipsec.simple_bn_ipsec(declaration(**ROUTE_DECL), dict(ROUTE_ENV, dest="198.51.100.1:12415"))
        self.assertEqual(out["errors"], [])
        nw = self.render("sbn-xfrm.network.j2", _sbn_ipsec=out)
        self.assertIn("Destination=198.51.100.0/30\nScope=link\nTable=4101", nw)
        self.assertNotIn("Gateway=", nw)

    def test_policy_mode_netdev_is_a_dummy(self):
        out = ipsec.simple_bn_ipsec(declaration(**PSK_DECL), ENV)
        nd = self.render("sbn-ipsec.netdev.j2", _sbn_ipsec=out)
        self.assertIn("Name=sbn-ipsec\nKind=dummy", nd)
        self.assertNotIn("if_id", self.render("swanctl-simple-bn.conf.j2", _sbn_ipsec=out))

    def test_initiator_keeps_its_actions(self):
        t = self.swanctl(PSK_DECL)
        self.assertNotIn("unique", t)
        self.assertNotIn("life_time", t)

    def test_interface_network_file(self):
        out = ipsec.simple_bn_ipsec(declaration(remote_ts_env="SBN_TS", **PSK_DECL), dict(ENV, remote_ts="192.0.2.50,192.0.2.64/28"))
        t = self.render("sbn-ipsec.network.j2", _sbn_ipsec=out)
        self.assertIn("Name=sbn-ipsec", t)
        self.assertIn("Address=203.0.113.10/32", t)
        self.assertEqual(re.findall(r"Destination=(\S+)", t), ["192.0.2.50/32", "192.0.2.64/28"])
        self.assertEqual(t.count("Type=unreachable"), 2)

    def receiver_env(self, decl, sbn_ipsec):
        return self.render("receiver.env.j2", simple_bn=decl, _sbn_ipsec=sbn_ipsec, _simple_bn_dest="192.0.2.50:30001",
                           _simple_bn_relay=False, _simple_bn_intake={"stdout": ""}, _simple_bn_relay_from="")

    def test_receiver_env_stage_0_and_1_unchanged(self):
        for stage, enc in ((0, "none"), (1, "wireguard"), (2, "receiver-transport")):
            decl = declaration(stage)
            if stage == 1:
                decl["wireguard"] = {"interface": "sbn-lab"}
            t = self.receiver_env(decl, ipsec.simple_bn_ipsec(decl, ENV))
            self.assertIn(f"SIMPLE_BN_ENCRYPTION={enc}\n", t)
            self.assertIn("SIMPLE_BN_SRC_ADDR=none\n", t)
            self.assertIn("SIMPLE_BN_WG_INTERFACE=" + ("sbn-lab" if stage == 1 else "none") + "\n", t)

    def test_receiver_env_ipsec(self):
        decl = declaration(**PSK_DECL)
        t = self.receiver_env(decl, ipsec.simple_bn_ipsec(decl, ENV))
        self.assertIn("SIMPLE_BN_ENCRYPTION=ipsec\n", t)
        self.assertIn("SIMPLE_BN_SRC_ADDR=203.0.113.10\n", t)
        self.assertNotIn(PSK, t)


class Sender(unittest.TestCase):
    def tearDown(self):
        sender.SRC_ADDR = None

    def test_ipsec_is_an_encryption_that_carries_real_data(self):
        self.assertIsNone(sender.guard("relay", "ipsec"))
        self.assertIsNotNone(sender.guard("relay", "none"))

    def test_src_addr_overrides_the_route_lookup(self):
        sender.SRC_ADDR = "127.0.0.1"
        self.assertEqual(sender.local_addr_for(("192.0.2.1", 9)), "127.0.0.1")

    def test_a_send_while_the_tunnel_is_down_fails_closed_then_recovers(self):
        rx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        rx.bind(("127.0.0.1", 0))
        rx.settimeout(2)
        self.addCleanup(rx.close)
        real, calls = sender.udp_connect, []

        def down_once(sock, dest):
            calls.append(dest)
            return "[Errno 113] No route to host" if len(calls) <= 2 else real(sock, dest)

        sender.udp_connect = down_once
        self.addCleanup(setattr, sender, "udp_connect", real)
        profiles = {"radar-a": dict(sender.PROFILES[next(iter(sender.PROFILES))], sic=1)}
        feed = sender.Feed("radar-a", 0, rx.getsockname(), __import__("random").Random(1), profiles)
        self.addCleanup(feed.sock.close)
        feed.send(b"\x22\x00\x04\x00")          # connect fails again: counted, nothing sent
        self.assertEqual(feed.state, "send-error")
        self.assertTrue(feed.last_error.startswith("connect:"))
        self.assertEqual(feed.total["send_errors"], 1)
        feed.send(b"\x22\x00\x04\x00")          # the tunnel is up: connected and sent
        self.assertEqual(feed.state, "sending")
        self.assertEqual(rx.recv(64), b"\x22\x00\x04\x00")


if __name__ == "__main__":
    unittest.main()
