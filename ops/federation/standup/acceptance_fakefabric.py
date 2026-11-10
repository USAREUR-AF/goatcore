"""A two-sovereign fabric in one process, for testing acceptance.sh without hosts.

It stands in for what phase 14 talks to, over the same HTTP shapes:

  * two cores' zenoh REST surfaces (GET a key expression, PUT a value), each a store,
    with the two admin-space answers the persistence check reads (`@/*/router` and
    `@/*/router/subscriber/<key>`, in zenoh 1.9's shapes): the core's own store
    subscribes to `from/<other side>/**`, and a test may add a diagnostic session;
  * each side's republisher: a PUT under `release/<set>/...` on one core, when the
    agreement offers `<set>` from that side, lands on the other core as
    `from/<side>/<set>/...` (the link and both edges are folded into this step);
  * the round-trip responder on each core (ops/federation/roundtrip/responder.py);
  * three probe sessions across the link on THEIR side (agreed, forged, impersonate),
    each a REST surface whose PUTs our edge judges and records in an ACL trace file,
    in the line shapes zenoh 1.9 prints;
  * a `spire-server` stand-in for each side, backed by CAs made with openssl.

Every behavior can be broken on purpose (see `Faults`), so a test proves the script
fails when the fabric does, not only that it passes when the fabric works.

This is test code. It models the boundary the script checks, not zenoh.
"""
import base64
import fnmatch
import hashlib
import http.server
import json
import os
import re
import subprocess
import threading
import urllib.parse


def _match(expr, key):
    """Zenoh key-expression matching, enough for what the script asks: `*` within
    one chunk, `**` any number of chunks, including none."""
    e, k = expr.split("/"), key.split("/")

    def m(i, j):
        if i == len(e):
            return j == len(k)
        if e[i] == "**":
            return any(m(i + 1, n) for n in range(j, len(k) + 1))
        return j < len(k) and fnmatch.fnmatchcase(k[j], e[i]) and m(i + 1, j + 1)

    return m(0, 0)


class Faults:
    """Switches that break one part of the fabric."""

    def __init__(self):
        self.republisher_down = set()      # sides whose republisher lifts nothing
        self.lift_everything = set()       # sides whose republisher lifts unoffered sets
        self.edge_admits_ops = False       # our edge lets their ops/** writes through
        self.responder_down = set()        # sides with no round-trip responder
        self.mangle_inbound = False        # inbound samples arrive with other bytes
        self.trace_silent = False          # our edge logs no ACL decisions
        self.authorize_forged = False      # our edge authorizes the forged name
        self.stale = {}                    # side -> {key: value} present on a core before the run
        self.broken_get = set()            # sides whose REST surface answers every GET with HTTP 500
        self.answers_everything = {}       # side -> one value its REST surface returns for every GET
        self.no_storage = set()            # sides whose core has no store for the other side's wire name
        self.storage_interest_refused = set()  # sides whose store's subscription the edge refuses: a
        #                                    sample arrives there only beside a diagnostic subscriber
        self.diagnostic = {}               # side -> key expression a client session subscribes to there
        self.admin_closed = set()          # sides whose admin space answers nothing


class Core:
    def __init__(self, name):
        self.name = name
        self.zid = hashlib.sha256(name.encode()).hexdigest()[:32]
        self.store = {}
        self.lock = threading.Lock()

    def get(self, expr):
        with self.lock:
            return [(k, v) for k, v in sorted(self.store.items()) if _match(expr, k)]


class Fabric:
    def __init__(self, agreement, us, tmp):
        self.ag = agreement
        self.us = us
        self.them = [p["sovereign_id"] for p in agreement["parties"] if p["sovereign_id"] != us][0]
        self.cores = {us: Core(us), self.them: Core(self.them)}
        self.faults = Faults()
        self.tmp = tmp
        self.trace_path = os.path.join(tmp, "edge-trace.log")
        open(self.trace_path, "w").close()
        self.servers = []
        self.zids = {"forged": "f" + "1" * 31, "impersonate": "f" + "2" * 31}
        self.diag_zid = "d" + "3" * 31

    # --- the agreement -------------------------------------------------------
    def offered(self, side):
        out = []
        for f in self.ag.get("flows", []):
            if f.get("from") == side:
                for r in f.get("release", []):
                    out.append(r[:-3] if r.endswith("/**") else r)
        return out

    def other(self, side):
        return self.them if side == self.us else self.us

    # --- what a PUT on a core sets in motion -----------------------------------
    def put(self, side, key, value):
        core = self.cores[side]
        with core.lock:
            core.store[key] = value
        if key.startswith("release/") and side not in self.faults.republisher_down:
            stem = None
            for s in self.offered(side):
                if key.startswith(s + "/"):
                    stem = s
            if stem is None and side in self.faults.lift_everything:
                stem = "/".join(key.split("/")[:2])
            if stem is not None:
                wire = "from/%s/%s" % (side, key[len("release/"):])
                if side == self.them and self.faults.mangle_inbound:
                    value = value + b"-mangled"
                self.deliver(self.other(side), wire, value)
        # Their core writing outside any wire name: crosses only if our edge is broken.
        if side == self.them and key.startswith("ops/") and self.faults.edge_admits_ops:
            self.deliver(self.us, key, value)

    # --- who subscribes on a core ---------------------------------------------
    def paused(self, side):
        return os.path.exists(os.path.join(self.tmp, "paused-" + side))

    def subscribers(self, side):
        """(key expression, client ids) for every subscriber on `side`'s core."""
        core = self.cores[side]
        subs = []
        if side not in self.faults.no_storage:
            subs.append(("from/%s/**" % self.other(side), [core.zid]))
        if side in self.faults.diagnostic and not self.paused(side):
            subs.append((self.faults.diagnostic[side], [self.diag_zid]))
        return subs

    def admin(self, side, expr):
        core = self.cores[side]
        if side in self.faults.admin_closed:
            return []
        if expr == "@/*/router":
            return [{"key": "@/%s/router" % core.zid, "value": {"zid": core.zid}, "encoding": "application/json"}]
        prefix = "@/*/router/subscriber/"
        if expr.startswith(prefix):
            key = expr[len(prefix):]
            return [{"key": "@/%s/router/subscriber/%s" % (core.zid, ke),
                     "value": {"clients": clients, "peers": [], "routers": [core.zid]}, "encoding": "application/json"}
                    for ke, clients in self.subscribers(side) if _match(ke, key)]
        return []

    def deliver(self, side, key, value):
        # The store keeps a sample when its own subscription carries it, or when the
        # edge refuses that subscription but a diagnostic session's interest pulls the
        # sample across anyway (the defect the persistence check exists for).
        store = side not in self.faults.no_storage
        interest = side not in self.faults.storage_interest_refused or any(
            _match(ke, key) for ke, clients in self.subscribers(side) if clients != [self.cores[side].zid])
        if store and interest:
            core = self.cores[side]
            with core.lock:
                core.store[key] = value
        self.respond(side, key, value)

    def respond(self, side, key, value):
        """The round-trip responder on `side`: a probe from the other side under its
        wire name is acknowledged under one of `side`'s offered sets."""
        if side in self.faults.responder_down:
            return
        sender = self.other(side)
        m = re.fullmatch(r"from/%s/(?:.+/)?fedcheck/([^/]+)" % re.escape(sender), key)
        if not m or not self.offered(side):
            return
        ack = json.dumps({"sha256": hashlib.sha256(value).hexdigest(), "observed_key": key}).encode()
        self.put(side, "%s/fedcheck-ack/%s" % (self.offered(side)[0], m.group(1)), ack)

    # --- the probe sessions on their side, judged by our edge -----------------
    def probe_put(self, which, key, value):
        lines = []
        if which == "agreed":
            zid, subject = "aaaa", "peering-%s" % self.them
            ok = key.startswith("from/%s/" % self.them) and bool(self.offered(self.them))
            lines.append("%s on %s is %s to Put (ingress) on %s" % (zid, subject, "authorized" if ok else "unauthorized", key))
            if ok:
                self.deliver(self.us, key, value)
        else:
            zid = self.zids[which]
            if which == "forged" and self.faults.authorize_forged:
                lines.append("%s on forged is authorized to Put (ingress) on %s" % (zid, key))
                self.deliver(self.us, key, value)
            else:
                lines.append("ACL: peer %s did not match any configured ACL subject" % zid)
        if not self.faults.trace_silent:
            with open(self.trace_path, "a") as f:
                for line in lines:
                    f.write("2026-11-09T00:00:00Z INFO zenoh::net::routing::interceptor::access_control: %s\n" % line)

    # --- serving ---------------------------------------------------------------
    def _handler(self, on_get, on_put, side=None):
        faults = self.faults

        class H(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                if side in faults.broken_get:
                    self.send_response(500)
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                rows = on_get(self.path.lstrip("/"))
                if side in faults.answers_everything:
                    rows = [{"key": self.path.lstrip("/"), "encoding": "zenoh/bytes",
                             "value": base64.b64encode(faults.answers_everything[side]).decode()}]
                body = json.dumps(rows, separators=(",", ":")).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_PUT(self):
                n = int(self.headers.get("Content-Length") or 0)
                on_put(self.path.lstrip("/"), self.rfile.read(n))
                self.send_response(200)
                self.send_header("Content-Length", "0")
                self.end_headers()

        return H

    def _serve(self, handler):
        srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        self.servers.append(srv)
        return "http://127.0.0.1:%d" % srv.server_address[1]

    def _rows(self, side):
        def get(expr):
            expr = urllib.parse.unquote(expr)
            if expr.startswith("@/"):
                return self.admin(side, expr)
            return [{"key": k, "value": base64.b64encode(v).decode(), "encoding": "zenoh/bytes"}
                    for k, v in self.cores[side].get(expr)]
        return get

    def start(self):
        for side in (self.us, self.them):
            for k, v in self.faults.stale.get(side, {}).items():
                self.cores[side].store[k] = v
        env = {
            "FED_OURS_REST": self._serve(self._handler(self._rows(self.us), lambda k, v: self.put(self.us, k, v), self.us)),
            "FED_THEIRS_REST": self._serve(self._handler(self._rows(self.them), lambda k, v: self.put(self.them, k, v),
                                                         self.them)),
            "FED_EDGE_LOG_OURS": "cat '%s'" % self.trace_path,
            "FED_PROBE_FORGED_ZID": self.zids["forged"],
            "FED_PROBE_IMPERSONATE_ZID": self.zids["impersonate"],
        }
        for which, var in (("agreed", "FED_PROBE_THEIRS_REST"), ("forged", "FED_PROBE_FORGED_REST"),
                           ("impersonate", "FED_PROBE_IMPERSONATE_REST")):
            env[var] = self._serve(self._handler(lambda e: [], lambda k, v, w=which: self.probe_put(w, k, v)))
        env.update(self._spire())
        return env

    def stop(self):
        for s in self.servers:
            s.shutdown()
            s.server_close()

    # --- spire-server stand-ins --------------------------------------------------
    def _spire(self):
        d = os.path.join(self.tmp, "spire")
        os.makedirs(d, exist_ok=True)

        def ossl(*args):
            subprocess.run(["openssl"] + list(args), check=True, capture_output=True)

        for side in (self.us, self.them):
            ossl("req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
                 "-keyout", os.path.join(d, side + "-ca.key"), "-out", os.path.join(d, side + "-ca.pem"),
                 "-days", "2", "-subj", "/CN=%s spire ca" % side,
                 "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign")
        td = {p["sovereign_id"]: p["trust_domain"] for p in self.ag["parties"]}
        ext = os.path.join(d, "leaf.ext")
        with open(ext, "w") as f:
            f.write("subjectAltName=URI:%s/federation-acceptance\n" % td[self.them])
        ossl("req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
             "-keyout", os.path.join(d, "leaf.key"), "-out", os.path.join(d, "leaf.csr"), "-subj", "/CN=leaf")
        ossl("x509", "-req", "-in", os.path.join(d, "leaf.csr"), "-CA", os.path.join(d, self.them + "-ca.pem"),
             "-CAkey", os.path.join(d, self.them + "-ca.key"), "-CAcreateserial", "-days", "1",
             "-extfile", ext, "-out", os.path.join(d, "leaf.pem"))
        env = {}
        for side, var in ((self.us, "FED_SPIRE_OURS"), (self.them, "FED_SPIRE_THEIRS")):
            script = os.path.join(d, "spire-" + side)
            other = self.other(side)
            with open(script, "w") as f:
                f.write("""#!/bin/sh
case "$1 $2" in
  "federation refresh") exit 0 ;;
  "bundle list") cat '%(d)s/%(other)s-ca.pem' ;;
  "bundle show") cat '%(d)s/%(side)s-ca.pem' ;;
  "x509 mint") cat '%(d)s/leaf.pem' '%(d)s/leaf.key' ;;
  *) echo "fake spire-server: $*" >&2; exit 1 ;;
esac
""" % {"d": d, "other": other, "side": side})
            os.chmod(script, 0o700)
            env[var] = script
        return env


def agreement(us="xaa", them="xab", ours=("release/all/**", "release/%(them)s/**"),
              theirs=("release/all/**", "release/%(us)s/**")):
    """A two-party agreement in the shape `goat federate compose` writes."""
    names = {"us": us, "them": them}

    def flow(frm, to, release):
        return {"from": frm, "to": to, "publish_under": "from/%s/**" % frm,
                "release": [r % names for r in release],
                "envelope": {"hz": 100, "bytes_per_message": 1048576, "distinct_keys": 5000}}

    flows = []
    if ours:
        flows.append(flow(us, them, ours))
    if theirs:
        flows.append(flow(them, us, theirs))
    return {
        "spec": "federation-agreement/v1",
        "expires": "2026-12-01",
        "flows": flows,
        "parties": [
            {"sovereign_id": s, "trust_domain": "spiffe://%s.example" % s, "peering_cn": "fed.%s.01" % s,
             "ca_fingerprint": "sha256:" + "0" * 64, "fabric_fips_image": True}
            for s in (us, them)
        ],
        "transport": {"kind": "wireguard", "link_subnet": "100.127.250.0/31"},
    }
